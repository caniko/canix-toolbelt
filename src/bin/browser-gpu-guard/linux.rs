//! Polling mitigation for browser DMA-BUF leaks and host GPU memory pressure.
//! A rapid leak or unreadable kernel accounting can outrun the guard.

use clap::Parser;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

const GIB: u64 = 1 << 20; // Kernel memory counters are in KiB.
const BROWSER_SLICE: &str = "app-browser.slice";
const CGROUPFS: &str = "/sys/fs/cgroup";
const GRACE: Duration = Duration::from_secs(5);
const BROWSERS: &[&str] = &[
    "floorp",
    "rdd process",
    "isolated web co",
    "webextensions",
    "firefox",
    "chromium",
    "chrome",
    "zoom",
    "slack",
    "element",
    "signal",
    "obsidian",
    "grayjay",
];
const PROTECTED: &[&str] = &[
    "cosmic-comp",
    "cosmic-panel",
    "cosmic-session",
    "pipewire",
    "wireplumber",
    "obs",
    "obsproject",
    "blender",
    "freecad",
    "comfyui",
    "llama",
    "immich",
    "t3code",
    "t3-",
    "opencode",
    "steam",
    "steamwebhelper",
    "gamescope",
    "xdg-desktop-portal",
    "xdg-desktop-por",
];

#[derive(Parser)]
#[command(about = "Mitigate browser DMA-BUF leaks and host GPU memory pressure")]
struct BrowserGpuGuardCmd {
    /// Polling interval in seconds
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
    interval_secs: u64,
    /// Sample once and exit
    #[arg(long)]
    once: bool,
    /// Report proposed containment without stopping anything
    #[arg(long)]
    dry_run: bool,
    /// Minimum FD count before checking symlink targets during normal polls
    #[arg(long, default_value_t = 4096)]
    fd_prefilter: usize,
    /// Per-process DMA-BUF FD limit
    #[arg(long, default_value_t = 4096)]
    dmabuf_limit: usize,
    /// Per-process total FD limit for the combined check
    #[arg(long, default_value_t = 16384)]
    total_fd_limit: usize,
    /// DMA-BUF FD limit when the total FD limit is reached
    #[arg(long, default_value_t = 2048)]
    dmabuf_with_total_limit: usize,
    /// Consecutive breached polls needed to intervene
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
    sustained_samples: u32,
    /// Minimum time between interventions on the same target
    #[arg(long, default_value_t = 60)]
    cooldown_secs: u64,
}

