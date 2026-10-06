//! Offline execution envelope. The controller supplies an already-authenticated
//! one-shot reservation and frozen inputs. Every backend waits for the mandatory
//! live namespace/cgroup observer. Network access is always private; this module
//! has no production credential, broker, forge or daemon-adapter authority.

mod helper;
mod process;
mod seccomp;
mod verifier;

pub use helper::helper_main;
pub use verifier::BoundaryEvidence;

use super::{
    Limits,
    budget::Budget,
    execution::{ExecutionBinding, ReservedExecution},
    files, is_hex,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MAX_MANIFEST: usize = 2 * 1024 * 1024;
const NAMESPACE_NAMES: [&str; 7] = ["mnt", "pid", "user", "net", "ipc", "cgroup", "uts"];

pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_digest(path: &Path) -> Result<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= 1024 * 1024 * 1024,
        "invalid or oversized executable"
    );
    let mut digest = Sha256::new();
    let mut buffer = [0; 65_536];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Explicit content identity supplied by the controller. Capturing bytes does
/// not approve an executable; the integrating controller owns that selection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Executable {
    pub path: PathBuf,
    pub sha256: String,
}

impl Executable {
    pub fn capture(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let executable = Self {
            sha256: file_digest(&path)?,
            path,
        };
        executable.validate()?;
        Ok(executable)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.path.is_absolute() && is_hex(&self.sha256, 64),
            "invalid executable identity"
        );
        let meta = fs::symlink_metadata(&self.path)?;
        ensure!(
            meta.is_file() && meta.mode() & 0o111 != 0 && meta.mode() & 0o022 == 0,
            "executable must be regular, executable and not group/world writable"
        );
        ensure!(
            file_digest(&self.path)? == self.sha256,
            "executable content changed"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerLimits {
    pub wall_seconds: u64,
    pub memory_bytes: u64,
    pub tasks: u64,
    pub cpu_percent: u64,
    pub scratch_bytes: u64,
    pub scratch_inodes: u64,
    pub output_bytes: u64,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self {
            wall_seconds: 20,
            memory_bytes: 1024 * 1024 * 1024,
            tasks: 64,
            cpu_percent: 100,
            scratch_bytes: 64 * 1024 * 1024,
            scratch_inodes: 8192,
            output_bytes: 1024 * 1024,
        }
    }
}

impl WorkerLimits {
    fn scratch_capacity(&self) -> Result<u64> {
        let page = nix::unistd::sysconf(nix::unistd::SysconfVar::PAGE_SIZE)?
            .context("cannot determine worker scratch allocation granularity")?;
        ensure!(
            page > 0,
            "cannot determine worker scratch allocation granularity"
        );
        let page = page as u64;
        let capacity = self.scratch_bytes / page * page;
        ensure!(
            capacity > 0,
            "worker scratch budget is smaller than a kernel page"
        );
        Ok(capacity)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            (2..=180).contains(&self.wall_seconds)
                && (64 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&self.memory_bytes)
                && (8..=256).contains(&self.tasks)
                && (1..=200).contains(&self.cpu_percent)
                && (1024 * 1024..=512 * 1024 * 1024).contains(&self.scratch_bytes)
                && (64..=65536).contains(&self.scratch_inodes)
                && (1024..=4 * 1024 * 1024).contains(&self.output_bytes),
            "worker limits exceed the offline contract"
        );
        self.scratch_capacity()?;
        Ok(())
    }
}

/// Controller-only manifest: private homes and a private network are fixed, not
/// selectable by PR data or an environment switch. Arguments/files are trusted
/// controller inputs, never repository configuration. Production brokerage is
/// intentionally not a supported execution mode in this source slice.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineSpec {
    pub systemd_run: Executable,
    pub systemctl: Executable,
    pub bubblewrap: Executable,
    pub helper: Executable,
    pub backend: Executable,
    pub arguments: Vec<String>,
    pub store_paths: Vec<PathBuf>,
    pub checkout: PathBuf,
    pub checkout_sha256: String,
    pub trusted_files: BTreeMap<String, Vec<u8>>,
    pub prompt: Vec<u8>,
    pub limits: WorkerLimits,
}

