use super::{Executable, MAX_MANIFEST, Manifest, hash};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub(super) fn read_line(stream: &mut UnixStream) -> Result<String> {
    let mut bytes = Vec::new();
    for _ in 0..4096 {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            return String::from_utf8(bytes).context("invalid worker control frame");
        }
        bytes.push(byte[0]);
    }
    anyhow::bail!("worker control frame exceeded its bound")
}

fn setup_scratch(bytes: u64, inodes: u64) -> Result<()> {
    use nix::mount::{MsFlags, mount};
    let options = format!("size={bytes},nr_inodes={inodes},mode=0700");
    mount::<str, str, str, str>(
        None,
        "/work",
        None,
        MsFlags::MS_REMOUNT | MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        Some(options.as_str()),
    )
    .context("cannot bound private scratch mount")?;
    for path in [
        "/work/home",
        "/work/tmp",
        "/work/config",
        "/work/data",
        "/work/state",
        "/work/cache",
    ] {
        fs::create_dir(path)?;
    }
    Ok(())
}

// Linux ifreq flag ioctls are not exposed by the safe socket API. Only the
// private helper calls this, after verifying all namespace identities.
#[allow(unsafe_code)]
fn setup_loopback() -> Result<()> {
    // Linux ifreq is 40 bytes on both supported 64-bit worker platforms.
    #[repr(C)]
    struct Interface {
        name: [u8; 16],
        data: [u8; 24],
    }
    let mut request = Interface {
        name: [0; 16],
        data: [0; 24],
    };
    request.name[..2].copy_from_slice(b"lo");
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socket};
    let socket = socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .context("cannot open private loopback setup socket")?;
    // SAFETY: kernel ifreq writes within the ABI-sized owned structure.
    ensure!(
        unsafe { nix::libc::ioctl(socket.as_raw_fd(), nix::libc::SIOCGIFFLAGS, &mut request) } == 0,
        "cannot inspect private loopback flags"
    );
    let flags = i16::from_ne_bytes([request.data[0], request.data[1]]) | nix::libc::IFF_UP as i16;
    request.data[..2].copy_from_slice(&flags.to_ne_bytes());
    // SAFETY: same valid ifreq; CAP_NET_ADMIN belongs only to the private network.
    ensure!(
        unsafe { nix::libc::ioctl(socket.as_raw_fd(), nix::libc::SIOCSIFFLAGS, &request) } == 0,
        "cannot bring private loopback up"
    );
    Ok(())
}

