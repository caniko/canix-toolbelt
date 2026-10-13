//! Private PID-namespace init for qualification. Not an operator frontend.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
fn supervise() -> std::io::Result<i32> {
    use std::process::Command;
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
    use std::time::Duration;

    // A fresh PID namespace plus its own /proc is mandatory. Refuse direct use.
    if std::process::id() != 1
        || std::fs::read_to_string("/proc/self/stat")?
            .split_whitespace()
            .next()
            != Some("1")
    {
        return Err(std::io::Error::other(
            "executor requires a fresh PID namespace and proc mount",
        ));
    }
    // The producer shares our mapped uid. Deny ptrace/proc-fd access to this
    // privileged supervisor rather than relying on host Yama configuration.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, Arc::clone(&cancelled))?;
    }
    let mut args = std::env::args_os().skip(1);
    let work = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing work tree"))?;
    let setpriv = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing privilege limiter"))?;
    let program = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing executable"))?;
    let cwd = std::env::current_dir()?;
    read_only_host(std::path::Path::new(&work))?;
    // Re-enter the remounted tree; inherited cwd refers to the old mount.
    std::env::set_current_dir(cwd)?;
    let mut child = Command::new(setpriv)
        .args([
            "--bounding-set=-all",
            "--inh-caps=-all",
            "--ambient-caps=-all",
            "--no-new-privs",
            "--",
        ])
        .arg(program)
        .args(args)
        .spawn()?;
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(125);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // After reaping the leader, orphans have been reparented to PID 1. Consult
    // kernel child accounting, not /proc, whose mount a privileged producer can
    // replace. A live child, zombie, or unknown wait error invalidates completion.
    // Exiting init makes the kernel kill/drain the whole namespace before the
    // launcher can reap us. wait() uses waitpid(-1), covering all process groups.
    match rustix::process::wait(rustix::process::WaitOptions::NOHANG) {
        Err(rustix::io::Errno::CHILD) => Ok(status.code().unwrap_or(125)),
        Ok(_) => Ok(125),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
fn read_only_host(work: &std::path::Path) -> std::io::Result<()> {
    use rustix::mount::{MountFlags, mount_bind, mount_bind_recursive, mount_remount};
    use std::os::unix::ffi::OsStringExt;
    // Clone all mounts into this private namespace before remounting them; host
    // mount flags never change. Read mountinfo before any producer is launched.
    mount_bind_recursive("/", "/")
        .map_err(|e| std::io::Error::other(format!("clone root mounts: {e}")))?;
    let info = std::fs::read("/proc/self/mountinfo")?;
    let mut mounts = std::collections::BTreeMap::new();
    for line in info.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let id = line
            .split(|b| *b == b' ')
            .next()
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| std::io::Error::other("invalid mount id"))?;
        let encoded = line
            .split(|b| *b == b' ')
            .nth(4)
            .ok_or_else(|| std::io::Error::other("malformed mountinfo"))?;
        let mut path = Vec::new();
        let mut i = 0;
        while i < encoded.len() {
            if encoded[i] == b'\\' {
                let octal = encoded
                    .get(i + 1..i + 4)
                    .ok_or_else(|| std::io::Error::other("short mount escape"))?;
                if !octal.iter().all(|b| (b'0'..=b'7').contains(b)) {
                    return Err(std::io::Error::other("invalid mount escape"));
                }
                path.push((octal[0] - b'0') * 64 + (octal[1] - b'0') * 8 + octal[2] - b'0');
                i += 4;
            } else {
                path.push(encoded[i]);
                i += 1;
            }
        }
        let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(path));
        let visible = rustix::fs::statx(
            rustix::fs::CWD,
            &path,
            rustix::fs::AtFlags::empty(),
            rustix::fs::StatxFlags::MNT_ID,
        )?;
        if visible.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 {
            return Err(std::io::Error::other(
                "kernel does not report mount identities",
            ));
        }
        // Covered mounts (notably the host proc subtree beneath mount-proc)
        // are inaccessible after we drop capabilities. Remount only the mount
        // actually reached at each path, checked against the kernel mount id.
        if visible.stx_mnt_id == id {
            let options = line
                .split(|b| *b == b' ')
                .nth(5)
                .ok_or_else(|| std::io::Error::other("missing mount options"))?;
            let mut flags = MountFlags::BIND | MountFlags::RDONLY;
            for option in options.split(|b| *b == b',') {
                flags |= match option {
                    b"nosuid" => MountFlags::NOSUID,
                    b"nodev" => MountFlags::NODEV,
                    b"noexec" => MountFlags::NOEXEC,
                    b"noatime" => MountFlags::NOATIME,
                    b"nodiratime" => MountFlags::NODIRATIME,
                    b"relatime" => MountFlags::RELATIME,
                    b"strictatime" => MountFlags::STRICTATIME,
                    _ => MountFlags::empty(),
                };
            }
            mounts.insert(path, flags);
        }
    }
    if !mounts.contains_key(std::path::Path::new("/")) {
        return Err(std::io::Error::other("visible root mount is missing"));
    }
    let mut mounts: Vec<_> = mounts.into_iter().collect();
    mounts.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (mount, flags) in mounts {
        mount_remount(&mount, flags, "")
            .map_err(|e| std::io::Error::other(format!("protect {}: {e}", mount.display())))?;
    }
    // The host caller creates this empty directory before launch, so no existing
    // source/input hardlink aliases can be writable through it. Cross-mount link
    // creation fails with EXDEV. Sources cannot be changed via alternate paths.
    mount_bind(work, work).map_err(|e| std::io::Error::other(format!("bind work: {e}")))?;
    mount_remount(
        work,
        MountFlags::BIND | MountFlags::NOSUID | MountFlags::NODEV,
        "",
    )
    .map_err(|e| std::io::Error::other(format!("writable work: {e}")))?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    match supervise() {
        Ok(code) => return std::process::ExitCode::from(u8::try_from(code).unwrap_or(125)),
        Err(error) => eprintln!("qualification executor: {error}"),
    }
    #[cfg(not(target_os = "linux"))]
    eprintln!("qualification execution requires Linux PID namespaces");
    std::process::ExitCode::from(125)
}