fn snapshot(path: &Path) -> Result<String> {
    let limits = Limits::default();
    files::snapshot(path, &limits, &Budget::new(&limits)?, false)
}

impl OfflineSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        systemd_run: Executable,
        systemctl: Executable,
        bubblewrap: Executable,
        helper: Executable,
        backend: Executable,
        arguments: Vec<String>,
        store_paths: Vec<PathBuf>,
        checkout: PathBuf,
        trusted_files: BTreeMap<String, Vec<u8>>,
        prompt: Vec<u8>,
        limits: WorkerLimits,
    ) -> Result<Self> {
        let checkout_sha256 = snapshot(&checkout)?;
        let spec = Self {
            systemd_run,
            systemctl,
            bubblewrap,
            helper,
            backend,
            arguments,
            store_paths,
            checkout,
            checkout_sha256,
            trusted_files,
            prompt,
            limits,
        };
        spec.validate()?;
        Ok(spec)
    }
    fn validate(&self) -> Result<()> {
        self.limits.validate()?;
        for executable in [
            &self.systemd_run,
            &self.systemctl,
            &self.bubblewrap,
            &self.helper,
            &self.backend,
        ] {
            executable.validate()?;
        }
        ensure!(
            is_hex(&self.checkout_sha256, 64) && snapshot(&self.checkout)? == self.checkout_sha256,
            "offline checkout inputs changed"
        );
        ensure!(
            self.prompt.len() <= 1024 * 1024
                && self.arguments.len() <= 32
                && self
                    .arguments
                    .iter()
                    .all(|arg| arg.len() <= 4096 && !arg.contains('\0')),
            "oversized offline backend inputs"
        );
        ensure!(
            self.trusted_files.len() <= 32
                && self
                    .trusted_files
                    .iter()
                    .all(|(name, bytes)| !name.is_empty()
                        && name.len() <= 64
                        && !matches!(
                            name.as_str(),
                            "." | ".." | "helper" | "manifest.json" | "gate.sock"
                        )
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                        && bytes.len() <= 1024 * 1024),
            "invalid trusted input files"
        );
        ensure!(
            !self.store_paths.is_empty() && self.store_paths.len() <= 1024,
            "missing or oversized immutable runtime closure"
        );
        for path in &self.store_paths {
            let parts: Vec<_> = path.components().collect();
            ensure!(
                parts.len() == 4
                    && path.starts_with("/nix/store")
                    && parts
                        .iter()
                        .all(|p| matches!(p, Component::RootDir | Component::Normal(_)))
                    && fs::symlink_metadata(path)?.is_dir(),
                "runtime closure must name exact store directories"
            );
        }
        ensure!(
            self.store_paths
                .iter()
                .any(|root| self.backend.path.starts_with(root)),
            "backend absent from immutable runtime closure"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_MANIFEST,
            "oversized offline manifest"
        );
        Ok(())
    }
    pub fn input_digest(&self) -> Result<String> {
        Ok(hash(&serde_json::to_vec(&(
            &self.checkout_sha256,
            &self.trusted_files,
            hash(&self.prompt),
        ))?))
    }
    pub fn policy_digest(&self) -> Result<String> {
        Ok(hash(&serde_json::to_vec(&(
            1,
            "isolated-network-only",
            &self.limits,
        ))?))
    }
    pub fn backend_digest(&self) -> Result<String> {
        Ok(hash(&serde_json::to_vec(&(
            1,
            &self.systemd_run,
            &self.systemctl,
            &self.bubblewrap,
            &self.helper,
            &self.backend,
            &self.arguments,
            &self.store_paths,
        ))?))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub binding: ExecutionBinding,
    pub spec: OfflineSpec,
    pub host_namespaces: BTreeMap<String, u64>,
    pub host_uid: u32,
    pub host_gid: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineReceipt {
    pub scope: String,
    pub binding: ExecutionBinding,
    pub boundary: BoundaryEvidence,
    pub backend_success: bool,
    pub cgroup_empty: bool,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    File::open(path.parent().context("missing result parent")?)?.sync_all()?;
    Ok(())
}

pub(super) fn control_command(executable: &Executable) -> Command {
    let mut command = Command::new(&executable.path);
    command
        .env_clear()
        .env(
            "XDG_RUNTIME_DIR",
            format!("/run/user/{}", nix::unistd::geteuid().as_raw()),
        )
        .stdin(Stdio::null());
    command
}

fn drain(
    mut input: impl Read,
    maximum: u64,
    exceeded: Arc<AtomicBool>,
    count: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let size = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => size,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if count.fetch_add(size as u64, Ordering::AcqRel) + size as u64 > maximum {
            exceeded.store(true, Ordering::Release);
        }
        let remaining = maximum.saturating_sub(bytes.len() as u64) as usize;
        bytes.extend_from_slice(&buffer[..size.min(remaining)]);
        if stop.load(Ordering::Acquire) && exceeded.load(Ordering::Acquire) {
            break;
        }
    }
    Ok(bytes)
}

