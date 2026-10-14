#![cfg(all(feature = "qualification", target_os = "linux"))]

use canix_toolbelt::qualification as q;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, sync::atomic::AtomicBool};

fn containment() -> q::ContainmentPrograms {
    let tool = |name| {
        std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join(name))
            .find(|path| path.is_file())
            .unwrap()
    };
    q::ContainmentPrograms {
        unshare: tool("unshare"),
        setpriv: tool("setpriv"),
        executor: env!("CARGO_BIN_EXE_canix-toolbelt-qualification-executor").into(),
    }
}

fn fixture() -> (
    tempfile::TempDir,
    q::CoverageRequirements,
    q::ArtifactIndex,
    Value,
) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("producer"), "frozen producer").unwrap();
    fs::write(root.path().join("consumer"), "frozen consumer artifact").unwrap();
    fs::write(root.path().join("contract"), "schema-86 contract").unwrap();
    let receipt = q::execute(
        &q::RunRequest {
            cwd: root.path().into(),
            sources: ["producer", "consumer"]
                .into_iter()
                .map(|name| {
                    (
                        name.into(),
                        q::InventorySpec {
                            root: root.path().into(),
                            paths: vec![name.into()],
                        },
                    )
                })
                .collect(),
            inputs: BTreeMap::from([("consumer-contract".into(), root.path().join("contract"))]),
            command: vec!["/bin/sh".into(), "-c".into(), "exit 0".into()],
            argument_zero: None,
            environment: BTreeMap::new(),
            tools: BTreeMap::new(),
            timeout_seconds: 30,
            containment: Some(containment()),
        },
        &root.path().join("run"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "{receipt:#?}; {}",
        fs::read_to_string(root.path().join("run/stderr.log")).unwrap()
    );
    assert_eq!(receipt.schema_version, 4);
    let receipts = BTreeMap::from([(
        "parity".into(),
        q::sha256_file(&root.path().join("run/receipt.json")).unwrap(),
    )]);
    let methods: BTreeMap<String, Value> = (0..174)
        .map(|i| (format!("original.method_{i:03}"), json!("passed")))
        .collect();
    let required = serde_json::from_value(json!({
        "schemaVersion": 1, "composition": receipt.composition,
        "obligations": {"postgres-parity": {"level": "consumer-conformance", "gate": "parity", "methods": methods.keys().collect::<Vec<_>>()}}
    })).unwrap();
    let report = json!({
        "schemaVersion": 1, "composition": receipt.composition,
        "obligations": {"postgres-parity": {"level": "consumer-conformance", "gate": "parity", "methods": methods}}
    });
    (root, required, receipts, report)
}

fn seal(
    root: &Path,
    label: &str,
    required: &q::CoverageRequirements,
    receipts: &q::ArtifactIndex,
    report: &Value,
) -> (std::path::PathBuf, String) {
    let report_path = root.join(format!("{label}.json"));
    fs::write(&report_path, serde_json::to_vec(report).unwrap()).unwrap();
    let destination = root.join(label);
    q::seal(
        &q::SealRequest {
            composition: required.composition.clone(),
            gates: BTreeMap::from([("parity".into(), root.join("run"))]),
            expected_receipts: receipts.clone(),
            artifacts: BTreeMap::from([("coverage.json".into(), report_path)]),
        },
        &destination,
    )
    .unwrap();
    let hash = q::sha256_file(&destination.join("artifacts/coverage.json")).unwrap();
    (destination, hash)
}

#[test]
fn all_original_selectors_require_passing_coverage_from_contained_receipts() {
    let (root, required, receipts, report) = fixture();
    let (directory, hash) = seal(root.path(), "sealed", &required, &receipts, &report);
    let verified = q::verify_coverage(&directory, &required, &receipts, &hash).unwrap();
    assert_eq!(verified.methods_verified, 174);
    assert_eq!(verified.obligations_verified, 1);
    for disposition in [
        "failed",
        "skipped",
        "pending",
        "cancelled",
        "missing",
        "extra",
    ] {
        let mut changed = report.clone();
        let methods = changed["obligations"]["postgres-parity"]["methods"]
            .as_object_mut()
            .unwrap();
        match disposition {
            "missing" => {
                methods.remove("original.method_000");
            }
            "extra" => {
                methods.insert("unselected.method".into(), "passed".into());
            }
            status => {
                methods.insert("original.method_000".into(), status.into());
            }
        }
        // Re-seal to exercise applicability, not just integrity rejection.
        let (directory, hash) = seal(root.path(), disposition, &required, &receipts, &changed);
        let error = q::verify_coverage(&directory, &required, &receipts, &hash).unwrap_err();
        assert!(
            error.to_string().contains("method"),
            "{disposition}: {error}"
        );
    }
}