pub(super) fn run() -> std::process::ExitCode {
    match BrowserGpuGuardCmd::parse().run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("browser-gpu-guard: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

impl BrowserGpuGuardCmd {
    fn run(self) -> io::Result<()> {
        let uid = nix::unistd::geteuid().as_raw();
        if uid == 0 && !self.dry_run {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "run browser-gpu-guard as the desktop user",
            ));
        }
        let procfs = Path::new("/proc");
        let mut stage1 = BTreeMap::new();
        let mut stage2 = 0;
        let mut cooldown = BTreeMap::new();
        loop {
            let started = Instant::now();
            let host = HostSample::read(procfs);
            let pressure = sustained(&mut stage2, host.breach(), self.sustained_samples);
            let inventory = processes(procfs, uid)?;
            let groups = self.sample(procfs, &inventory, pressure, uid);

            cooldown.retain(|_, last: &mut Instant| {
                last.elapsed() < Duration::from_secs(self.cooldown_secs)
            });
            stage1.retain(|target, _| groups.contains_key(target));
            for (target, group) in &groups {
                sustained(
                    stage1.entry(target.clone()).or_default(),
                    group.fd_breach,
                    self.sustained_samples,
                );
            }

            if self.once {
                eprintln!(
                    "browser-gpu-guard sample tracked_targets={} stage2_breach={} {}",
                    groups.len(),
                    host.breach(),
                    host.counters(),
                );
            }

            let victim = pressure
                .then(|| {
                    groups
                        .iter()
                        .filter(|(target, _)| !cooldown.contains_key(*target))
                        .max_by_key(|(_, group)| group.risk())
                })
                .flatten();
            if let Some((target, group)) = victim {
                self.act("stage2-gpu-memory-pressure", &host, target, group, uid);
                cooldown.insert(target.clone(), Instant::now());
                stage1.remove(target);
                // Require fresh evidence that stopping this target did not relieve pressure.
                stage2 = 0;
            } else {
                for (target, group) in groups
                    .iter()
                    .filter(|(target, _)| stage1[*target] >= self.sustained_samples)
                {
                    if !cooldown.contains_key(target) {
                        self.act("stage1-dmabuf-fd-leak", &host, target, group, uid);
                        cooldown.insert(target.clone(), Instant::now());
                    }
                }
            }
            if self.once {
                return Ok(());
            }
            thread::sleep(
                Duration::from_secs(self.interval_secs).saturating_sub(started.elapsed()),
            );
        }
    }

    fn fd_breach(&self, total: usize, dmabuf: usize) -> bool {
        dmabuf >= self.dmabuf_limit
            || (total >= self.total_fd_limit && dmabuf >= self.dmabuf_with_total_limit)
    }

    fn sample(
        &self,
        procfs: &Path,
        inventory: &BTreeMap<i32, Proc>,
        pressure: bool,
        uid: u32,
    ) -> BTreeMap<Target, Group> {
        let mut groups = BTreeMap::<_, Group>::new();
        let mut isolated = BTreeMap::new();
        for process in inventory
            .values()
            .filter(|p| p.tracked(inventory.get(&p.parent)))
        {
            let dir = procfs.join(process.id.pid.to_string()).join("fd");
            let total = fs::read_dir(&dir)
                .ok()
                .map(|entries| entries.flatten().count());
            let dmabuf = match total {
                Some(total) if pressure || total >= self.fd_prefilter => count_dmabufs(&dir),
                _ => None,
            };
            let isolated = *isolated.entry(process.cgroup.as_str()).or_insert_with(|| {
                user_unit(&process.cgroup, uid).is_some()
                    && exclusive_unit(Path::new(CGROUPFS), &process.cgroup, |pid| {
                        inventory.get(&pid).is_some_and(|member| {
                            member.cgroup == process.cgroup
                                && member.tracked(inventory.get(&member.parent))
                        })
                    })
                    .is_some()
            });
            let group = groups
                .entry(process.target(inventory, isolated))
                .or_default();
            group.members.push(process.id);
            group.total += total.unwrap_or(0);
            group.dmabuf += dmabuf.unwrap_or(0);
            group.rss_swap += process.rss_swap;
            group.fd_breach |= total
                .zip(dmabuf)
                .is_some_and(|(total, dmabuf)| self.fd_breach(total, dmabuf));
        }
        groups
    }

    fn act(&self, stage: &str, host: &HostSample, target: &Target, group: &Group, uid: u32) {
        let action = if self.dry_run {
            "[dry-run] would stop"
        } else {
            "stop"
        };
        let pids: Vec<_> = group.members.iter().map(|id| id.pid).collect();
        eprintln!(
            "browser-gpu-guard {action} stage={stage} target={target:?} pids={pids:?} dmabuf_fds={} total_fds={} {}",
            group.dmabuf,
            group.total,
            host.counters(),
        );
        if !self.dry_run {
            contain(target, &group.members, uid);
        }
    }
}

fn sustained(samples: &mut u32, breach: bool, limit: u32) -> bool {
    *samples = if breach { samples.saturating_add(1) } else { 0 };
    *samples >= limit
}

#[derive(Default)]
struct Group {
    members: Vec<ProcId>,
    dmabuf: usize,
    total: usize,
    rss_swap: u64,
    fd_breach: bool,
}

impl Group {
    fn risk(&self) -> (usize, usize, u64) {
        (self.dmabuf, self.total, self.rss_swap)
    }
}

fn count_dmabufs(dir: &Path) -> Option<usize> {
    Some(
        fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|entry| fs::read_link(entry.path()).ok())
            .filter(|path| path.to_string_lossy().contains("dmabuf"))
            .count(),
    )
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key == name).then_some(value)
    })
}