struct Unit<'a> {
    spec: &'a OfflineSpec,
    name: String,
    cgroup: Option<PathBuf>,
    empty: bool,
}

impl Unit<'_> {
    fn cleanup(&mut self) -> Result<()> {
        let output = process::output(
            control_command(&self.spec.systemctl).args(["--user", "stop", &self.name]),
        )?;
        if !output.status.success() {
            let state = verifier::unit_properties(self.spec, &self.name)?;
            ensure!(
                state.get("LoadState").is_some_and(|s| s == "not-found"),
                "offline unit cleanup failed; retain execution quarantine: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        if let Some(path) = &self.cgroup {
            loop {
                match fs::read_to_string(path.join("cgroup.events")) {
                    Ok(events) if events.lines().any(|line| line == "populated 0") => break,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Ok(_) => ensure!(
                        Instant::now() < deadline,
                        "worker cgroup is still populated"
                    ),
                    Err(error) => return Err(error.into()),
                }
                thread::sleep(Duration::from_millis(10));
            }
        } else {
            let state = verifier::unit_properties(self.spec, &self.name)?;
            ensure!(
                state
                    .get("ActiveState")
                    .is_some_and(|s| matches!(s.as_str(), "inactive" | "failed"))
                    && state.get("MainPID").is_some_and(|s| s == "0"),
                "unverified worker unit remains active"
            );
        }
        self.empty = true;
        let _ = process::output(control_command(&self.spec.systemctl).args([
            "--user",
            "reset-failed",
            &self.name,
        ]));
        Ok(())
    }
}

impl Drop for Unit<'_> {
    fn drop(&mut self) {
        if !self.empty {
            let _ = self.cleanup();
        }
    }
}

