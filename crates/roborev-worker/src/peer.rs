//! Kernel-authenticated direct adapter/daemon lineage for request-exclusive jobs.
//! This primitive does not establish job/database/unit custody, invocation policy,
//! input integrity or write-authority separation. Those require controller checks.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_ticks: u64,
    pub effective_uid: u32,
    pub executable_device: u64,
    pub executable_inode: u64,
    pub cgroup: String,
    pub pid_namespace_inode: u64,
}

struct Process {
    identity: ProcessIdentity,
    parent_pid: u32,
    pidfd: OwnedFd,
}

fn pidfd(pid: u32) -> Result<OwnedFd> {
    ensure!(pid > 0 && pid <= i32::MAX as u32, "invalid process ID");
    let pid = rustix::process::Pid::from_raw(pid as i32).context("invalid process ID")?;
    rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .context("pin process identity")
}

// Signal zero checks existence/permission, not exit. Sticky pidfd readiness
// observes exit even for an unreaped zombie, without signalling another UID.
#[allow(unsafe_code)]
fn live(descriptor: &OwnedFd) -> Result<()> {
    let mut event = nix::libc::pollfd {
        fd: descriptor.as_raw_fd(),
        events: nix::libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll receives one valid owned pollfd and a zero bounded timeout.
    let ready = unsafe { nix::libc::poll(&mut event, 1, 0) };
    if ready < 0 {
        return Err(std::io::Error::last_os_error())
            .context("observe pinned process exit readiness");
    }
    ensure!(
        ready == 0 && event.revents == 0,
        "pinned process has exited or its lifetime descriptor is invalid"
    );
    Ok(())
}

fn read_bounded(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut data = Vec::new();
    File::open(path)?.take(65_537).read_to_end(&mut data)?;
    ensure!(
        data.len() <= 65_536,
        "oversized process identity observation"
    );
    Ok(String::from_utf8(data)?)
}

fn observe(pid: u32) -> Result<Process> {
    observe_pinned(pid, pidfd(pid)?)
}

fn observe_pinned(pid: u32, pinned: OwnedFd) -> Result<Process> {
    live(&pinned)?;
    let root = PathBuf::from(format!("/proc/{pid}"));
    let stat = read_bounded(&root.join("stat"))?;
    // comm may contain spaces or parentheses. Numeric fields follow its LAST
    // closing parenthesis; whitespace tokenization of the whole record is wrong.
    let (prefix, suffix) = stat.rsplit_once(") ").context("invalid process stat")?;
    ensure!(
        prefix.starts_with(&format!("{pid} (")),
        "process stat PID changed"
    );
    let fields: Vec<_> = suffix.split_whitespace().collect();
    ensure!(
        fields.len() >= 20 && !matches!(fields[0], "Z" | "X" | "x"),
        "process is terminal or stat is incomplete"
    );
    let parent_pid = fields[1].parse()?;
    let start_ticks = fields[19].parse()?;
    let status = read_bounded(&root.join("status"))?;
    let uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .context("process effective UID missing")?;
    let effective_uid = uid
        .split_whitespace()
        .nth(1)
        .context("invalid process UID")?
        .parse()?;
    let executable = fs::metadata(root.join("exe"))?;
    ensure!(executable.is_file(), "process executable is not regular");
    let cgroup = read_bounded(&root.join("cgroup"))?;
    ensure!(
        cgroup.lines().count() == 1 && cgroup.starts_with("0::/"),
        "unified process cgroup required"
    );
    let namespace = fs::metadata(root.join("ns/pid"))?;
    live(&pinned)?;
    Ok(Process {
        identity: ProcessIdentity {
            pid,
            start_ticks,
            effective_uid,
            executable_device: executable.dev(),
            executable_inode: executable.ino(),
            cgroup,
            pid_namespace_inode: namespace.ino(),
        },
        parent_pid,
        pidfd: pinned,
    })
}

/// Pin the live daemon generation and approved adapter's loaded executable image.
/// The caller must establish immutable executable, unit/config/root custody before
/// capture. Script/interpreter identity alone cannot authenticate a script.
/// This authority cannot be serialized or restored for another PID generation.
pub struct DirectAdapterAuthority {
    daemon: Arc<Process>,
    adapter_device: u64,
    adapter_inode: u64,
    _adapter: File,
}

/// Kernel-authenticated adapter connection. The pidfd prevents numeric PID reuse
/// from restoring authority. Retaining this is not a backend-boundary attestation.
pub struct AuthenticatedAdapter {
    process: Process,
    daemon: Arc<Process>,
}

impl AuthenticatedAdapter {
    pub fn identity(&self) -> &ProcessIdentity {
        &self.process.identity
    }

    pub fn check_live(&self) -> Result<()> {
        live(&self.daemon.pidfd)?;
        live(&self.process.pidfd)?;
        let process = observe(self.process.identity.pid)?;
        ensure!(
            process.identity == self.process.identity
                && process.parent_pid == self.daemon.identity.pid
                && observe(self.daemon.identity.pid)?.identity == self.daemon.identity,
            "authenticated adapter or original daemon generation changed"
        );
        live(&self.daemon.pidfd)?;
        live(&self.process.pidfd)?;
        Ok(())
    }

    /// Independently observe the approved process's argument vector. The caller
    /// checks the exact admitted argv; a delivery's self-reported argv is data.
    pub fn arguments(&self) -> Result<Vec<String>> {
        self.check_live()?;
        let text = read_bounded(&PathBuf::from(format!(
            "/proc/{}/cmdline",
            self.process.identity.pid
        )))?;
        ensure!(
            text.ends_with('\0'),
            "incomplete adapter argument observation"
        );
        let arguments: Vec<_> = text[..text.len() - 1]
            .split('\0')
            .map(str::to_owned)
            .collect();
        ensure!(
            !arguments.is_empty() && arguments.len() <= 32,
            "invalid adapter arguments"
        );
        self.check_live()?;
        Ok(arguments)
    }

    /// Pin the actual cwd directory through trusted procfs; never open a caller's
    /// submitted path as input authority. Snapshot verification and custody of the
    /// complete daemon preparation root remain required before backend admission.
    pub fn checkout_directory(&self) -> Result<File> {
        use std::os::unix::fs::OpenOptionsExt;
        self.check_live()?;
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_CLOEXEC)
            .open(format!("/proc/{}/cwd", self.process.identity.pid))?;
        ensure!(
            directory.metadata()?.is_dir(),
            "adapter cwd is not a directory"
        );
        self.check_live()?;
        Ok(directory)
    }
}

