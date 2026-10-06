#![cfg(all(feature = "roborev-execution-tests", target_os = "linux"))]

use canix_toolbelt_roborev_worker::{
    Binding,
    admission::{Admission, AdmissionBinding, AdmissionPhase, JobIdentity},
    execution::{ExecutionBinding, ExecutionState},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn private(root: &Path) {
    fs::create_dir(root).unwrap();
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn fsync_io_errors_never_grant_enqueue_dispatch_or_backend_execution() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let compiler = std::env::var_os("CANIX_TEST_CC")
        .expect("CANIX_TEST_CC requires the approved shell's immutable C compiler");
    assert!(Path::new(&compiler).is_absolute() && Path::new(&compiler).starts_with("/nix/store"));
    let library = root.path().join("fsync-fault.so");
    let compilation = Command::new(compiler)
        .args(["-shared", "-fPIC", "-Wall", "-Werror"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roborev-fsync-fault.c"))
        .arg("-o")
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    for action in ["enqueue-only", "dispatch", "reserve-only"] {
        for kind in ["file", "directory"] {
            let state = root.path().join(format!("{action}-{kind}"));
            private(&state);
            let executions = state.join("executions");
            private(&executions);
            let binding = AdmissionBinding {
                request: Binding {
                    request_url: "https://github.com/example/repo/pull/1".into(),
                    request_id: "durability-fixture".into(),
                    authorized_request_sha256: "a".repeat(64),
                    execution_policy_sha256: "b".repeat(64),
                    base: "c".repeat(40),
                    head: "d".repeat(40),
                },
                controller_id: "e".repeat(64),
                daemon_identity_sha256: "f".repeat(64),
            };
            let admission = Admission::register(&state, &binding).unwrap();
            let job = JobIdentity {
                id: 1,
                uuid: "01234567-89ab-4cde-8fab-0123456789ab".into(),
            };
            let mut fence = None;
            if action != "enqueue-only" {
                admission.begin_enqueue().unwrap();
                admission.bind_job(&job).unwrap();
            }
            if action == "reserve-only" {
                admission.begin_dispatch().unwrap();
                let execution = ExecutionBinding {
                    request: binding.request.clone(),
                    controller_id: binding.controller_id.clone(),
                    daemon_identity_sha256: binding.daemon_identity_sha256.clone(),
                    job_id: job.id,
                    job_uuid: job.uuid.clone(),
                    execution_id: "1".repeat(64),
                    input_manifest_sha256: "2".repeat(64),
                    backend_manifest_sha256: "3".repeat(64),
                };
                fence = Some(
                    admission
                        .register_execution(&executions, &execution)
                        .unwrap(),
                );
            }
            let handle = state.join("original.json");
            fs::write(&handle, serde_json::to_vec(&admission).unwrap()).unwrap();
            fs::set_permissions(&handle, fs::Permissions::from_mode(0o600)).unwrap();
            let outcome = Command::new(env!("CARGO_BIN_EXE_roborev-admission-fixture"))
                .args([action, handle.to_str().unwrap()])
                .env("LD_PRELOAD", &library)
                .env("CANIX_ADMISSION_FSYNC_FAULT", kind)
                .output()
                .unwrap();
            assert!(!outcome.status.success(), "{action}/{kind}");
            assert!(
                outcome.stdout.is_empty(),
                "effect grant escaped {action}/{kind}"
            );
            assert!(
                String::from_utf8_lossy(&outcome.stderr).contains("Input/output error"),
                "{}",
                String::from_utf8_lossy(&outcome.stderr)
            );
            match (action, kind) {
                ("enqueue-only", "file") => {
                    assert_eq!(admission.phase().unwrap(), AdmissionPhase::Admitted)
                }
                ("enqueue-only", "directory") => {
                    assert_eq!(admission.phase().unwrap(), AdmissionPhase::EnqueueUnknown);
                    assert!(admission.begin_enqueue().is_err());
                }
                ("dispatch", "file") => {
                    assert_eq!(admission.phase().unwrap(), AdmissionPhase::JobBound)
                }
                ("dispatch", "directory") => {
                    assert_eq!(admission.phase().unwrap(), AdmissionPhase::DispatchUnknown);
                    assert!(admission.begin_dispatch().is_err());
                }
                ("reserve-only", "file") => assert_eq!(
                    fence.as_ref().unwrap().state().unwrap(),
                    ExecutionState::Ready
                ),
                ("reserve-only", "directory") => {
                    assert_eq!(
                        fence.as_ref().unwrap().state().unwrap(),
                        ExecutionState::Unknown
                    );
                    assert!(admission.reserve_execution().is_err());
                }
                _ => unreachable!(),
            }
        }
    }
}