fn kib(text: &str, name: &str) -> Option<u64> {
    let mut parts = field(text, name)?.split_whitespace();
    let value = parts.next()?.parse().ok()?;
    (parts.next() == Some("kB")).then_some(value)
}

struct Meminfo {
    total: u64,
    available: u64,
    shmem: u64,
    swap_total: u64,
    swap_free: u64,
}

impl Meminfo {
    fn parse(text: &str) -> Option<Self> {
        let total = kib(text, "MemTotal")?;
        if total == 0 {
            return None;
        }
        Some(Self {
            total,
            available: kib(text, "MemAvailable")?,
            shmem: kib(text, "Shmem")?,
            swap_total: kib(text, "SwapTotal")?,
            swap_free: kib(text, "SwapFree")?,
        })
    }
}

fn full_avg10(text: &str) -> Option<f64> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next()? == "full").then_some(parts)
        })
        .flatten()
        .find_map(|word| word.strip_prefix("avg10="))?
        .parse::<f64>()
        .ok()
        .filter(|avg| (0.0..=100.0).contains(avg))
}

fn max_gtt_ratio(drm: &Path) -> Option<f64> {
    fs::read_dir(drm)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            name.to_str()?.strip_prefix("card")?.parse::<u32>().ok()?;
            let device = entry.path().join("device");
            let read = |name| {
                fs::read_to_string(device.join(name))
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()
            };
            let total = read("mem_info_gtt_total")?;
            if total == 0 {
                return None;
            }
            Some(read("mem_info_gtt_used")? as f64 / total as f64)
        })
        .reduce(f64::max)
}

struct HostSample {
    mem: Option<Meminfo>,
    psi: Option<f64>,
    gtt: Option<f64>,
}

impl HostSample {
    fn read(procfs: &Path) -> Self {
        Self {
            mem: fs::read_to_string(procfs.join("meminfo"))
                .ok()
                .and_then(|text| Meminfo::parse(&text)),
            psi: fs::read_to_string(procfs.join("pressure/memory"))
                .ok()
                .and_then(|text| full_avg10(&text)),
            gtt: max_gtt_ratio(Path::new("/sys/class/drm")),
        }
    }

    fn breach(&self) -> bool {
        let Some(mem) = &self.mem else { return false };
        let low_ram = mem.available < (mem.total / 10).clamp(GIB, 4 * GIB);
        let low_swap = mem.swap_total > 0
            && mem.swap_free < (mem.swap_total.saturating_mul(15) / 100).max(GIB);
        let stalled = self.psi.is_some_and(|psi| psi >= 25.0);
        let anomaly = self.gtt.is_some_and(|gtt| gtt >= 0.70) || mem.shmem >= mem.total.div_ceil(4);
        low_ram && (low_swap || stalled) && anomaly
    }

    fn counters(&self) -> String {
        let mem = self.mem.as_ref();
        let kib = |value: Option<u64>| value.map_or_else(|| "unknown".into(), |n| n.to_string());
        let gtt = self
            .gtt
            .map_or_else(|| "unknown".into(), |ratio| format!("{:.1}", ratio * 100.0));
        let psi = self
            .psi
            .map_or_else(|| "unknown".into(), |percent| format!("{percent:.1}"));
        format!(
            "mem_available_kib={} swap_free_kib={} shmem_kib={} gtt_pct={gtt} psi_full_avg10_pct={psi}",
            kib(mem.map(|m| m.available)),
            kib(mem.map(|m| m.swap_free)),
            kib(mem.map(|m| m.shmem)),
        )
    }
}

fn browser_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let name = lower.trim_start_matches('.');
    BROWSERS.iter().any(|family| {
        name.strip_prefix(family)
            .is_some_and(|tail| !tail.starts_with(|c: char| c.is_ascii_alphanumeric()))
    })
}

fn protected_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase().replace("\\x2d", "-");
    PROTECTED.iter().any(|name| {
        lower.match_indices(name).any(|(start, _)| {
            let end = start + name.len();
            (start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric())
                && (*name == "t3-"
                    || !lower
                        .as_bytes()
                        .get(end)
                        .is_some_and(u8::is_ascii_alphanumeric))
        })
    })
}

