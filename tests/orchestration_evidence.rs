#![cfg(all(unix, feature = "orchestration"))]
use canix_toolbelt::orchestration::evidence::*;
use std::{collections::BTreeMap, fs, path::PathBuf};

fn mirror_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

#[test]
fn exact_evidence_bytes_and_foreign_edits_are_preserved() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
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
fn acknowledged_receiver_deletion_remains_a_conflict() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::create_dir(source.path().join("evidence")).unwrap();
    fs::write(source.path().join("evidence/receipt"), b"initial").unwrap();
    let allowed = vec![PathBuf::from("evidence")];
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    let acknowledged = apply(target.path(), &allowed, &batch).unwrap();
    fs::remove_file(target.path().join("evidence/receipt")).unwrap();
    fs::write(source.path().join("evidence/receipt"), b"source update").unwrap();
    let update = collect(source.path(), &allowed, &acknowledged.accepted).unwrap();
    assert_eq!(update.files.len(), 1);
    assert!(update.files[0].expected.is_some());
    let receipt = apply(target.path(), &allowed, &update).unwrap();
    assert_eq!(receipt.conflicts, vec!["evidence/receipt"]);
    assert!(receipt.accepted.is_empty());
    assert!(!target.path().join("evidence/receipt").exists());
}

#[test]
fn conflicting_file_and_descendant_paths_fail_before_any_write() {
    let source = tempfile::tempdir().unwrap();
    fs::create_dir(source.path().join("evidence")).unwrap();
    fs::write(source.path().join("evidence/first"), b"parent bytes").unwrap();
    fs::write(source.path().join("evidence/second"), b"child bytes").unwrap();
    let allowed = vec![PathBuf::from("evidence")];
    let mut batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    batch.files[0].path = "evidence/parent".into();
    batch.files[1].path = "evidence/parent/child".into();
    for _ in 0..2 {
        let target = mirror_dir();
        assert!(apply(target.path(), &allowed, &batch).is_err());
        assert!(!target.path().join("evidence").exists());
        batch.files.reverse();
    }
}

#[test]
fn tampered_payload_or_unowned_path_cannot_be_applied() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
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
    let target = mirror_dir();
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

#[test]
fn unchanged_acknowledged_files_are_verified_without_retransmitting_bytes() {
    for delete in [false, true] {
        let source = tempfile::tempdir().unwrap();
        let target = mirror_dir();
        fs::write(source.path().join("handoff"), b"exact").unwrap();
        let allowed = vec![PathBuf::from("handoff")];
        let first = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let known = apply(target.path(), &allowed, &first).unwrap().accepted;
        let verified = collect(source.path(), &allowed, &known).unwrap();
        assert!(verified.files.is_empty());
        assert_eq!(verified.verify.len(), 1);
        assert_eq!(
            apply(target.path(), &allowed, &verified).unwrap().accepted,
            known
        );
        if delete {
            fs::remove_file(target.path().join("handoff")).unwrap();
        } else {
            fs::write(target.path().join("handoff"), b"independent").unwrap();
        }
        let receipt = apply(target.path(), &allowed, &verified).unwrap();
        assert_eq!(receipt.conflicts, vec!["handoff"]);
        assert!(receipt.accepted.is_empty());
        if delete {
            assert!(!target.path().join("handoff").exists());
        } else {
            assert_eq!(
                fs::read(target.path().join("handoff")).unwrap(),
                b"independent"
            );
        }
    }
}

