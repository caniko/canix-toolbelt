#![cfg(all(feature = "qualification", unix))]

use canix_toolbelt::qualification as q;
use std::{collections::BTreeMap, fs, sync::atomic::AtomicBool};

#[test]
fn canonical_program_identity_preserves_the_requested_multicall_name() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source"), "immutable source").unwrap();
    let request = q::RunRequest {
        cwd: root.path().into(),
        sources: BTreeMap::from([(
            "source".into(),
            q::InventorySpec {
                root: root.path().into(),
                paths: vec!["source".into()],
            },
        )]),
        inputs: BTreeMap::new(),
        command: vec!["true".into()],
        environment: BTreeMap::new(),
        tools: BTreeMap::new(),
        timeout_seconds: 30,
    };
    let receipt = q::execute(&request, &root.path().join("run"), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "{}",
        fs::read_to_string(root.path().join("run/stderr.log")).unwrap()
    );
    q::verify_run(&root.path().join("run"), Some(&receipt.composition)).unwrap();
    let identity = receipt.executable.unwrap();
    assert_eq!(identity.sha256, q::sha256_file(&identity.path).unwrap());
}