fn browser_slice(cgroup: &str) -> bool {
    cgroup.split('/').any(|part| part == BROWSER_SLICE)
}

fn leaf_unit(cgroup: &str) -> Option<&str> {
    let unit = cgroup.rsplit('/').next()?;
    let stem = unit
        .strip_suffix(".scope")
        .or_else(|| unit.strip_suffix(".service"))?;
    // No systemctl patterns, option names, login scopes, or user manager units.
    (!stem.is_empty()
        && !unit.starts_with('-')
        && !unit.starts_with("session-")
        && !unit.starts_with("user@")
        && unit
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":_.@\\-".contains(&b)))
    .then_some(unit)
}

#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "systemd unit type suffixes are case-sensitive, not file extensions"
)]
fn user_unit(cgroup: &str, uid: u32) -> Option<&str> {
    let rest = cgroup.strip_prefix(&format!("/user.slice/user-{uid}.slice/user@{uid}.service/"))?;
    let unit = leaf_unit(rest)?;
    if rest
        .rsplit_once('/')
        .is_some_and(|(slices, _)| !slices.split('/').all(|part| part.ends_with(".slice")))
    {
        return None;
    }
    // Don't stop general-purpose services such as D-Bus or session helpers.
    (browser_slice(rest)
        || (unit.ends_with(".scope") && (unit.starts_with("app-") || unit.starts_with("run-")))
        || browser_name(unit))
    .then_some(unit)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProcId {
    pid: i32,
    start: u64,
}

struct Proc {
    id: ProcId,
    parent: i32,
    comm: String,
    exe: String,
    cgroup: String,
    rss_swap: u64,
}

impl Proc {
    fn read(procfs: &Path, pid: i32, uid: u32) -> Option<Self> {
        let dir = procfs.join(pid.to_string());
        let read = |name| fs::read_to_string(dir.join(name)).ok();
        let stat = read("stat")?;
        let mut stat = stat.rsplit_once(") ")?.1.split_whitespace();
        if matches!(stat.next()?, "Z" | "X") {
            return None;
        }
        let parent = stat.next()?.parse().ok()?;
        let start_ticks = stat.nth(17)?.parse().ok()?;
        let status = read("status")?;
        let mut ids = field(&status, "Uid")?.split_whitespace();
        if ids.next()?.parse::<u32>().ok()? != uid || ids.next()?.parse::<u32>().ok()? != uid {
            return None;
        }
        let cgroups = read("cgroup")?;
        let cgroup = cgroups.lines().find_map(|line| line.strip_prefix("0::"))?;
        // `comm` can contain embedded newlines; retain them for matching.
        let comm = read("comm")?.trim_end_matches('\n').to_owned();
        let exe = fs::read_link(dir.join("exe"))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(Self {
            id: ProcId {
                pid,
                start: start_ticks,
            },
            parent,
            comm,
            exe,
            cgroup: cgroup.into(),
            rss_swap: kib(&status, "VmRSS")
                .unwrap_or(0)
                .saturating_add(kib(&status, "VmSwap").unwrap_or(0)),
        })
    }

    fn excluded(&self) -> bool {
        self.id.pid <= 1
            || u32::try_from(self.id.pid) == Ok(std::process::id())
            || [&self.comm, &self.exe, &self.cgroup]
                .iter()
                .any(|name| protected_name(name))
    }

    fn family_match(&self) -> bool {
        browser_name(&self.comm) || browser_name(self.exe.rsplit('/').next().unwrap_or_default())
    }

    fn tracked(&self, parent: Option<&Self>) -> bool {
        !self.excluded()
            && !parent.is_some_and(Self::excluded)
            && (browser_slice(&self.cgroup)
                || self.family_match()
                || parent.is_some_and(Self::family_match))
    }

    fn target(&self, procs: &BTreeMap<i32, Self>, isolated: bool) -> Target {
        if isolated {
            return Target::Unit(self.cgroup.clone());
        }
        let mut root = self;
        for _ in 0..procs.len() {
            let Some(parent) = procs.get(&root.parent) else {
                break;
            };
            // Discard inconsistent ancestry observed while the process table changed.
            if parent.id.start > root.id.start
                || parent.id == root.id
                || parent.cgroup != self.cgroup
                || !parent.tracked(procs.get(&parent.parent))
            {
                break;
            }
            root = parent;
        }
        Target::Tree {
            root: root.id,
            cgroup: self.cgroup.clone(),
        }
    }
}

fn processes(procfs: &Path, uid: u32) -> io::Result<BTreeMap<i32, Proc>> {
    Ok(fs::read_dir(procfs)?
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            Some((pid, Proc::read(procfs, pid, uid)?))
        })
        .collect())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Target {
    Unit(String),
    Tree { root: ProcId, cgroup: String },
}

