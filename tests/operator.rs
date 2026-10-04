#![cfg(unix)]

use canix_toolbelt::operator::{run, OperatorConfig, Outcome, ServiceManager, Stage, StageResult};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Default)]
struct Services {
    calls: Vec<String>,
    failures: BTreeMap<String, usize>,
    interrupt_after: Option<String>,
    interrupt_on_start: Option<usize>,
    restore_fails: bool,
    start_fails: bool,
    stop_fails: bool,
    stage_stop_fails: bool,
    restore_fail_unit: Option<String>,
    worker_stopped: Option<Arc<AtomicBool>>,
}

impl ServiceManager for Services {
    fn active(&mut self, unit: &str) -> io::Result<bool> {
        Ok(matches!(unit, "worker.service" | "other.service"))
    }

    fn stop(&mut self, unit: &str) -> io::Result<()> {
        self.calls.push(format!("stop:{unit}"));
        if self.stop_fails || (self.stage_stop_fails && unit == "one.service") {
            return Err(io::Error::other("stop failed"));
        }
        if unit == "worker.service" {
            if let Some(stopped) = &self.worker_stopped {
                stopped.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    fn restore(&mut self, unit: &str) -> io::Result<()> {
        self.calls.push(format!("restore:{unit}"));
        if self.restore_fails || self.restore_fail_unit.as_deref() == Some(unit) {
            return Err(io::Error::other("restore failed"));
        }
        Ok(())
    }

    fn start(&mut self, unit: &str, cancelled: &AtomicBool) -> io::Result<StageResult> {
        self.calls.push(format!("start:{unit}"));
        if self.start_fails {
            return Err(io::Error::other("start failed"));
        }
        let starts = self
            .calls
            .iter()
            .filter(|call| call.starts_with("start:"))
            .count();
        if self.interrupt_after.as_deref() == Some(unit) || self.interrupt_on_start == Some(starts)
        {
            cancelled.store(true, Ordering::SeqCst);
        }
        let failures = self.failures.entry(unit.into()).or_default();
        let success = *failures == 0;
        *failures = failures.saturating_sub(1);
        Ok(StageResult {
            success,
            result: if success { "success" } else { "exit-code" }.into(),
            exit_status: if success { 0 } else { 1 },
        })
    }
}

fn config(root: &std::path::Path) -> OperatorConfig {
    OperatorConfig {
        name: "fixture".into(),
        contract_id: "fixture-v1".into(),
        state_dir: root.into(),
        request_path: root.join("requested"),
        sentinel_path: root.join("running"),
        max_attempts: 2,
        retry_delays: vec!["0s".into()],
        quiesce_units: vec!["worker.service".into()],
        stages: vec![
            Stage {
                name: "one".into(),
                unit: "one.service".into(),
            },
            Stage {
                name: "two".into(),
                unit: "two.service".into(),
            },
        ],
    }
}

#[test]
fn retries_and_restores_only_previously_active_workers() {
    let dir = tempfile::tempdir().unwrap();
    let mut services = Services::default();
    services.failures.insert("one.service".into(), 1);
    assert_eq!(
        run(&config(dir.path()), &mut services, &AtomicBool::new(false)).unwrap(),
        Outcome::Succeeded
    );
    assert_eq!(
        services.calls,
        [
            "stop:worker.service",
            "start:one.service",
            "stop:one.service",
            "start:one.service",
            "start:two.service",
            "restore:worker.service"
        ]
    );
    assert!(!dir.path().join("requested").exists());
    assert!(!dir.path().join("running").exists());
}

#[test]
fn interruption_resumes_checkpoints_and_rejects_changed_contract() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut services = Services {
        interrupt_after: Some("two.service".into()),
        ..Default::default()
    };
    assert_eq!(
        run(&config, &mut services, &AtomicBool::new(false)).unwrap(),
        Outcome::Interrupted
    );
    assert!(config.request_path.exists());
    assert!(services.calls.contains(&"restore:worker.service".into()));
    let original = std::fs::read(config.state_dir.join("state.json")).unwrap();
    for changed in [
        {
            let mut c = config.clone();
            c.stages.reverse();
            c
        },
        {
            let mut c = config.clone();
            c.contract_id = "new-package-same-units".into();
            c
        },
    ] {
        let mut refused = Services::default();
        assert!(run(&changed, &mut refused, &AtomicBool::new(false)).is_err());
        assert!(refused.calls.is_empty());
        assert_eq!(
            std::fs::read(config.state_dir.join("state.json")).unwrap(),
            original
        );
        assert!(config.request_path.exists());
    }
    let mut resumed = Services::default();
    assert_eq!(
        run(&config, &mut resumed, &AtomicBool::new(false)).unwrap(),
        Outcome::Succeeded
    );
    assert!(!resumed.calls.contains(&"start:one.service".into()));
    assert!(resumed.calls.contains(&"start:two.service".into()));
}

#[test]
fn quiesce_errors_restore_workers_and_keep_recovery_intent() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut services = Services {
        stop_fails: true,
        ..Default::default()
    };
    assert!(run(&config, &mut services, &AtomicBool::new(false)).is_err());
    assert!(config.request_path.exists());
    assert!(!config.sentinel_path.exists());
    assert!(services.calls.contains(&"restore:worker.service".into()));
}

#[test]
fn service_manager_errors_exhaust_the_persisted_retry_budget() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut services = Services {
        start_fails: true,
        ..Default::default()
    };
    assert_eq!(
        run(&config, &mut services, &AtomicBool::new(false)).unwrap(),
        Outcome::Failed
    );
    for unit in ["one.service", "two.service"] {
        assert_eq!(
            services
                .calls
                .iter()
                .filter(|call| **call == format!("start:{unit}"))
                .count(),
            2
        );
    }
    assert!(!config.request_path.exists());
    assert!(!config.sentinel_path.exists());
    assert!(services.calls.contains(&"restore:worker.service".into()));
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("state.json")).unwrap()).unwrap();
    assert_eq!(state["failures"], serde_json::json!({"one": 2, "two": 2}));
}

