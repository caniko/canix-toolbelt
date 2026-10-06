#![cfg(target_os = "linux")]

use canix_toolbelt_roborev_worker::{
    Binding,
    admission::{Admission, AdmissionBinding, AdmissionPhase, JobIdentity},
    execution::ExecutionBinding,
    offline::{BoundaryEvidence, OfflineReceipt},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};

const OUTPUT: &[u8] = b"{\"type\":\"text\",\"part\":{\"type\":\"text\",\"text\":\"fixture\"}}\n";

fn private() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn admitted(root: &Path, executions: &Path) -> (Admission, ExecutionBinding, JobIdentity) {
    let binding = AdmissionBinding {
        request: Binding {
            request_url: "https://github.com/example/repo/pull/1".into(),
            request_id: "completion-fixture".into(),
            authorized_request_sha256: "a".repeat(64),
            execution_policy_sha256: "b".repeat(64),
            base: "c".repeat(40),
            head: "d".repeat(40),
        },
        controller_id: "e".repeat(64),
        daemon_identity_sha256: "f".repeat(64),
    };
    let job = JobIdentity {
        id: 1,
        uuid: "01234567-89ab-4cde-8fab-0123456789ab".into(),
    };
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
    let admission = Admission::register(root, &binding).unwrap();
    admission.begin_enqueue().unwrap();
    admission.bind_job(&job).unwrap();
    admission.begin_dispatch().unwrap();
    admission
        .register_execution(executions, &execution)
        .unwrap();
    drop(admission.reserve_execution().unwrap());
    (admission, execution, job)
}

// This is fixture evidence for retention integrity, not a claimed native worker
// pass. Production must obtain the original receipt from the live verifier.
fn receipt(binding: &ExecutionBinding, output: &[u8]) -> OfflineReceipt {
    OfflineReceipt {
        scope: "offline-envelope-only; not forge acceptance or daemon authentication".into(),
        binding: binding.clone(),
        boundary: BoundaryEvidence {
            pid: 123,
            start_ticks: 456,
            invocation_id: "4".repeat(32),
            cgroup: "/fixture".into(),
            no_new_privileges: true,
            seccomp_filters: 1,
            capabilities: [0; 5],
            namespaces: BTreeMap::new(),
            mounts: BTreeMap::new(),
            scratch_bytes: 1024,
            scratch_inodes: 64,
        },
        backend_success: true,
        cgroup_empty: true,
        stdout_sha256: format!("{:x}", Sha256::digest(output)),
        stderr_sha256: format!("{:x}", Sha256::digest([])),
    }
}

fn source(root: &Path, receipt: &OfflineReceipt, output: &[u8]) {
    write(
        &root.join("receipt.json"),
        &serde_json::to_vec(receipt).unwrap(),
    );
    write(&root.join("stdout"), output);
    write(&root.join("stderr"), b"");
}

