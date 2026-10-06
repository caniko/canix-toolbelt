use canix_toolbelt::operator::{OperatorConfig, Outcome, ServiceManager, Stage, StageResult, run};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Services {
    calls: Vec<String>,
    failures: BTreeMap<String, usize>,
    interrupt_after: Option<String>,
    restore_fails: bool,
    start_fails: bool,
}
impl ServiceManager for Services {
    fn active(&mut self, unit: &str) -> io::Result<bool> {
        Ok(unit == "worker.service")
    }
    fn stop(&mut self, unit: &str) -> io::Result<()> {
        self.calls.push(format!("stop:{unit}"));
        Ok(())
    }
    fn restore(&mut self, unit: &str) -> io::Result<()> {
        self.calls.push(format!("restore:{unit}"));
        if self.restore_fails {
            return Err(io::Error::other("restore failed"));
        }
        Ok(())
    }
    fn start(&mut self, unit: &str, cancelled: &AtomicBool) -> io::Result<StageResult> {
        self.calls.push(format!("start:{unit}"));
        if self.start_fails {
            return Err(io::Error::other("start failed"));
        }
        if self.interrupt_after.as_deref() == Some(unit) {
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
            "start:one.service",
            "start:two.service",
            "restore:worker.service"
        ]
    );
    assert!(!dir.path().join("requested").exists());
    assert!(!dir.path().join("running").exists());
}
#[test]
fn interruption_resumes_checkpoints_and_rejects_changed_stage_contract() {
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
    let mut changed = config.clone();
    changed.stages.reverse();
    assert!(run(&changed, &mut Services::default(), &AtomicBool::new(false)).is_err());
    let mut resumed = Services::default();
    assert_eq!(
        run(&config, &mut resumed, &AtomicBool::new(false)).unwrap(),
        Outcome::Succeeded
    );
    assert!(!resumed.calls.contains(&"start:one.service".into()));
    assert!(resumed.calls.contains(&"start:two.service".into()));
}

#[test]
fn start_errors_restore_workers_and_keep_recovery_intent() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut services = Services {
        start_fails: true,
        ..Default::default()
    };
    assert!(run(&config, &mut services, &AtomicBool::new(false)).is_err());
    assert!(config.request_path.exists());
    assert!(!config.sentinel_path.exists());
    assert!(services.calls.contains(&"restore:worker.service".into()));
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
