#![cfg(feature = "qualification")]

use canix_toolbelt::qualification::{self as q, InventorySpec};
use std::fs;
use tempfile::tempdir;

#[test]
fn inventory_is_deterministic_and_binds_executable_bits() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("a"), b"hello\n").unwrap();
    fs::write(root.path().join("b"), b"world\n").unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["b".into(), "a".into()],
    };
    let first = q::inventory(&spec).unwrap();
    let reversed = InventorySpec {
        paths: vec!["a".into(), "b".into()],
        ..spec.clone()
    };
    assert_eq!(first, q::inventory(&reversed).unwrap());
    assert_eq!(
        first.files["a"].sha256,
        "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.path().join("a"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_ne!(first, q::inventory(&spec).unwrap());
    }
}

#[test]
fn index_verification_rejects_traversal_and_tampering() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("log"), b"hello\n").unwrap();
    let hash = q::sha256_file(&root.path().join("log")).unwrap();
    let index = [("log".to_owned(), hash)].into();
    assert_eq!(q::verify_index(root.path(), &index).unwrap(), 1);
    fs::write(root.path().join("log"), b"changed").unwrap();
    assert!(q::verify_index(root.path(), &index).is_err());
    let traversal = [("../log".to_owned(), "0".repeat(64))].into();
    assert!(q::verify_index(root.path(), &traversal).is_err());
}

#[cfg(unix)]
#[test]
fn inventory_rejects_escaping_links_and_records_internal_link_identity() {
    use std::os::unix::fs::symlink;
    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"secret").unwrap();
    symlink(outside.path().join("secret"), root.path().join("escape")).unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["escape".into()],
    };
    assert!(q::inventory(&spec).is_err());
    fs::write(root.path().join("source"), b"source").unwrap();
    symlink("source", root.path().join("link")).unwrap();
    let snapshot = q::inventory(&InventorySpec {
        paths: vec!["link".into()],
        ..spec
    })
    .unwrap();
    assert_eq!(snapshot.files["link"].link.as_deref(), Some("source"));
}

#[cfg(target_os = "linux")]
fn run_fixture(script: &str, timeout_seconds: u64) -> (tempfile::TempDir, q::Receipt) {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source"), b"original").unwrap();
    fs::write(root.path().join("input"), b"pinned").unwrap();
    let request = q::RunRequest {
        cwd: root.path().into(),
        sources: [(
            "producer".into(),
            InventorySpec {
                root: root.path().into(),
                paths: vec!["source".into()],
            },
        )]
        .into(),
        inputs: [("lock".into(), root.path().join("input"))].into(),
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        environment: Default::default(),
        tools: Default::default(),
        timeout_seconds,
        containment: Some(q::ContainmentPrograms {
            unshare: std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|p| p.join("unshare"))
                .find(|p| p.is_file())
                .unwrap(),
            executor: env!("CARGO_BIN_EXE_canix-toolbelt-qualification-executor").into(),
        }),
    };
    let receipt = q::execute(&request, &root.path().join("run"), &Default::default()).unwrap();
    (root, receipt)
}