#[test]
fn authoritative_deletions_remove_only_acknowledged_receiver_bytes() {
    for foreign in [false, true] {
        let source = tempfile::tempdir().unwrap();
        let target = mirror_dir();
        fs::create_dir(source.path().join("evidence")).unwrap();
        fs::write(source.path().join("evidence/receipt"), b"exact").unwrap();
        let allowed = vec![PathBuf::from("evidence")];
        let first = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let known = apply(target.path(), &allowed, &first).unwrap().accepted;
        fs::remove_dir_all(source.path().join("evidence")).unwrap();
        let deletion = collect(source.path(), &allowed, &known).unwrap();
        assert_eq!(deletion.delete.len(), 1);
        if foreign {
            fs::write(target.path().join("evidence/receipt"), b"independent").unwrap();
        }
        let receipt = apply(target.path(), &allowed, &deletion).unwrap();
        if foreign {
            assert_eq!(receipt.conflicts, vec!["evidence/receipt"]);
            assert!(receipt.removed.is_empty());
            assert_eq!(
                fs::read(target.path().join("evidence/receipt")).unwrap(),
                b"independent"
            );
        } else {
            assert_eq!(receipt.removed, known);
            assert!(!target.path().join("evidence/receipt").exists());
            assert_eq!(
                apply(target.path(), &allowed, &deletion).unwrap().removed,
                known
            );
        }
    }
}

#[test]
fn forged_file_counts_are_rejected_before_any_receiver_write() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::write(source.path().join("seed"), b"").unwrap();
    let seed = collect(source.path(), &[PathBuf::from("seed")], &BTreeMap::new())
        .unwrap()
        .files
        .remove(0);
    let batch = Batch {
        files: (0..10_001)
            .map(|index| Artifact {
                path: format!("evidence/{index}"),
                ..seed.clone()
            })
            .collect(),
        ..Batch::default()
    };
    assert!(apply(target.path(), &[PathBuf::from("evidence")], &batch).is_err());
    assert!(!target.path().join("evidence").exists());
}