impl DirectAdapterAuthority {
    pub fn capture(daemon_pid: u32, approved_adapter: &Path) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        ensure!(
            approved_adapter.is_absolute(),
            "approved adapter must be absolute"
        );
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
            .open(approved_adapter)?;
        let adapter = file.metadata()?;
        ensure!(
            adapter.is_file() && adapter.mode() & 0o111 != 0 && adapter.mode() & 0o022 == 0,
            "approved adapter must be a protected executable image"
        );
        Ok(Self {
            daemon: Arc::new(observe(daemon_pid)?),
            adapter_device: adapter.dev(),
            adapter_inode: adapter.ino(),
            _adapter: file,
        })
    }

    pub fn daemon_identity(&self) -> &ProcessIdentity {
        &self.daemon.identity
    }

    /// Inspect SO_PEERCRED, never caller-supplied PID/job/model/path claims. Only
    /// the approved image directly launched by the ORIGINAL live daemon is
    /// eligible; unrelated same-UID clients and grandchildren are rejected.
    /// All descriptors and /proc observations must be in the controller's trusted
    /// host namespaces. Same-UID hostile host processes require separate custody.
    pub fn authenticate(&self, stream: &UnixStream) -> Result<AuthenticatedAdapter> {
        let credentials =
            nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
                .context("read kernel adapter credentials")?;
        ensure!(
            credentials.pid() > 0 && credentials.uid() == self.daemon.identity.effective_uid,
            "adapter peer UID does not match daemon"
        );
        let peer_pidfd = socket_peer_pidfd(stream)?;
        live(&peer_pidfd)?;
        live(&self.daemon.pidfd)?;
        ensure!(
            observe(self.daemon.identity.pid)?.identity == self.daemon.identity,
            "original daemon generation changed"
        );
        let process = observe_pinned(credentials.pid() as u32, peer_pidfd)?;
        ensure!(
            process.parent_pid == self.daemon.identity.pid,
            "adapter is not a direct child of the original daemon"
        );
        ensure!(
            process.identity.effective_uid == credentials.uid()
                && process.identity.executable_device == self.adapter_device
                && process.identity.executable_inode == self.adapter_inode
                && process.identity.cgroup == self.daemon.identity.cgroup
                && process.identity.pid_namespace_inode == self.daemon.identity.pid_namespace_inode,
            "adapter executable/UID/cgroup/namespace differs from admitted daemon policy"
        );
        live(&self.daemon.pidfd)?;
        live(&process.pidfd)?;
        Ok(AuthenticatedAdapter {
            process,
            daemon: Arc::clone(&self.daemon),
        })
    }
}

/// SO_PEERCRED contains a PID from connection establishment and cannot pin that
/// original process after exit/PID reuse. SO_PEERPIDFD (Linux 6.5+) associates the
/// lifetime descriptor with THIS socket. Unsupported kernels fail closed.
#[allow(unsafe_code)]
fn socket_peer_pidfd(stream: &UnixStream) -> Result<OwnedFd> {
    use std::os::fd::FromRawFd;
    let mut descriptor: nix::libc::c_int = -1;
    let mut size = std::mem::size_of_val(&descriptor) as nix::libc::socklen_t;
    // SAFETY: descriptor and size are initialized fixed-size owned buffers.
    let status = unsafe {
        nix::libc::getsockopt(
            stream.as_raw_fd(),
            nix::libc::SOL_SOCKET,
            nix::libc::SO_PEERPIDFD,
            (&mut descriptor as *mut nix::libc::c_int).cast(),
            &mut size,
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error())
            .context("socket-associated peer pidfd required");
    }
    ensure!(descriptor >= 0, "invalid socket-associated peer pidfd");
    // SAFETY: successful SO_PEERPIDFD returned a new owned descriptor.
    let peer = unsafe { OwnedFd::from_raw_fd(descriptor) };
    ensure!(
        size as usize == std::mem::size_of::<nix::libc::c_int>(),
        "invalid peer pidfd response size"
    );
    Ok(peer)
}
