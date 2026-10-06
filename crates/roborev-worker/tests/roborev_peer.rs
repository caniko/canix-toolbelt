#![cfg(all(feature = "roborev-execution-tests", target_os = "linux"))]

use canix_toolbelt_roborev_worker::peer::DirectAdapterAuthority;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn client(endpoint: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_roborev-admission-fixture"))
        .args(["peer-client", endpoint.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn accept(listener: &UnixListener) -> UnixStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("bounded peer accept: {error}"),
        }
    }
}

#[test]
fn direct_loaded_adapter_is_authenticated_and_dead_peer_cannot_be_reused() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = root.path().join("peer.sock");
    let listener = UnixListener::bind(&endpoint).unwrap();
    let authority = DirectAdapterAuthority::capture(
        std::process::id(),
        Path::new(env!("CARGO_BIN_EXE_roborev-admission-fixture")),
    )
    .unwrap();
    let mut child = client(&endpoint);
    let mut stream = accept(&listener);
    stream.read_exact(&mut [0u8; 1]).unwrap();
    let peer = authority.authenticate(&stream).unwrap();
    assert_eq!(peer.identity().pid, child.id());
    peer.check_live().unwrap();
    assert_eq!(
        peer.arguments().unwrap()[1..],
        ["peer-client", endpoint.to_str().unwrap()]
    );
    assert!(
        peer.checkout_directory()
            .unwrap()
            .metadata()
            .unwrap()
            .is_dir()
    );
    let daemon_that_will_exit = DirectAdapterAuthority::capture(
        child.id(),
        Path::new(env!("CARGO_BIN_EXE_roborev-admission-fixture")),
    )
    .unwrap();
    stream.write_all(b"x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        if stat.rsplit_once(") ").unwrap().1.starts_with("Z ") {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("fixture did not exit into the unreaped zombie state");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Do not reap before checking: signal zero still succeeds for this zombie,
    // whereas the socket-derived pidfd must already report sticky exit readiness.
    assert!(peer.check_live().is_err());
    assert!(authority.authenticate(&stream).is_err());
    assert!(child.wait().unwrap().success());
    assert!(peer.check_live().is_err());
    assert!(authority.authenticate(&stream).is_err());
    assert!(daemon_that_will_exit.authenticate(&stream).is_err());
}

#[test]
fn wrong_loaded_image_and_same_uid_unrelated_connection_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = root.path().join("peer.sock");
    let listener = UnixListener::bind(&endpoint).unwrap();
    let authority =
        DirectAdapterAuthority::capture(std::process::id(), &std::env::current_exe().unwrap())
            .unwrap();
    let mut child = client(&endpoint);
    let mut stream = accept(&listener);
    stream.read_exact(&mut [0u8; 1]).unwrap();
    assert!(authority.authenticate(&stream).is_err());
    stream.write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    let _client = UnixStream::connect(&endpoint).unwrap();
    let stream = accept(&listener);
    assert!(authority.authenticate(&stream).is_err());
}

#[test]
fn executable_replacement_and_unsafe_images_never_match_approved_identity() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = root.path().join("peer.sock");
    let listener = UnixListener::bind(&endpoint).unwrap();
    let image = root.path().join("approved");
    std::fs::copy(env!("CARGO_BIN_EXE_roborev-admission-fixture"), &image).unwrap();
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o755)).unwrap();
    let authority = DirectAdapterAuthority::capture(std::process::id(), &image).unwrap();
    std::fs::rename(&image, root.path().join("original")).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_roborev-admission-fixture"), &image).unwrap();
    let mut child = Command::new(&image)
        .args(["peer-client", endpoint.to_str().unwrap()])
        .spawn()
        .unwrap();
    let mut stream = accept(&listener);
    stream.read_exact(&mut [0u8; 1]).unwrap();
    assert!(authority.authenticate(&stream).is_err());
    stream.write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(DirectAdapterAuthority::capture(std::process::id(), &image).is_err());
    std::os::unix::fs::symlink(root.path().join("original"), root.path().join("link")).unwrap();
    assert!(
        DirectAdapterAuthority::capture(std::process::id(), &root.path().join("link")).is_err()
    );
}
