#![cfg(all(feature = "roborev-execution-tests", target_os = "linux"))]

use canix_toolbelt_roborev_worker::{
    Binding,
    execution::{ExecutionBinding, ExecutionFence, ExecutionState},
};
use std::{
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

fn private_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
fn registration_and_recovery_reject_relative_paths_before_mutation() {
    let root = private_root();
    let relative = root.path().strip_prefix(Path::new("/")).unwrap();
    assert!(
        ExecutionFence::register(relative, &binding())
            .unwrap_err()
            .to_string()
            .contains("absolute")
    );
    assert!(
        ExecutionFence::load(relative, &binding())
            .unwrap_err()
            .to_string()
            .contains("absolute")
    );
    assert!(!root.path().join(binding().request.key()).exists());
    let fence = ExecutionFence::register(root.path(), &binding()).unwrap();
    let mut serialized = serde_json::to_value(&fence).unwrap();
    serialized["anchor"]["entry"] = relative
        .join(binding().request.key())
        .display()
        .to_string()
        .into();
    let restored: ExecutionFence = serde_json::from_value(serialized).unwrap();
    assert!(
        restored
            .reserve()
            .unwrap_err()
            .to_string()
            .contains("absolute")
    );
}

fn binding() -> ExecutionBinding {
    ExecutionBinding {
        request: Binding {
            request_url: "https://forge.invalid/owner/repo/pull/1".into(),
            request_id: "request-one".into(),
            authorized_request_sha256: "a".repeat(64),
            execution_policy_sha256: "b".repeat(64),
            base: "c".repeat(40),
            head: "d".repeat(40),
        },
        controller_id: "e".repeat(64),
        daemon_identity_sha256: "f".repeat(64),
        job_id: 1,
        job_uuid: "01234567-89ab-4cde-8fab-0123456789ab".into(),
        execution_id: "1".repeat(64),
        input_manifest_sha256: "2".repeat(64),
        backend_manifest_sha256: "3".repeat(64),
    }
}

struct FixtureChild {
    child: Child,
    ready: mpsc::Receiver<std::io::Result<()>>,
    output: Arc<Mutex<Vec<u8>>>,
    reaped: bool,
}

impl FixtureChild {
    fn start(root: &Path, binding: &ExecutionBinding, suppress_ready: bool) -> Self {
        let path = root.join("binding.json");
        fs::write(&path, serde_json::to_vec(binding).unwrap()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_roborev-execution-fixture"));
        command
            .env_clear()
            .args([root, &path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if suppress_ready {
            command.env("CANIX_EXECUTION_FIXTURE_NO_READY", "1");
        }
        let mut child = command.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let captured = output.clone();
        let (sender, ready) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| {
                for _ in 0..4096 {
                    let mut byte = [0u8; 1];
                    if stdout.read(&mut byte)? == 0 {
                        return Ok(());
                    }
                    captured.lock().unwrap().push(byte[0]);
                    if byte[0] == b'\n' {
                        return Ok(());
                    }
                }
                Err(std::io::Error::other("oversized fixture readiness"))
            })();
            let _ = sender.send(result);
        });
        Self {
            child,
            ready,
            output,
            reaped: false,
        }
    }

    fn wait_ready(&mut self, timeout: Duration) -> anyhow::Result<()> {
        let received = self.ready.recv_timeout(timeout);
        let bytes = self.output.lock().unwrap().clone();
        let output = String::from_utf8_lossy(&bytes);
        anyhow::ensure!(
            received.is_ok(),
            "fixture readiness timeout/disconnect; stdout: {output:?}"
        );
        received??;
        anyhow::ensure!(
            output == "reserved\n",
            "invalid fixture readiness: {output:?}"
        );
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        if self.reaped {
            return Ok(());
        }
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.child.try_wait()?.is_some() {
                self.reaped = true;
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "fixture failed to exit after kill"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("fixture cleanup failed: {error:#}");
        }
    }
}

#[test]
fn reservation_is_durable_before_dispatch_and_cannot_be_reissued() {
    let root = private_root();
    let binding = binding();
    let fence = ExecutionFence::register(root.path(), &binding).unwrap();
    assert_eq!(fence.state().unwrap(), ExecutionState::Ready);
    let reservation = fence.reserve().unwrap();
    assert_eq!(reservation.binding(), &binding);
    assert!(fence.reserve().is_err());
    drop(reservation);
    assert_eq!(fence.state().unwrap(), ExecutionState::Unknown);
    assert!(fence.reserve().is_err());
    let recovered = ExecutionFence::load(root.path(), &binding).unwrap();
    assert_eq!(recovered.state().unwrap(), ExecutionState::Unknown);
    assert!(recovered.reserve().is_err());
}

#[test]
fn daemon_restart_or_changed_comparison_cannot_open_another_backend_generation() {
    let root = private_root();
    let binding = binding();
    let fence = ExecutionFence::register(root.path(), &binding).unwrap();
    drop(fence.reserve().unwrap());
    for field in [
        "head",
        "base",
        "policy",
        "controller",
        "daemon",
        "job",
        "uuid",
        "execution",
        "input",
        "backend",
    ] {
        let mut different = binding.clone();
        match field {
            "head" => different.request.head = "4".repeat(40),
            "base" => different.request.base = "4".repeat(40),
            "policy" => different.request.execution_policy_sha256 = "4".repeat(64),
            "controller" => different.controller_id = "4".repeat(64),
            "daemon" => different.daemon_identity_sha256 = "4".repeat(64),
            "job" => different.job_id = 2,
            "uuid" => different.job_uuid = "11234567-89ab-4cde-8fab-0123456789ab".into(),
            "execution" => different.execution_id = "4".repeat(64),
            "input" => different.input_manifest_sha256 = "4".repeat(64),
            "backend" => different.backend_manifest_sha256 = "4".repeat(64),
            _ => unreachable!(),
        }
        assert!(
            ExecutionFence::load(root.path(), &different).is_err(),
            "{field}"
        );
        assert!(
            ExecutionFence::register(root.path(), &different).is_err(),
            "{field}"
        );
    }
    assert_eq!(fence.state().unwrap(), ExecutionState::Unknown);
}

#[test]
fn missing_or_replaced_state_never_recreates_a_reservation() {
    for case in ["ledger", "lock", "replacement", "directory"] {
        let root = private_root();
        let binding = binding();
        let fence = ExecutionFence::register(root.path(), &binding).unwrap();
        drop(fence.reserve().unwrap());
        let entry = root.path().join(binding.request.key());
        match case {
            "ledger" => fs::remove_file(entry.join("execution.json")).unwrap(),
            "lock" => fs::remove_file(entry.join("execution.lock")).unwrap(),
            "replacement" => {
                fs::rename(entry.join("execution.lock"), entry.join("old.lock")).unwrap();
                fs::write(entry.join("execution.lock"), [0u8; 32]).unwrap();
                fs::set_permissions(
                    entry.join("execution.lock"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            "directory" => {
                fs::rename(&entry, root.path().join("old-entry")).unwrap();
                fs::create_dir(&entry).unwrap();
                fs::set_permissions(&entry, fs::Permissions::from_mode(0o700)).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(fence.state().is_err(), "{case}");
        assert!(fence.reserve().is_err(), "{case}");
        assert!(
            ExecutionFence::load(root.path(), &binding).is_err(),
            "{case}"
        );
        assert!(
            !entry.join("execution.lock").exists() || case == "replacement" || case == "ledger"
        );
    }
}

#[test]
fn killed_controller_releases_contention_but_retains_unknown() {
    let root = private_root();
    let binding = binding();
    let fence = ExecutionFence::register(root.path(), &binding).unwrap();
    let mut child = FixtureChild::start(root.path(), &binding, false);
    child.wait_ready(Duration::from_secs(5)).unwrap();
    assert!(
        fence
            .reserve()
            .unwrap_err()
            .to_string()
            .contains("another controller")
    );
    child.stop().unwrap();
    let restored = ExecutionFence::load(root.path(), &binding).unwrap();
    assert_eq!(restored.state().unwrap(), ExecutionState::Unknown);
    assert!(
        restored
            .reserve()
            .unwrap_err()
            .to_string()
            .contains("replay forbidden")
    );
}

#[test]
fn stalled_fixture_readiness_has_a_deadline_and_is_reaped() {
    let root = private_root();
    let binding = binding();
    ExecutionFence::register(root.path(), &binding).unwrap();
    let mut child = FixtureChild::start(root.path(), &binding, true);
    let pid = nix::unistd::Pid::from_raw(child.child.id() as i32);
    assert!(
        child
            .wait_ready(Duration::from_millis(100))
            .unwrap_err()
            .to_string()
            .contains("stdout")
    );
    drop(child);
    assert!(matches!(
        nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)),
        Err(nix::errno::Errno::ECHILD)
    ));
}

#[test]
fn restored_registration_and_invalid_ledgers_fail_closed() {
    for case in [
        "schema",
        "extra",
        "truncated",
        "symlink",
        "hardlink",
        "public",
    ] {
        let root = private_root();
        let binding = binding();
        let fence = ExecutionFence::register(root.path(), &binding).unwrap();
        let serialized = serde_json::to_vec(&fence).unwrap();
        drop(fence.reserve().unwrap());
        let restored: ExecutionFence = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored.state().unwrap(), ExecutionState::Unknown);
        let ledger = root
            .path()
            .join(binding.request.key())
            .join("execution.json");
        match case {
            "schema" | "extra" => {
                let mut journal: serde_json::Value =
                    serde_json::from_slice(&fs::read(&ledger).unwrap()).unwrap();
                if case == "schema" {
                    journal["schema_version"] = 1.into();
                } else {
                    journal["extra"] = true.into();
                }
                fs::write(&ledger, serde_json::to_vec(&journal).unwrap()).unwrap();
            }
            "truncated" => fs::write(&ledger, b"{").unwrap(),
            "symlink" => {
                let target = root.path().join("external.json");
                fs::rename(&ledger, &target).unwrap();
                symlink(target, &ledger).unwrap();
            }
            "hardlink" => fs::hard_link(&ledger, root.path().join("shared.json")).unwrap(),
            "public" => fs::set_permissions(&ledger, fs::Permissions::from_mode(0o644)).unwrap(),
            _ => unreachable!(),
        }
        assert!(restored.state().is_err(), "{case}");
        assert!(restored.reserve().is_err(), "{case}");
        assert!(
            ExecutionFence::load(root.path(), &binding).is_err(),
            "{case}"
        );
    }
}