#[test]
fn unconfirmed_stage_stop_retains_fences_and_does_not_restore_workers() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut services = Services {
        interrupt_after: Some("one.service".into()),
        stage_stop_fails: true,
        ..Default::default()
    };
    assert!(run(&config, &mut services, &AtomicBool::new(false)).is_err());
    assert!(config.request_path.exists());
    assert!(config.sentinel_path.exists());
    assert!(!services
        .calls
        .iter()
        .any(|call| call.starts_with("restore:")));
    assert!(canix_toolbelt::operator::cancel(&config).is_err());
    let mut recovered = Services::default();
    assert_eq!(
        run(&config, &mut recovered, &AtomicBool::new(false)).unwrap(),
        Outcome::Succeeded
    );
    assert_eq!(recovered.calls[0], "stop:one.service");
    assert!(!config.sentinel_path.exists());
}

#[test]
fn readiness_follows_worker_quiescence_and_the_durable_fence() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let stopped = Arc::new(AtomicBool::new(false));
    let notified = AtomicBool::new(false);
    let mut services = Services {
        worker_stopped: Some(Arc::clone(&stopped)),
        ..Default::default()
    };
    canix_toolbelt::operator::run_with_ready(
        &config,
        &mut services,
        &AtomicBool::new(false),
        || {
            assert!(stopped.load(Ordering::SeqCst));
            assert!(config.request_path.exists());
            assert!(config.sentinel_path.exists());
            notified.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .unwrap();
    assert!(notified.load(Ordering::SeqCst));
}

#[test]
fn terminal_leftover_requests_do_not_create_another_run() {
    for failed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let mut first = Services {
            start_fails: failed,
            ..Default::default()
        };
        let outcome = run(&config, &mut first, &AtomicBool::new(false)).unwrap();
        let path = config.state_dir.join("state.json");
        let before: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // Model a crash between saving the terminal state and marker removal.
        std::fs::write(&config.request_path, "").unwrap();
        let mut resumed = Services::default();
        assert_eq!(
            run(&config, &mut resumed, &AtomicBool::new(false)).unwrap(),
            outcome
        );
        assert!(resumed.calls.is_empty());
        let after: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(before["run_id"], after["run_id"]);
        assert!(!config.request_path.exists());
    }
}

