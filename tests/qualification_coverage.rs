#![cfg(feature = "qualification")]

use canix_toolbelt::qualification as q;
use serde_json::json;
use std::{collections::BTreeMap, fs, sync::atomic::AtomicBool};

fn fixture() -> (
    tempfile::TempDir,
    q::CoverageRequirements,
    q::ArtifactIndex,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source"), "frozen source").unwrap();
    fs::write(root.path().join("consumer"), "frozen consumer artifact").unwrap();
    fs::write(root.path().join("contract"), "schema-86 contract").unwrap();
    let receipt = q::execute(
        &q::RunRequest {
            cwd: root.path().into(),
            sources: BTreeMap::from([
                (
                    "producer".into(),
                    q::InventorySpec {
                        root: root.path().into(),
                        paths: vec!["source".into()],
                    },
                ),
                (
                    "consumer".into(),
                    q::InventorySpec {
                        root: root.path().into(),
                        paths: vec!["consumer".into()],
                    },
                ),
            ]),
            inputs: BTreeMap::from([("consumer-contract".into(), root.path().join("contract"))]),
            command: vec!["true".into()],
            environment: BTreeMap::new(),
            tools: BTreeMap::new(),
            timeout_seconds: 30,
        },
        &root.path().join("run"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    let receipts = BTreeMap::from([(
        "parity".into(),
        q::sha256_file(&root.path().join("run/receipt.json")).unwrap(),
    )]);
    let methods: BTreeMap<String, serde_json::Value> = (0..174)
        .map(|i| (format!("original.method_{i:03}"), json!("passed")))
        .collect();
    let required: q::CoverageRequirements = serde_json::from_value(json!({
        "schemaVersion": 1, "composition": receipt.composition,
        "obligations": {"postgres-parity": {"level": "consumer-conformance", "gate": "parity", "methods": methods.keys().collect::<Vec<_>>()}}
    })).unwrap();
    fs::write(root.path().join("coverage.json"), serde_json::to_vec(&json!({
        "schemaVersion":1, "composition":receipt.composition,
        "obligations":{"postgres-parity":{"level":"consumer-conformance", "gate":"parity", "methods":methods}}
    })).unwrap()).unwrap();
    q::seal(
        &q::SealRequest {
            composition: receipt.composition,
            gates: BTreeMap::from([("parity".into(), root.path().join("run"))]),
            expected_receipts: receipts.clone(),
            artifacts: BTreeMap::from([(
                "coverage.json".into(),
                root.path().join("coverage.json"),
            )]),
        },
        &root.path().join("sealed"),
    )
    .unwrap();
    let hash = q::sha256_file(&root.path().join("sealed/artifacts/coverage.json")).unwrap();
    (root, required, receipts, hash)
}

#[test]
fn every_original_method_requires_executed_passing_coverage() {
    let (root, required, receipts, hash) = fixture();
    let verified =
        q::verify_coverage(&root.path().join("sealed"), &required, &receipts, &hash).unwrap();
    assert_eq!(verified.methods_verified, 174);
    assert_eq!(verified.obligations_verified, 1);
    let mut missing = required.clone();
    missing
        .obligations
        .get_mut("postgres-parity")
        .unwrap()
        .methods
        .insert("original.missing".into());
    assert!(q::verify_coverage(&root.path().join("sealed"), &missing, &receipts, &hash).is_err());
}

#[test]
fn producer_success_cannot_satisfy_consumer_or_live_acceptance() {
    let (root, mut required, receipts, hash) = fixture();
    required
        .obligations
        .get_mut("postgres-parity")
        .unwrap()
        .level = q::CoverageLevel::Producer;
    assert!(q::verify_coverage(&root.path().join("sealed"), &required, &receipts, &hash).is_err());
    let mut value = serde_json::to_value(&required).unwrap();
    value["obligations"]["postgres-parity"]["level"] = "live-acceptance".into();
    assert!(serde_json::from_value::<q::CoverageRequirements>(value).is_err());
}

#[test]
fn coverage_cannot_reselect_its_composition_gates_or_trusted_digest() {
    let (root, required, receipts, hash) = fixture();
    assert!(
        q::verify_coverage(
            &root.path().join("sealed"),
            &required,
            &receipts,
            &"0".repeat(64)
        )
        .is_err()
    );
    assert!(
        q::verify_coverage(
            &root.path().join("sealed"),
            &required,
            &BTreeMap::new(),
            &hash
        )
        .is_err()
    );
    let mut changed = required.clone();
    changed
        .composition
        .inputs
        .insert("consumer-contract".into(), "0".repeat(64));
    assert!(q::verify_coverage(&root.path().join("sealed"), &changed, &receipts, &hash).is_err());
    let mut empty = required.clone();
    empty.obligations.clear();
    assert!(q::verify_coverage(&root.path().join("sealed"), &empty, &receipts, &hash).is_err());
}

#[test]
fn authentic_but_skipped_failed_pending_cancelled_or_incomplete_reports_fail() {
    let (root, required, receipts, _) = fixture();
    let path = root.path().join("sealed/artifacts/coverage.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for status in [
        "skipped",
        "failed",
        "pending",
        "cancelled",
        "missing",
        "extra",
    ] {
        let mut changed = original.clone();
        let methods = changed["obligations"]["postgres-parity"]["methods"]
            .as_object_mut()
            .unwrap();
        match status {
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
        // Re-seal the modified report to distinguish coverage rejection from
        // mere integrity rejection. Provider custody can bind a failing report.
        fs::write(
            root.path().join("coverage.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        let destination = root.path().join(status);
        q::seal(
            &q::SealRequest {
                composition: required.composition.clone(),
                gates: BTreeMap::from([("parity".into(), root.path().join("run"))]),
                expected_receipts: receipts.clone(),
                artifacts: BTreeMap::from([(
                    "coverage.json".into(),
                    root.path().join("coverage.json"),
                )]),
            },
            &destination,
        )
        .unwrap();
        let hash = q::sha256_file(&destination.join("artifacts/coverage.json")).unwrap();
        let error = q::verify_coverage(&destination, &required, &receipts, &hash).unwrap_err();
        assert!(error.to_string().contains("method"), "{status}: {error}");
    }
}

#[test]
fn valid_seals_without_explicit_consumer_or_contract_bindings_are_insufficient() {
    let (root, original, _, _) = fixture();
    for missing in ["consumer", "consumer-contract"] {
        let mut request: q::RunRequest =
            serde_json::from_slice(&fs::read(root.path().join("run/request.json")).unwrap())
                .unwrap();
        request.sources.remove(missing);
        request.inputs.remove(missing);
        let run = root.path().join(format!("run-without-{missing}"));
        let receipt = q::execute(&request, &run, &AtomicBool::new(false)).unwrap();
        let mut required = original.clone();
        required.composition = receipt.composition.clone();
        let mut report: serde_json::Value =
            serde_json::from_slice(&fs::read(root.path().join("coverage.json")).unwrap()).unwrap();
        report["composition"] = serde_json::to_value(&receipt.composition).unwrap();
        let file = root.path().join(format!("coverage-without-{missing}.json"));
        fs::write(&file, serde_json::to_vec(&report).unwrap()).unwrap();
        let receipts = BTreeMap::from([(
            "parity".into(),
            q::sha256_file(&run.join("receipt.json")).unwrap(),
        )]);
        let sealed = root.path().join(format!("sealed-without-{missing}"));
        q::seal(
            &q::SealRequest {
                composition: receipt.composition,
                gates: BTreeMap::from([("parity".into(), run)]),
                expected_receipts: receipts.clone(),
                artifacts: BTreeMap::from([("coverage.json".into(), file)]),
            },
            &sealed,
        )
        .unwrap();
        let hash = q::sha256_file(&sealed.join("artifacts/coverage.json")).unwrap();
        let error = q::verify_coverage(&sealed, &required, &receipts, &hash).unwrap_err();
        assert!(
            error.to_string().contains("consumer conformance"),
            "{missing}: {error}"
        );
    }
}

#[test]
fn duplicate_method_keys_cannot_shadow_a_skipped_execution() {
    let raw = r#"{"schemaVersion":1,"composition":{"schemaVersion":1,"sources":{},"inputs":{}},
      "obligations":{"test":{"level":"producer","gate":"gate","methods":{"original":"skipped","original":"passed"}}}}"#;
    assert!(serde_json::from_str::<q::CoverageReport>(raw).is_err());
}

#[cfg(feature = "cli")]
#[test]
fn standalone_coverage_command_reports_only_integrity_and_applicability() {
    let (root, required, receipts, hash) = fixture();
    let expected = root.path().join("expected.json");
    let index = root.path().join("receipts.json");
    fs::write(&expected, serde_json::to_vec(&required).unwrap()).unwrap();
    fs::write(&index, serde_json::to_vec(&receipts).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
        .args(["qualification", "verify-coverage"])
        .arg(root.path().join("sealed"))
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
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["scope"], "coverage-integrity-and-applicability");
    assert_eq!(result["methodsVerified"], 174);
}
