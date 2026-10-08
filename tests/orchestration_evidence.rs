#![cfg(all(unix, feature = "orchestration"))]
use canix_toolbelt::orchestration::evidence::*;
use std::{collections::BTreeMap, fs, path::PathBuf};

#[test]
fn exact_evidence_bytes_and_foreign_edits_are_preserved() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    fs::create_dir(source.path().join("evidence")).unwrap();
    fs::write(source.path().join("evidence/receipt"), b"\0binary\xff").unwrap();
    let allowed = vec![PathBuf::from("evidence")];
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    let receipt = apply(target.path(), &allowed, &batch).unwrap();
    assert_eq!(
        fs::read(target.path().join("evidence/receipt")).unwrap(),
        b"\0binary\xff"
    );
    assert_eq!(receipt.accepted.len(), 1);
    fs::write(target.path().join("evidence/receipt"), b"foreign edit").unwrap();
    let receipt = apply(target.path(), &allowed, &batch).unwrap();
    assert_eq!(receipt.conflicts, vec!["evidence/receipt"]);
    assert_eq!(
        fs::read(target.path().join("evidence/receipt")).unwrap(),
        b"foreign edit"
    );
}

#[test]
fn tampered_payload_or_unowned_path_cannot_be_applied() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    fs::write(source.path().join("handoff"), b"exact").unwrap();
    let allowed = vec![PathBuf::from("handoff")];
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    let mut tampered = batch.clone();
    tampered.files[0].bytes.push(0);
    assert!(apply(target.path(), &allowed, &tampered).is_err());
    tampered = batch.clone();
    tampered.files[0].path = "../escape".into();
    assert!(apply(target.path(), &allowed, &tampered).is_err());
    assert!(!target.path().join("handoff").exists());
}

#[test]
fn symlinks_checkouts_and_large_files_remain_host_local() {
    use std::os::unix::fs::symlink;
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    fs::create_dir(source.path().join("evidence")).unwrap();
    symlink("/etc/passwd", source.path().join("evidence/link")).unwrap();
    fs::create_dir(source.path().join("evidence/project")).unwrap();
    fs::create_dir(source.path().join("evidence/project/.git")).unwrap();
    fs::write(source.path().join("evidence/project/source"), b"checkout").unwrap();
    fs::File::create(source.path().join("evidence/large"))
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    let allowed = vec![PathBuf::from("evidence")];
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    assert!(batch.files.is_empty());
    assert_eq!(batch.omitted.len(), 3);
    symlink(source.path(), target.path().join("evidence")).unwrap();
    let mut batch = batch;
    fs::write(source.path().join("evidence/good"), b"good").unwrap();
    batch.files = collect(source.path(), &allowed, &BTreeMap::new())
        .unwrap()
        .files;
    assert!(apply(target.path(), &allowed, &batch).is_err());
}