#[test]
fn original_complete_output_survives_source_loss_and_cannot_authorize_another_backend() {
    let state = private();
    let executions = private();
    let results = private();
    let retained = private();
    let (admission, binding, job) = admitted(state.path(), executions.path());
    let receipt = receipt(&binding, OUTPUT);
    source(results.path(), &receipt, OUTPUT);
    admission
        .retain_offline_completion(retained.path(), results.path(), &receipt)
        .unwrap();
    fs::remove_dir_all(results.path()).unwrap();
    let recovered: Admission =
        serde_json::from_slice(&serde_json::to_vec(&admission).unwrap()).unwrap();
    assert_eq!(
        recovered.phase().unwrap(),
        AdmissionPhase::CompletionRetained
    );
    for _ in 0..2 {
        assert_eq!(
            recovered
                .retained_offline_output(&binding, &job, true)
                .unwrap(),
            OUTPUT
        );
        assert!(recovered.reserve_execution().is_err());
        assert!(recovered.begin_enqueue().is_err());
        assert!(recovered.begin_dispatch().is_err());
    }
    assert!(
        recovered
            .retain_offline_completion(retained.path(), results.path(), &receipt)
            .is_err()
    );
    assert!(
        recovered
            .retained_offline_output(&binding, &job, false)
            .is_err()
    );
    for field in ["input", "backend", "job", "execution", "comparison"] {
        let mut different = binding.clone();
        match field {
            "input" => different.input_manifest_sha256 = "5".repeat(64),
            "backend" => different.backend_manifest_sha256 = "5".repeat(64),
            "job" => different.job_uuid = "11234567-89ab-4cde-8fab-0123456789ab".into(),
            "execution" => different.execution_id = "5".repeat(64),
            "comparison" => different.request.head = "5".repeat(40),
            _ => unreachable!(),
        }
        assert!(
            recovered
                .retained_offline_output(&different, &job, true)
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn missing_changed_or_replaced_completion_never_recreates_output() {
    for case in [
        "missing",
        "changed",
        "replacement",
        "directory",
        "receipt",
        "public",
        "hardlink",
        "fence",
    ] {
        let state = private();
        let executions = private();
        let results = private();
        let retained = private();
        let (admission, binding, job) = admitted(state.path(), executions.path());
        let receipt = receipt(&binding, OUTPUT);
        source(results.path(), &receipt, OUTPUT);
        admission
            .retain_offline_completion(retained.path(), results.path(), &receipt)
            .unwrap();
        let entry = retained.path().join(binding.request.key());
        let output = entry.join("stdout");
        match case {
            "missing" => fs::remove_file(&output).unwrap(),
            "changed" => write(&output, b"changed"),
            "replacement" => {
                fs::rename(&output, entry.join("original.stdout")).unwrap();
                write(&output, OUTPUT);
            }
            "directory" => {
                fs::rename(&entry, retained.path().join("original")).unwrap();
                fs::create_dir(&entry).unwrap();
                fs::set_permissions(&entry, fs::Permissions::from_mode(0o700)).unwrap();
                source(&entry, &receipt, OUTPUT);
            }
            "receipt" => write(&entry.join("receipt.json"), b"{}"),
            "public" => fs::set_permissions(&output, fs::Permissions::from_mode(0o644)).unwrap(),
            "hardlink" => fs::hard_link(&output, entry.join("shared.stdout")).unwrap(),
            "fence" => fs::remove_dir_all(executions.path().join(binding.request.key())).unwrap(),
            _ => unreachable!(),
        }
        assert!(
            admission
                .retained_offline_output(&binding, &job, true)
                .is_err(),
            "{case}"
        );
        assert!(admission.reserve_execution().is_err(), "{case}");
        assert!(
            admission
                .retain_offline_completion(retained.path(), results.path(), &receipt)
                .is_err(),
            "{case}"
        );
    }
}

#[test]
fn failed_incomplete_or_interrupted_retention_stays_unknown_without_retry() {
    for case in [
        "backend_failed",
        "not_empty",
        "truncated",
        "empty",
        "install_collision",
    ] {
        let state = private();
        let executions = private();
        let results = private();
        let retained = private();
        let (admission, binding, job) = admitted(state.path(), executions.path());
        let mut receipt = receipt(&binding, OUTPUT);
        match case {
            "backend_failed" => receipt.backend_success = false,
            "not_empty" => receipt.cgroup_empty = false,
            "empty" => receipt.stdout_sha256 = format!("{:x}", Sha256::digest([])),
            "install_collision" => {
                let entry = retained.path().join(binding.request.key());
                fs::create_dir(&entry).unwrap();
                fs::set_permissions(entry, fs::Permissions::from_mode(0o700)).unwrap();
            }
            "truncated" => {}
            _ => unreachable!(),
        }
        source(
            results.path(),
            &receipt,
            if case == "empty" { b"" } else { OUTPUT },
        );
        if case == "truncated" {
            write(&results.path().join("stdout"), b"truncated");
        }
        assert!(
            admission
                .retain_offline_completion(retained.path(), results.path(), &receipt)
                .is_err(),
            "{case}"
        );
        assert_eq!(
            admission.phase().unwrap(),
            AdmissionPhase::CompletionRetentionUnknown
        );
        source(results.path(), &self::receipt(&binding, OUTPUT), OUTPUT);
        assert!(
            admission
                .retain_offline_completion(retained.path(), results.path(), &receipt)
                .is_err(),
            "{case}"
        );
        assert!(
            admission
                .retained_offline_output(&binding, &job, true)
                .is_err(),
            "{case}"
        );
        assert!(admission.reserve_execution().is_err(), "{case}");
    }
}