#[cfg(target_os = "linux")]
#[test]
fn receipt_accepts_only_completed_unchanged_execution_and_verified_logs() {
    let (root, receipt) = run_fixture("printf stdout; printf stderr >&2", 5);
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "stderr: {}",
        fs::read_to_string(root.path().join("run/stderr.log")).unwrap()
    );
    assert_eq!(
        q::verify_run(&root.path().join("run"), Some(&receipt.composition)).unwrap(),
        receipt
    );
    let mut other = receipt.composition.clone();
    other.inputs.insert("lock".into(), "0".repeat(64));
    assert!(q::verify_run(&root.path().join("run"), Some(&other)).is_err());
    fs::write(root.path().join("run/stdout.log"), b"tampered").unwrap();
    assert!(q::verify_run(&root.path().join("run"), None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn failure_source_change_input_change_and_timeout_are_never_acceptance() {
    for (script, timeout, expected) in [
        ("exit 7", 5, q::RunStatus::Failed),
        ("printf changed > source", 5, q::RunStatus::SourceChanged),
        ("printf changed > input", 5, q::RunStatus::SourceChanged),
        ("sleep 10", 1, q::RunStatus::TimedOut),
    ] {
        let (root, receipt) = run_fixture(script, timeout);
        assert_eq!(receipt.status, expected);
        assert!(q::verify_run(&root.path().join("run"), None).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn cancellation_and_existing_attempt_fail_closed() {
    let (root, receipt) = run_fixture("true", 5);
    let request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    assert!(q::execute(&request, &root.path().join("run"), &Default::default()).is_err());
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let cancelled = q::execute(&request, &root.path().join("cancelled"), &cancel).unwrap();
    assert_eq!(cancelled.status, q::RunStatus::Cancelled);
    assert_ne!(receipt.status, cancelled.status);
}

#[test]
fn cargo_graph_reports_target_kinds_and_shortest_workspace_paths() {
    let metadata = serde_json::json!({
        "packages": [
            {"id":"app", "name":"app", "version":"1.0.0", "source":null},
            {"id":"macro", "name":"derive", "version":"1.0.0", "source":"registry+test"},
            {"id":"parser", "name":"syn", "version":"2.0.119", "source":"registry+test"}
        ],
        "workspace_members":["app"],
        "resolve":{"nodes":[
            {"id":"app", "features":[], "deps":[{"pkg":"macro", "dep_kinds":[{"kind":null,"target":null}]}]},
            {"id":"macro", "features":["full"], "deps":[{"pkg":"parser", "dep_kinds":[{"kind":"build","target":"cfg(windows)"}]}]},
            {"id":"parser", "features":[], "deps":[]}
        ]}
    });
    let graph = q::graph::Metadata::parse(&serde_json::to_vec(&metadata).unwrap()).unwrap();
    let parents = graph.parents("syn", Some("2.0.119")).unwrap();
    assert_eq!(parents[0].parent, "derive");
    assert_eq!(parents[0].path, ["app@1.0.0", "derive@1.0.0"]);
    assert_eq!(parents[0].dependency_kinds[0]["target"], "cfg(windows)");
    assert!(graph.parents("ttf-parser", None).unwrap().is_empty());
    let mut broken = metadata;
    broken["resolve"]["nodes"][1]["deps"][0]["pkg"] = "unknown".into();
    assert!(q::graph::Metadata::parse(&serde_json::to_vec(&broken).unwrap()).is_err());
}

#[test]
fn legacy_receipt_integrity_is_distinct_from_acceptance() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source.json"), b"{\n  \"a\": \"abc\"\n}\n").unwrap();
    fs::write(root.path().join("stdout.log"), b"out").unwrap();
    fs::write(root.path().join("stderr.log"), b"err").unwrap();
    let receipt = serde_json::json!({"exit":6, "sourceUnchanged":true,
        "sourceSha256":q::sha256_file(&root.path().join("source.json")).unwrap(),
        "stdoutSha256":q::sha256_file(&root.path().join("stdout.log")).unwrap(),
        "stderrSha256":q::sha256_file(&root.path().join("stderr.log")).unwrap()});
    fs::write(
        root.path().join("receipt.json"),
        q::json_bytes(&receipt).unwrap(),
    )
    .unwrap();
    let report = q::verify_legacy_run(root.path()).unwrap();
    assert!(!report.acceptable);
    assert_eq!(report.execution_exit, 6);
    fs::write(root.path().join("stderr.log"), b"altered").unwrap();
    assert!(q::verify_legacy_run(root.path()).is_err());
}

#[test]
fn source_archive_preserves_bytes_and_semantics_and_detects_corruption() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/a"), b"source\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a", root.path().join("src/link")).unwrap();
    }
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["src".into()],
    };
    let expected = q::inventory(&spec).unwrap();
    q::export_source(&spec, &root.path().join("source.tar")).unwrap();
    q::verify_source_archive(&root.path().join("source.tar"), &expected).unwrap();
    assert!(q::export_source(&spec, &root.path().join("source.tar")).is_err());
    fs::write(root.path().join("source.tar"), b"broken").unwrap();
    assert!(q::verify_source_archive(&root.path().join("source.tar"), &expected).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn seals_require_matching_compositions_and_preserve_the_gate_set() {
    let (root, receipt) = run_fixture("true", 5);
    let request = q::SealRequest {
        composition: receipt.composition.clone(),
        gates: [("required".into(), root.path().join("run"))].into(),
        expected_receipts: [(
            "required".into(),
            q::sha256_file(&root.path().join("run/receipt.json")).unwrap(),
        )]
        .into(),
        artifacts: [("source".into(), root.path().join("source"))].into(),
    };
    let seal = q::seal(&request, &root.path().join("sealed")).unwrap();
    assert_eq!(seal.gates.len(), 1);
    q::verify_seal(
        &root.path().join("sealed"),
        &request.composition,
        &request.expected_receipts,
    )
    .unwrap();
    assert!(
        q::verify_seal(
            &root.path().join("sealed"),
            &request.composition,
            &[("absent".into(), "0".repeat(64))].into()
        )
        .is_err()
    );
    let mut other = request.clone();
    other
        .composition
        .inputs
        .insert("lock".into(), "0".repeat(64));
    assert!(q::seal(&other, &root.path().join("wrong")).is_err());
    fs::write(
        root.path().join("sealed/gates/required/stdout.log"),
        b"altered",
    )
    .unwrap();
    assert!(
        q::verify_seal(
            &root.path().join("sealed"),
            &request.composition,
            &request.expected_receipts
        )
        .is_err()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn seal_cannot_substitute_a_different_passing_attempt_for_the_selected_gate() {
    let (root, receipt) = run_fixture("true", 5);
    let request = q::SealRequest {
        composition: receipt.composition,
        gates: [("required".into(), root.path().join("run"))].into(),
        expected_receipts: [("required".into(), "0".repeat(64))].into(),
        artifacts: Default::default(),
    };
    assert!(q::seal(&request, &root.path().join("wrong-attempt")).is_err());
}

#[cfg(all(feature = "cli", target_os = "linux"))]
#[test]
fn standalone_cli_runs_verifies_and_reports_failed_legacy_evidence() {
    use std::process::Command;
    let (root, receipt) = run_fixture("true", 5);
    let exe = env!("CARGO_BIN_EXE_canix-toolbelt");
    let output = Command::new(exe)
        .args(["qualification", "verify-run"])
        .arg(root.path().join("run"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: q::Receipt = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual, receipt);
    let help = Command::new(exe)
        .args(["qualification", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("verify-seal"));
}

#[test]
fn graph_delta_detects_same_version_distribution_changes() {
    let document = |source: &str| {
        serde_json::json!({
            "packages":[{"id":"app", "name":"app", "version":"1.0.0", "source":source}],
            "workspace_members":["app"], "resolve":{"nodes":[{"id":"app", "features":[], "deps":[]}]}
        })
    };
    let before =
        q::graph::Metadata::parse(&serde_json::to_vec(&document("git+before")).unwrap()).unwrap();
    let after =
        q::graph::Metadata::parse(&serde_json::to_vec(&document("registry+after")).unwrap())
            .unwrap();
    let delta = before.delta(&after);
    assert_eq!(delta.removed[0].source.as_deref(), Some("git+before"));
    assert_eq!(delta.added[0].source.as_deref(), Some("registry+after"));
}

#[test]
fn parent_path_ties_preserve_the_python_cargo_edge_order() {
    let packages = ["root", "z-first", "a-second", "parent", "dependency"]
        .map(|id| serde_json::json!({"id":id,"name":id,"version":"1.0.0","source":null}));
    let metadata = serde_json::json!({
        "packages": packages,
        "workspace_members":["root"], "resolve":{"nodes":[
            {"id":"root","features":[],"deps":[{"pkg":"z-first","dep_kinds":[]},{"pkg":"a-second","dep_kinds":[]}]},
            {"id":"z-first","features":[],"deps":[{"pkg":"parent","dep_kinds":[]}]},
            {"id":"a-second","features":[],"deps":[{"pkg":"parent","dep_kinds":[]}]},
            {"id":"parent","features":[],"deps":[{"pkg":"dependency","dep_kinds":[]}]},
            {"id":"dependency","features":[],"deps":[]}
        ]}
    });
    let graph = q::graph::Metadata::parse(&serde_json::to_vec(&metadata).unwrap()).unwrap();
    assert_eq!(
        graph.parents("dependency", None).unwrap()[0].path,
        ["root@1.0.0", "z-first@1.0.0", "parent@1.0.0"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn successful_parent_with_running_children_is_not_acceptance() {
    let (root, receipt) = run_fixture("(sleep 1; printf late) & printf done", 5);
    assert_eq!(receipt.status, q::RunStatus::Failed);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(
        fs::read(root.path().join("run/stdout.log")).unwrap(),
        b"done"
    );
    assert!(q::verify_run(&root.path().join("run"), None).is_err());
}

#[test]
fn legacy_source_archives_accept_git_directories_without_losing_file_verification() {
    let root = tempdir().unwrap();
    let path = root.path().join("git-source.tar");
    let mut builder = tar::Builder::new(fs::File::create(&path).unwrap());
    let metadata = format!("52 comment={}\n", "0".repeat(40));
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::XGlobalHeader);
    header.set_size(metadata.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "pax_global_header", metadata.as_bytes())
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, "src/", std::io::empty())
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(6);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "src/a", &b"hello\n"[..])
        .unwrap();
    builder.finish().unwrap();
    let expected = [(
        "src/a".into(),
        "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03".into(),
    )]
    .into();
    assert_eq!(q::verify_legacy_archive(&path, &expected).unwrap(), 1);
    assert!(q::verify_legacy_archive(&path, &[("src/a".into(), "0".repeat(64))].into()).is_err());
}

#[test]
fn graph_rejects_packages_without_resolve_nodes() {
    let metadata = serde_json::json!({
        "packages":[
            {"id":"app","name":"app","version":"1.0.0","source":null},
            {"id":"orphan","name":"orphan","version":"1.0.0","source":null}
        ],
        "workspace_members":["app"],
        "resolve":{"nodes":[{"id":"app","features":[],"deps":[]}]}
    });
    assert!(q::graph::Metadata::parse(&serde_json::to_vec(&metadata).unwrap()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn seal_rejects_unindexed_gate_files_and_external_gate_links() {
    let (root, receipt) = run_fixture("true", 5);
    let request = q::SealRequest {
        composition: receipt.composition,
        gates: [("required".into(), root.path().join("run"))].into(),
        expected_receipts: [(
            "required".into(),
            q::sha256_file(&root.path().join("run/receipt.json")).unwrap(),
        )]
        .into(),
        artifacts: [("dummy".into(), root.path().join("source"))].into(),
    };
    let handoff = root.path().join("sealed");
    let seal = q::seal(&request, &handoff).unwrap();
    for file in ["request.json", "receipt.json", "stdout.log", "stderr.log"] {
        let mut document = serde_json::to_value(&seal).unwrap();
        document["artifacts"]
            .as_object_mut()
            .unwrap()
            .remove(&format!("gates/required/{file}"));
        fs::write(handoff.join("seal.json"), q::json_bytes(&document).unwrap()).unwrap();
        assert!(
            q::verify_seal(&handoff, &request.composition, &request.expected_receipts).is_err(),
            "missing {file}"
        );
    }
    let mut document = serde_json::to_value(&seal).unwrap();
    document["artifacts"]
        .as_object_mut()
        .unwrap()
        .retain(|name, _| !name.starts_with("gates/"));
    fs::write(handoff.join("seal.json"), q::json_bytes(&document).unwrap()).unwrap();
    fs::remove_dir_all(handoff.join("gates/required")).unwrap();
    std::os::unix::fs::symlink(root.path().join("run"), handoff.join("gates/required")).unwrap();
    assert!(q::verify_seal(&handoff, &request.composition, &request.expected_receipts).is_err());
}

#[cfg(unix)]
#[test]
fn source_archive_roundtrips_long_link_targets_deterministically() {
    let root = tempdir().unwrap();
    let target = format!("{}/file", "nested".repeat(20));
    fs::create_dir_all(root.path().join(&target).parent().unwrap()).unwrap();
    fs::write(root.path().join(&target), b"long link contents").unwrap();
    std::os::unix::fs::symlink(&target, root.path().join("link")).unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec![target.into(), "link".into()],
    };
    let first = root.path().join("first.tar");
    let second = root.path().join("second.tar");
    let expected = q::export_source(&spec, &first).unwrap();
    q::export_source(&spec, &second).unwrap();
    q::verify_source_archive(&first, &expected).unwrap();
    assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
}

#[cfg(unix)]
#[test]
fn hashing_rejects_devices_and_fifos_without_waiting_for_a_writer() {
    assert!(q::sha256_file(std::path::Path::new("/dev/zero")).is_err());
    let root = tempdir().unwrap();
    let fifo = root.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert!(q::sha256_file(&fifo).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn replacing_input_with_fifo_fails_post_capture_without_hanging() {
    let (_, receipt) = run_fixture("rm input; mkfifo input", 5);
    assert_eq!(receipt.status, q::RunStatus::SourceChanged);
    assert!(receipt.error.as_deref().unwrap().contains("regular"));
}

#[cfg(target_os = "linux")]
#[test]
fn path_search_skips_nonexecutable_files_and_records_the_executed_binary() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    fs::create_dir(root.path().join("first")).unwrap();
    fs::create_dir(root.path().join("second")).unwrap();
    fs::write(root.path().join("first/tool"), b"not executable").unwrap();
    let executable = root.path().join("second/tool");
    fs::write(&executable, b"#!/bin/sh\nprintf selected").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    request.command = vec!["tool".into()];
    request
        .environment
        .insert("PATH".into(), "first:second".into());
    let receipt = q::execute(&request, &root.path().join("search"), &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{:?}", receipt.error);
    assert_eq!(
        receipt.executable.unwrap().path,
        executable.canonicalize().unwrap()
    );
    assert_eq!(
        fs::read(root.path().join("search/stdout.log")).unwrap(),
        b"selected"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn version_probes_retain_both_streams() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("tool");
    fs::write(
        &tool,
        b"#!/bin/sh\nprintf stdout-version; printf stderr-version >&2",
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools.insert("tool".into(), tool);
    let receipt = q::execute(&request, &root.path().join("versions"), &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{:?}", receipt.error);
    assert!(receipt.tools["tool"].version.contains("stdout-version"));
    assert!(receipt.tools["tool"].version.contains("stderr-version"));
}

#[cfg(target_os = "linux")]
#[test]
fn setsid_descendants_cannot_write_after_success_timeout_or_cancellation() {
    for (script, timeout, expected) in [
        (
            "setsid sh -c 'touch ready; sleep 1; printf late; printf changed > source' & while ! test -f ready; do sleep 0.01; done; printf done",
            5,
            q::RunStatus::Failed,
        ),
        (
            "setsid sh -c 'touch ready; sleep 2; printf late; printf changed > source' & while ! test -f ready; do sleep 0.01; done; sleep 10",
            1,
            q::RunStatus::TimedOut,
        ),
    ] {
        let (root, receipt) = run_fixture(script, timeout);
        assert_eq!(receipt.status, expected, "{:?}", receipt.error);
        let logs = fs::read(root.path().join("run/stdout.log")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
        assert_eq!(fs::read(root.path().join("run/stdout.log")).unwrap(), logs);
    }
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command[2] = "setsid sh -c 'touch ready; sleep 1; printf late; printf changed > source' & while ! test -f ready; do sleep 0.01; done; sleep 10".into();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let receipt = std::thread::scope(|scope| {
        let task =
            scope.spawn(|| q::execute(&request, &root.path().join("cancel"), &cancelled).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !root.path().join("ready").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        task.join().unwrap()
    });
    assert_eq!(receipt.status, q::RunStatus::Cancelled);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
    assert!(
        fs::read(root.path().join("cancel/stdout.log"))
            .unwrap()
            .is_empty()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn version_probes_use_the_same_descendant_containment() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("tool");
    fs::write(&tool, b"#!/bin/sh\nsetsid sh -c 'touch ready; sleep 1; printf changed > source' &\nwhile ! test -f ready; do sleep 0.01; done\nprintf version").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools.insert("detached".into(), tool);
    let receipt = q::execute(&request, &root.path().join("probe"), &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Failed);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
}

#[cfg(all(feature = "cli", target_os = "linux"))]
#[test]
fn inherited_unrecorded_build_settings_are_cleared_and_explicit_settings_are_bound() {
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command[2] = "printf '%s:%s:%s:%s' \"${CC-cleared}\" \"${RUSTUP_TOOLCHAIN-cleared}\" \"${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER-cleared}\" \"${QUALIFICATION_SENTINEL-cleared}\"".into();
    request
        .environment
        .insert("CC".into(), "explicit-compiler".into());
    let path = root.path().join("environment-request.json");
    fs::write(&path, q::json_bytes(&request).unwrap()).unwrap();
    let destination = root.path().join("environment-run");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
        .args(["qualification", "run", "--request"])
        .arg(path)
        .arg("--out")
        .arg(&destination)
        .env("CC", "inherited-compiler")
        .env("RUSTUP_TOOLCHAIN", "unrecorded-toolchain")
        .env(
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
            "unrecorded-linker",
        )
        .env("QUALIFICATION_SENTINEL", "unrecorded-value")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: q::Receipt = q::read_json(&destination.join("receipt.json")).unwrap();
    assert_eq!(receipt.environment["CC"], "explicit-compiler");
    assert!(!receipt.environment.contains_key("RUSTUP_TOOLCHAIN"));
    assert_eq!(
        fs::read(destination.join("stdout.log")).unwrap(),
        b"explicit-compiler:cleared:cleared:cleared"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn missing_containment_and_uncontained_legacy_receipts_fail_closed() {
    let (root, receipt) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.containment = None;
    assert!(
        q::execute(
            &request,
            &root.path().join("uncontained"),
            &Default::default()
        )
        .is_err()
    );
    assert!(!root.path().join("uncontained").exists());
    let mut document = serde_json::to_value(receipt).unwrap();
    document["schemaVersion"] = 1.into();
    document.as_object_mut().unwrap().remove("containment");
    fs::write(
        root.path().join("run/receipt.json"),
        q::json_bytes(&document).unwrap(),
    )
    .unwrap();
    assert!(q::verify_run(&root.path().join("run"), None).is_err());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn execution_is_rejected_before_spawn_without_native_containment() {
    let root = tempdir().unwrap();
    let request = q::RunRequest {
        cwd: root.path().into(),
        sources: Default::default(),
        inputs: Default::default(),
        command: vec!["never-spawn".into()],
        environment: Default::default(),
        tools: Default::default(),
        timeout_seconds: 1,
        containment: None,
    };
    assert!(
        q::execute(&request, &root.path().join("run"), &Default::default())
            .unwrap_err()
            .to_string()
            .contains("Linux PID namespace")
    );
    assert!(!root.path().join("run").exists());
}

#[test]
fn recursive_inventory_uses_portable_component_paths() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("src/nested")).unwrap();
    fs::write(root.path().join("src/nested/file"), b"portable").unwrap();
    let recursive = InventorySpec {
        root: root.path().into(),
        paths: vec!["src".into()],
    };
    let explicit = InventorySpec {
        paths: vec!["src/nested/file".into()],
        ..recursive.clone()
    };
    let snapshot = q::inventory(&recursive).unwrap();
    assert_eq!(snapshot, q::inventory(&explicit).unwrap());
    assert!(snapshot.files.contains_key("src/nested/file"));
    let archive = root.path().join("source.tar");
    q::export_source(&recursive, &archive).unwrap();
    q::verify_source_archive(&archive, &snapshot).unwrap();
    #[cfg(windows)]
    assert_eq!(
        snapshot,
        q::inventory(&InventorySpec {
            paths: vec![r"src\nested\file".into()],
            ..recursive
        })
        .unwrap()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn request_requirements_cannot_be_omitted_or_substituted_in_a_receipt() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("tool");
    fs::write(&tool, b"#!/bin/sh\nprintf version").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools.insert("shell".into(), tool);
    request
        .environment
        .insert("RUSTFLAGS".into(), "explicit".into());
    let directory = root.path().join("requirements");
    let receipt = q::execute(&request, &directory, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed);
    q::verify_run(&directory, None).unwrap();
    for mutation in [
        "tool",
        "tool-path",
        "environment",
        "unshare",
        "executor",
        "command",
    ] {
        let mut changed = receipt.clone();
        match mutation {
            "tool" => {
                changed.tools.clear();
            }
            "tool-path" => {
                changed.tools.get_mut("shell").unwrap().path = "/different/tool".into();
            }
            "environment" => {
                changed
                    .environment
                    .insert("RUSTFLAGS".into(), "different".into());
            }
            "unshare" | "executor" => {
                changed.containment.get_mut(mutation).unwrap().path =
                    "/different/containment".into();
            }
            "command" => {
                changed.executable.as_mut().unwrap().path = "/different/command".into();
            }
            _ => unreachable!(),
        }
        fs::write(
            directory.join("receipt.json"),
            q::json_bytes(&changed).unwrap(),
        )
        .unwrap();
        assert!(
            q::verify_run(&directory, None).is_err(),
            "accepted {mutation}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn privileged_producer_cannot_hide_children_by_overmounting_proc() {
    let root = tempdir().unwrap();
    let unshare = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("unshare"))
        .find(|p| p.is_file())
        .unwrap();
    let output = std::process::Command::new(unshare)
        .args(["--user", "--map-root-user", "--pid", "--fork", "--mount-proc", "--kill-child=KILL", "--forward-signals", "--"])
        .arg(env!("CARGO_BIN_EXE_canix-toolbelt-qualification-executor"))
        .args(["/bin/sh", "-c", "mount -t tmpfs tmpfs /proc || exit 99; setsid sh -c 'touch ready; sleep 1; printf late > late' & while ! test -f ready; do sleep 0.01; done; printf mounted"])
        .current_dir(root.path()).output().unwrap();
    assert_eq!(
        output.stdout,
        b"mounted",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(125));
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(!root.path().join("late").exists());
}