#[test]
fn partial_restoration_never_requiesces_recovered_workers() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.quiesce_units.push("other.service".into());
    let mut first = Services {
        restore_fail_unit: Some("other.service".into()),
        ..Default::default()
    };
    assert!(run(&config, &mut first, &AtomicBool::new(false)).is_err());
    assert!(first.calls.contains(&"restore:worker.service".into()));
    let mut resumed = Services::default();
    assert_eq!(
        run(&config, &mut resumed, &AtomicBool::new(false)).unwrap(),
        Outcome::Succeeded
    );
    assert_eq!(resumed.calls, ["restore:other.service"]);
}

#[test]
fn failed_restoration_remains_owned_and_cannot_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut services = Services {
        restore_fails: true,
        ..Default::default()
    };
    assert!(run(&config, &mut services, &AtomicBool::new(false)).is_err());
    assert!(config.request_path.exists());
    assert!(canix_toolbelt::operator::cancel(&config).is_err());
    let mut resumed = Services::default();
    assert_eq!(
        run(&config, &mut resumed, &AtomicBool::new(false)).unwrap(),
        Outcome::Succeeded
    );
    assert!(!resumed.calls.iter().any(|s| s.starts_with("start:")));
    assert!(resumed.calls.contains(&"restore:worker.service".into()));
}

#[test]
fn kernel_lock_excludes_run_and_cancel_without_editing_markers() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    std::fs::write(&config.request_path, "owned").unwrap();
    let lock = std::fs::File::create(dir.path().join("operator.lock")).unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    assert_eq!(
        run(&config, &mut Services::default(), &AtomicBool::new(false))
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        canix_toolbelt::operator::cancel(&config)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        std::fs::read_to_string(&config.request_path).unwrap(),
        "owned"
    );
    drop(lock);
    canix_toolbelt::operator::cancel(&config).unwrap();
    assert!(!config.request_path.exists());
}

#[test]
fn legacy_interrupted_state_is_preserved_without_running_new_units() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    std::fs::write(&config.request_path, "").unwrap();
    let path = config.state_dir.join("state.json");
    let legacy = br#"{"run_id":"legacy","state":"interrupted","result":"interrupted","current_index":0,"stage_count":2,"report":"legacy-report.json","updated_at":0}"#;
    std::fs::write(&path, legacy).unwrap();
    let mut services = Services::default();
    assert!(run(&config, &mut services, &AtomicBool::new(false)).is_err());
    assert!(services.calls.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), legacy);
    assert!(config.request_path.exists());
}

#[test]
fn recorded_failures_keep_the_retry_budget_across_interruption() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut first = Services {
        interrupt_on_start: Some(2),
        ..Default::default()
    };
    first.failures.insert("one.service".into(), 2);
    assert_eq!(
        run(&config, &mut first, &AtomicBool::new(false)).unwrap(),
        Outcome::Interrupted
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("state.json")).unwrap()).unwrap();
    assert_eq!(state["failures"]["one"], 1);
    let mut resumed = Services::default();
    resumed.failures.insert("one.service".into(), 2);
    assert_eq!(
        run(&config, &mut resumed, &AtomicBool::new(false)).unwrap(),
        Outcome::Failed
    );
    assert_eq!(
        resumed
            .calls
            .iter()
            .filter(|call| *call == "start:one.service")
            .count(),
        1
    );
    assert!(resumed.calls.contains(&"start:two.service".into()));
    assert!(!config.request_path.exists());
}