#[test]
fn expectations_cannot_be_selected_from_the_successful_subset() {
    let (root, required, receipts, report) = fixture();
    let (directory, hash) = seal(root.path(), "sealed", &required, &receipts, &report);
    assert!(q::verify_coverage(&directory, &required, &receipts, &"0".repeat(64)).is_err());
    assert!(q::verify_coverage(&directory, &required, &BTreeMap::new(), &hash).is_err());
    let mut changed = required.clone();
    changed
        .composition
        .inputs
        .insert("consumer-contract".into(), "0".repeat(64));
    assert!(q::verify_coverage(&directory, &changed, &receipts, &hash).is_err());
    let mut changed = required.clone();
    changed.obligations.clear();
    assert!(q::verify_coverage(&directory, &changed, &receipts, &hash).is_err());
    let mut changed = required.clone();
    changed
        .obligations
        .get_mut("postgres-parity")
        .unwrap()
        .methods
        .insert("original.missing".into());
    assert!(q::verify_coverage(&directory, &changed, &receipts, &hash).is_err());
}

#[test]
fn producer_only_reports_cannot_cover_consumer_or_live_obligations() {
    let (root, required, receipts, mut report) = fixture();
    report["obligations"]["postgres-parity"]["level"] = "producer".into();
    let (directory, hash) = seal(root.path(), "producer-only", &required, &receipts, &report);
    assert!(q::verify_coverage(&directory, &required, &receipts, &hash).is_err());
    let mut unsupported = serde_json::to_value(&required).unwrap();
    unsupported["obligations"]["postgres-parity"]["level"] = "live-acceptance".into();
    assert!(serde_json::from_value::<q::CoverageRequirements>(unsupported).is_err());
}

#[test]
fn duplicate_method_keys_cannot_shadow_a_skipped_result() {
    let raw = r#"{"schemaVersion":1,"composition":{"schemaVersion":1,"sources":{},"inputs":{}},"obligations":{"test":{"level":"producer","gate":"gate","methods":{"method":"skipped","method":"passed"}}}}"#;
    assert!(serde_json::from_str::<q::CoverageReport>(raw).is_err());
}

#[test]
fn sealed_reports_without_explicit_consumer_bindings_are_insufficient() {
    let (root, original, _, mut report) = fixture();
    let original_request: q::RunRequest =
        q::read_json(&root.path().join("run/request.json")).unwrap();
    for missing in ["consumer", "consumer-contract"] {
        let mut request = original_request.clone();
        request.sources.remove(missing);
        request.inputs.remove(missing);
        let run = root.path().join(format!("run-without-{missing}"));
        let receipt = q::execute(&request, &run, &AtomicBool::new(false)).unwrap();
        assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
        let mut required = original.clone();
        required.composition = receipt.composition.clone();
        report["composition"] = serde_json::to_value(&receipt.composition).unwrap();
        let file = root.path().join(format!("report-without-{missing}.json"));
        fs::write(&file, serde_json::to_vec(&report).unwrap()).unwrap();
        let receipts = BTreeMap::from([(
            "parity".into(),
            q::sha256_file(&run.join("receipt.json")).unwrap(),
        )]);
        let directory = root.path().join(format!("sealed-without-{missing}"));
        q::seal(
            &q::SealRequest {
                composition: receipt.composition,
                gates: BTreeMap::from([("parity".into(), run)]),
                expected_receipts: receipts.clone(),
                artifacts: BTreeMap::from([("coverage.json".into(), file)]),
            },
            &directory,
        )
        .unwrap();
        let hash = q::sha256_file(&directory.join("artifacts/coverage.json")).unwrap();
        let error = q::verify_coverage(&directory, &required, &receipts, &hash).unwrap_err();
        assert!(
            error.to_string().contains("consumer conformance"),
            "{missing}: {error}"
        );
    }
}

#[cfg(feature = "cli")]
#[test]
fn standalone_cli_reports_only_integrity_and_applicability() {
    let (root, required, receipts, report) = fixture();
    let (directory, hash) = seal(root.path(), "sealed", &required, &receipts, &report);
    let expected = root.path().join("expected.json");
    let index = root.path().join("receipts.json");
    fs::write(&expected, serde_json::to_vec(&required).unwrap()).unwrap();
    fs::write(&index, serde_json::to_vec(&receipts).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
        .args(["qualification", "verify-coverage"])
        .arg(directory)
        .arg("--expected")
        .arg(expected)
        .arg("--receipts")
        .arg(index)
        .arg("--report-sha256")
        .arg(hash)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["scope"], "coverage-integrity-and-applicability");
    assert_eq!(result["methodsVerified"], 174);
}
