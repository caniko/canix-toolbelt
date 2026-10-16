//! Private PID-namespace init for qualification. Not an operator frontend.
#![deny(unsafe_code)]

// Keep PID 1 independent of the full operator/library dependency graph. Its
// binary is byte-hashed for each run; even debug builds must remain small.
#[cfg(target_os = "linux")]
#[path = "../qualification/boundary.rs"]
mod boundary;
#[cfg(target_os = "linux")]
#[path = "../qualification/execution.rs"]
mod execution;
#[cfg(target_os = "linux")]
#[path = "../qualification/json.rs"]
mod json;
#[cfg(target_os = "linux")]
#[path = "../qualification/syscall_watch.rs"]
mod syscall_watch;
#[cfg(target_os = "linux")]
#[path = "../qualification/work_tree.rs"]
mod work_tree;

#[cfg(target_os = "linux")]
fn supervise() -> std::io::Result<i32> {
    use std::process::Command;
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};

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
    close_inherited_descriptors()?;
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
    let boundary_path = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing boundary policy"))?;
    let boundary_sha256 = args
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| std::io::Error::other("missing boundary policy identity"))?;
    let boundary_bytes = json::read_bound_bytes(
        json::regular_file(std::path::Path::new(&boundary_path))?,
        Some(&boundary_sha256),
    )?;
    let policy: boundary::Boundary = serde_json::from_slice(&boundary_bytes)?;
    if policy.read_only_paths.iter().any(|path| {
        !boundary::SYSTEM_PATHS
            .iter()
            .any(|system| path == std::path::Path::new(system))
    }) {
        return Err(std::io::Error::other("unsupported public runtime mount"));
    }
    let max_log_bytes = args
        .next()
        .and_then(|value| value.to_str().and_then(|s| s.parse::<u64>().ok()))
        .filter(|limit| *limit > 0 && *limit <= boundary::MAX_LOG_BYTES)
        .ok_or_else(|| std::io::Error::other("invalid log bound"))?;
    let setpriv = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing privilege limiter"))?;
    // Bootstrap may have executed from a sealed anonymous descriptor. Re-entry
    // uses the canonical path only after minimal_root has digest-checked its
    // private snapshot, never a host pathname or a deleted memfd pseudo-path.
    let executor = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing executor snapshot path"))?;
    let program = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing executable"))?;
    let argument_zero = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing argument zero"))?;
    let cwd = std::env::current_dir()?;
    // Rust descriptors are CLOEXEC. Only non-dumpable PID 1 retains this old
    // mount handle, and it never follows work-tree symlinks during retention.
    use std::os::fd::AsRawFd;
    let retained_work = std::fs::File::open(&work)?;
    minimal_root(
        &policy,
        &boundary_bytes,
        std::path::Path::new(&work),
        &cwd,
        &cancelled,
    )?;
    read_only_host(std::path::Path::new(&work))?;
    // Re-enter the remounted tree; inherited cwd refers to the old mount.
    std::env::set_current_dir(cwd)?;
    syscall_watch::prepare()?;
    let child = Command::new(setpriv)
        .args([
            "--bounding-set=-all",
            "--inh-caps=-all",
            "--ambient-caps=-all",
            "--no-new-privs",
            "--",
        ])
        // setpriv cannot override argv[0]. Re-enter this byte-bound executable
        // as a minimal exec adapter *after* dropping capabilities and setting
        // no-new-privs, then execute the canonical producer with its requested
        // name. This role never performs namespace or mount operations.
        .arg(executor)
        .arg("--producer-exec")
        .arg(max_log_bytes.to_string())
        .arg(boundary_sha256)
        .arg(program)
        .arg(argument_zero)
        .args(args)
        .env_clear()
        // fd 0 is a private report channel opened by the host caller. Never let
        // the producer inherit it or reach it through the supervisor's proc fds.
        .stdin(std::process::Stdio::null())
        .spawn()?;
    let execution = syscall_watch::watch(
        child.id().try_into().map_err(std::io::Error::other)?,
        &cancelled,
    )?;
    // The observer accounts for fork/clone descendants through kernel events,
    // never a producer-controlled /proc listing. Any remaining tracee prevents
    // retention and acceptance. Exiting PID 1 kills/drains the namespace before
    // the launcher can reap us, including detached process groups.
    let drained = execution.descendants_drained;
    let capacity = rustix::fs::statvfs(&work)?;
    if capacity.f_bavail == 0 || capacity.f_favail == 0 {
        return Err(std::io::Error::other(
            "producer exhausted the aggregate work-tree quota",
        ));
    }
    if drained {
        let retained =
            std::path::PathBuf::from(format!("/proc/self/fd/{}", retained_work.as_raw_fd()));
        work_tree::copy_tree(std::path::Path::new(&work), &retained, &|| {
            cancelled.load(Ordering::Relaxed)
        })?;
    }
    let mut bytes = serde_json::to_vec_pretty(&execution)?;
    bytes.push(b'\n');
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let count = rustix::io::write(rustix::stdio::stdin(), remaining)?;
        if count == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        remaining = &remaining[count..];
    }
    rustix::fs::fsync(rustix::stdio::stdin())?;
    Ok(if drained {
        execution.exit_code.unwrap_or(125)
    } else {
        125
    })
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn close_inherited_descriptors() -> std::io::Result<()> {
    // SAFETY: this runs once in freshly exec'd, single-threaded PID 1 before any
    // Rust-owned file, signal registration or child exists. No OwnedFd values
    // refer to the inherited descriptors. Preserve only stdin/stdout/stderr;
    // closing arbitrary caller handles here cannot invalidate caller memory.
    // No /proc enumeration race or RLIMIT-based bound can leave high FDs open.
    if unsafe { libc::close_range(3, u32::MAX, 0) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
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
                    b"nosymfollow" => MountFlags::NOSYMFOLLOW,
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
    // minimal_root has already chrooted: `work` now names the private quota tmpfs,
    // not the host-side retained directory. Re-enable writes on that actual mount
    // after the blanket protection above. Cross-mount hard links fail with EXDEV.
    mount_bind(work, work).map_err(|e| std::io::Error::other(format!("bind work: {e}")))?;
    mount_remount(
        work,
        MountFlags::BIND | MountFlags::NOSUID | MountFlags::NODEV,
        "",
    )
    .map_err(|e| std::io::Error::other(format!("writable work: {e}")))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn producer_exec() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let mut args = std::env::args_os().skip(2);
    let Some(max_log_bytes) = args
        .next()
        .and_then(|value| value.to_str().and_then(|s| s.parse::<u64>().ok()))
        .filter(|limit| *limit > 0 && *limit <= boundary::MAX_LOG_BYTES)
    else {
        return std::io::Error::other("invalid producer output bound");
    };
    let Some(boundary_sha256) = args.next().and_then(|value| value.into_string().ok()) else {
        return std::io::Error::other("missing producer boundary identity");
    };
    let Some(program) = args.next() else {
        return std::io::Error::other("missing producer executable");
    };
    let Some(argument_zero) = args.next().filter(|value| !value.is_empty()) else {
        return std::io::Error::other("missing producer argument zero");
    };
    if std::process::id() == 1 {
        return std::io::Error::other("producer exec cannot replace namespace init");
    }
    let policy: boundary::Boundary = match json::read_bound_json(
        std::path::Path::new("/.qualification-boundary.json"),
        Some(&boundary_sha256),
    ) {
        Ok(policy) => policy,
        Err(error) => return error,
    };
    if let Err(error) = rustix::process::setrlimit(
        rustix::process::Resource::Fsize,
        rustix::process::Rlimit {
            current: Some(max_log_bytes),
            maximum: Some(max_log_bytes),
        },
    ) {
        return error.into();
    }
    if let Err(error) = syscall_watch::ready() {
        return error;
    }
    std::process::Command::new(program)
        .arg0(argument_zero)
        .args(args)
        .env_clear()
        .envs(policy.environment)
        .exec()
}

#[cfg(target_os = "linux")]
fn minimal_root(
    policy: &boundary::Boundary,
    boundary_bytes: &[u8],
    work: &std::path::Path,
    cwd: &std::path::Path,
    cancelled: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    use rustix::mount::{MountFlags, mount, mount_bind, mount_bind_recursive};
    use std::io::Write;
    // Only this single-threaded namespace init synthesizes private parents.
    // Canonical creation modes must not change the producer's inherited umask.
    let umask = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o022));
    let root = &policy.sandbox;
    // ponytail: a private input snapshot is capped at 1 GiB/131072 inodes.
    // Larger selections need a separately qualified disk-snapshot engine.
    mount(
        "tmpfs",
        root,
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV,
        c"size=1g,nr_inodes=131072,mode=0755",
    )?;
    let target = |path: &std::path::Path| -> std::io::Result<std::path::PathBuf> {
        let path = path.strip_prefix("/").map_err(std::io::Error::other)?;
        if path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(std::io::Error::other("non-normal boundary mount path"));
        }
        Ok(root.join(path))
    };
    let snapshot = root.join(".qualification-inputs");
    std::fs::create_dir(&snapshot)?;
    let snapshot_path = |path: &std::path::Path| -> std::io::Result<std::path::PathBuf> {
        Ok(snapshot.join(
            target(path)?
                .strip_prefix(root)
                .map_err(std::io::Error::other)?,
        ))
    };
    for path in &policy.snapshot_directories {
        use std::os::unix::fs::PermissionsExt;
        let directory = snapshot_path(path)?;
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755))?;
    }
    // Copy only inventoried files, never a live directory traversal. A host can
    // still mutate the opened inode, but only a copy hashing to the independently
    // selected digest can reach the producer. Symlinks are recreated from policy.
    for (path, expected) in &policy.snapshot_files {
        if expected.link.is_some() {
            continue;
        }
        let destination = snapshot_path(path)?;
        std::fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| std::io::Error::other("missing snapshot parent"))?,
        )?;
        snapshot_file(path, &destination, expected, cancelled)?;
    }
    for (path, expected) in &policy.snapshot_files {
        if let Some(link) = &expected.link {
            let destination = snapshot_path(path)?;
            std::fs::create_dir_all(
                destination
                    .parent()
                    .ok_or_else(|| std::io::Error::other("missing link parent"))?,
            )?;
            std::os::unix::fs::symlink(link, destination)?;
        }
    }
    let mut selections = policy.snapshot_mounts.clone();
    selections.sort_by_key(|path| path.components().count());
    selections.dedup();
    // Prepare placeholders while root contains only private tmpfs objects. No
    // mkdir/create is allowed through a later public-system bind mount.
    for path in &selections {
        let source = snapshot_path(path)?;
        let destination = target(path)?;
        if source.is_dir() {
            std::fs::create_dir_all(&destination)?;
        } else {
            std::fs::create_dir_all(
                destination
                    .parent()
                    .ok_or_else(|| std::io::Error::other("missing snapshot mount parent"))?,
            )?;
            if !destination.exists() {
                std::fs::File::create(destination)?;
            }
        }
    }
    let mut paths = policy.read_only_paths.clone();
    paths.sort_by_key(|path| path.components().count());
    paths.dedup();
    for source in paths {
        let destination = target(&source)?;
        let metadata = std::fs::metadata(&source)?;
        if metadata.is_dir() {
            // NAR/store objects cannot contain live sockets. Other dependency
            // trees are admitted only when no host control channel is exposed.
            if source != std::path::Path::new("/nix/store") {
                reject_control_channels(&source)?;
            }
            std::fs::create_dir_all(&destination)?;
            mount_bind_recursive(&source, &destination)?;
        } else {
            // Regular files and the four standard random/null devices only;
            // sockets, FIFOs and arbitrary device dependencies fail closed.
            if !metadata.is_file()
                && !["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"]
                    .iter()
                    .any(|path| source == std::path::Path::new(path))
            {
                return Err(std::io::Error::other("unsupported boundary mount input"));
            }
            std::fs::create_dir_all(
                destination
                    .parent()
                    .ok_or_else(|| std::io::Error::other("missing mount parent"))?,
            )?;
            if !destination.exists() {
                std::fs::File::create(&destination)?;
            }
            mount_bind(&source, &destination)?;
        }
    }
    for path in selections {
        // Selected paths are never live host bind mounts, including selections
        // beneath the platform-owned runtime trees mounted above.
        mount_bind(snapshot_path(&path)?, target(&path)?)?;
    }
    let work_target = target(work)?;
    std::fs::create_dir_all(&work_target)?;
    mount(
        "tmpfs",
        &work_target,
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV,
        c"size=256m,nr_inodes=16384",
    )?;
    work_tree::copy_tree(work, &work_target, &|| {
        cancelled.load(std::sync::atomic::Ordering::Relaxed)
    })?;
    let config = root.join(".qualification-boundary.json");
    // Copy the admitted snapshot into our private tmpfs, not a bind of the
    // externally writable host inode. Non-dumpable PID 1 owns the only view;
    // read_only_host protects it before the final exec adapter reads it again.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config)?
        .write_all(boundary_bytes)?;
    let proc = root.join("proc");
    std::fs::create_dir(&proc)?;
    mount_bind_recursive("/proc", &proc)?;
    std::fs::create_dir_all(target(cwd)?)?;
    rustix::process::chroot(root)?;
    std::env::set_current_dir("/")?;
    rustix::process::umask(umask);
    Ok(())
}

