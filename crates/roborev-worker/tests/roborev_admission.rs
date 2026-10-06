#![cfg(target_os = "linux")]

use canix_toolbelt_roborev_worker::{
    Binding,
    admission::{Admission, AdmissionBinding, AdmissionPhase, JobIdentity},
    execution::{ExecutionBinding, ExecutionState},
};
use std::{fs, os::unix::fs::PermissionsExt};

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn binding() -> AdmissionBinding {
    AdmissionBinding {
        request: Binding {
            request_url: "https://github.com/example/repo/pull/1".into(),
            request_id: "original-request".into(),
            authorized_request_sha256: "a".repeat(64),
            execution_policy_sha256: "b".repeat(64),
            base: "c".repeat(40),
            head: "d".repeat(40),
        },
        controller_id: "e".repeat(64),
        daemon_identity_sha256: "f".repeat(64),
    }
}

fn job() -> JobIdentity {
    JobIdentity {
        id: 1,
        uuid: "01234567-89ab-4cde-8fab-0123456789ab".into(),
    }
}

fn execution(admission: &AdmissionBinding) -> ExecutionBinding {
    ExecutionBinding {
        request: admission.request.clone(),
        controller_id: admission.controller_id.clone(),
        daemon_identity_sha256: admission.daemon_identity_sha256.clone(),
        job_id: job().id,
        job_uuid: job().uuid,
        execution_id: "1".repeat(64),
        input_manifest_sha256: "2".repeat(64),
        backend_manifest_sha256: "3".repeat(64),
    }
}

#[test]
fn no_effect_before_admission_and_binding_and_no_repeat_after_reply_loss() {
    let root = root();
    let binding = binding();
    let admission = Admission::register(root.path(), &binding).unwrap();
    assert_eq!(admission.phase().unwrap(), AdmissionPhase::Admitted);
    assert!(admission.bind_job(&job()).is_err());
    assert!(admission.begin_dispatch().is_err());
    admission.begin_enqueue().unwrap();
    // This represents a dropped response after an effect may already have happened.
    let restored: Admission =
        serde_json::from_slice(&serde_json::to_vec(&admission).unwrap()).unwrap();
    assert_eq!(restored.phase().unwrap(), AdmissionPhase::EnqueueUnknown);
    assert!(restored.begin_enqueue().is_err());
    assert!(restored.begin_dispatch().is_err());
    restored.bind_job(&job()).unwrap();
    restored.bind_job(&job()).unwrap();
    assert_eq!(restored.phase().unwrap(), AdmissionPhase::JobBound);
    assert!(restored.bind_job(&JobIdentity { id: 2, ..job() }).is_err());
    assert!(Admission::register(root.path(), &binding).is_err());
    restored.begin_dispatch().unwrap();
    assert_eq!(restored.phase().unwrap(), AdmissionPhase::DispatchUnknown);
    assert!(restored.begin_dispatch().is_err());
    assert!(restored.begin_enqueue().is_err());
}

#[test]
fn actual_adapter_inputs_can_be_bound_after_dispatch_but_before_backend_reservation() {
    let state = root();
    let executions = root();
    let binding = binding();
    let admission = Admission::register(state.path(), &binding).unwrap();
    let execution = execution(&binding);
    assert!(
        admission
            .register_execution(executions.path(), &execution)
            .is_err()
    );
    admission.begin_enqueue().unwrap();
    admission.bind_job(&job()).unwrap();
    admission.begin_dispatch().unwrap();
    let fence = admission
        .register_execution(executions.path(), &execution)
        .unwrap();
    assert_eq!(fence.state().unwrap(), ExecutionState::Ready);
    assert_eq!(
        admission.phase().unwrap(),
        AdmissionPhase::ExecutionRegistered
    );
    let reserved = admission.reserve_execution().unwrap();
    assert_eq!(reserved.binding(), &execution);
    drop(reserved);
    assert_eq!(fence.state().unwrap(), ExecutionState::Unknown);
    assert!(admission.reserve_execution().is_err());
    assert!(
        admission
            .register_execution(executions.path(), &execution)
            .is_err()
    );
}