#[test]
fn overlapping_roots_have_the_same_unique_budget_as_one_root() {
    let source = tempfile::tempdir().unwrap();
    fs::create_dir_all(source.path().join("evidence/reports")).unwrap();
    for index in 0..6 {
        fs::write(
            source.path().join(format!("evidence/reports/{index}")),
            vec![index as u8; 2 * 1024 * 1024],
        )
        .unwrap();
    }
    fs::write(source.path().join("evidence/z-last"), b"last").unwrap();
    let one = collect(
        source.path(),
        &[PathBuf::from("evidence")],
        &BTreeMap::new(),
    )
    .unwrap();
    let overlap = collect(
        source.path(),
        &[
            PathBuf::from("evidence/reports"),
            PathBuf::from("evidence"),
            PathBuf::from("evidence/reports"),
        ],
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        one.files
            .iter()
            .map(|item| (
                &item.path,
                &item.sha256,
                &item.expected,
                item.bytes.as_slice()
            ))
            .collect::<Vec<_>>(),
        overlap
            .files
            .iter()
            .map(|item| (
                &item.path,
                &item.sha256,
                &item.expected,
                item.bytes.as_slice()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(one.omitted, overlap.omitted);
    assert_eq!(one.deferred, overlap.deferred);
    assert_eq!(one.files.len(), 6);
    assert_eq!(one.deferred.len(), 1);
    assert!(
        !overlap
            .deferred
            .iter()
            .any(|path| overlap.files.iter().any(|file| &file.path == path))
    );
}

#[test]
fn file_to_directory_transition_requires_a_retained_deletion_before_descendants() {
    for foreign in [false, true] {
        let source = tempfile::tempdir().unwrap();
        let target = mirror_dir();
        fs::create_dir(source.path().join("evidence")).unwrap();
        fs::write(source.path().join("evidence/a"), b"acknowledged file").unwrap();
        let allowed = vec![PathBuf::from("evidence")];
        let initial = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let mut known = apply(target.path(), &allowed, &initial).unwrap().accepted;
        fs::remove_file(source.path().join("evidence/a")).unwrap();
        fs::create_dir(source.path().join("evidence/a")).unwrap();
        fs::write(source.path().join("evidence/a/b"), b"new descendant").unwrap();
        let transition = collect(source.path(), &allowed, &known).unwrap();
        assert!(
            transition.files.is_empty(),
            "descendant must wait for deletion receipt"
        );
        assert_eq!(transition.delete.len(), 1);
        assert_eq!(transition.delete[0].path, "evidence/a");
        if foreign {
            fs::write(target.path().join("evidence/a"), b"foreign bytes").unwrap();
        }
        let receipt = apply(target.path(), &allowed, &transition).unwrap();
        if foreign {
            assert!(receipt.removed.is_empty());
            assert_eq!(receipt.conflicts, vec!["evidence/a"]);
            assert_eq!(
                fs::read(target.path().join("evidence/a")).unwrap(),
                b"foreign bytes"
            );
            assert!(
                collect(source.path(), &allowed, &known)
                    .unwrap()
                    .files
                    .is_empty()
            );
        } else {
            assert_eq!(receipt.removed, known);
            // Only the durably retained exact removal bindings release descendants.
            for (path, sha) in receipt.removed {
                assert_eq!(known.remove(&path), Some(sha));
            }
            let descendants = collect(source.path(), &allowed, &known).unwrap();
            assert_eq!(descendants.files.len(), 1);
            assert_eq!(descendants.files[0].path, "evidence/a/b");
            apply(target.path(), &allowed, &descendants).unwrap();
            assert_eq!(
                fs::read(target.path().join("evidence/a/b")).unwrap(),
                b"new descendant"
            );
        }
    }
}

#[test]
fn directory_to_file_transition_stages_descendant_removal_before_replacement() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::create_dir_all(source.path().join("evidence/a/nested")).unwrap();
    fs::write(
        source.path().join("evidence/a/nested/b"),
        b"acknowledged descendant",
    )
    .unwrap();
    let allowed = vec![PathBuf::from("evidence")];
    let initial = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    let mut known = apply(target.path(), &allowed, &initial).unwrap().accepted;
    fs::remove_dir_all(source.path().join("evidence/a")).unwrap();
    fs::write(source.path().join("evidence/a"), b"replacement file").unwrap();
    let transition = collect(source.path(), &allowed, &known)
        .expect("regular ancestor must stage descendant deletion");
    assert!(transition.files.is_empty());
    assert_eq!(transition.delete.len(), 1);
    let receipt = apply(target.path(), &allowed, &transition).unwrap();
    assert_eq!(receipt.removed, known);
    for (path, sha) in receipt.removed {
        assert_eq!(known.remove(&path), Some(sha));
    }
    let replacement = collect(source.path(), &allowed, &known).unwrap();
    apply(target.path(), &allowed, &replacement).unwrap();
    assert_eq!(
        fs::read(target.path().join("evidence/a")).unwrap(),
        b"replacement file"
    );
}

#[test]
fn receiver_leaf_type_changes_are_conflicts_while_unrelated_artifacts_advance() {
    use std::os::unix::fs::symlink;
    for kind in ["directory", "symlink", "fifo", "socket", "oversized"] {
        let source = tempfile::tempdir().unwrap();
        let target = mirror_dir();
        fs::create_dir(source.path().join("evidence")).unwrap();
        fs::write(source.path().join("evidence/a"), b"acknowledged").unwrap();
        let allowed = vec![PathBuf::from("evidence")];
        let first = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let known = apply(target.path(), &allowed, &first).unwrap().accepted;
        let path = target.path().join("evidence/a");
        fs::remove_file(&path).unwrap();
        let mut socket = None;
        match kind {
            "directory" => fs::create_dir(&path).unwrap(),
            "symlink" => symlink("/nonexistent-foreign-evidence", &path).unwrap(),
            "fifo" => rustix::fs::mknodat(
                rustix::fs::CWD,
                &path,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_bits_truncate(0o600),
                0,
            )
            .unwrap(),
            "socket" => socket = Some(std::os::unix::net::UnixListener::bind(&path).unwrap()),
            _ => fs::File::create(&path)
                .unwrap()
                .set_len(8 * 1024 * 1024 + 1)
                .unwrap(),
        }
        fs::write(source.path().join("evidence/a"), b"source changed").unwrap();
        fs::write(source.path().join("evidence/b"), b"independent addition").unwrap();
        let batch = collect(source.path(), &allowed, &known).unwrap();
        let receipt = apply(target.path(), &allowed, &batch)
            .expect("leaf type change must be a per-path conflict");
        assert_eq!(receipt.conflicts, vec!["evidence/a"]);
        assert!(receipt.accepted.contains_key("evidence/b"));
        assert_eq!(
            fs::read(target.path().join("evidence/b")).unwrap(),
            b"independent addition"
        );
        assert!(
            fs::symlink_metadata(path).is_ok(),
            "foreign leaf was removed"
        );
        drop(socket);
    }
}

#[test]
fn escaped_transition_inventory_cannot_exceed_the_encoded_payload_budget() {
    let target = mirror_dir();
    let prefix = format!("evidence/{}/", vec!["\u{1}".repeat(255); 4].join("/"));
    let sha = "a".repeat(64);
    let batch = Batch {
        delete: (0..5000)
            .map(|id| Identity {
                path: format!("{prefix}{id}/old"),
                sha256: sha.clone(),
            })
            .collect(),
        prune: (0..5000).map(|id| format!("{prefix}{id}")).collect(),
        deferred: (0..5000).map(|id| format!("{prefix}{id}")).collect(),
        ..Batch::default()
    };
    let error = apply(target.path(), &[PathBuf::from("evidence")], &batch).unwrap_err();
    assert!(error.to_string().contains("encoded"), "{error}");
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
}

#[test]
fn directory_retirement_preserves_foreign_content_and_retains_retry_identities() {
    for foreign in ["file", "empty-directory", "checkout", "nested-checkout"] {
        let source = tempfile::tempdir().unwrap();
        let target = mirror_dir();
        fs::create_dir_all(source.path().join("evidence/a/nested")).unwrap();
        fs::write(source.path().join("evidence/a/nested/b"), b"old bytes").unwrap();
        let allowed = vec![PathBuf::from("evidence")];
        let initial = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
        let known = apply(target.path(), &allowed, &initial).unwrap().accepted;
        fs::remove_dir_all(source.path().join("evidence/a")).unwrap();
        fs::write(source.path().join("evidence/a"), b"new file").unwrap();
        fs::write(
            source.path().join("evidence/unrelated"),
            b"independent progress",
        )
        .unwrap();
        let foreign_path = target
            .path()
            .join("evidence/a")
            .join(if foreign == "checkout" {
                ".git"
            } else if foreign == "nested-checkout" {
                "nested/.git"
            } else {
                "foreign"
            });
        if foreign == "file" {
            fs::write(&foreign_path, b"foreign bytes").unwrap();
        } else {
            fs::create_dir(&foreign_path).unwrap();
        }
        let transition = collect(source.path(), &allowed, &known).unwrap();
        let receipt = apply(target.path(), &allowed, &transition).unwrap();
        assert!(receipt.removed.is_empty());
        assert!(receipt.pruned.is_empty());
        assert!(receipt.conflicts.contains(&"evidence/a".into()));
        assert!(receipt.accepted.contains_key("evidence/unrelated"));
        assert!(foreign_path.exists());
        if foreign.contains("checkout") {
            assert_eq!(
                fs::read(target.path().join("evidence/a/nested/b")).unwrap(),
                b"old bytes"
            );
        }
        let replay = collect(source.path(), &allowed, &known).unwrap();
        assert!(replay.files.iter().all(|item| item.path != "evidence/a"));
        assert_eq!(replay.prune, transition.prune);
        if foreign == "file" {
            fs::remove_file(foreign_path).unwrap();
        } else {
            fs::remove_dir(foreign_path).unwrap();
        }
        let receipt = apply(target.path(), &allowed, &replay).unwrap();
        assert_eq!(receipt.removed, known);
        assert_eq!(receipt.pruned, vec!["evidence/a"]);
        let receipt = apply(target.path(), &allowed, &replay).unwrap();
        assert_eq!(receipt.removed, known);
        assert_eq!(receipt.pruned, vec!["evidence/a"]);
    }
}

#[test]
fn blocked_receiver_ancestors_are_conflicts_for_writes_verifies_and_deletions() {
    use std::os::unix::fs::symlink;
    for kind in ["file", "symlink", "checkout"] {
        for change in ["update", "verify", "delete"] {
            let source = tempfile::tempdir().unwrap();
            let target = mirror_dir();
            let outside = tempfile::tempdir().unwrap();
            fs::create_dir_all(source.path().join("evidence/nested")).unwrap();
            fs::write(source.path().join("evidence/nested/a"), b"acknowledged").unwrap();
            let allowed = vec![PathBuf::from("evidence")];
            let known = apply(
                target.path(),
                &allowed,
                &collect(source.path(), &allowed, &BTreeMap::new()).unwrap(),
            )
            .unwrap()
            .accepted;
            fs::remove_dir_all(target.path().join("evidence/nested")).unwrap();
            let path = target.path().join("evidence/nested");
            match kind {
                "file" => fs::write(&path, b"foreign ancestor").unwrap(),
                "symlink" => {
                    fs::write(outside.path().join("a"), b"outside sentinel").unwrap();
                    symlink(outside.path(), &path).unwrap();
                }
                _ => {
                    fs::create_dir(&path).unwrap();
                    fs::create_dir(path.join(".git")).unwrap();
                    fs::write(path.join("a"), b"checkout sentinel").unwrap();
                }
            }
            if change == "update" {
                fs::write(source.path().join("evidence/nested/a"), b"new owned").unwrap();
            }
            if change == "delete" {
                fs::remove_file(source.path().join("evidence/nested/a")).unwrap();
            }
            fs::write(source.path().join("evidence/independent"), b"independent").unwrap();
            let batch = collect(source.path(), &allowed, &known).unwrap();
            let receipt = apply(target.path(), &allowed, &batch).unwrap();
            assert!(receipt.conflicts.contains(&"evidence/nested/a".into()));
            assert!(receipt.accepted.contains_key("evidence/independent"));
            assert!(!receipt.accepted.contains_key("evidence/nested/a"));
            assert!(receipt.removed.is_empty());
            if kind == "file" {
                assert_eq!(fs::read(path).unwrap(), b"foreign ancestor");
            } else if kind == "symlink" {
                assert_eq!(
                    fs::read(outside.path().join("a")).unwrap(),
                    b"outside sentinel"
                );
            } else {
                assert_eq!(fs::read(path.join("a")).unwrap(), b"checkout sentinel");
            }
        }
    }
}

#[test]
fn oversized_directory_transitions_drain_acknowledged_descendants_in_bounded_exchanges() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::create_dir_all(source.path().join("evidence/a/nested")).unwrap();
    for id in 0..15000 {
        fs::write(
            source.path().join(format!("evidence/a/nested/{id:05}")),
            b"old",
        )
        .unwrap();
    }
    let allowed = vec![PathBuf::from("evidence")];
    let mut known = BTreeMap::new();
    for _ in 0..2 {
        let batch = collect(source.path(), &allowed, &known).unwrap();
        known.extend(apply(target.path(), &allowed, &batch).unwrap().accepted);
    }
    assert_eq!(known.len(), 15000);
    fs::remove_dir_all(source.path().join("evidence/a")).unwrap();
    fs::write(source.path().join("evidence/a"), b"new file").unwrap();
    for count in [5000, 0] {
        let batch = collect(source.path(), &allowed, &known).unwrap();
        assert!(batch.files.is_empty());
        assert!(batch.delete.len() <= 10000);
        let receipt = apply(target.path(), &allowed, &batch).unwrap();
        assert!(receipt.conflicts.is_empty());
        for (path, sha) in receipt.removed {
            assert_eq!(known.remove(&path).as_deref(), Some(sha.as_str()));
        }
        assert_eq!(known.len(), count);
        if count > 0 {
            assert!(receipt.pruned.is_empty());
            assert!(target.path().join("evidence/a").is_dir());
        } else {
            assert_eq!(receipt.pruned, vec!["evidence/a"]);
        }
    }
    let batch = collect(source.path(), &allowed, &known).unwrap();
    assert_eq!(
        apply(target.path(), &allowed, &batch)
            .unwrap()
            .accepted
            .len(),
        1
    );
    assert_eq!(
        fs::read(target.path().join("evidence/a")).unwrap(),
        b"new file"
    );
}

#[test]
fn source_inventory_is_bounded_across_directories_and_in_depth() {
    let source = tempfile::tempdir().unwrap();
    let mut deep = source.path().join("evidence");
    for _ in 0..65 {
        deep = deep.join("d");
    }
    fs::create_dir_all(&deep).unwrap();
    assert!(
        collect(
            source.path(),
            &[PathBuf::from("evidence")],
            &BTreeMap::new()
        )
        .is_err(),
        "unbounded directory depth accepted"
    );
    let broad = tempfile::tempdir().unwrap();
    for parent in ["left", "right"] {
        for child in 0..10001 {
            fs::create_dir_all(broad.path().join(format!("evidence/{parent}/{child}"))).unwrap();
        }
    }
    assert!(
        collect(broad.path(), &[PathBuf::from("evidence")], &BTreeMap::new()).is_err(),
        "per-directory limits allowed an oversized aggregate walk"
    );
}

#[test]
fn private_mirror_writers_serialize_on_the_pinned_root_before_mutating() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::write(source.path().join("handoff"), b"incoming").unwrap();
    let allowed = vec![PathBuf::from("handoff")];
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    let owner = fs::File::open(target.path()).unwrap();
    fs2::FileExt::try_lock_exclusive(&owner).unwrap();
    let contender = apply(target.path(), &allowed, &batch);
    assert!(contender.is_err(), "another mirror writer holds the lease");
    assert!(!target.path().join("handoff").exists());
    fs2::FileExt::unlock(&owner).unwrap();
    assert_eq!(
        apply(target.path(), &allowed, &batch)
            .unwrap()
            .accepted
            .len(),
        1
    );
    assert_eq!(
        fs::read(target.path().join("handoff")).unwrap(),
        b"incoming"
    );
}

