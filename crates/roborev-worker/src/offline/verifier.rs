//! Mandatory observations of the live gated helper. Manifest claims and backend
//! output cannot mint admission. These checks precede the controller's GO frame.
use super::{OfflineSpec, control_command, file_digest};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::{
        fd::AsFd,
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryEvidence {
    pub pid: i32,
    pub start_ticks: u64,
    pub invocation_id: String,
    pub cgroup: String,
    pub no_new_privileges: bool,
    pub seccomp_filters: u64,
    pub capabilities: [u64; 5],
    pub namespaces: BTreeMap<String, u64>,
    pub mounts: BTreeMap<String, String>,
    pub scratch_bytes: u64,
    pub scratch_inodes: u64,
}

pub(super) fn unit_properties(spec: &OfflineSpec, name: &str) -> Result<BTreeMap<String, String>> {
    let output = super::process::output(control_command(&spec.systemctl).args(["--user", "show", name,
        "--property=LoadState,ActiveState,MainPID,InvocationID,ControlGroup,KillMode,Delegate,MemoryMax,MemorySwapMax,TasksMax,CPUQuotaPerSecUSec,NoNewPrivileges"])
        )?;
    ensure!(
        output.stdout.len() <= 65_536,
        "oversized systemd worker properties"
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let mut properties = BTreeMap::new();
    for line in text.lines() {
        let (name, value) = line
            .split_once('=')
            .context("invalid systemd worker property")?;
        ensure!(
            properties
                .insert(name.to_owned(), value.to_owned())
                .is_none(),
            "duplicate systemd worker property"
        );
    }
    ensure!(
        output.status.success()
            || properties
                .get("LoadState")
                .is_some_and(|s| s == "not-found"),
        "systemd worker inspection failed"
    );
    Ok(properties)
}

fn required<'a>(properties: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    properties
        .get(key)
        .map(String::as_str)
        .context(format!("missing worker property {key}"))
}

fn start_ticks(stat: &str) -> Result<u64> {
    stat.rsplit_once(')')
        .context("invalid worker process identity")?
        .1
        .split_whitespace()
        .nth(19)
        .context("missing worker process start time")?
        .parse()
        .context("invalid worker process start time")
}

fn status_values(status: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for line in status.lines() {
        if let Some((name, value)) = line.split_once(':') {
            ensure!(
                values
                    .insert(name.to_owned(), value.trim().to_owned())
                    .is_none(),
                "duplicate process status value"
            );
        }
    }
    Ok(values)
}

fn mount_path(value: &str) -> Result<String> {
    let mut result = Vec::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            ensure!(
                i + 3 < bytes.len()
                    && bytes[i + 1..i + 4]
                        .iter()
                        .all(|b| (b'0'..=b'7').contains(b)),
                "invalid mountinfo escape"
            );
            let value = (bytes[i + 1] - b'0') as u16 * 64
                + (bytes[i + 2] - b'0') as u16 * 8
                + (bytes[i + 3] - b'0') as u16;
            ensure!(value <= 255 && value != 0, "invalid mountinfo path byte");
            result.push(value as u8);
            i += 4;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(result).context("non-UTF8 mountinfo path")
}

fn same_inode(left: &Path, right: &Path) -> Result<bool> {
    let left = fs::metadata(left)?;
    let right = fs::metadata(right)?;
    Ok((left.dev(), left.ino()) == (right.dev(), right.ino()))
}

fn mounts(text: &str) -> Result<BTreeMap<String, (String, BTreeSet<String>)>> {
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let (before, after) = line.split_once(" - ").context("invalid mountinfo record")?;
        let fields: Vec<_> = before.split_whitespace().collect();
        let rest: Vec<_> = after.split_whitespace().collect();
        ensure!(
            fields.len() >= 6 && rest.len() == 3,
            "incomplete mountinfo record"
        );
        let point = mount_path(fields[4])?;
        let options = fields[5].split(',').map(str::to_owned).collect();
        ensure!(
            result
                .insert(point, (rest[0].to_owned(), options))
                .is_none(),
            "duplicate mountpoint hides worker authority"
        );
    }
    Ok(result)
}