#[cfg(target_os = "linux")]
fn snapshot_file(
    source: &std::path::Path,
    destination: &std::path::Path,
    expected: &boundary::SourceFile,
    cancelled: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        sync::atomic::Ordering,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32)
        .open(source)?;
    let before = file.metadata()?;
    if !before.is_file()
        || before.len() > 1024 * 1024 * 1024
        || (before.permissions().mode() & 0o111 != 0) != expected.executable
    {
        return Err(std::io::Error::other(
            "unsupported snapshot file or executable mode changed",
        ));
    }
    let mut input = (&file).take(before.len() + 1);
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    if input.limit() != 1 || format!("{:x}", hash.finalize()) != expected.sha256 {
        return Err(std::io::Error::other("selected snapshot bytes differ"));
    }
    output.set_permissions(std::fs::Permissions::from_mode(if expected.executable {
        0o755
    } else {
        0o644
    }))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_control_channels(root: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::FileTypeExt;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.file_type().is_socket() || metadata.file_type().is_fifo() {
                return Err(std::io::Error::other(
                    "host control channel in runtime dependency tree",
                ));
            }
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--producer-exec")) {
        eprintln!("qualification producer exec: {}", producer_exec());
        return std::process::ExitCode::from(125);
    }
    #[cfg(target_os = "linux")]
    match supervise() {
        Ok(code) => return std::process::ExitCode::from(u8::try_from(code).unwrap_or(125)),
        Err(error) => eprintln!("qualification executor: {error}"),
    }
    #[cfg(not(target_os = "linux"))]
    eprintln!("qualification execution requires Linux PID namespaces");
    std::process::ExitCode::from(125)
}