#[test]
fn retained_mirror_lease_covers_local_writers_and_exchange_without_reacquisition() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    let allowed = vec![PathBuf::from("handoff")];
    fs::write(source.path().join("handoff"), b"incoming").unwrap();
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    let mut mirror = Mirror::acquire(target.path()).unwrap();
    assert!(Mirror::acquire(target.path()).is_err());
    assert!(apply(target.path(), &allowed, &batch).is_err());
    assert!(!target.path().join("handoff").exists());
    mirror.apply(&allowed, &batch).unwrap();
    fs::write(target.path().join("handoff"), b"authorized local edit").unwrap();
    assert_eq!(
        mirror.apply(&allowed, &batch).unwrap().conflicts,
        vec!["handoff"]
    );
    drop(mirror);
    assert_eq!(
        apply(target.path(), &allowed, &batch).unwrap().conflicts,
        vec!["handoff"]
    );
    assert_eq!(
        fs::read(target.path().join("handoff")).unwrap(),
        b"authorized local edit"
    );
}

#[test]
fn mirror_rejects_public_or_symlink_roots_before_any_mutation() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::write(source.path().join("handoff"), b"incoming").unwrap();
    let allowed = vec![PathBuf::from("handoff")];
    let batch = collect(source.path(), &allowed, &BTreeMap::new()).unwrap();
    fs::set_permissions(target.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(apply(target.path(), &allowed, &batch).is_err());
    assert!(!target.path().join("handoff").exists());
    fs::set_permissions(target.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let links = tempfile::tempdir().unwrap();
    symlink(target.path(), links.path().join("mirror")).unwrap();
    assert!(apply(&links.path().join("mirror"), &allowed, &batch).is_err());
    assert!(!target.path().join("handoff").exists());
}

#[test]
fn empty_allowlist_roots_never_grant_receiver_wide_mutation_scope() {
    let source = tempfile::tempdir().unwrap();
    let target = mirror_dir();
    fs::write(source.path().join("handoff"), b"incoming").unwrap();
    let batch = collect(source.path(), &[PathBuf::from("handoff")], &BTreeMap::new()).unwrap();
    for allowed in [
        vec![PathBuf::new()],
        vec![PathBuf::from("evidence"), PathBuf::new()],
    ] {
        assert!(apply(target.path(), &allowed, &batch).is_err());
        assert!(!target.path().join("handoff").exists());
    }
}