// Linux securebits/bounding-set/capset ABI is confined to this private setup
// function; no caller can provide pointers, capability sets or a target PID.
#[allow(unsafe_code)]
fn drop_capabilities() -> Result<()> {
    // Lock out root/exec capability regeneration before clearing all sets.
    // SAFETY: prctl scalar arguments, operating on this process only.
    ensure!(
        unsafe { nix::libc::prctl(nix::libc::PR_SET_SECUREBITS, 15, 0, 0, 0) } == 0,
        "cannot lock worker securebits"
    );
    for capability in 0..64 {
        // SAFETY: drop this process's bounding capability; unknown Linux caps
        // return EINVAL, not a capability that can be retained accidentally.
        if unsafe { nix::libc::prctl(nix::libc::PR_CAPBSET_DROP, capability, 0, 0, 0) } != 0 {
            ensure!(
                std::io::Error::last_os_error().raw_os_error() == Some(nix::libc::EINVAL),
                "cannot drop worker capability bounding set"
            );
        }
    }
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    struct Caps {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x2008_0522,
        pid: 0,
    };
    let data = [
        Caps {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
        Caps {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
    ];
    // SAFETY: Linux v3 capset header and two 32-bit capability records are valid
    // for this process; all buffers remain live throughout the syscall.
    ensure!(
        unsafe { nix::libc::syscall(nix::libc::SYS_capset, &header, data.as_ptr()) } == 0,
        "cannot clear worker capability sets"
    );
    nix::sys::prctl::set_no_new_privs().context("cannot set worker no-new-privileges")?;
    Ok(())
}

fn prompt_file(bytes: &[u8]) -> Result<File> {
    use nix::{
        fcntl::{FcntlArg, SealFlag, fcntl},
        sys::memfd::{MFdFlags, memfd_create},
    };
    let descriptor = memfd_create(
        c"canix-review-prompt",
        MFdFlags::MFD_CLOEXEC | MFdFlags::MFD_ALLOW_SEALING,
    )
    .context("cannot create immutable worker prompt")?;
    let mut file = File::from(descriptor);
    file.write_all(bytes)?;
    file.seek(SeekFrom::Start(0))?;
    fcntl(
        &file,
        FcntlArg::F_ADD_SEALS(
            SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_SEAL,
        ),
    )
    .context("cannot seal worker prompt")?;
    Ok(file)
}

/// Trusted helper protocol used by the Toolbelt worker executable and native helper. It
/// cannot select a shared backend, inherit operator configuration, or exec until
/// the controller has observed this exact process in the expected live envelope.
pub fn helper_main() -> Result<()> {
    ensure!(
        std::env::args_os()
            .skip(1)
            .eq([std::ffi::OsString::from("__canix-roborev-worker")]),
        "invalid worker helper invocation"
    );
    let mut bytes = Vec::new();
    File::open("/control/manifest.json")?
        .take(MAX_MANIFEST as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_MANIFEST,
        "oversized worker helper manifest"
    );
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("invalid worker helper manifest")?;
    let uid_map = fs::read_to_string("/proc/self/uid_map")?;
    let gid_map = fs::read_to_string("/proc/self/gid_map")?;
    ensure!(
        manifest.host_uid > 0
            && uid_map.split_whitespace().eq([
                "0".to_owned(),
                manifest.host_uid.to_string(),
                "1".to_owned()
            ])
            && gid_map.split_whitespace().eq([
                "0".to_owned(),
                manifest.host_gid.to_string(),
                "1".to_owned()
            ])
            && nix::unistd::geteuid().is_root(),
        "worker setup must run inside a single-mapped unprivileged user namespace"
    );
    for (name, original) in &manifest.host_namespaces {
        ensure!(
            fs::metadata(format!("/proc/self/ns/{name}"))?.ino() != *original,
            "worker helper lacks a private namespace"
        );
    }
    ensure!(
        manifest
            .host_namespaces
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            == super::NAMESPACE_NAMES.into_iter().collect(),
        "missing controller namespace identities"
    );
    ensure!(
        super::file_digest(Path::new("/control/helper"))? == manifest.spec.helper.sha256,
        "loaded worker helper changed"
    );
    let backend: &Executable = &manifest.spec.backend;
    backend.validate()?;
    manifest.spec.limits.validate()?;
    setup_scratch(
        manifest.spec.limits.scratch_capacity()?,
        manifest.spec.limits.scratch_inodes,
    )?;
    setup_loopback()?;
    drop_capabilities()?;
    super::seccomp::install()?;
    let mut connection = UnixStream::connect("/control/gate.sock")?;
    connection.set_read_timeout(Some(Duration::from_secs(manifest.spec.limits.wall_seconds)))?;
    connection.set_write_timeout(Some(Duration::from_secs(2)))?;
    connection.write_all(format!("{}\n", hash(&bytes)).as_bytes())?;
    ensure!(
        read_line(&mut connection)? == manifest.binding.input_manifest_sha256,
        "worker admission was not granted"
    );
    drop(connection);
    let path = manifest
        .spec
        .store_paths
        .iter()
        .map(|root| root.join("bin").display().to_string())
        .collect::<Vec<_>>()
        .join(":");
    let mut command = Command::new(&backend.path);
    command
        .args(&manifest.spec.arguments)
        .env_clear()
        .current_dir("/repo")
        .env("PATH", path)
        .env("HOME", "/work/home")
        .env("USER", "review")
        .env("TMPDIR", "/work/tmp")
        .env("XDG_CONFIG_HOME", "/work/config")
        .env("XDG_DATA_HOME", "/work/data")
        .env("XDG_STATE_HOME", "/work/state")
        .env("XDG_CACHE_HOME", "/work/cache")
        .env("OPENCODE_CONFIG_PROJECT_DISABLE", "1")
        .env("OPENCODE_DIRECT", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::from(prompt_file(&manifest.spec.prompt)?));
    if manifest.spec.trusted_files.contains_key("opencode.json") {
        command.env("OPENCODE_CONFIG", "/control/opencode.json");
    }
    Err(command.exec()).context("offline backend exec failed")
}