/// Consume an existing durable UNKNOWN reservation, start one private envelope,
/// observe it before backend exec, and retain complete bounded output only after
/// cleanup and input revalidation. Errors never authorize another reservation.
/// The integrating controller must authenticate its daemon adapter separately.
pub fn run_offline(
    reservation: ReservedExecution,
    spec: &OfflineSpec,
    result: &Path,
) -> Result<OfflineReceipt> {
    ensure!(
        !nix::unistd::geteuid().is_root(),
        "offline worker requires an unprivileged controller user manager"
    );
    spec.validate()?;
    let binding = reservation.binding();
    ensure!(
        binding.input_manifest_sha256 == spec.input_digest()?
            && binding.backend_manifest_sha256 == spec.backend_digest()?
            && binding.request.execution_policy_sha256 == spec.policy_digest()?,
        "worker execution binding changed"
    );
    let parent = result
        .parent()
        .context("worker result requires a private parent")?;
    files::open_directory(parent)?;
    let meta = fs::metadata(parent)?;
    ensure!(
        meta.uid() == nix::unistd::geteuid().as_raw() && meta.mode() & 0o077 == 0,
        "worker controller directory is not private"
    );
    fs::DirBuilder::new()
        .mode(0o700)
        .create(result)
        .context("worker result already exists; replay forbidden")?;
    let control = result.join("control");
    fs::DirBuilder::new().mode(0o700).create(&control)?;
    fs::copy(&spec.helper.path, control.join("helper"))?;
    fs::set_permissions(control.join("helper"), fs::Permissions::from_mode(0o500))?;
    ensure!(
        file_digest(&control.join("helper"))? == spec.helper.sha256,
        "helper copy changed"
    );
    let host_namespaces = NAMESPACE_NAMES
        .into_iter()
        .map(|name| {
            Ok((
                name.to_owned(),
                fs::metadata(format!("/proc/self/ns/{name}"))?.ino(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let manifest = Manifest {
        binding: binding.clone(),
        spec: spec.clone(),
        host_namespaces,
        host_uid: nix::unistd::geteuid().as_raw(),
        host_gid: nix::unistd::getegid().as_raw(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    ensure!(
        manifest_bytes.len() <= MAX_MANIFEST,
        "oversized bound manifest"
    );
    write_new(&control.join("manifest.json"), &manifest_bytes)?;
    for (name, bytes) in &spec.trusted_files {
        write_new(&control.join(name), bytes)?;
    }
    let socket = control.join("gate.sock");
    ensure!(
        socket.as_os_str().as_encoded_bytes().len() < 104,
        "worker controller socket path too long"
    );
    let listener = UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let name = format!("canix-rr-offline-{}.service", binding.execution_id);
    let existing = verifier::unit_properties(spec, &name)?;
    ensure!(
        existing.get("LoadState").is_some_and(|s| s == "not-found"),
        "execution unit already exists; replay forbidden"
    );
    let mut unit = Unit {
        spec,
        name,
        cgroup: None,
        empty: false,
    };
    write_new(
        &result.join("launch.json"),
        &serde_json::to_vec(
            &serde_json::json!({"binding": binding, "manifest_sha256": hash(&manifest_bytes), "unit": unit.name}),
        )?,
    )?;
    let mut command = control_command(&spec.systemd_run);
    command
        .args(["--user", "--wait", "--pipe", "--quiet"])
        .arg(format!("--unit={}", unit.name));
    for property in [
        format!("RuntimeMaxSec={}s", spec.limits.wall_seconds),
        format!("MemoryMax={}", spec.limits.memory_bytes),
        "MemorySwapMax=0".into(),
        format!("TasksMax={}", spec.limits.tasks),
        format!("CPUQuota={}%", spec.limits.cpu_percent),
        "KillMode=control-group".into(),
        "Delegate=no".into(),
        "NoNewPrivileges=yes".into(),
        "TimeoutStopSec=2s".into(),
        "SendSIGKILL=yes".into(),
        "LimitCORE=0".into(),
        "UMask=0077".into(),
    ] {
        command.arg(format!("--property={property}"));
    }
    command
        .arg(&spec.bubblewrap.path)
        .args([
            "--unshare-all",
            "--unshare-user",
            "--unshare-cgroup",
            "--die-with-parent",
            "--new-session",
            "--uid",
            "0",
            "--gid",
            "0",
            "--cap-drop",
            "ALL",
            "--cap-add",
            "CAP_SYS_ADMIN",
            "--cap-add",
            "CAP_SETPCAP",
            "--cap-add",
            "CAP_NET_ADMIN",
            "--clearenv",
            "--proc",
            "/proc",
            "--dir",
            "/dev",
            "--size",
        ])
        .arg(spec.limits.scratch_capacity()?.to_string())
        .args(["--tmpfs", "/work", "--ro-bind"])
        .arg(&control)
        .arg("/control")
        .arg("--ro-bind")
        .arg(&spec.checkout)
        .arg("/repo");
    for device in [
        "/dev/null",
        "/dev/zero",
        "/dev/full",
        "/dev/random",
        "/dev/urandom",
    ] {
        command.args(["--dev-bind", device, device]);
    }
    for path in &spec.store_paths {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    command.args([
        "--symlink",
        "/work/tmp",
        "/tmp",
        "--remount-ro",
        "/",
        "--chdir",
        "/repo",
        "--",
        "/control/helper",
        "__canix-roborev-worker",
    ]);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = process::ChildGuard(command.spawn().context("start offline worker service")?);
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout = child.0.stdout.take().context("missing worker stdout")?;
    let stderr = child.0.stderr.take().context("missing worker stderr")?;
    process::nonblocking(&stdout)?;
    process::nonblocking(&stderr)?;
    let count = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let out_flag = exceeded.clone();
    let err_flag = exceeded.clone();
    let maximum = spec.limits.output_bytes;
    let out_count = count.clone();
    let err_count = count.clone();
    let out_stop = stop.clone();
    let err_stop = stop.clone();
    let out_thread = thread::spawn(move || drain(stdout, maximum, out_flag, out_count, out_stop));
    let err_thread = thread::spawn(move || drain(stderr, maximum, err_flag, err_count, err_stop));
    let work = (|| -> Result<BoundaryEvidence> {
        let (mut connection, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            ensure!(
                child.0.try_wait()?.is_none(),
                "worker setup failed before mandatory verification"
            );
            ensure!(
                start.elapsed() < Duration::from_secs(spec.limits.wall_seconds),
                "worker admission deadline exceeded"
            );
            thread::sleep(Duration::from_millis(10));
        };
        connection.set_read_timeout(Some(Duration::from_secs(2)))?;
        connection.set_write_timeout(Some(Duration::from_secs(2)))?;
        let ready = helper::read_line(&mut connection)?;
        ensure!(
            ready == hash(&manifest_bytes),
            "worker readiness differs from bound manifest"
        );
        let evidence = verifier::observe(spec, &unit.name, &connection, &control)?;
        unit.cgroup =
            Some(Path::new("/sys/fs/cgroup").join(evidence.cgroup.trim_start_matches('/')));
        write_new(
            &result.join("boundary.json"),
            &serde_json::to_vec(&evidence)?,
        )?;
        drop(listener);
        fs::remove_file(&socket)?;
        connection.write_all(format!("{}\n", binding.input_manifest_sha256).as_bytes())?;
        drop(connection);
        loop {
            ensure!(
                !exceeded.load(Ordering::Acquire),
                "worker output budget exceeded"
            );
            if let Some(status) = child.0.try_wait()? {
                ensure!(status.success(), "offline backend or worker unit failed");
                break;
            }
            ensure!(
                start.elapsed() < Duration::from_secs(spec.limits.wall_seconds),
                "worker runtime deadline exceeded"
            );
            thread::sleep(Duration::from_millis(10));
        }
        Ok(evidence)
    })();
    let cleanup = unit.cleanup();
    // The unit is authoritative; killing the client is only bounded controller
    // cleanup, and never a substitute for observed cgroup emptiness.
    if child.0.try_wait()?.is_none() {
        let _ = child.0.kill();
    }
    let _ = child.0.wait();
    stop.store(true, Ordering::Release);
    let stdout = out_thread
        .join()
        .map_err(|_| anyhow::anyhow!("worker output reader failed"))??;
    let stderr = err_thread
        .join()
        .map_err(|_| anyhow::anyhow!("worker error reader failed"))??;
    write_new(
        &result.join("cleanup.json"),
        &serde_json::to_vec(&serde_json::json!({"cgroup_empty": unit.empty, "unit": unit.name}))?,
    )?;
    write_new(&result.join("captured.stdout"), &stdout)?;
    write_new(&result.join("captured.stderr"), &stderr)?;
    cleanup?;
    let boundary =
        work.with_context(|| format!("worker stderr: {}", String::from_utf8_lossy(&stderr)))?;
    ensure!(
        !exceeded.load(Ordering::Acquire),
        "worker output budget exceeded"
    );
    spec.validate()?;
    write_new(&result.join("stdout"), &stdout)?;
    write_new(&result.join("stderr"), &stderr)?;
    let receipt = OfflineReceipt {
        scope: "offline-envelope-only; not forge acceptance or daemon authentication".into(),
        binding: binding.clone(),
        boundary,
        backend_success: true,
        cgroup_empty: unit.empty,
        stdout_sha256: hash(&stdout),
        stderr_sha256: hash(&stderr),
    };
    write_new(&result.join("receipt.json"), &serde_json::to_vec(&receipt)?)?;
    Ok(receipt)
}
