#![cfg(feature = "cli")]

use std::process::Command;

#[test]
fn standalone_cli_loads_manifest_without_external_programs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runtime.pkl");
    std::fs::write(&path, include_str!("../examples/runtime.pkl")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
        .env("PATH", directory.path())
        .args(["runtime", "show", "--path"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let manifest: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["endpoints"]["api"]["port"], 8032);
    assert_eq!(
        manifest["secrets"]["token"]["agenixPath"],
        "/run/example/token"
    );
}

#[test]
fn standalone_cli_failure_has_no_json_on_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
        .args(["runtime", "show", "--path"])
        .arg(directory.path().join("missing.pkl"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.pkl"));
}