impl Target {
    fn cgroup(&self) -> &str {
        match self {
            Self::Unit(cgroup) | Self::Tree { cgroup, .. } => cgroup,
        }
    }
}

/// Group an isolated unit for ranking only. A mixed or delegated unit is
/// ranked by browser process tree instead.
fn exclusive_unit(
    cgroupfs: &Path,
    cgroup: &str,
    mut eligible: impl FnMut(i32) -> bool,
) -> Option<Vec<i32>> {
    let dir = cgroupfs.join(cgroup.strip_prefix('/')?);
    for entry in fs::read_dir(&dir).ok()? {
        if entry.ok()?.file_type().ok()?.is_dir() {
            return None;
        }
    }
    let members = fs::read_to_string(dir.join("cgroup.procs"))
        .ok()?
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<Vec<i32>, _>>()
        .ok()?;
    (!members.is_empty() && members.iter().copied().all(&mut eligible)).then_some(members)
}

fn checked_process(procfs: &Path, pid: i32, uid: u32) -> Option<Proc> {
    let process = Proc::read(procfs, pid, uid)?;
    let parent = Proc::read(procfs, process.parent, uid);
    process.tracked(parent.as_ref()).then_some(process)
}

fn contain(target: &Target, members: &[ProcId], uid: u32) {
    let procfs = Path::new("/proc");
    let pins = members
        .iter()
        .filter_map(|&id| Pin::open(procfs, id, uid, target.cgroup()))
        .collect::<Vec<_>>();
    // Never stop the unit: its membership can change after validation, and
    // systemd can propagate stops to other units.
    signal_then_kill(procfs, &pins, uid);
}

struct Pin {
    process: Proc,
    fd: OwnedFd,
}

impl Pin {
    fn open(procfs: &Path, id: ProcId, uid: u32, cgroup: &str) -> Option<Self> {
        let fd = match pidfd_open(Pid::from_raw(id.pid)?, PidfdFlags::empty()) {
            Ok(fd) => fd,
            Err(error) => {
                if error != rustix::io::Errno::SRCH {
                    eprintln!("browser-gpu-guard pidfd_open pid={}: {error}", id.pid);
                }
                return None;
            }
        };
        let process = checked_process(procfs, id.pid, uid)?;
        (process.id == id && process.cgroup == cgroup).then_some(Self { process, fd })
    }

    fn eligible(&self, procfs: &Path, uid: u32) -> bool {
        let Some(current) = Proc::read(procfs, self.process.id.pid, uid) else {
            return false;
        };
        let parent = Proc::read(procfs, current.parent, uid);
        current.id == self.process.id
            && current.cgroup == self.process.cgroup
            && !current.excluded()
            && !parent.as_ref().is_some_and(Proc::excluded)
            && (current.tracked(parent.as_ref())
                // A helper classified through its parent can be orphaned by TERM.
                || (current.parent != self.process.parent
                    && current.comm == self.process.comm
                    && current.exe == self.process.exe))
    }

    fn signal(&self, procfs: &Path, uid: u32, signal: Signal) {
        if !self.eligible(procfs, uid) {
            return;
        }
        // A reused PID cannot receive a signal through this descriptor.
        if let Err(error) = pidfd_send_signal(&self.fd, signal) {
            if error != rustix::io::Errno::SRCH {
                eprintln!(
                    "browser-gpu-guard signal={signal:?} pid={}: {error}",
                    self.process.id.pid
                );
            }
        }
    }
}

