#![cfg(all(feature = "roborev-execution-tests", target_os = "linux"))]

use canix_toolbelt_roborev_worker::{
    Binding,
    admission::{Admission, AdmissionBinding, AdmissionPhase},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn independent_controllers_receive_at_most_one_enqueue_grant() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let binding = AdmissionBinding {
        request: Binding {
            request_url: "https://github.com/example/repo/pull/1".into(),
            request_id: "concurrent-admission".into(),
            authorized_request_sha256: "a".repeat(64),
            execution_policy_sha256: "b".repeat(64),
            base: "c".repeat(40),
            head: "d".repeat(40),
        },
        controller_id: "e".repeat(64),
        daemon_identity_sha256: "f".repeat(64),
    };
    let admission = Admission::register(root.path(), &binding).unwrap();
    let handle = root.path().join("original-handle.json");
    fs::write(&handle, serde_json::to_vec(&admission).unwrap()).unwrap();
    fs::set_permissions(&handle, fs::Permissions::from_mode(0o600)).unwrap();
    let mut children: Vec<_> = (0..12)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_roborev-admission-fixture"))
                .args(["enqueue-only", handle.to_str().unwrap()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut successes = 0;
    let mut stalled = 0;
    for child in &mut children {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                successes += usize::from(status.success());
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                stalled += 1;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(stalled, 0);
    assert_eq!(successes, 1);
    assert_eq!(admission.phase().unwrap(), AdmissionPhase::EnqueueUnknown);
    assert!(admission.begin_enqueue().is_err());
}