#[test]
fn independent_admission_rejects_replacement_registration_after_fence_loss() {
    let state = root();
    let executions = root();
    let binding = binding();
    let admission = Admission::register(state.path(), &binding).unwrap();
    admission.begin_enqueue().unwrap();
    admission.bind_job(&job()).unwrap();
    admission.begin_dispatch().unwrap();
    admission
        .register_execution(executions.path(), &execution(&binding))
        .unwrap();
    drop(admission.reserve_execution().unwrap());
    fs::remove_dir_all(executions.path().join(binding.request.key())).unwrap();
    assert!(admission.reserve_execution().is_err());
    assert!(
        admission
            .register_execution(executions.path(), &execution(&binding))
            .is_err()
    );
    assert!(!executions.path().join(binding.request.key()).exists());
}

#[test]
fn interrupted_fence_registration_is_never_retried() {
    let state = root();
    let executions = root();
    let binding = binding();
    let admission = Admission::register(state.path(), &binding).unwrap();
    admission.begin_enqueue().unwrap();
    admission.bind_job(&job()).unwrap();
    admission.begin_dispatch().unwrap();
    // A concrete register failure happens after the independent journal records UNKNOWN.
    fs::create_dir(executions.path().join(binding.request.key())).unwrap();
    assert!(
        admission
            .register_execution(executions.path(), &execution(&binding))
            .is_err()
    );
    assert_eq!(
        admission.phase().unwrap(),
        AdmissionPhase::ExecutionRegistrationUnknown
    );
    fs::remove_dir(executions.path().join(binding.request.key())).unwrap();
    assert!(
        admission
            .register_execution(executions.path(), &execution(&binding))
            .is_err()
    );
    assert!(admission.reserve_execution().is_err());
    assert!(!executions.path().join(binding.request.key()).exists());
}

#[test]
fn wrong_execution_identities_do_not_consume_original_admission() {
    for field in ["request", "controller", "daemon", "job", "uuid"] {
        let state = root();
        let executions = root();
        let binding = binding();
        let admission = Admission::register(state.path(), &binding).unwrap();
        admission.begin_enqueue().unwrap();
        admission.bind_job(&job()).unwrap();
        admission.begin_dispatch().unwrap();
        let mut wrong = execution(&binding);
        match field {
            "request" => wrong.request.head = "4".repeat(40),
            "controller" => wrong.controller_id = "4".repeat(64),
            "daemon" => wrong.daemon_identity_sha256 = "4".repeat(64),
            "job" => wrong.job_id = 2,
            "uuid" => wrong.job_uuid = "11234567-89ab-4cde-8fab-0123456789ab".into(),
            _ => unreachable!(),
        }
        assert!(
            admission
                .register_execution(executions.path(), &wrong)
                .is_err(),
            "{field}"
        );
        assert_eq!(admission.phase().unwrap(), AdmissionPhase::DispatchUnknown);
        assert!(!executions.path().join(binding.request.key()).exists());
    }
}

#[test]
fn missing_replaced_or_invalid_admission_retains_no_effect_authority() {
    for case in ["journal", "anchor", "replacement", "invalid", "public"] {
        let state = root();
        let binding = binding();
        let admission = Admission::register(state.path(), &binding).unwrap();
        admission.begin_enqueue().unwrap();
        let entry = state.path().join(binding.request.key());
        let journal = entry.join("admission.json");
        match case {
            "journal" => fs::remove_file(&journal).unwrap(),
            "anchor" => fs::remove_file(entry.join("admission.lock")).unwrap(),
            "replacement" => {
                fs::rename(&entry, state.path().join("original")).unwrap();
                Admission::register(state.path(), &binding).unwrap();
            }
            "invalid" => fs::write(&journal, b"{}").unwrap(),
            "public" => fs::set_permissions(&journal, fs::Permissions::from_mode(0o644)).unwrap(),
            _ => unreachable!(),
        }
        assert!(admission.phase().is_err(), "{case}");
        assert!(admission.begin_enqueue().is_err(), "{case}");
        assert!(admission.bind_job(&job()).is_err(), "{case}");
        assert!(admission.begin_dispatch().is_err(), "{case}");
    }
}