fn wait(pins: &[Pin]) {
    use nix::poll::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    let mut pending: Vec<_> = pins
        .iter()
        .map(|pin| PollFd::new(pin.fd.as_fd(), PollFlags::POLLIN))
        .collect();
    let started = Instant::now();
    while !pending.is_empty() && started.elapsed() < GRACE {
        let remaining_ms =
            u16::try_from(GRACE.saturating_sub(started.elapsed()).as_millis()).unwrap_or(u16::MAX);
        match poll(&mut pending, remaining_ms) {
            Ok(_) => pending.retain(|fd| fd.revents().is_some_and(|events| events.is_empty())),
            Err(nix::errno::Errno::EINTR) => (),
            Err(error) => {
                eprintln!("browser-gpu-guard waiting on pidfds: {error}");
                thread::sleep(GRACE.saturating_sub(started.elapsed()));
                break;
            }
        }
    }
}

fn signal_then_kill(procfs: &Path, pins: &[Pin], uid: u32) {
    for pin in pins {
        pin.signal(procfs, uid, Signal::TERM);
    }
    wait(pins);
    for pin in pins {
        pin.signal(procfs, uid, Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Proc {
        fn fixture(pid: i32, parent: i32, comm: &str, cgroup: &str) -> Self {
            Self {
                id: ProcId {
                    pid,
                    start: u64::try_from(pid).unwrap(),
                },
                parent,
                comm: comm.into(),
                exe: String::new(),
                cgroup: cgroup.into(),
                rss_swap: 0,
            }
        }
    }

    #[test]
    fn parses_kernel_memory_counters_and_full_psi() {
        let mem = Meminfo::parse(
            "MemTotal: 8192000 kB\nMemFree: 20 kB\nMemAvailable: 101 kB\n\
             SwapTotal: 999 kB\nSwapFree: 201 kB\nShmem: 301 kB\n",
        )
        .unwrap();
        assert_eq!((mem.total, mem.available, mem.shmem), (8_192_000, 101, 301));
        assert_eq!((mem.swap_total, mem.swap_free), (999, 201));
        assert!(Meminfo::parse("MemTotal: 1024 kB\n").is_none());
        assert_eq!(
            full_avg10("some avg10=90.00 avg60=11.00\nfull avg10=25.50 avg60=0.00\n"),
            Some(25.5)
        );
        assert_eq!(full_avg10("some avg10=90.00\n"), None);
        assert_eq!(full_avg10("full avg10=NaN\n"), None);
    }

    #[test]
    fn measures_max_gtt_ratio_across_cards_that_report_both_counters() {
        let drm = tempfile::tempdir().unwrap();
        for (card, used, total) in [
            ("card0", "900", "1000"),
            ("card1", "995", "10000"),
            ("card2", "123", "0"),
            ("card3-DP-1", "1000", "1000"),
        ] {
            let device = drm.path().join(card).join("device");
            fs::create_dir_all(&device).unwrap();
            fs::write(device.join("mem_info_gtt_used"), used).unwrap();
            fs::write(device.join("mem_info_gtt_total"), total).unwrap();
        }
        fs::create_dir(drm.path().join("card4")).unwrap();
        assert_eq!(max_gtt_ratio(drm.path()), Some(0.9));
        assert_eq!(max_gtt_ratio(&drm.path().join("absent")), None);
    }

    #[test]
    fn tracks_browser_families_but_protected_apps_override_browser_slice() {
        for comm in [
            "Firefox",
            "Zoom",
            "floorp",
            "RDD Process",
            "Isolated Web Co",
            "Obsidian",
        ] {
            assert!(browser_name(comm), "{comm}");
            assert!(!protected_name(comm), "{comm}");
        }
        for comm in [
            "cosmic-comp",
            "obs",
            "blender",
            "t3code",
            "pipewire",
            "steam",
            "gamescope",
            "xdg-desktop-portal",
        ] {
            assert!(protected_name(comm), "{comm}");
            assert!(protected_name(&format!("app-{comm}-123.scope")), "{comm}");
            let process = Proc::fixture(50, 1, comm, "/app-browser.slice/run-test.scope");
            assert!(!process.tracked(None), "{comm}");
        }
        let mut process = Proc::fixture(50, 1, "helper", "/app-browser.slice/run-test.scope");
        assert!(process.tracked(None));
        process.cgroup = "/app.slice/app-comfyui.scope".into();
        process.exe = "electron".into();
        assert!(!process.tracked(None));
        process.cgroup = "/session-5.scope".into();
        assert!(!process.tracked(None), "unidentified Electron app");
        process.exe = "chromium".into();
        assert!(process.tracked(None));
        let parent = Proc::fixture(49, 1, "zoom.real", "/session-5.scope");
        process.exe.clear();
        assert!(process.tracked(Some(&parent)));
        assert!(!process.tracked(None));
    }

    #[test]
    fn extracts_only_leaf_units_not_sessions_or_slices() {
        for (path, expected) in [
            ("app-browser.slice/run-r123.scope", Some("run-r123.scope")),
            (
                "app-cosmic-floorp-123.scope",
                Some("app-cosmic-floorp-123.scope"),
            ),
            (
                "app-browser.slice/app-zoom.service",
                Some("app-zoom.service"),
            ),
            ("session-5.scope", None),
            ("app-browser.slice", None),
            ("/user@1000.service", None),
            ("/app-browser.slice/run.scope/delegated", None),
            ("/app.slice/evil*.scope", None),
        ] {
            assert_eq!(leaf_unit(path), expected, "{path}");
        }
    }

    #[test]
    fn stage1_latches_consecutive_per_target_breaches_and_resets() {
        let args = defaults();
        let mut latch = 0;
        assert!(!sustained(&mut latch, true, args.sustained_samples));
        assert!(!sustained(&mut latch, true, args.sustained_samples));
        assert!(!sustained(&mut latch, false, args.sustained_samples));
        assert!(!sustained(&mut latch, true, args.sustained_samples));
        assert!(!sustained(&mut latch, true, args.sustained_samples));
        assert!(sustained(&mut latch, true, args.sustained_samples));
        assert!(!args.fd_breach(16_383, 2_048));
        assert!(!args.fd_breach(16_384, 2_047));
        assert!(args.fd_breach(16_384, 2_048));
        assert!(args.fd_breach(4_096, 4_096));
    }

    #[test]
    fn stage2_thresholds_scale_across_hosts_with_optional_gpu_accounting() {
        for (total, low_ram) in [
            (64 * GIB, 4 * GIB),
            (16 * GIB, 16 * GIB / 10),
            (8 * GIB, GIB),
        ] {
            let mut host = HostSample {
                mem: Some(Meminfo {
                    total,
                    available: low_ram,
                    shmem: 0,
                    swap_total: 16 * GIB,
                    swap_free: 16 * GIB * 15 / 100 - 1,
                }),
                gtt: Some(0.70),
                psi: None,
            };
            assert!(!host.breach(), "RAM exactly at threshold");
            host.mem.as_mut().unwrap().available -= 1;
            assert!(host.breach(), "low RAM, low swap and high GTT");
            host.gtt = None;
            assert!(!host.breach(), "no GTT or shmem anomaly");
            host.mem.as_mut().unwrap().shmem = total / 4;
            assert!(host.breach(), "shmem can substitute for unavailable GTT");
            host.mem.as_mut().unwrap().swap_free += 1;
            assert!(!host.breach(), "swap exactly at threshold");
            host.mem.as_mut().unwrap().swap_total = 0;
            host.mem.as_mut().unwrap().swap_free = 0;
            assert!(!host.breach(), "swap disabled is not swap exhaustion");
            host.psi = Some(25.0);
            assert!(
                host.breach(),
                "full PSI can substitute for unavailable swap"
            );
        }
    }

    #[test]
    fn stage2_ranks_by_dmabuf_then_total_fds_then_rss_and_swap() {
        let group = |dmabuf, total, rss_swap| Group {
            dmabuf,
            total,
            rss_swap,
            ..Group::default()
        };
        let candidates = [
            group(10, 10, 1),
            group(9, 9000, 9000),
            group(10, 11, 0),
            group(10, 11, 1),
        ];
        let winner = candidates.iter().max_by_key(|group| group.risk()).unwrap();
        assert!(std::ptr::eq(winner, &raw const candidates[3]));
    }

    #[test]
    fn mixed_and_delegated_units_are_not_grouped_as_units() {
        let cgroupfs = tempfile::tempdir().unwrap();
        let scope = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-edge.scope";
        let dir = cgroupfs.path().join(&scope[1..]);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("cgroup.procs"), "50\n51\n").unwrap();
        assert_eq!(user_unit(scope, 1000), Some("app-edge.scope"));
        assert_eq!(user_unit(scope, 1001), None);
        assert!(user_unit(&scope.replace("app.slice", "arbitrary.service"), 1000).is_none());
        assert!(exclusive_unit(cgroupfs.path(), scope, |pid| pid == 50).is_none());
        assert!(exclusive_unit(cgroupfs.path(), scope, |_| true).is_some());
        fs::create_dir(dir.join("delegated")).unwrap();
        assert!(exclusive_unit(cgroupfs.path(), scope, |_| true).is_none());
    }

    #[test]
    fn stage2_reads_below_prefilter_and_pid_fallback_groups_only_tracked_members() {
        let procfs = tempfile::tempdir().unwrap();
        let cgroup = "/session-5.scope";
        let inventory = BTreeMap::from([
            (50, Proc::fixture(50, 1, "floorp", cgroup)),
            (51, Proc::fixture(51, 50, "RDD Process", cgroup)),
            (52, Proc::fixture(52, 50, "obs", cgroup)),
        ]);
        let fd_dir = procfs.path().join("51/fd");
        fs::create_dir_all(&fd_dir).unwrap();
        std::os::unix::fs::symlink("anon_inode:dmabuf", fd_dir.join("0")).unwrap();
        std::os::unix::fs::symlink("socket:[1234]", fd_dir.join("1")).unwrap();

        let mut args = defaults();
        args.dmabuf_limit = 1;
        let normal = args.sample(procfs.path(), &inventory, false, 1000);
        let pressure = args.sample(procfs.path(), &inventory, true, 1000);
        let target = inventory[&50].target(&inventory, false);
        assert_eq!(normal.len(), 1);
        assert_eq!(normal[&target].total, 2);
        assert_eq!(normal[&target].dmabuf, 0);
        assert!(!normal[&target].fd_breach);
        assert_eq!(pressure[&target].dmabuf, 1);
        assert!(pressure[&target].fd_breach);
        assert_eq!(
            pressure[&target].members,
            [inventory[&50].id, inventory[&51].id]
        );
    }

    #[test]
    fn proc_identity_requires_matching_real_and_effective_uid() {
        let procfs = tempfile::tempdir().unwrap();
        let dir = procfs.path().join("50");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("stat"),
            format!("50 (a ) b) S 26{} 12345 0\n", " 0".repeat(17)),
        )
        .unwrap();
        fs::write(dir.join("comm"), "Firefox\n").unwrap();
        fs::write(dir.join("cgroup"), "0::/session-5.scope\n").unwrap();
        fs::write(
            dir.join("status"),
            "Uid:\t1000\t1000\t1000\t1000\nVmRSS: 8 kB\nVmSwap: 3 kB\n",
        )
        .unwrap();
        let process = Proc::read(procfs.path(), 50, 1000).unwrap();
        assert_eq!(
            process.id,
            ProcId {
                pid: 50,
                start: 12345
            }
        );
        assert_eq!((process.parent, process.rss_swap), (26, 11));
        assert!(Proc::read(procfs.path(), 50, 1001).is_none());
        fs::write(dir.join("status"), "Uid:\t1000\t1001\t1001\t1001\n").unwrap();
        assert!(Proc::read(procfs.path(), 50, 1000).is_none());
    }

    fn defaults() -> BrowserGpuGuardCmd {
        BrowserGpuGuardCmd::parse_from(["guard"])
    }
}