pub(super) fn observe(
    spec: &OfflineSpec,
    unit: &str,
    stream: &UnixStream,
    control: &Path,
) -> Result<BoundaryEvidence> {
    let credentials =
        nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)?;
    ensure!(
        credentials.uid() == nix::unistd::geteuid().as_raw() && credentials.pid() > 0,
        "unauthenticated worker control peer"
    );
    let pid =
        rustix::process::Pid::from_raw(credentials.pid()).context("invalid worker process ID")?;
    let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .context("cannot pin worker process identity")?;
    let proc = PathBuf::from(format!("/proc/{}", credentials.pid()));
    let start = start_ticks(&fs::read_to_string(proc.join("stat"))?)?;
    let properties = unit_properties(spec, unit)?;
    let invocation = required(&properties, "InvocationID")?.to_owned();
    ensure!(
        super::super::is_hex(&invocation, 32)
            && required(&properties, "ActiveState")? == "active"
            && required(&properties, "KillMode")? == "control-group"
            && required(&properties, "Delegate")? == "no"
            && required(&properties, "NoNewPrivileges")? == "yes",
        "worker unit lacks required identity or lifetime controls"
    );
    for (name, expected) in [
        ("MemoryMax", spec.limits.memory_bytes),
        ("MemorySwapMax", 0),
        ("TasksMax", spec.limits.tasks),
    ] {
        ensure!(
            required(&properties, name)?.parse::<u64>()? == expected,
            "worker cgroup limit changed: {name}"
        );
    }
    let group = required(&properties, "ControlGroup")?.to_owned();
    ensure!(
        group.starts_with("/user.slice/")
            && group.ends_with(unit)
            && !group.split('/').any(|p| p == ".."),
        "unexpected worker cgroup path"
    );
    ensure!(
        fs::read_to_string(proc.join("cgroup"))?
            .lines()
            .eq([format!("0::{group}")]),
        "worker peer is outside the exact service cgroup"
    );
    let cgroup = Path::new("/sys/fs/cgroup").join(group.trim_start_matches('/'));
    ensure!(
        fs::read_to_string(cgroup.join("cgroup.subtree_control"))?
            .trim()
            .is_empty(),
        "worker cgroup is delegated"
    );
    for (name, expected) in [
        ("memory.max", spec.limits.memory_bytes),
        ("memory.swap.max", 0),
        ("pids.max", spec.limits.tasks),
    ] {
        ensure!(
            fs::read_to_string(cgroup.join(name))?
                .trim()
                .parse::<u64>()?
                == expected,
            "kernel worker cgroup limit differs: {name}"
        );
    }
    let cpu = fs::read_to_string(cgroup.join("cpu.max"))?;
    let values: Vec<_> = cpu.split_whitespace().collect();
    ensure!(values.len() == 2, "invalid worker CPU quota");
    let quota: u64 = values[0].parse()?;
    let period: u64 = values[1].parse()?;
    ensure!(
        period > 0
            && quota > 0
            && quota as u128 * 100 == period as u128 * spec.limits.cpu_percent as u128,
        "kernel worker CPU quota differs"
    );
    let status = status_values(&fs::read_to_string(proc.join("status"))?)?;
    let no_new_privileges = required(&status, "NoNewPrivs")? == "1";
    let seccomp_filters: u64 = required(&status, "Seccomp_filters")?.parse()?;
    ensure!(
        required(&status, "Seccomp")? == "2" && seccomp_filters > 0,
        "worker syscall confinement is missing"
    );
    let mut capabilities = [0; 5];
    for (index, key) in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"]
        .iter()
        .enumerate()
    {
        capabilities[index] = u64::from_str_radix(required(&status, key)?, 16)?;
    }
    ensure!(
        no_new_privileges && capabilities == [0; 5],
        "worker retains privilege or executable capability authority"
    );
    let mut namespaces = BTreeMap::new();
    for name in super::NAMESPACE_NAMES {
        let inode = fs::metadata(proc.join("ns").join(name))?.ino();
        ensure!(
            inode != fs::metadata(format!("/proc/self/ns/{name}"))?.ino(),
            "worker namespace is shared: {name}"
        );
        namespaces.insert(name.to_owned(), inode);
    }
    let root = proc.join("root");
    ensure!(
        file_digest(&root.join("control/helper"))? == spec.helper.sha256
            && same_inode(&root.join("control/helper"), &control.join("helper"))?,
        "worker helper image differs from admitted copy"
    );
    ensure!(
        same_inode(&proc.join("exe"), &control.join("helper"))?,
        "control peer is not the gated helper"
    );
    ensure!(
        same_inode(&root.join("repo"), &spec.checkout)?,
        "worker checkout mount changed"
    );
    let observed = mounts(&fs::read_to_string(proc.join("mountinfo"))?)?;
    let mut allowed = BTreeSet::from(
        [
            "/",
            "/proc",
            "/proc/sys",
            "/proc/irq",
            "/proc/bus",
            "/proc/sysrq-trigger",
            "/dev/null",
            "/dev/zero",
            "/dev/full",
            "/dev/random",
            "/dev/urandom",
            "/work",
            "/repo",
            "/control",
        ]
        .map(str::to_owned),
    );
    allowed.extend(
        spec.store_paths
            .iter()
            .map(|path| path.display().to_string()),
    );
    let mut evidence_mounts = BTreeMap::new();
    for (point, (kind, options)) in &observed {
        ensure!(
            allowed.contains(point),
            "unexpected worker mount authority: {point}"
        );
        if point != "/proc" && point != "/work" && !point.starts_with("/dev/") {
            ensure!(options.contains("ro"), "worker mount is writable: {point}");
        }
        evidence_mounts.insert(
            point.clone(),
            format!(
                "{kind}:{}",
                options.iter().cloned().collect::<Vec<_>>().join(",")
            ),
        );
    }
    for point in ["/", "/repo", "/control", "/work", "/proc"] {
        ensure!(
            observed.contains_key(point),
            "required worker mount missing: {point}"
        );
    }
    for path in &spec.store_paths {
        ensure!(
            observed.contains_key(path.to_str().context("non-UTF8 runtime closure path")?)
                && same_inode(&root.join(path.strip_prefix("/")?), path)?,
            "immutable runtime mount differs from approved closure"
        );
    }
    ensure!(
        observed
            .get("/proc")
            .is_some_and(|(kind, _)| kind == "proc")
            && observed
                .get("/work")
                .is_some_and(|(kind, _)| kind == "tmpfs"),
        "worker process or scratch mount type differs"
    );
    for point in ["sys", "run", "home", "data"] {
        ensure!(
            !root.join(point).exists(),
            "protected host root is visible: {point}"
        );
    }
    let scratch = root.join("work");
    let stat = nix::sys::statvfs::statvfs(&scratch).context("cannot observe scratch capacity")?;
    let scratch_bytes = stat
        .blocks()
        .checked_mul(stat.fragment_size())
        .context("scratch capacity overflow")?;
    let scratch_inodes = stat.files();
    ensure!(
        scratch_bytes <= spec.limits.scratch_bytes
            && scratch_inodes <= spec.limits.scratch_inodes
            && scratch_bytes > 0
            && scratch_inodes > 0,
        "scratch capacity exceeds worker aggregate budget"
    );
    let (_, options) = observed.get("/work").context("missing scratch mount")?;
    ensure!(
        options.contains("nosuid") && options.contains("nodev") && options.contains("noexec"),
        "scratch retains executable or device authority"
    );
    for entry in fs::read_dir(proc.join("fd"))? {
        let entry = entry?;
        let target = fs::read_link(entry.path())?;
        let text = target.to_string_lossy();
        ensure!(
            text == "/dev/null" || text.starts_with("pipe:[") || text.starts_with("socket:["),
            "worker inherited an unexpected descriptor"
        );
    }
    ensure!(
        start_ticks(&fs::read_to_string(proc.join("stat"))?)? == start,
        "worker process identity changed during verification"
    );
    let mut poll = [nix::poll::PollFd::new(
        pidfd.as_fd(),
        nix::poll::PollFlags::POLLIN,
    )];
    ensure!(
        nix::poll::poll(&mut poll, nix::poll::PollTimeout::ZERO)? == 0,
        "worker exited during mandatory verification"
    );
    Ok(BoundaryEvidence {
        pid: credentials.pid(),
        start_ticks: start,
        invocation_id: invocation,
        cgroup: group,
        no_new_privileges,
        seccomp_filters,
        capabilities,
        namespaces,
        mounts: evidence_mounts,
        scratch_bytes,
        scratch_inodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_identity_parser_handles_parentheses_in_comm() {
        let stat = "42 (a ) b) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 1234 20";
        assert_eq!(start_ticks(stat).unwrap(), 1234);
        assert!(start_ticks("42 (incomplete)").is_err());
    }
    #[test]
    fn ambiguous_mounts_and_invalid_escapes_cannot_qualify() {
        let line = "1 0 0:1 / /repo ro - tmpfs tmpfs rw";
        assert!(mounts(&format!("{line}\n{line}\n")).is_err());
        assert!(mount_path("/repo\\999").is_err());
        assert_eq!(mount_path("/a\\040b").unwrap(), "/a b");
    }
}
