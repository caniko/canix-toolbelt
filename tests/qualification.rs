#![cfg(feature = "qualification")]

use canix_toolbelt::qualification::{self as q, InventorySpec};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn current_review_source_file_modes_must_be_representable() {
    let root = tempdir().unwrap();
    let out = tempdir().unwrap();
    let source = root.path().join("source");
    fs::write(&source, b"source").unwrap();
    std::os::unix::fs::symlink("source", root.path().join("alias")).unwrap();
    for mode in [0o600, 0o666, 0o700, 0o744, 0o775, 0o644, 0o755] {
        fs::set_permissions(&source, fs::Permissions::from_mode(mode)).unwrap();
        for selection in ["source", "alias"] {
            let spec = InventorySpec {
                root: root.path().into(),
                paths: vec![selection.into()],
            };
            let representable = matches!(mode, 0o644 | 0o755);
            assert_eq!(
                q::inventory(&spec).is_ok(),
                representable,
                "{selection}: {mode:o}"
            );
            if !representable {
                assert!(
                    q::export_source(&spec, &out.path().join(format!("{selection}-{mode:o}.tar")))
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn current_review_local_pax_metadata_cannot_add_unrepresented_semantics() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source"), b"source").unwrap();
    let expected = q::inventory(&InventorySpec {
        root: root.path().into(),
        paths: vec!["source".into()],
    })
    .unwrap();
    let legacy = [("source".into(), expected.files["source"].sha256.clone())].into();
    for (index, key, value) in [
        (0, "SCHILY.xattr.user.qualification", "metadata"),
        (1, "SCHILY.acl.access", "user::rw-,group::r--,other::r--"),
        (2, "mtime", "123"),
        (3, "uid", "123"),
    ] {
        let archive = root.path().join(format!("pax-{index}.tar"));
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        builder
            .append_pax_extensions([(key, value.as_bytes())])
            .unwrap();
        let mut header = tar::Header::new_ustar();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(0o644);
        header.set_size(6);
        header.set_cksum();
        builder
            .append_data(&mut header, "source", &b"source"[..])
            .unwrap();
        builder.finish().unwrap();
        drop(builder);
        assert!(
            q::verify_source_archive(&archive, &expected).is_err(),
            "admitted local PAX {key}"
        );
        assert_eq!(q::verify_legacy_archive(&archive, &legacy).unwrap(), 1);
    }
}

#[cfg(unix)]
#[test]
fn final_review_archive_root_mode_and_timestamps_are_canonical() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::set_permissions(root.path().join("src"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.path().join("src/file"), b"source").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("file", root.path().join("src/link")).unwrap();
    let expected = q::inventory(&InventorySpec {
        root: root.path().into(),
        paths: vec!["src".into()],
    })
    .unwrap();
    for (bad_name, mode, mtime) in [
        (".", 0o000, 0),
        (".", 0o755, 123),
        ("src", 0o755, 123),
        ("src/file", 0o644, 123),
        ("src/link", 0o644, 123),
        ("", 0o755, 0),
    ] {
        let archive = root
            .path()
            .join(format!("{}.tar", bad_name.replace('/', "-")));
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        for name in [".", "src", "src/file", "src/link"] {
            let mut header = tar::Header::new_gnu();
            header.set_uid(0);
            header.set_gid(0);
            let directory = name == "." || name == "src";
            header.set_mode(if name == bad_name {
                mode
            } else if directory {
                0o755
            } else {
                0o644
            });
            header.set_mtime(if name == bad_name { mtime } else { 0 });
            header.set_size(if name == "src/file" { 6 } else { 0 });
            header.set_entry_type(if directory {
                tar::EntryType::Directory
            } else if name == "src/link" {
                tar::EntryType::Symlink
            } else {
                tar::EntryType::Regular
            });
            if name == "src/link" {
                header.set_link_name("file").unwrap();
            }
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    name,
                    if name == "src/file" {
                        &b"source"[..]
                    } else {
                        &b""[..]
                    },
                )
                .unwrap();
        }
        builder.finish().unwrap();
        drop(builder);
        assert_eq!(
            q::verify_source_archive(&archive, &expected).is_ok(),
            bad_name.is_empty(),
            "{bad_name}, mode={mode:o}, mtime={mtime}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn final_review_oversized_logs_are_rejected_before_run_and_seal_hashing() {
    let (root, receipt) = run_fixture("true", 30);
    let run = root.path().join("run");
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    let request = q::SealRequest {
        composition: receipt.composition,
        gates: [("required".into(), run.clone())].into(),
        expected_receipts: [(
            "required".into(),
            q::sha256_file(&run.join("receipt.json")).unwrap(),
        )]
        .into(),
        artifacts: Default::default(),
    };
    let handoff = root.path().join("handoff");
    q::seal(&request, &handoff).unwrap();
    for path in [
        run.join("stdout.log"),
        handoff.join("gates/required/stdout.log"),
    ] {
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(512 * 1024 * 1024 + 1)
            .unwrap();
    }
    for error in [
        q::verify_run(&run, None).unwrap_err(),
        q::verify_seal(&handoff, &request.composition, &request.expected_receipts).unwrap_err(),
    ] {
        assert!(error.to_string().contains("log size"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn final_review_git_discovery_bounds_entries_and_stderr_and_reaps() {
    let root = tempdir().unwrap();
    let git = root.path().join("git");
    for script in [
        "i=0; while [ $i -lt 131073 ]; do printf 'file-%s\\0' \"$i\"; i=$((i+1)); done",
        "head -c 67108865 /dev/zero >&2; exit 1",
    ] {
        fs::write(&git, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
        let result = q::git_inventory_spec(root.path(), &git);
        assert!(result.is_err(), "unrepresentable Git output was admitted");
        let error = result.unwrap_err();
        assert!(error.to_string().contains("budget"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn final_review_git_discovery_does_not_wait_for_inherited_pipe_writers() {
    let root = tempdir().unwrap();
    let git = root.path().join("git");
    fs::write(&git, "#!/bin/sh\nsleep 10 &\nprintf 'file\\0'\n").unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    let started = std::time::Instant::now();
    let error = q::git_inventory_spec(root.path(), &git).unwrap_err();
    assert!(error.to_string().contains("stayed open"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn final_review_actual_git_archive_remains_content_only_compatible() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/file"), b"source").unwrap();
    for args in [vec!["init"], vec!["add", "src/file"], vec!["write-tree"]] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(root.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let tree = std::process::Command::new("git")
        .arg("-C")
        .arg(root.path())
        .arg("write-tree")
        .output()
        .unwrap();
    let path = root.path().join("git.tar");
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(["archive", "--format=tar"])
            .arg(String::from_utf8(tree.stdout).unwrap().trim())
            .arg("-o")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let index = [(
        "src/file".into(),
        q::sha256_file(&root.path().join("src/file")).unwrap(),
    )]
    .into();
    assert_eq!(q::verify_legacy_archive(&path, &index).unwrap(), 1);
    let expected = q::inventory(&InventorySpec {
        root: root.path().into(),
        paths: vec!["src/file".into()],
    })
    .unwrap();
    assert!(q::verify_source_archive(&path, &expected).is_err());
}

#[test]
fn late_review_archive_ownership_is_canonical_only_for_current_evidence() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source"), b"source").unwrap();
    let expected = q::inventory(&InventorySpec {
        root: root.path().into(),
        paths: vec!["source".into()],
    })
    .unwrap();
    let legacy = [("source".into(), expected.files["source"].sha256.clone())].into();
    for (index, uid, gid, user, group) in [
        (0, 123, 0, "", ""),
        (1, 0, 123, "", ""),
        (2, 0, 0, "unexpected", ""),
        (3, 0, 0, "", "unexpected"),
    ] {
        let archive = root.path().join(format!("owner-{index}.tar"));
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_size(6);
        header.set_uid(uid);
        header.set_gid(gid);
        header.set_username(user).unwrap();
        header.set_groupname(group).unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "source", &b"source"[..])
            .unwrap();
        builder.finish().unwrap();
        drop(builder);
        assert!(
            q::verify_source_archive(&archive, &expected).is_err(),
            "ownership case {index} admitted"
        );
        assert_eq!(q::verify_legacy_archive(&archive, &legacy).unwrap(), 1);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn late_review_portable_receipts_require_effective_arguments_deadline_and_work_paths() {
    let (root, passed) = run_fixture("true", 30);
    assert_eq!(passed.status, q::RunStatus::Passed, "{passed:#?}");
    let run = root.path().join("run");
    q::verify_run(&run, None).unwrap();
    let original_request: serde_json::Value = q::read_json(&run.join("request.json")).unwrap();
    let original_receipt: serde_json::Value = q::read_json(&run.join("receipt.json")).unwrap();
    let original_boundary: serde_json::Value = q::read_json(&run.join("boundary.json")).unwrap();
    let mut admitted = Vec::new();
    for case in [
        "argumentZero",
        "elapsedMs",
        "lateElapsedMs",
        "QUALIFICATION_WORK_DIR",
        "TMPDIR",
        "HOME",
        "missingHome",
        "sandbox",
        "missingNull",
    ] {
        let mut request = original_request.clone();
        let mut receipt = original_receipt.clone();
        let mut boundary = original_boundary.clone();
        match case {
            "argumentZero" => {
                request.as_object_mut().unwrap().remove("argumentZero");
            }
            "elapsedMs" => {
                receipt["elapsedMs"] = (request["timeoutSeconds"].as_u64().unwrap() * 1000).into()
            }
            "lateElapsedMs" => {
                receipt["elapsedMs"] =
                    (request["timeoutSeconds"].as_u64().unwrap() * 1000 + 1).into()
            }
            "sandbox" => boundary["sandbox"] = "/unrelated/sandbox".into(),
            "missingNull" => {
                let mounts = boundary["read_only_paths"].as_array_mut().unwrap();
                assert!(mounts.iter().any(|path| path == "/dev/null"));
                mounts.retain(|path| path != "/dev/null");
            }
            name => {
                let key = if name == "missingHome" { "HOME" } else { name };
                for environment in [
                    &mut request["environment"],
                    &mut receipt["environment"],
                    &mut boundary["environment"],
                ] {
                    if name == "missingHome" {
                        environment.as_object_mut().unwrap().remove(key);
                    } else {
                        environment[key] = "/unrelated".into();
                    }
                }
            }
        }
        fs::write(run.join("request.json"), q::json_bytes(&request).unwrap()).unwrap();
        fs::write(run.join("boundary.json"), q::json_bytes(&boundary).unwrap()).unwrap();
        receipt["requestSha256"] = q::sha256_file(&run.join("request.json")).unwrap().into();
        receipt["boundarySha256"] = q::sha256_file(&run.join("boundary.json")).unwrap().into();
        fs::write(run.join("receipt.json"), q::json_bytes(&receipt).unwrap()).unwrap();
        if q::verify_run(&run, None).is_ok() {
            admitted.push(case);
        }
    }
    assert!(
        admitted.is_empty(),
        "impossible retained requests accepted: {admitted:?}"
    );
}

#[cfg(unix)]
#[test]
fn late_review_source_symlinks_do_not_silently_change_spelling() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("dir")).unwrap();
    fs::set_permissions(root.path().join("dir"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.path().join("dir/target"), b"source").unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["dir".into(), "alias".into()],
    };
    for target in ["dir//target", "dir/./target", "./dir//target"] {
        std::os::unix::fs::symlink(target, root.path().join("alias")).unwrap();
        assert!(
            q::inventory(&spec).is_err(),
            "normalized source link {target}"
        );
        fs::remove_file(root.path().join("alias")).unwrap();
    }
    std::os::unix::fs::symlink("dir/target", root.path().join("alias")).unwrap();
    assert_eq!(
        q::inventory(&spec).unwrap().files["alias"].link.as_deref(),
        Some("dir/target")
    );
}

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
fn inventories_reject_unrepresented_empty_directories() {
    let root = tempdir().unwrap();
    let out = tempdir().unwrap();
    fs::create_dir(root.path().join("tree")).unwrap();
    #[cfg(unix)]
    fs::set_permissions(root.path().join("tree"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.path().join("tree/file"), b"source").unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["tree".into()],
    };
    let expected = q::inventory(&spec).unwrap();
    fs::create_dir(root.path().join("tree/empty")).unwrap();
    assert!(q::inventory(&spec).is_err());
    assert!(q::export_source(&spec, &out.path().join("rejected.tar")).is_err());
    // A tar must not introduce the same unbound filesystem condition either.
    let archive = out.path().join("extra-directory.tar");
    let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
    for name in ["tree", "tree/empty", "tree/file"] {
        let file = name == "tree/file";
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(if file { 0o644 } else { 0o755 });
        header.set_entry_type(if file {
            tar::EntryType::Regular
        } else {
            tar::EntryType::Directory
        });
        header.set_size(if file { 6 } else { 0 });
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                name,
                if file { &b"source"[..] } else { &b""[..] },
            )
            .unwrap();
    }
    builder.finish().unwrap();
    assert!(q::verify_source_archive(&archive, &expected).is_err());
}

#[cfg(unix)]
#[test]
fn finalization_rejects_unrepresented_directory_modes() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let out = tempdir().unwrap();
    let tree = root.path().join("tree");
    fs::create_dir(&tree).unwrap();
    fs::set_permissions(&tree, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(tree.join("file"), b"source").unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["tree".into()],
    };
    let expected = q::inventory(&spec).unwrap();
    for mode in [0o700, 0o555, 0o775] {
        fs::set_permissions(&tree, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            fs::metadata(&tree).unwrap().permissions().mode() & 0o7777,
            mode
        );
        let archive = out.path().join(format!("mode-{mode:o}.tar"));
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        for name in ["tree", "tree/file"] {
            let file = name == "tree/file";
            let mut header = tar::Header::new_gnu();
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_mode(if file { 0o644 } else { mode });
            header.set_entry_type(if file {
                tar::EntryType::Regular
            } else {
                tar::EntryType::Directory
            });
            header.set_size(if file { 6 } else { 0 });
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    name,
                    if file { &b"source"[..] } else { &b""[..] },
                )
                .unwrap();
        }
        builder.finish().unwrap();
        let inventory_rejected = q::inventory(&spec).is_err();
        let export_rejected =
            q::export_source(&spec, &out.path().join(format!("export-{mode:o}.tar"))).is_err();
        let archive_rejected = q::verify_source_archive(&archive, &expected).is_err();
        assert!(
            inventory_rejected && export_rejected && archive_rejected,
            "mode {mode:o}: inventory={inventory_rejected}, export={export_rejected}, archive={archive_rejected}"
        );
    }
    fs::set_permissions(&tree, fs::Permissions::from_mode(0o755)).unwrap();
    let archive = out.path().join("canonical.tar");
    assert_eq!(q::export_source(&spec, &archive).unwrap(), expected);
    q::verify_source_archive(&archive, &expected).unwrap();
    // Nix filters setgid chmod. Feed that mode through the shared source/archive
    // mode validator using a tar header instead of depending on that syscall.
    let archive = out.path().join("setgid-directory.tar");
    let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
    let mut header = tar::Header::new_gnu();
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(0o2755);
    header.set_cksum();
    builder
        .append_data(&mut header, "tree", std::io::empty())
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_mode(0o644);
    header.set_size(6);
    header.set_cksum();
    builder
        .append_data(&mut header, "tree/file", &b"source"[..])
        .unwrap();
    builder.finish().unwrap();
    drop(builder);
    assert!(q::verify_source_archive(&archive, &expected).is_err());
}

#[test]
fn trust_selection_maps_reject_shadowed_json_keys() {
    let file = r#"{"sha256":"digest","executable":false,"link":null}"#;
    let inventory = format!(r#"{{"schemaVersion":1,"files":{{"source":{file},"source":{file}}}}}"#);
    assert!(serde_json::from_str::<q::Inventory>(&inventory).is_err());
    let seal = r#"{"composition":{"schemaVersion":1,"sources":{},"inputs":{}},"gates":{"test":"failed","test":"passed"},"expectedReceipts":{},"artifacts":{}}"#;
    assert!(serde_json::from_str::<q::SealRequest>(seal).is_err());
    for (field, value) in [
        ("sources", r#"{"root":"/source","paths":["file"]}"#),
        ("inputs", r#""/lock""#),
        ("tools", r#""/tool""#),
        ("environment", r#""setting""#),
        ("toolArgumentZeros", r#""alias""#),
    ] {
        let request = format!(
            r#"{{"cwd":"/source","sources":{{}},"inputs":{{}},"command":["true"],"timeoutSeconds":1,"{field}":{{"label":{value},"label":{value}}}}}"#
        );
        // Remove the single default field so failure can only be the duplicate
        // map key, rather than a repeated struct field.
        let request = request.replacen(&format!(r#""{field}":{{}},"#), "", 1);
        let error = serde_json::from_str::<q::RunRequest>(&request).unwrap_err();
        assert!(error.to_string().contains("duplicate"), "{field}: {error}");
    }
}

#[test]
fn public_json_reads_reject_oversized_documents_before_deserialization() {
    let root = tempdir().unwrap();
    let path = root.path().join("oversized.json");
    fs::File::create(&path)
        .unwrap()
        .set_len(65 * 1024 * 1024)
        .unwrap();
    let error = q::read_json::<serde_json::Value>(&path).unwrap_err();
    assert!(error.to_string().contains("exceeds 64 MiB"), "{error}");
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

#[cfg(unix)]
#[test]
fn portable_source_links_reject_drive_relative_targets() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("C:target"), b"source").unwrap();
    std::os::unix::fs::symlink("C:target", root.path().join("link")).unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["link".into()],
    };
    assert!(q::inventory(&spec).is_err());
    let archive = root.path().join("source.tar");
    assert!(q::export_source(&spec, &archive).is_err());
}

#[cfg(unix)]
#[test]
fn inventory_and_export_reject_hard_linked_source_files() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source"), b"source").unwrap();
    fs::hard_link(root.path().join("source"), root.path().join("alias")).unwrap();
    std::os::unix::fs::symlink("source", root.path().join("link")).unwrap();
    let archive = root.path().join("source.tar");
    for paths in [vec!["source", "alias"], vec!["source"], vec!["link"]] {
        let spec = InventorySpec {
            root: root.path().into(),
            paths: paths.into_iter().map(Into::into).collect(),
        };
        let error = q::inventory(&spec).unwrap_err();
        assert!(error.to_string().contains("hard link"), "{error}");
        assert!(q::export_source(&spec, &archive).is_err());
        assert!(!archive.exists());
    }
    // A byte-identical independent copy is representable; an inode alias is not.
    fs::remove_file(root.path().join("alias")).unwrap();
    fs::copy(root.path().join("source"), root.path().join("alias")).unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["source".into(), "alias".into(), "link".into()],
    };
    let inventory = q::export_source(&spec, &archive).unwrap();
    q::verify_source_archive(&archive, &inventory).unwrap();
}

#[cfg(target_os = "linux")]
fn run_fixture(script: &str, timeout_seconds: u64) -> (tempfile::TempDir, q::Receipt) {
    // The no-op seed provisions a retained request for the test below it. Cold
    // hosted namespace/dependency admission can exceed five seconds; keep the
    // actual timeout/cancellation payloads' requested budgets unchanged.
    let timeout_seconds = if script == "true" {
        timeout_seconds.max(30)
    } else {
        timeout_seconds
    };
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
        argument_zero: None,
        environment: Default::default(),
        tools: Default::default(),
        tool_argument_zeros: Default::default(),
        runtime_paths: Default::default(),
        timeout_seconds,
        containment: Some(q::ContainmentPrograms {
            unshare: std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|p| p.join("unshare"))
                .find(|p| p.is_file())
                .unwrap(),
            executor: env!("CARGO_BIN_EXE_canix-toolbelt-qualification-executor").into(),
            setpriv: std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|p| p.join("setpriv"))
                .find(|p| p.is_file())
                .unwrap(),
        }),
    };
    let receipt = q::execute(&request, &root.path().join("run"), &Default::default())
        .unwrap_or_else(|error| {
            panic!(
                "fixture capture failed: {error}; timeout={timeout_seconds}s; executor bytes={}",
                fs::metadata(&request.containment.as_ref().unwrap().executor)
                    .unwrap()
                    .len()
            )
        });
    (root, receipt)
}

#[cfg(target_os = "linux")]
#[test]
fn review_resolution_probe_work_files_are_not_limited_to_one_mib() {
    let (root, _) = run_fixture("true", 30);
    let tool = root.path().join("cache-tool");
    fs::write(&tool, b"#!/bin/sh\nset -eu\nhead -c 2097152 /dev/zero > \"$QUALIFICATION_WORK_DIR/cache\"\nprintf tiny-version\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.tools.insert("cache".into(), tool);
    request.command[2] = "test \"$(wc -c < \"$QUALIFICATION_WORK_DIR/cache\")\" = 2097152".into();
    let run = root.path().join("probe-cache");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(receipt.tools["cache"].version, "tiny-version");
    q::verify_run(&run, None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn review_resolution_probe_log_overflow_is_latched_even_when_handled() {
    for stderr in [false, true] {
        let (root, _) = run_fixture("true", 30);
        let tool = root.path().join("noisy-tool");
        fs::write(
            &tool,
            format!(
                "#!/bin/sh\ntrap '' PIPE\nhead -c 2097152 /dev/zero {}\nexit 0\n",
                if stderr { ">&2" } else { "" }
            ),
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let mut request: q::RunRequest =
            q::read_json(&root.path().join("run/request.json")).unwrap();
        request.tools.insert("noisy".into(), tool);
        request.command[2] = "printf main-ran".into();
        let run = root.path().join("probe-overflow");
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        assert_eq!(receipt.status, q::RunStatus::Failed, "{receipt:#?}");
        assert!(fs::read(run.join("stdout.log")).unwrap().is_empty());
        for file in ["tool-noisy.log", "tool-noisy.log.stderr"] {
            assert!(fs::metadata(run.join(file)).unwrap().len() <= 1024 * 1024);
        }
        assert!(q::verify_run(&run, None).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn review_resolution_portable_program_paths_must_be_canonical_and_absolute() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    // Ubuntu's /bin/sh is Dash, which rejects the required --version probe.
    request.command[0] = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("bash"))
        .find(|path| path.is_file())
        .unwrap()
        .to_str()
        .unwrap()
        .into();
    request
        .tools
        .insert("shell".into(), request.command[0].clone().into());
    let run = root.path().join("program-paths");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    q::verify_run(&run, None).unwrap();
    let original_request: serde_json::Value = q::read_json(&run.join("request.json")).unwrap();
    let original_receipt: serde_json::Value = q::read_json(&run.join("receipt.json")).unwrap();
    let original_boundary: serde_json::Value = q::read_json(&run.join("boundary.json")).unwrap();
    let mut admitted = Vec::new();
    for name in ["unshare", "executor", "setpriv", "executable", "shell"] {
        for prefix in ["relative/", "/tmp/../", "/tmp/./"] {
            let mut request = original_request.clone();
            let mut receipt = original_receipt.clone();
            let mut boundary = original_boundary.clone();
            let path = match name {
                "executable" => &mut request["command"][0],
                "shell" => &mut request["tools"][name],
                _ => &mut request["containment"][name],
            };
            let old = path.as_str().unwrap().to_owned();
            let changed = format!("{prefix}{}", old.trim_start_matches('/'));
            *path = changed.clone().into();
            match name {
                "executable" => receipt["executable"]["path"] = changed.clone().into(),
                "shell" => receipt["tools"][name]["path"] = changed.clone().into(),
                _ => receipt["containment"][name]["path"] = changed.clone().into(),
            }
            // Main and tool may share the shell's canonical inode/path.
            if old == original_request["command"][0].as_str().unwrap() {
                request["command"][0] = changed.clone().into();
                request["tools"]["shell"] = changed.clone().into();
                receipt["executable"]["path"] = changed.clone().into();
                receipt["tools"]["shell"]["path"] = changed.clone().into();
            }
            let file = boundary["snapshot_files"]
                .as_object_mut()
                .unwrap()
                .remove(&old)
                .unwrap();
            boundary["snapshot_files"]
                .as_object_mut()
                .unwrap()
                .insert(changed.clone(), file);
            for mount in boundary["snapshot_mounts"].as_array_mut().unwrap() {
                if mount.as_str() == Some(&old) {
                    *mount = changed.clone().into();
                }
            }
            fs::write(run.join("request.json"), q::json_bytes(&request).unwrap()).unwrap();
            fs::write(run.join("boundary.json"), q::json_bytes(&boundary).unwrap()).unwrap();
            receipt["requestSha256"] = q::sha256_file(&run.join("request.json")).unwrap().into();
            receipt["boundarySha256"] = q::sha256_file(&run.join("boundary.json")).unwrap().into();
            fs::write(run.join("receipt.json"), q::json_bytes(&receipt).unwrap()).unwrap();
            if q::verify_run(&run, None).is_ok() {
                admitted.push((name, prefix));
            }
        }
    }
    assert!(
        admitted.is_empty(),
        "noncanonical program paths accepted: {admitted:?}"
    );
}

#[test]
fn merge_repair_archive_regular_files_require_canonical_modes() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source"), b"source").unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["source".into()],
    };
    let mut inventory = q::inventory(&spec).unwrap();
    let mut admitted = Vec::new();
    for mode in [
        0o4755, 0o2755, 0o1755, 0o777, 0o700, 0o640, 0o4644, 0o666, 0, 0o755, 0o644, 0o104755,
        0o40644, 0o100644, 0o100755,
    ] {
        inventory.files.get_mut("source").unwrap().executable = mode & 0o111 != 0;
        let archive = root.path().join(format!("mode-{mode:o}.tar"));
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(6);
        header.set_mode(mode);
        header.set_cksum();
        builder
            .append_data(&mut header, "source", &b"source"[..])
            .unwrap();
        builder.finish().unwrap();
        drop(builder);
        let accepted = q::verify_source_archive(&archive, &inventory).is_ok();
        if accepted != matches!(mode, 0o644 | 0o755 | 0o100644 | 0o100755) {
            admitted.push(mode);
        }
    }
    assert!(
        admitted.is_empty(),
        "incorrectly admitted/rejected modes: {admitted:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn merge_repair_hard_linked_symlink_inodes_are_not_representable() {
    use std::os::unix::fs::MetadataExt;
    let root = tempdir().unwrap();
    fs::write(root.path().join("target"), b"target").unwrap();
    std::os::unix::fs::symlink("target", root.path().join("link")).unwrap();
    fs::hard_link(root.path().join("link"), root.path().join("alias")).unwrap();
    assert_eq!(fs::metadata(root.path().join("target")).unwrap().nlink(), 1);
    assert_eq!(
        fs::symlink_metadata(root.path().join("link"))
            .unwrap()
            .nlink(),
        2
    );
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["target".into(), "link".into(), "alias".into()],
    };
    let archive = root.path().join("source.tar");
    let inventoried = q::inventory(&spec).is_ok();
    let exported = q::export_source(&spec, &archive).is_ok();
    assert!(
        !inventoried && !exported,
        "inventory={inventoried}, export={exported}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn successor_portable_source_selections_must_cover_the_retained_inventory() {
    let (root, _) = run_fixture("true", 30);
    fs::create_dir(root.path().join("sub")).unwrap();
    fs::set_permissions(root.path().join("sub"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("../source", root.path().join("sub/link")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request
        .sources
        .get_mut("producer")
        .unwrap()
        .paths
        .push("sub".into());
    let run = root.path().join("selected-run");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    let request: q::RunRequest = q::read_json(&run.join("request.json")).unwrap();
    let boundary: serde_json::Value = q::read_json(&run.join("boundary.json")).unwrap();
    let handoff = root.path().join("handoff");
    q::seal(
        &q::SealRequest {
            composition: receipt.composition.clone(),
            gates: [("build".into(), run.clone())].into(),
            expected_receipts: [(
                "build".into(),
                q::sha256_file(&run.join("receipt.json")).unwrap(),
            )]
            .into(),
            artifacts: Default::default(),
        },
        &handoff,
    )
    .unwrap();
    let original_seal = fs::read(handoff.join("seal.json")).unwrap();
    let mut admitted = Vec::new();
    for (paths, expected) in [
        (vec![], false),
        (vec!["unrelated"], false),
        (vec!["../source"], false),
        (vec!["source", "absent"], false),
        (vec!["source", "./sub/link"], false),
        (vec!["source", "./sub"], true),
    ] {
        let mut request = request.clone();
        request.sources.get_mut("producer").unwrap().paths = paths.iter().map(Into::into).collect();
        let mut boundary = boundary.clone();
        let mounts = boundary["snapshot_mounts"].as_array_mut().unwrap();
        mounts.retain(|path| {
            ![root.path().join("source"), root.path().join("sub")]
                .iter()
                .any(|selected| path.as_str() == selected.to_str())
        });
        mounts.extend(
            paths
                .iter()
                .map(|path| serde_json::json!(root.path().join(path))),
        );
        fs::write(run.join("request.json"), q::json_bytes(&request).unwrap()).unwrap();
        fs::write(run.join("boundary.json"), q::json_bytes(&boundary).unwrap()).unwrap();
        let mut changed = receipt.clone();
        changed.request_sha256 = q::sha256_file(&run.join("request.json")).unwrap();
        changed.boundary_sha256 = q::sha256_file(&run.join("boundary.json")).unwrap();
        fs::write(run.join("receipt.json"), q::json_bytes(&changed).unwrap()).unwrap();
        let required: q::ArtifactIndex = [(
            "build".into(),
            q::sha256_file(&run.join("receipt.json")).unwrap(),
        )]
        .into();
        let mut seal: q::Seal = serde_json::from_slice(&original_seal).unwrap();
        seal.gates = required.clone();
        for file in ["request.json", "boundary.json", "receipt.json"] {
            let path = format!("gates/build/{file}");
            fs::copy(run.join(file), handoff.join(&path)).unwrap();
            seal.artifacts
                .insert(path, q::sha256_file(&run.join(file)).unwrap());
        }
        fs::write(handoff.join("seal.json"), q::json_bytes(&seal).unwrap()).unwrap();
        if q::verify_run(&run, None).is_ok() != expected
            || q::verify_seal(&handoff, &changed.composition, &required).is_ok() != expected
        {
            admitted.push(paths);
        }
    }
    assert!(
        admitted.is_empty(),
        "incorrect source selection verdicts: {admitted:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn successor_source_snapshot_timestamps_match_canonical_archive_times() {
    let (root, _) = run_fixture("true", 30);
    fs::create_dir(root.path().join("selected")).unwrap();
    fs::set_permissions(
        root.path().join("selected"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::write(root.path().join("selected/file"), b"selected").unwrap();
    std::os::unix::fs::symlink("file", root.path().join("selected/link")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request
        .sources
        .get_mut("producer")
        .unwrap()
        .paths
        .push("selected".into());
    let script = "set -eu; for path in . .. source selected selected/file selected/link; do stat -c '%X:%Y' \"$path\"; test \"$(stat -c '%X:%Y' \"$path\")\" = 0:0; done";
    request.command[2] = script.into();
    let tool = root.path().join("timestamp-tool");
    fs::write(&tool, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools.insert("timestamps".into(), tool);
    let mut compositions = Vec::new();
    for time in [1, 2] {
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(time);
        fs::File::open(root.path().join("selected/file"))
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_accessed(timestamp)
                    .set_modified(timestamp),
            )
            .unwrap();
        let run = root.path().join(format!("timestamps-{time}"));
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        assert_eq!(
            receipt.status,
            q::RunStatus::Passed,
            "{receipt:#?}; {}",
            fs::read_to_string(run.join("stdout.log")).unwrap()
        );
        assert_eq!(receipt.tools["timestamps"].version, "0:0\n".repeat(6));
        q::verify_run(&run, None).unwrap();
        compositions.push(receipt.composition);
    }
    assert_eq!(compositions[0], compositions[1]);
}

#[cfg(target_os = "linux")]
#[test]
fn successor_replaced_log_paths_cannot_become_qualified_output() {
    let mut accepted = Vec::new();
    for probe in [false, true] {
        let (root, _) = run_fixture("true", 30);
        let mut request: q::RunRequest =
            q::read_json(&root.path().join("run/request.json")).unwrap();
        request.command[2] = "printf native-output; printf native-error >&2".into();
        if probe {
            request.tools.insert("probe".into(), "/bin/sh".into());
        }
        let run = root.path().join("replaced-logs");
        let unshare = request.containment.as_ref().unwrap().unshare.clone();
        let mv = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("mv"))
            .find(|path| path.is_file())
            .unwrap();
        let launcher = root.path().join("replace-logs-launcher");
        let logs = if probe {
            ["tool-probe.log", "tool-probe.log.stderr"]
        } else {
            ["stdout.log", "stderr.log"]
        };
        fs::write(&launcher, format!("#!/bin/sh\n{unshare:?} \"$@\"\nstatus=$?\nfor log in {:?} {:?}; do {mv:?} \"$log\" \"$log.original\"; printf forged > \"$log\"; done\nexit \"$status\"\n", run.join(logs[0]), run.join(logs[1]))).unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        request.containment.as_mut().unwrap().unshare = launcher;
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        if receipt.status == q::RunStatus::Passed || q::verify_run(&run, None).is_ok() {
            accepted.push(probe);
        }
    }
    assert!(
        accepted.is_empty(),
        "forged log paths accepted, probe={accepted:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn new_review_same_inode_log_injection_cannot_qualify() {
    for probe in [false, true] {
        let (root, _) = run_fixture("true", 30);
        let mut request: q::RunRequest =
            q::read_json(&root.path().join("run/request.json")).unwrap();
        request.command[2] = "printf native-output; printf native-error >&2".into();
        if probe {
            request.tools.insert("probe".into(), "/bin/sh".into());
        }
        let run = root.path().join("inplace-logs");
        let unshare = &request.containment.as_ref().unwrap().unshare;
        let launcher = root.path().join("inplace-launcher");
        let logs = if probe {
            ["tool-probe.log", "tool-probe.log.stderr"]
        } else {
            ["stdout.log", "stderr.log"]
        };
        fs::write(&launcher, format!("#!/bin/sh\n{unshare:?} \"$@\"\nstatus=$?\nfor log in {:?} {:?}; do printf injected > \"$log\"; done\nexit \"$status\"\n", run.join(logs[0]), run.join(logs[1]))).unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        request.containment.as_mut().unwrap().unshare = launcher;
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        assert_ne!(
            receipt.status,
            q::RunStatus::Passed,
            "probe={probe}: {receipt:#?}"
        );
        assert!(q::verify_run(&run, None).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn new_review_live_preview_writes_never_select_completed_log_bytes() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command[2] = "printf ready; printf native-error >&2; kill -STOP $$; printf done".into();
    let pid_file = record_launcher(&mut request, root.path());
    let run = root.path().join("preview-injection");
    let receipt = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &run, &Default::default()).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while fs::read(run.join("stdout.log.live")).unwrap_or_default() != b"ready"
            && !task.is_finished()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(fs::read(run.join("stdout.log.live")).unwrap(), b"ready");
        assert!(!run.join("stdout.log").exists());
        for name in ["stdout.log.live", "stderr.log.live"] {
            fs::write(run.join(name), b"injected").unwrap();
        }
        while !task.is_finished() && std::time::Instant::now() < deadline {
            resume_launcher(&pid_file);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        task.join().unwrap()
    });
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(fs::read(run.join("stdout.log")).unwrap(), b"readydone");
    assert_eq!(fs::read(run.join("stderr.log")).unwrap(), b"native-error");
    q::verify_run(&run, None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn new_review_replaced_work_cannot_overwrite_an_unrelated_directory() {
    let (root, _) = run_fixture("true", 30);
    let victim = tempdir().unwrap();
    fs::write(victim.path().join("keep"), b"unrelated").unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let run = root.path().join("redirected-work");
    let launcher = root.path().join("redirect-work-launcher");
    let unshare = &request.containment.as_ref().unwrap().unshare;
    let program = |name: &str| {
        std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join(name))
            .find(|path| path.is_file())
            .unwrap()
    };
    let mv = program("mv");
    let ln = program("ln");
    fs::write(
        &launcher,
        format!(
            "#!/bin/sh\nset -eu\n{mv:?} {:?} {:?}\n{ln:?} -s {:?} {:?}\nexec {unshare:?} \"$@\"\n",
            run.join("work"),
            run.join("work.original"),
            victim.path(),
            run.join("work")
        ),
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    request.containment.as_mut().unwrap().unshare = launcher;
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(fs::read(victim.path().join("keep")).unwrap(), b"unrelated");
    assert_ne!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert!(q::verify_run(&run, None).is_err());
}

#[test]
fn new_review_directory_selection_ignores_lexically_adjacent_siblings() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("a")).unwrap();
    fs::write(root.path().join("a/file"), b"nested").unwrap();
    fs::write(root.path().join("a-foo"), b"sibling").unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["a".into(), "a-foo".into()],
    };
    let inventory = q::inventory(&spec).unwrap();
    assert_eq!(inventory.files.len(), 2);
    let out = tempdir().unwrap();
    q::export_source(&spec, &out.path().join("source.tar")).unwrap();
    q::verify_source_archive(&out.path().join("source.tar"), &inventory).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn new_review_probe_work_timestamps_reach_the_next_probe_and_main() {
    let (root, _) = run_fixture("true", 30);
    let create = root.path().join("create-cache");
    let inspect = root.path().join("inspect-cache");
    fs::write(&create, b"#!/bin/sh\nset -eu\ncd \"$QUALIFICATION_WORK_DIR\"\nmkdir nested\nprintf cache > nested/file\nln -s file nested/link\ntouch -h -d @123 nested/file nested/link nested .\nprintf created\n").unwrap();
    let check = "set -eu; cd \"$QUALIFICATION_WORK_DIR\"; for path in nested/file nested/link nested .; do test \"$(stat -c %X:%Y \"$path\")\" = 123:123; done; printf preserved";
    fs::write(&inspect, format!("#!/bin/sh\n{check}\n")).unwrap();
    for path in [&create, &inspect] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.tools.insert("a-create".into(), create);
    request.tools.insert("b-inspect".into(), inspect);
    request.command[2] = check.into();
    let run = root.path().join("timestamp-transitions");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(receipt.tools["b-inspect"].version, "preserved");
    assert_eq!(fs::read(run.join("stdout.log")).unwrap(), b"preserved");
    q::verify_run(&run, None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn new_review_portable_cwd_input_runtime_and_sandbox_paths_are_canonical() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.runtime_paths.push(request.inputs["lock"].clone());
    let run = root.path().join("portable-paths");
    q::execute(&request, &run, &Default::default()).unwrap();
    q::verify_run(&run, None).unwrap();
    let original_request: serde_json::Value = q::read_json(&run.join("request.json")).unwrap();
    let original_receipt: serde_json::Value = q::read_json(&run.join("receipt.json")).unwrap();
    let original_boundary: serde_json::Value = q::read_json(&run.join("boundary.json")).unwrap();
    let mut accepted = Vec::new();
    for field in ["cwd", "input", "sandbox"] {
        for prefix in ["relative/", "/tmp/../", "/tmp/./"] {
            let mut request = original_request.clone();
            let mut receipt = original_receipt.clone();
            let mut boundary = original_boundary.clone();
            let old = match field {
                "cwd" => request["cwd"].as_str(),
                "input" => request["inputs"]["lock"].as_str(),
                _ => boundary["sandbox"].as_str(),
            }
            .unwrap()
            .to_owned();
            let changed = format!("{prefix}{}", old.trim_start_matches('/'));
            match field {
                "cwd" => request["cwd"] = changed.into(),
                "sandbox" => boundary["sandbox"] = changed.into(),
                _ => {
                    request["inputs"]["lock"] = changed.clone().into();
                    request["runtimePaths"][0] = changed.clone().into();
                    let file = boundary["snapshot_files"]
                        .as_object_mut()
                        .unwrap()
                        .remove(&old)
                        .unwrap();
                    boundary["snapshot_files"]
                        .as_object_mut()
                        .unwrap()
                        .insert(changed.clone(), file);
                    for path in boundary["snapshot_mounts"].as_array_mut().unwrap() {
                        if path.as_str() == Some(&old) {
                            *path = changed.clone().into();
                        }
                    }
                }
            }
            fs::write(run.join("request.json"), q::json_bytes(&request).unwrap()).unwrap();
            fs::write(run.join("boundary.json"), q::json_bytes(&boundary).unwrap()).unwrap();
            receipt["requestSha256"] = q::sha256_file(&run.join("request.json")).unwrap().into();
            receipt["boundarySha256"] = q::sha256_file(&run.join("boundary.json")).unwrap().into();
            fs::write(run.join("receipt.json"), q::json_bytes(&receipt).unwrap()).unwrap();
            if q::verify_run(&run, None).is_ok() {
                accepted.push((field, prefix));
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "noncanonical paths accepted: {accepted:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn successor_host_work_injection_cannot_cross_invocation_boundaries() {
    let mut accepted = Vec::new();
    for after_probe in [false, true] {
        let (root, _) = run_fixture("true", 30);
        let mut request: q::RunRequest =
            q::read_json(&root.path().join("run/request.json")).unwrap();
        let run = root.path().join("injected-work");
        let tool = root.path().join("seed-work");
        fs::write(
            &tool,
            b"#!/bin/sh\nprintf approved > \"$QUALIFICATION_WORK_DIR/config\"\nprintf version\n",
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        if after_probe {
            request.tools.insert("seed".into(), tool);
        }
        request.command[2] =
            "test \"$(cat \"$QUALIFICATION_WORK_DIR/config\")\" = injected && printf injected-main"
                .into();
        let unshare = request.containment.as_ref().unwrap().unshare.clone();
        let launcher = root.path().join("inject-work-launcher");
        let injection = format!("printf injected > {:?}\n", run.join("work/config"));
        fs::write(
            &launcher,
            format!(
                "#!/bin/sh\n{}\n{unshare:?} \"$@\"\nstatus=$?\n{}\nexit \"$status\"\n",
                if after_probe { "" } else { &injection },
                if after_probe { &injection } else { "" }
            ),
        )
        .unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        request.containment.as_mut().unwrap().unshare = launcher;
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        if receipt.status == q::RunStatus::Passed || q::verify_run(&run, None).is_ok() {
            accepted.push(after_probe);
        }
    }
    assert!(
        accepted.is_empty(),
        "injected work qualified, after_probe={accepted:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn merge_repair_snapshot_parent_modes_do_not_inherit_launcher_umask() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let unshare = request.containment.as_ref().unwrap().unshare.clone();
    let launcher = root.path().join("umask-launcher");
    request.containment.as_mut().unwrap().unshare = launcher.clone();
    let mut failures = Vec::new();
    for mask in [0o022, 0o077, 0o777] {
        fs::write(
            &launcher,
            format!("#!/bin/sh\numask {mask:04o}\nexec {unshare:?} \"$@\"\n"),
        )
        .unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        request.command[2] = format!(
            "set -eu; test \"$(stat -c %a .)\" = 755; test \"$(stat -c %a ..)\" = 755; test \"$(stat -c %a /.qualification-boundary.json)\" = 644; test \"$(umask)\" = {mask:04o}; cat source"
        );
        let run = root.path().join(format!("mask-{mask:o}"));
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        if receipt.status != q::RunStatus::Passed {
            failures.push((mask, fs::read_to_string(run.join("stderr.log")).unwrap()));
        } else {
            q::verify_run(&run, None).unwrap();
        }
    }
    assert!(
        failures.is_empty(),
        "umask-dependent execution: {failures:?}"
    );
}

#[cfg(target_os = "linux")]
fn record_launcher(request: &mut q::RunRequest, root: &std::path::Path) -> std::path::PathBuf {
    let containment = request.containment.as_mut().unwrap();
    let unshare = &containment.unshare;
    let launcher = root.join("record-launcher");
    let pid_file = root.join("launcher.pid");
    fs::write(
        &launcher,
        format!("#!/bin/sh\nprintf '%s' \"$$\" > {pid_file:?}\nexec {unshare:?} \"$@\"\n"),
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    containment.unshare = launcher;
    pid_file
}

#[cfg(target_os = "linux")]
fn resume_launcher(pid_file: &std::path::Path) {
    use rustix::process::{Pid, Signal, kill_process_group};
    let group = Pid::from_raw(fs::read_to_string(pid_file).unwrap().parse().unwrap()).unwrap();
    let _ = kill_process_group(group, Signal::CONT);
}

#[cfg(target_os = "linux")]
#[test]
fn private_work_tmpfs_is_writable_after_chroot() {
    let (root, receipt) = run_fixture(
        r#"set -eu
        test "$(stat -f -c %T "$QUALIFICATION_WORK_DIR")" = tmpfs
        for dir in "$QUALIFICATION_WORK_DIR" "$TMPDIR" "$HOME" "$CARGO_HOME" "$CARGO_TARGET_DIR"; do
            mkdir -p "$dir"
            printf writable > "$dir/probe"
        done
        if (printf changed > source) 2>/dev/null; then exit 91; fi"#,
        30,
    );
    let run = root.path().join("run");
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    for name in ["probe", "home/probe", "cargo-home/probe", "target/probe"] {
        assert_eq!(fs::read(run.join("work").join(name)).unwrap(), b"writable");
    }
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
    q::verify_run(&run, Some(&receipt.composition)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn finalization_portable_receipts_and_seals_reject_malformed_input_or_source_digests() {
    let (root, receipt) = run_fixture("true", 30);
    let run = root.path().join("run");
    let handoff = root.path().join("handoff");
    q::seal(
        &q::SealRequest {
            composition: receipt.composition.clone(),
            gates: [("build".into(), run.clone())].into(),
            expected_receipts: [(
                "build".into(),
                q::sha256_file(&run.join("receipt.json")).unwrap(),
            )]
            .into(),
            artifacts: Default::default(),
        },
        &handoff,
    )
    .unwrap();
    for (source, malformed) in [false, true].into_iter().flat_map(|source| {
        [
            String::new(),
            "g".repeat(64),
            "a".repeat(63),
            "a".repeat(65),
        ]
        .into_iter()
        .map(move |malformed| (source, malformed))
    }) {
        let mut rewritten = receipt.clone();
        if source {
            let inventory = rewritten.inventories.get_mut("producer").unwrap();
            inventory.files.get_mut("source").unwrap().sha256 = malformed;
            rewritten
                .composition
                .sources
                .insert("producer".into(), q::fingerprint(inventory).unwrap());
        } else {
            rewritten
                .composition
                .inputs
                .insert("lock".into(), malformed);
        }
        rewritten.after = Some(rewritten.composition.clone());
        let bytes = q::json_bytes(&rewritten).unwrap();
        fs::write(run.join("receipt.json"), &bytes).unwrap();
        fs::write(handoff.join("gates/build/receipt.json"), &bytes).unwrap();
        let hash = q::sha256_file(&run.join("receipt.json")).unwrap();
        let required: q::ArtifactIndex = [("build".into(), hash.clone())].into();
        let mut seal: q::Seal = q::read_json(&handoff.join("seal.json")).unwrap();
        seal.composition = rewritten.composition.clone();
        seal.gates = required.clone();
        seal.artifacts
            .insert("gates/build/receipt.json".into(), hash);
        fs::write(handoff.join("seal.json"), q::json_bytes(&seal).unwrap()).unwrap();
        assert!(q::verify_run(&run, Some(&rewritten.composition)).is_err());
        assert!(q::verify_seal(&handoff, &rewritten.composition, &required).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn finalization_rejects_unselected_symlink_targets() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 30);
    fs::create_dir(root.path().join("sub")).unwrap();
    fs::set_permissions(root.path().join("sub"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.path().join("target"), b"selected target").unwrap();
    std::os::unix::fs::symlink("../target", root.path().join("sub/link")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.sources.get_mut("producer").unwrap().paths = vec!["sub".into()];
    request.command[2] = "test -L sub/link && cat sub/link".into();
    let rejected = root.path().join("unselected-link-target");
    let error = q::execute(&request, &rejected, &Default::default()).unwrap_err();
    assert!(error.to_string().contains("symlink target"), "{error}");
    assert!(
        !rejected.exists(),
        "unmounted targets must fail before launch"
    );
    request
        .sources
        .get_mut("producer")
        .unwrap()
        .paths
        .push("target".into());
    let accepted = root.path().join("selected-link-target");
    let receipt = q::execute(&request, &accepted, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(
        fs::read(accepted.join("stdout.log")).unwrap(),
        b"selected target"
    );
    q::verify_run(&accepted, None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn review_round_portable_runs_and_seals_reject_unselected_link_targets() {
    let (root, _) = run_fixture("true", 30);
    fs::create_dir(root.path().join("sub")).unwrap();
    fs::set_permissions(root.path().join("sub"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.path().join("target"), b"target").unwrap();
    std::os::unix::fs::symlink("../target", root.path().join("sub/link")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.sources.get_mut("producer").unwrap().paths = vec!["sub".into(), "target".into()];
    let run = root.path().join("linked");
    let mut receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    let handoff = root.path().join("handoff");
    q::seal(
        &q::SealRequest {
            composition: receipt.composition.clone(),
            gates: [("build".into(), run.clone())].into(),
            expected_receipts: [(
                "build".into(),
                q::sha256_file(&run.join("receipt.json")).unwrap(),
            )]
            .into(),
            artifacts: Default::default(),
        },
        &handoff,
    )
    .unwrap();

    // Rebind every containing identity too: rejection must be semantic, not an
    // accidental stale request, boundary, composition or external receipt hash.
    let inventory = receipt.inventories.get_mut("producer").unwrap();
    inventory.files.remove("target");
    receipt
        .composition
        .sources
        .insert("producer".into(), q::fingerprint(inventory).unwrap());
    receipt.after = Some(receipt.composition.clone());
    let mut request: q::RunRequest = q::read_json(&run.join("request.json")).unwrap();
    request.sources.get_mut("producer").unwrap().paths = vec!["sub".into()];
    fs::write(run.join("request.json"), q::json_bytes(&request).unwrap()).unwrap();
    receipt.request_sha256 = q::sha256_file(&run.join("request.json")).unwrap();
    let mut boundary: serde_json::Value = q::read_json(&run.join("boundary.json")).unwrap();
    let target = root.path().join("target");
    boundary["snapshot_files"]
        .as_object_mut()
        .unwrap()
        .remove(target.to_str().unwrap());
    boundary["snapshot_mounts"]
        .as_array_mut()
        .unwrap()
        .retain(|path| path.as_str() != target.to_str());
    fs::write(run.join("boundary.json"), q::json_bytes(&boundary).unwrap()).unwrap();
    receipt.boundary_sha256 = q::sha256_file(&run.join("boundary.json")).unwrap();
    fs::write(run.join("receipt.json"), q::json_bytes(&receipt).unwrap()).unwrap();
    let required: q::ArtifactIndex = [(
        "build".into(),
        q::sha256_file(&run.join("receipt.json")).unwrap(),
    )]
    .into();
    let mut seal: q::Seal = q::read_json(&handoff.join("seal.json")).unwrap();
    seal.composition = receipt.composition.clone();
    seal.gates = required.clone();
    for file in ["request.json", "boundary.json", "receipt.json"] {
        let path = format!("gates/build/{file}");
        fs::copy(run.join(file), handoff.join(&path)).unwrap();
        seal.artifacts
            .insert(path, q::sha256_file(&run.join(file)).unwrap());
    }
    fs::write(handoff.join("seal.json"), q::json_bytes(&seal).unwrap()).unwrap();
    let run_rejected = q::verify_run(&run, Some(&receipt.composition)).is_err();
    let seal_rejected = q::verify_seal(&handoff, &receipt.composition, &required).is_err();
    assert!(
        run_rejected && seal_rejected,
        "run rejected={run_rejected}, seal rejected={seal_rejected}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn review_round_bootstrap_executor_substitution_cannot_forge_success() {
    for replace in [false, true] {
        let (root, _) = run_fixture("true", 30);
        let mut request: q::RunRequest =
            q::read_json(&root.path().join("run/request.json")).unwrap();
        let tools = request.containment.as_mut().unwrap();
        let executor = root.path().join("executor");
        fs::copy(&tools.executor, &executor).unwrap();
        tools.executor = executor.clone();
        let unshare = tools.unshare.clone();
        let fake = root.path().join("fake-executor");
        fs::write(&fake, b"#!/bin/sh\nprintf '{\"exitCode\":0,\"signal\":null,\"descendantsDrained\":true}' >&0\nprintf forged-bootstrap\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let backup = root.path().join("executor.approved");
        let launcher = root.path().join("launcher");
        let cp = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("cp"))
            .find(|path| path.is_file())
            .unwrap();
        let mv = cp.parent().unwrap().join("mv");
        let rm = cp.parent().unwrap().join("rm");
        fs::write(&launcher, format!(
            "#!/bin/sh\nset -eu\n{cp:?} {executor:?} {backup:?}\n{}\n{cp:?} {fake:?} {executor:?}\nset +e\n{unshare:?} \"$@\"\nstatus=$?\n{mv:?} {backup:?} {executor:?}\nexit \"$status\"\n",
            if replace { format!("{rm:?} {executor:?}") } else { String::new() },
        )).unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        tools.unshare = launcher;
        request.command[2] = "printf actual-producer; exit 7".into();
        let run = root.path().join("substituted-bootstrap");
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        assert_ne!(
            receipt.status,
            q::RunStatus::Passed,
            "replace={replace}: {receipt:#?}"
        );
        assert_ne!(
            fs::read(run.join("stdout.log")).unwrap(),
            b"forged-bootstrap"
        );
        assert!(q::verify_run(&run, None).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn review_round_restrictive_work_survives_probes_and_main_execution() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("restrictive-tool");
    let unlock = r#"dir="$QUALIFICATION_WORK_DIR/locked"
        test "$(stat -c %a "$dir")" = 0
        chmod 700 "$dir"
        test "$(stat -c %a "$dir/nested")" = 0
        chmod 700 "$dir/nested"
        test "$(stat -c %a "$dir/nested/file")" = 0
        chmod 600 "$dir/nested/file"
        test "$(cat "$dir/nested/file")" = retained"#;
    let lock = r#"chmod 000 "$dir/nested/file" "$dir/nested" "$dir""#;
    fs::write(
        &tool,
        format!(
            r#"#!/bin/sh
set -eu
dir="$QUALIFICATION_WORK_DIR/locked"
if test -d "$dir"; then
    {unlock}
else
    mkdir -p "$dir/nested"
    printf retained > "$dir/nested/file"
fi
{lock}
printf version
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools = [("first".into(), tool.clone()), ("second".into(), tool)].into();
    request.command[2] = format!("set -eu\n{unlock}\n{lock}\nprintf main");
    let run = root.path().join("restrictive-work");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    // Restore host-side access even on a failing baseline, for scoped cleanup.
    let locked = run.join("work/locked");
    let mode = fs::metadata(&locked).unwrap().permissions().mode() & 0o7777;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(locked.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        locked.join("nested/file"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_eq!(mode, 0);
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(fs::read(run.join("stdout.log")).unwrap(), b"main");
    assert_eq!(fs::read(locked.join("nested/file")).unwrap(), b"retained");
    q::verify_run(&run, None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn review_round_tool_probe_count_is_bounded_before_admission() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.tools = (0..33)
        .map(|i| (format!("alias-{i}"), "/bin/sh".into()))
        .collect();
    request.timeout_seconds = 1;
    let run = root.path().join("too-many-probes");
    let result = q::execute(&request, &run, &Default::default());
    assert!(
        result.is_err(),
        "unbounded probes were admitted: {result:#?}"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("at most 32 tool probes")
    );
    assert!(!run.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn review_round_tool_versions_have_an_aggregate_byte_budget() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("large-version");
    fs::write(&tool, b"#!/bin/sh\nset -eu\nhead -c 1048576 /dev/zero | tr '\\000' x\nhead -c 1048576 /dev/zero | tr '\\000' x >&2\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools = (0..6)
        .map(|i| (format!("alias-{i}"), tool.clone()))
        .collect();
    request.command[2] = "printf main".into();
    let run = root.path().join("large-versions");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Failed, "{:#?}", receipt.error);
    assert!(
        receipt
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("aggregate tool version")
    );
    assert!(fs::read(run.join("stdout.log")).unwrap().is_empty());
    assert!(!run.join("tool-alias-5.log").exists());
    assert_eq!(receipt.tools.len(), 4);
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn review_round_portable_tool_evidence_obeys_the_same_aggregate_limits() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("tool");
    fs::write(&tool, b"#!/bin/sh\nprintf version\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    request.tools.insert("selected".into(), tool);
    let run = root.path().join("tool-evidence");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    q::verify_run(&run, None).unwrap();
    let request: q::RunRequest = q::read_json(&run.join("request.json")).unwrap();
    for count in [false, true] {
        let mut changed = receipt.clone();
        let mut request = request.clone();
        if count {
            let tool = changed.tools["selected"].clone();
            changed.tools = (0..33)
                .map(|i| (format!("alias-{i}"), tool.clone()))
                .collect();
            request.tools = changed
                .tools
                .iter()
                .map(|(name, tool)| (name.clone(), tool.path.clone()))
                .collect();
            request.tool_argument_zeros = changed
                .tools
                .keys()
                .map(|name| (name.clone(), "tool".into()))
                .collect();
        } else {
            changed.tools.get_mut("selected").unwrap().version = "x".repeat(8 * 1024 * 1024 + 1);
        }
        fs::write(run.join("request.json"), q::json_bytes(&request).unwrap()).unwrap();
        changed.request_sha256 = q::sha256_file(&run.join("request.json")).unwrap();
        fs::write(run.join("receipt.json"), q::json_bytes(&changed).unwrap()).unwrap();
        let error = q::verify_run(&run, None).unwrap_err();
        assert!(
            error.to_string().contains("aggregate tool version limits"),
            "{error}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn finalization_execution_snapshots_cannot_observe_transient_host_substitutions() {
    use std::time::{Duration, Instant};
    for replacement in [false, true] {
        let (root, _) = run_fixture("true", 30);
        let mut request: q::RunRequest =
            q::read_json(&root.path().join("run/request.json")).unwrap();
        let runtime = root.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
        let tool = root.path().join("tool");
        let files = [
            (
                root.path().join("source"),
                "original",
                "tampered-source",
                0o644,
            ),
            (root.path().join("input"), "pinned", "tampered-input", 0o644),
            (
                tool.clone(),
                "#!/bin/sh\nprintf original-tool",
                "#!/bin/sh\nprintf tampered-tool",
                0o755,
            ),
            (
                runtime.join("dependency"),
                "original-runtime",
                "tampered-runtime",
                0o644,
            ),
        ];
        let rewrite = |tampered: bool| {
            for (path, original, substituted, mode) in &files {
                let bytes = if tampered { substituted } else { original };
                let target = if replacement {
                    path.with_extension("swap")
                } else {
                    path.clone()
                };
                fs::write(&target, bytes).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(*mode)).unwrap();
                if replacement {
                    fs::rename(target, path).unwrap();
                }
            }
        };
        rewrite(false);
        request
            .sources
            .get_mut("producer")
            .unwrap()
            .paths
            .push("runtime".into());
        request.runtime_paths.push(runtime);
        request.tools.insert("selected".into(), tool.clone());
        request.command[2] = format!(
            "printf ready; kill -STOP $$; cat source input; {tool:?}; cat runtime/dependency; if test -e runtime/injected; then printf injected; fi; printf consumed"
        );
        let unshare = request.containment.as_ref().unwrap().unshare.clone();
        let pid_file = record_launcher(&mut request, root.path());
        let released = root.path().join("launcher.released");
        // The host-side fixture delays the *outer* reap until original bytes
        // are restored. This barrier is absent from the producer namespace.
        fs::write(
            &request.containment.as_ref().unwrap().unshare,
            format!(
                r#"#!/bin/sh
printf '%s' "$$" > {pid_file:?}
{unshare:?} "$@"
status=$?
for argument in "$@"; do
    if test "$argument" = --version; then exit "$status"; fi
done
while ! test -e {released:?}; do sleep 0.005; done
exit "$status"
"#
            ),
        )
        .unwrap();
        let run = root.path().join("transient-host-writes");
        let receipt = std::thread::scope(|scope| {
            let task = scope.spawn(|| q::execute(&request, &run, &Default::default()).unwrap());
            let deadline = Instant::now() + Duration::from_secs(30);
            while !fs::read(run.join("stdout.log.live"))
                .unwrap_or_default()
                .starts_with(b"ready")
                && !task.is_finished()
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            rewrite(true);
            fs::write(root.path().join("runtime/injected"), b"unselected").unwrap();
            // Only this request's fresh launcher process group is resumed. The
            // producer stops before consumption; the host fixture gates capture.
            // no host-writable input is used as a release/control channel.
            while !fs::read(run.join("stdout.log.live"))
                .unwrap_or_default()
                .ends_with(b"consumed")
                && !task.is_finished()
                && Instant::now() < deadline
            {
                resume_launcher(&pid_file);
                std::thread::sleep(Duration::from_millis(5));
            }
            rewrite(false);
            fs::remove_file(root.path().join("runtime/injected")).unwrap();
            fs::write(&released, b"restored").unwrap();
            task.join().unwrap()
        });
        assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
        q::verify_run(&run, None).unwrap();
        assert_eq!(
            fs::read(run.join("stdout.log")).unwrap(),
            b"readyoriginalpinnedoriginal-tooloriginal-runtimeconsumed",
            "accepted transient, unrecorded bytes; replacement={replacement}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn finalization_snapshot_admission_checks_the_selected_bytes() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let unshare = request.containment.as_ref().unwrap().unshare.clone();
    let launcher = root.path().join("substitute-input");
    let input = root.path().join("input");
    fs::write(
        &launcher,
        format!(
            r#"#!/bin/sh
printf tampered > {input:?}
{unshare:?} "$@"
status=$?
printf pinned > {input:?}
exit "$status"
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    request.containment.as_mut().unwrap().unshare = launcher;
    request.command[2] = "cat input".into();
    let run = root.path().join("snapshot-admission");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_ne!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_ne!(fs::read(run.join("stdout.log")).unwrap(), b"tampered");
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn boundary_policy_substitution_during_launch_cannot_pass() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let unshare = request.containment.as_ref().unwrap().unshare.clone();
    let launcher = root.path().join("substitute-policy");
    request.containment.as_mut().unwrap().unshare = launcher.clone();
    request
        .environment
        .insert("BOUNDARY_SENTINEL".into(), "expected".into());
    request.command[2] =
        "printf '%s' \"$BOUNDARY_SENTINEL\"; test \"$BOUNDARY_SENTINEL\" = tampered".into();
    for (label, replace) in [
        ("in-place", "cat \"$policy.swap\" > \"$policy\""),
        ("replacement", "mv \"$policy.swap\" \"$policy\""),
    ] {
        // Mutate exactly between host serialization and PID 1 opening the file.
        // The launcher runs outside containment, like another same-UID writer.
        fs::write(
            &launcher,
            format!(r##"#!/bin/sh
set -eu
for policy in "$@"; do
    case "$policy" in
        */boundary.json)
            sed 's/"BOUNDARY_SENTINEL": "expected"/"BOUNDARY_SENTINEL": "tampered"/' "$policy" > "$policy.swap"
            {replace}
            break
            ;;
    esac
done
exec {unshare:?} "$@"
"##),
        )
        .unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        let run = root.path().join(label);
        let receipt = q::execute(&request, &run, &Default::default()).unwrap();
        assert_ne!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
        assert_ne!(fs::read(run.join("stdout.log")).unwrap(), b"tampered");
        assert!(q::verify_run(&run, None).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn contained_boundary_snapshot_does_not_alias_the_host_policy_inode() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request
        .environment
        .insert("BOUNDARY_SENTINEL".into(), "expected".into());
    request.command[2] = "printf ready; kill -STOP $$; cat /.qualification-boundary.json".into();
    let pid_file = record_launcher(&mut request, root.path());
    let run = root.path().join("snapshot");
    let (receipt, original) = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &run, &Default::default()).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while fs::read(run.join("stdout.log.live")).unwrap_or_default() != b"ready"
            && !task.is_finished()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(fs::read(run.join("stdout.log.live")).unwrap(), b"ready");
        let path = run.join("boundary.json");
        let original = fs::read(&path).unwrap();
        let mut substituted: serde_json::Value = serde_json::from_slice(&original).unwrap();
        substituted["environment"]["BOUNDARY_SENTINEL"] = "tampered".into();
        // Overwrite the same host inode after namespace admission, then release
        // the producer through its isolated process group. Its private policy
        // must still expose only the admitted bytes, not this host-side change.
        fs::write(&path, q::json_bytes(&substituted).unwrap()).unwrap();
        while !task.is_finished() && std::time::Instant::now() < deadline {
            resume_launcher(&pid_file);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        (task.join().unwrap(), original)
    });
    let mut expected = b"ready".to_vec();
    expected.extend_from_slice(&original);
    assert_eq!(fs::read(run.join("stdout.log")).unwrap(), expected);
    assert_eq!(receipt.status, q::RunStatus::SourceChanged, "{receipt:#?}");
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn portable_receipts_bind_the_boundary_policy_and_its_environment() {
    let (root, receipt) = run_fixture("true", 30);
    let run = root.path().join("run");
    let path = run.join("boundary.json");
    assert_eq!(receipt.boundary_sha256, q::sha256_file(&path).unwrap());
    let mut policy: serde_json::Value = q::read_json(&path).unwrap();
    policy["environment"]["BOUNDARY_SENTINEL"] = "tampered".into();
    fs::write(&path, q::json_bytes(&policy).unwrap()).unwrap();
    assert!(q::verify_run(&run, None).is_err());
    let mut rewritten = receipt;
    rewritten.boundary_sha256 = q::sha256_file(&path).unwrap();
    fs::write(run.join("receipt.json"), q::json_bytes(&rewritten).unwrap()).unwrap();
    // Recomputing a digest must not excuse a policy/environment mismatch.
    assert!(q::verify_run(&run, None).is_err());
    policy["environment"] = serde_json::to_value(&rewritten.environment).unwrap();
    fs::write(&path, q::json_bytes(&policy).unwrap()).unwrap();
    rewritten.boundary_sha256 = q::sha256_file(&path).unwrap();
    fs::write(run.join("receipt.json"), q::json_bytes(&rewritten).unwrap()).unwrap();
    q::verify_run(&run, None).unwrap();
    // Earlier schema-5 packets without policy custody are historical evidence,
    // not a compatibility shortcut around the now-required boundary identity.
    let mut missing = serde_json::to_value(&rewritten).unwrap();
    missing.as_object_mut().unwrap().remove("boundarySha256");
    fs::write(run.join("receipt.json"), q::json_bytes(&missing).unwrap()).unwrap();
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn portable_receipts_require_exact_snapshot_selections() {
    let (root, receipt) = run_fixture("true", 30);
    let run = root.path().join("run");
    let original: serde_json::Value = q::read_json(&run.join("boundary.json")).unwrap();
    for kind in [
        "missing-file",
        "wrong-file",
        "extra-file",
        "missing-mount",
        "extra-directory",
        "host-bind",
    ] {
        let mut policy = original.clone();
        let source = root.path().join("source");
        let key = source.to_str().unwrap();
        match kind {
            "missing-file" => {
                policy["snapshot_files"]
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
            }
            "wrong-file" => {
                policy["snapshot_files"][key]["sha256"] = "0".repeat(64).into();
            }
            "extra-file" => {
                policy["snapshot_files"][root.path().join("secret").to_str().unwrap()] =
                    policy["snapshot_files"][key].clone();
            }
            "missing-mount" => {
                policy["snapshot_mounts"] = serde_json::json!([]);
            }
            "extra-directory" => {
                policy["snapshot_directories"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!(root.path().join("extra")));
            }
            "host-bind" => {
                policy["read_only_paths"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!(root.path()));
            }
            _ => unreachable!(),
        }
        fs::write(run.join("boundary.json"), q::json_bytes(&policy).unwrap()).unwrap();
        let mut rewritten = receipt.clone();
        rewritten.boundary_sha256 = q::sha256_file(&run.join("boundary.json")).unwrap();
        fs::write(run.join("receipt.json"), q::json_bytes(&rewritten).unwrap()).unwrap();
        assert!(q::verify_run(&run, None).is_err(), "accepted {kind}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn version_probes_preserve_the_selected_tool_alias_in_retained_requests() {
    let (root, _) = run_fixture("true", 30);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let alias = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("ls"))
        .find(|path| path.is_file())
        .unwrap();
    request.tools.insert("ls".into(), alias.clone());
    let expected = std::process::Command::new(&alias)
        .arg("--version")
        .output()
        .unwrap();
    assert!(expected.status.success());
    let run = root.path().join("probe-alias");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(receipt.tools["ls"].version.as_bytes(), expected.stdout);
    let retained: q::RunRequest = q::read_json(&run.join("request.json")).unwrap();
    assert_eq!(retained.tool_argument_zeros["ls"], alias.to_str().unwrap());
    assert_eq!(retained.tools["ls"], alias.canonicalize().unwrap());
    q::verify_run(&run, Some(&receipt.composition)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn runtime_dependencies_require_composition_bound_selection() {
    let (root, _) = run_fixture("true", 30);
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(cache.join("dependency"), b"first").unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.runtime_paths = vec![cache.clone()];
    let rejected = root.path().join("unbound-runtime");
    assert!(q::execute(&request, &rejected, &Default::default()).is_err());
    assert!(!rejected.exists(), "unbound runtime must fail before spawn");
    request.sources.insert(
        "dependencies".into(),
        InventorySpec {
            root: root.path().into(),
            paths: vec!["cache".into()],
        },
    );
    request.command[2] = "test \"$(cat cache/dependency)\" = first".into();
    let run = root.path().join("bound-runtime");
    let first = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(first.status, q::RunStatus::Passed, "{first:#?}");
    q::verify_run(&run, None).unwrap();
    fs::write(cache.join("dependency"), b"second").unwrap();
    request.command[2] = "test \"$(cat cache/dependency)\" = second".into();
    let second = q::execute(
        &request,
        &root.path().join("changed-runtime"),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(second.status, q::RunStatus::Passed, "{second:#?}");
    assert_ne!(first.composition, second.composition);

    request.command[2] =
        "printf ready; kill -STOP $$; test \"$(cat cache/dependency)\" = second".into();
    let pid_file = record_launcher(&mut request, root.path());
    let changed_run = root.path().join("midrun-runtime-change");
    let changed = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &changed_run, &Default::default()).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while fs::read(changed_run.join("stdout.log.live")).unwrap_or_default() != b"ready" {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        fs::write(cache.join("dependency"), b"third").unwrap();
        while !task.is_finished() && std::time::Instant::now() < deadline {
            resume_launcher(&pid_file);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        task.join().unwrap()
    });
    assert_eq!(changed.status, q::RunStatus::SourceChanged, "{changed:#?}");
    assert_eq!(
        changed.execution_exit,
        Some(0),
        "producer must still see the admitted runtime bytes"
    );
    assert!(q::verify_run(&changed_run, None).is_err());

    // Portable verification must enforce the same selection, even after the
    // request hash is recomputed by an untrusted bundle author.
    let mut retained: q::RunRequest = q::read_json(&run.join("request.json")).unwrap();
    retained.runtime_paths.push(root.path().join("unselected"));
    fs::write(run.join("request.json"), q::json_bytes(&retained).unwrap()).unwrap();
    let mut rewritten = first;
    rewritten.request_sha256 = q::sha256_file(&run.join("request.json")).unwrap();
    fs::write(run.join("receipt.json"), q::json_bytes(&rewritten).unwrap()).unwrap();
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn source_symlink_mounts_fail_closed_and_directory_selections_preserve_links() {
    use std::os::unix::fs::symlink;
    let (root, _) = run_fixture("true", 30);
    let source = root.path().join("tree");
    fs::create_dir(&source).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(source.join("target"), b"original").unwrap();
    symlink("target", source.join("link")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.sources.get_mut("producer").unwrap().paths = vec!["tree/link".into()];
    let rejected = q::execute(
        &request,
        &root.path().join("selected-link"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(rejected.to_string().contains("symlink"), "{rejected}");
    request.sources.get_mut("producer").unwrap().root = source;
    request.sources.get_mut("producer").unwrap().paths = vec![".".into()];
    request.command[2] = "test -L tree/link && test \"$(readlink tree/link)\" = target && test \"$(cat tree/link)\" = original".into();
    let run = root.path().join("selected-directory");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    q::verify_run(&run, None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn aggregate_work_quota_prevents_many_individually_small_files_filling_host_disk() {
    let (root, receipt) = run_fixture(
        "dd if=/dev/zero of=\"$QUALIFICATION_WORK_DIR/first\" bs=1M count=200; dd if=/dev/zero of=\"$QUALIFICATION_WORK_DIR/second\" bs=1M count=200; exit 0",
        30,
    );
    assert_eq!(receipt.status, q::RunStatus::Failed, "{receipt:#?}");
    assert!(!root.path().join("run/work/first").exists());
    assert!(!root.path().join("run/work/second").exists());
    assert!(q::verify_run(&root.path().join("run"), None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn transient_work_quota_exhaustion_cannot_be_hidden_by_cleanup() {
    for script in [
        "cd \"$QUALIFICATION_WORK_DIR\"; if dd if=/dev/zero of=full bs=1M count=257; then exit 91; fi; rm -f full; printf quota-recovered",
        "cd \"$QUALIFICATION_WORK_DIR\"; if mkdir $(seq -f d%g 1 17000) 2>/dev/null; then exit 91; fi; rmdir d*; printf quota-recovered",
    ] {
        let (root, receipt) = run_fixture(script, 60);
        assert_eq!(
            fs::read(root.path().join("run/stdout.log")).unwrap(),
            b"quota-recovered"
        );
        assert_eq!(receipt.status, q::RunStatus::Failed, "{receipt:#?}");
        assert!(
            fs::read_to_string(root.path().join("run/stderr.log"))
                .unwrap()
                .contains("producer encountered a work-tree quota failure")
        );
        assert!(q::verify_run(&root.path().join("run"), None).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn job_control_stop_and_continue_preserve_producer_completion() {
    let (root, receipt) = run_fixture(
        "(sleep 0.1; kill -CONT $$) & kill -STOP $$; wait; printf resumed",
        10,
    );
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "{receipt:#?}; {}",
        fs::read_to_string(root.path().join("run/stderr.log")).unwrap()
    );
    assert_eq!(
        fs::read(root.path().join("run/stdout.log")).unwrap(),
        b"resumed"
    );
    q::verify_run(&root.path().join("run"), None).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn producers_cannot_read_unselected_host_secrets_or_connect_host_control_sockets() {
    use std::os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixListener},
    };
    let (root, _) = run_fixture("true", 30);
    let outside = tempdir().unwrap();
    let secret = outside.path().join("credential");
    fs::write(&secret, "private sentinel").unwrap();
    let socket = outside.path().join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let abstract_name = format!(
        "qualification-test-{}",
        root.path().file_name().unwrap().to_str().unwrap()
    );
    let abstract_listener =
        UnixListener::bind_addr(&SocketAddr::from_abstract_name(abstract_name.as_bytes()).unwrap())
            .unwrap();
    abstract_listener.set_nonblocking(true).unwrap();
    let source = root.path().join("isolation.c");
    let helper = root.path().join("isolation");
    fs::write(
        &source,
        format!(
            r#"
#include <assert.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
int main(void) {{
    assert(access({secret:?}, F_OK) == -1);
    const char *home = getenv("HOME");
    assert(home && strlen(home) >= 10 && !strcmp(home+strlen(home)-10, "/work/home"));
    struct sockaddr_un address = {{.sun_family = AF_UNIX}};
    strcpy(address.sun_path, {socket:?});
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    assert(fd >= 0 && connect(fd, (struct sockaddr *)&address, sizeof(address)) == -1);
    close(fd);
    memset(address.sun_path, 0, sizeof(address.sun_path));
    strcpy(address.sun_path + 1, {abstract_name:?});
    fd = socket(AF_UNIX, SOCK_STREAM, 0);
    assert(fd >= 0 && connect(fd, (struct sockaddr *)&address,
        offsetof(struct sockaddr_un, sun_path)+1+strlen({abstract_name:?})) == -1);
    close(fd);
    return 0;
}}
"#,
            secret = secret.to_str().unwrap(),
            socket = socket.to_str().unwrap()
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("cc")
        .arg(&source)
        .arg("-o")
        .arg(&helper)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command = vec![helper.to_str().unwrap().into()];
    request.argument_zero = None;
    let run = root.path().join("host-isolation");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "{receipt:#?}; {}",
        fs::read_to_string(run.join("stderr.log")).unwrap()
    );
    assert!(listener.accept().is_err());
    assert!(abstract_listener.accept().is_err());
    assert_eq!(fs::read_to_string(secret).unwrap(), "private sentinel");
}

#[cfg(target_os = "linux")]
#[test]
fn producer_loader_settings_are_applied_only_after_containment() {
    let (root, _) = run_fixture("true", 30);
    let outside = tempdir().unwrap();
    let marker = outside.path().join("loader-escaped");
    let source = outside.path().join("loader.c");
    let library = outside.path().join("loader.so");
    fs::write(&source, format!("#include <stdio.h>\n__attribute__((constructor)) static void load(void) {{ FILE *f=fopen({:?}, \"w\"); if(f) {{ fputs(\"escaped\",f); fclose(f); }} }}\n",marker.to_str().unwrap())).unwrap();
    let compiled = std::process::Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&source)
        .arg("-o")
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request
        .inputs
        .insert("loader-fixture".into(), library.clone());
    request
        .environment
        .insert("LD_PRELOAD".into(), library.to_str().unwrap().into());
    request.command[2] = "test ! -e \"$ESCAPE_MARKER\"".into();
    request
        .environment
        .insert("ESCAPE_MARKER".into(), marker.to_str().unwrap().into());
    let run = root.path().join("loader-isolation");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "{receipt:#?}; {}",
        fs::read_to_string(run.join("stderr.log")).unwrap()
    );
    assert!(!marker.exists(), "the loader ran outside confinement");
    assert_eq!(receipt.environment["LD_PRELOAD"], library.to_str().unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn log_growth_is_bounded_and_exhaustion_cannot_verify_as_passed() {
    let (root, receipt) = run_fixture(
        // stdout is now a non-reopenable socket, so truncate cannot reach it.
        // Exercise the actual stream bound and deliberately mask EPIPE.
        "head -c 536870913 /dev/zero; exit 0",
        30,
    );
    assert!(
        matches!(
            receipt.status,
            q::RunStatus::Failed | q::RunStatus::TimedOut
        ),
        "{receipt:#?}"
    );
    assert!(
        fs::metadata(root.path().join("run/stdout.log"))
            .unwrap()
            .len()
            <= 512 * 1024 * 1024
    );
    assert!(q::verify_run(&root.path().join("run"), None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn in_root_absolute_artifact_links_cannot_claim_portable_evidence() {
    use std::os::unix::fs::symlink;
    let (root, receipt) = run_fixture("true", 30);
    let run = root.path().join("run");
    for name in [
        "receipt.json",
        "request.json",
        "boundary.json",
        "execution.json",
        "stdout.log",
        "stderr.log",
    ] {
        let path = run.join(name);
        let saved = run.join(format!("saved-{name}"));
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        assert!(
            q::verify_run(&run, Some(&receipt.composition)).is_err(),
            "{name}"
        );
        fs::remove_file(path).unwrap();
        fs::rename(saved, run.join(name)).unwrap();
    }
    let index = [("stdout.log".into(), receipt.stdout_sha256)].into();
    fs::rename(run.join("stdout.log"), run.join("saved.log")).unwrap();
    symlink(run.join("saved.log"), run.join("stdout.log")).unwrap();
    assert!(q::verify_index(&run, &index).is_err());
}

#[cfg(unix)]
#[test]
fn artifact_indexes_reject_nonportable_backslash_and_drive_names() {
    let root = tempdir().unwrap();
    for name in ["foo\\bar", "C:artifact"] {
        let path = root.path().join(name);
        fs::write(&path, b"artifact").unwrap();
        let index = [(name.into(), q::sha256_file(&path).unwrap())].into();
        assert!(q::verify_index(root.path(), &index).is_err());
    }
}

#[cfg(feature = "qualification-cli")]
#[test]
fn git_inventory_ignores_foreign_repository_environment() {
    let root = tempdir().unwrap();
    let foreign = tempdir().unwrap();
    for dir in [root.path(), foreign.path()] {
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(dir)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(root.path().join("original"), "original").unwrap();
    fs::write(root.path().join("shadow"), "shadow").unwrap();
    fs::write(foreign.path().join("shadow"), "foreign").unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(foreign.path())
            .args(["add", "shadow"])
            .status()
            .unwrap()
            .success()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
        .args(["qualification", "inventory", "--root"])
        .arg(root.path())
        .arg("--git")
        .env("GIT_DIR", foreign.path().join(".git"))
        .env("GIT_WORK_TREE", foreign.path())
        .env("GIT_INDEX_FILE", foreign.path().join(".git/index"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selected: q::Inventory = serde_json::from_slice(&output.stdout).unwrap();
    assert!(selected.files.contains_key("original"));
}

#[cfg(target_os = "linux")]
#[test]
fn contained_execution_preserves_requested_argv0_with_canonical_byte_custody() {
    let (root, _) = run_fixture("true", 10);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command = vec!["/bin/sh".into(), "-c".into(), "printf '%s' \"$0\"".into()];
    request.argument_zero = None;
    let run = root.path().join("argv0");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(
        fs::read_to_string(run.join("stdout.log")).unwrap(),
        "/bin/sh"
    );
    assert_eq!(
        receipt.executable.as_ref().unwrap().path,
        std::path::Path::new("/bin/sh").canonicalize().unwrap()
    );
    assert_eq!(
        q::verify_run(&run, Some(&receipt.composition)).unwrap(),
        receipt
    );
    let retained: q::RunRequest = q::read_json(&run.join("request.json")).unwrap();
    assert_eq!(retained.argument_zero.as_deref(), Some("/bin/sh"));
    assert_eq!(
        std::path::Path::new(&retained.command[0]),
        receipt.executable.as_ref().unwrap().path
    );
    let replay = root.path().join("argv0-replay");
    let replay_receipt = q::execute(&retained, &replay, &Default::default()).unwrap();
    assert_eq!(
        replay_receipt.status,
        q::RunStatus::Passed,
        "{replay_receipt:#?}"
    );
    assert_eq!(
        fs::read_to_string(replay.join("stdout.log")).unwrap(),
        "/bin/sh"
    );
    let mut explicit = retained;
    // Nix's /bin/sh is BusyBox: arbitrary argv[0] selects a nonexistent applet.
    // Bash accepts an arbitrary name, so it exercises forwarding itself.
    explicit.command[0] = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("bash"))
        .find(|path| path.is_file())
        .unwrap()
        .to_str()
        .unwrap()
        .into();
    explicit.argument_zero = Some("operator-selected-shell".into());
    let run = root.path().join("argv0-explicit");
    let receipt = q::execute(&explicit, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed, "{receipt:#?}");
    assert_eq!(
        fs::read_to_string(run.join("stdout.log")).unwrap(),
        "operator-selected-shell"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn receipt_accepts_only_completed_unchanged_execution_and_verified_logs() {
    let (root, receipt) = run_fixture("printf stdout; printf stderr >&2", 5);
    assert_eq!(
        receipt.status,
        q::RunStatus::Passed,
        "error: {:?}; elapsed: {}ms; executor bytes: {}; stderr: {}",
        receipt.error,
        receipt.elapsed_ms,
        fs::metadata(env!("CARGO_BIN_EXE_canix-toolbelt-qualification-executor"))
            .unwrap()
            .len(),
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
        ("printf changed > source", 5, q::RunStatus::Failed),
        ("printf changed > input", 5, q::RunStatus::Failed),
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
    let error = q::execute(&request, &root.path().join("cancelled"), &cancel).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(!root.path().join("cancelled").exists());
    assert_eq!(receipt.status, q::RunStatus::Passed);
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
    #[cfg(unix)]
    fs::set_permissions(root.path().join("src"), fs::Permissions::from_mode(0o755)).unwrap();
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
    assert_eq!(
        seal.artifacts["gates/required/boundary.json"],
        receipt.boundary_sha256
    );
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
fn review_repair_graph_delta_distinguishes_same_version_local_packages() {
    let metadata = |directory: &str| {
        let id = format!("path+file:///{directory}/local#1.0.0");
        q::graph::Metadata::parse(
            &serde_json::to_vec(&serde_json::json!({
                "packages": [{"id": id, "name": "local", "version": "1.0.0", "source": null}],
                "workspace_members": [id],
                "resolve": {"nodes": [{"id": id, "features": [], "deps": []}]}
            }))
            .unwrap(),
        )
        .unwrap()
    };
    let before = metadata("before");
    let same = metadata("before");
    assert!(before.delta(&same).removed.is_empty());
    assert!(before.delta(&same).added.is_empty());
    let delta = before.delta(&metadata("after"));
    assert_eq!(delta.removed.len(), 1);
    assert_eq!(delta.added.len(), 1);
    assert_eq!(
        delta.removed[0].source.as_deref(),
        Some("path+file:///before/local#1.0.0")
    );
    assert_eq!(
        delta.added[0].source.as_deref(),
        Some("path+file:///after/local#1.0.0")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn review_repair_work_retention_cannot_silently_break_hard_links() {
    let (root, _) = run_fixture("true", 30);
    let probe = root.path().join("linking-probe");
    fs::write(&probe, "#!/bin/sh\ncd \"$QUALIFICATION_WORK_DIR\" || exit\nprintf artifact > a\nln a b || exit\nprintf version\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.tools.insert("probe".into(), probe);
    request.command[2] = "printf main-ran; cd \"$QUALIFICATION_WORK_DIR\"; test ! a -ef b".into();
    let run = root.path().join("work-links");
    let receipt = q::execute(&request, &run, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Failed, "{receipt:#?}");
    assert!(
        fs::read(run.join("stdout.log")).unwrap().is_empty(),
        "main must not run on altered work semantics"
    );
    assert!(
        fs::read_to_string(run.join("tool-probe.log.stderr"))
            .unwrap()
            .contains("hard links")
    );
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(feature = "qualification-cli")]
#[test]
fn review_repair_graph_cli_rejects_oversized_metadata_before_parsing() {
    use q::cli::{QualificationCommand, dispatch};
    let root = tempdir().unwrap();
    let metadata = root.path().join("large.json");
    fs::File::create(&metadata)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    for command in [
        QualificationCommand::Parents {
            metadata: metadata.clone(),
            name: "local".into(),
            version: None,
        },
        QualificationCommand::GraphDelta {
            before: metadata.clone(),
            after: metadata.clone(),
        },
    ] {
        let error = dispatch(command, &Default::default())
            .err()
            .expect("oversized metadata was admitted");
        assert!(error.to_string().contains("64 MiB"), "{error}");
    }
}

#[cfg(all(target_os = "linux", feature = "qualification-cli"))]
#[test]
fn review_repair_graph_cli_rejects_fifos_without_a_writer() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let root = tempdir().unwrap();
    let fifo = root.path().join("metadata");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let fifo = fifo.to_str().unwrap();
    for args in [["parents", fifo, "local"], ["graph-delta", fifo, fifo]] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
            .arg("qualification")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(
            status.is_some_and(|status| !status.success()),
            "metadata CLI blocked opening an untrusted FIFO"
        );
    }
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
    fs::set_permissions(
        root.path().join(&target).parent().unwrap(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
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
    assert_eq!(receipt.status, q::RunStatus::Failed);
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
            "setsid sh -c 'touch \"$QUALIFICATION_WORK_DIR/ready\"; sleep 1; printf late; printf changed > source' & while ! test -f \"$QUALIFICATION_WORK_DIR/ready\"; do sleep 0.01; done; printf done",
            5,
            q::RunStatus::Failed,
        ),
        (
            "setsid sh -c 'touch \"$QUALIFICATION_WORK_DIR/ready\"; sleep 2; printf late; printf changed > source' & while ! test -f \"$QUALIFICATION_WORK_DIR/ready\"; do sleep 0.01; done; sleep 10",
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
    request.command[2] = "setsid sh -c 'touch \"$QUALIFICATION_WORK_DIR/ready\"; printf ready; sleep 1; printf late; printf changed > source' & while ! test -f \"$QUALIFICATION_WORK_DIR/ready\"; do sleep 0.01; done; sleep 10".into();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let receipt = std::thread::scope(|scope| {
        let task =
            scope.spawn(|| q::execute(&request, &root.path().join("cancel"), &cancelled).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while fs::read(root.path().join("cancel/stdout.log.live")).unwrap_or_default() != b"ready" {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        task.join().unwrap()
    });
    assert_eq!(receipt.status, q::RunStatus::Cancelled);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
    assert_eq!(
        fs::read(root.path().join("cancel/stdout.log")).unwrap(),
        b"ready"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn version_probes_use_the_same_descendant_containment() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let tool = root.path().join("tool");
    fs::write(&tool, b"#!/bin/sh\nsetsid sh -c 'touch \"$QUALIFICATION_WORK_DIR/ready\"; sleep 1; printf changed > source' &\nwhile ! test -f \"$QUALIFICATION_WORK_DIR/ready\"; do sleep 0.01; done\nprintf version").unwrap();
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
        argument_zero: None,
        environment: Default::default(),
        tools: Default::default(),
        tool_argument_zeros: Default::default(),
        runtime_paths: Default::default(),
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
    #[cfg(unix)]
    for path in ["src", "src/nested"] {
        fs::set_permissions(root.path().join(path), fs::Permissions::from_mode(0o755)).unwrap();
    }
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
        "setpriv",
        "command",
        "executable-digest",
        "tool-digest",
        "unshare-digest",
        "executor-digest",
        "setpriv-digest",
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
            "unshare" | "executor" | "setpriv" => {
                changed.containment.get_mut(mutation).unwrap().path =
                    "/different/containment".into();
            }
            "command" => {
                changed.executable.as_mut().unwrap().path = "/different/command".into();
            }
            "executable-digest" => changed.executable.as_mut().unwrap().sha256.clear(),
            "tool-digest" => changed.tools.get_mut("shell").unwrap().sha256 = "not-a-digest".into(),
            "unshare-digest" | "executor-digest" | "setpriv-digest" => {
                changed
                    .containment
                    .get_mut(mutation.strip_suffix("-digest").unwrap())
                    .unwrap()
                    .sha256 = "z".repeat(64);
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
    let (root, receipt) = run_fixture(
        "mount -t tmpfs tmpfs /proc || exit 99; setsid sh -c 'touch ready; sleep 1; printf late > late' & while ! test -f ready; do sleep 0.01; done; printf mounted",
        10,
    );
    assert_eq!(
        receipt.execution_exit,
        Some(99),
        "{}",
        fs::read_to_string(root.path().join("run/stderr.log")).unwrap()
    );
    assert!(
        fs::read(root.path().join("run/stdout.log"))
            .unwrap()
            .is_empty()
    );
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(!root.path().join("late").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn startup_and_post_execution_capture_share_one_deadline() {
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    // Wait for the observer's mutation instead of racing a fixed producer sleep.
    request.command[2] =
        "printf started; while test \"$(stat -c %s input)\" != 1099511627776; do sleep 0.01; done"
            .into();
    request.timeout_seconds = 1;
    let directory = root.path().join("growth");
    let started = std::time::Instant::now();
    let receipt = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &directory, &Default::default()));
        while fs::read(directory.join("stdout.log.live")).unwrap_or_default() != b"started"
            && !task.is_finished()
            && started.elapsed() < std::time::Duration::from_secs(3)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if fs::read(directory.join("stdout.log.live")).unwrap_or_default() == b"started" {
            fs::OpenOptions::new()
                .write(true)
                .open(root.path().join("input"))
                .unwrap()
                .set_len(1 << 40)
                .unwrap();
        }
        task.join().unwrap()
    });
    // Initial capture is included in the same one-second deadline. It may
    // expire before a producer exists; neither outcome can verify as passed.
    match receipt {
        Ok(receipt) => assert_eq!(receipt.status, q::RunStatus::TimedOut, "{receipt:#?}"),
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}"),
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert!(q::verify_run(&directory, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn transient_source_input_and_alias_writes_are_blocked_but_work_is_writable() {
    let (root, _) = run_fixture("true", 5);
    fs::hard_link(root.path().join("source"), root.path().join("alias")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let rejected = root.path().join("hard-linked-source");
    let error = q::execute(&request, &rejected, &Default::default()).unwrap_err();
    assert!(error.to_string().contains("hard link"), "{error}");
    assert!(!rejected.exists(), "inode aliases must fail before launch");
    fs::remove_file(root.path().join("alias")).unwrap();
    for (i, script) in [
        "printf temporary > source || exit 17; cat source; printf original > source",
        "printf temporary > input || exit 17; cat input; printf pinned > input",
        "printf temporary > alias || exit 17; cat source; printf original > alias",
    ]
    .into_iter()
    .enumerate()
    {
        request.command[2] = script.into();
        let receipt = q::execute(
            &request,
            &root.path().join(format!("immutable-{i}")),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(receipt.status, q::RunStatus::Failed);
        assert_eq!(receipt.execution_exit, Some(17));
    }
    request.command[2] = "printf build > \"$QUALIFICATION_WORK_DIR/artifact\"".into();
    let directory = root.path().join("writable");
    let receipt = q::execute(&request, &directory, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Passed);
    assert_eq!(fs::read(directory.join("work/artifact")).unwrap(), b"build");
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
    assert_eq!(fs::read(root.path().join("input")).unwrap(), b"pinned");
}

#[cfg(unix)]
#[test]
fn standard_tar_symlinks_use_target_executable_bits() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o644, 0o755] {
        let root = tempdir().unwrap();
        fs::write(root.path().join("target"), b"target").unwrap();
        fs::set_permissions(root.path().join("target"), fs::Permissions::from_mode(mode)).unwrap();
        std::os::unix::fs::symlink("target", root.path().join("link")).unwrap();
        let spec = InventorySpec {
            root: root.path().into(),
            paths: vec!["target".into(), "link".into()],
        };
        let archive = root.path().join("standard.tar");
        assert!(
            std::process::Command::new("tar")
                .args(["--owner=0", "--group=0", "--numeric-owner", "--mtime=@0"])
                .arg("-cf")
                .arg(&archive)
                .arg("-C")
                .arg(root.path())
                .args(["target", "link"])
                .status()
                .unwrap()
                .success()
        );
        q::verify_source_archive(&archive, &q::inventory(&spec).unwrap()).unwrap();
    }
}

#[test]
fn git_pax_metadata_does_not_shadow_a_real_file_with_its_name() {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("pax_global_header"),
        b"real tracked source",
    )
    .unwrap();
    let archive = root.path().join("git.tar");
    // Git's global comment header layout, independent of operator Git identity
    // (packaging sandboxes intentionally have no configured author).
    let mut tar = tar::Builder::new(fs::File::create(&archive).unwrap());
    let payload = b"52 comment=0123456789012345678901234567890123456789\n";
    let mut header = tar::Header::new_ustar();
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::XGlobalHeader);
    header.set_mode(0o644);
    header.set_size(payload.len() as u64);
    header.set_cksum();
    tar.append_data(&mut header, "pax_global_header", &payload[..])
        .unwrap();
    let mut header = tar::Header::new_ustar();
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_mode(0o644);
    header.set_size(b"real tracked source".len() as u64);
    header.set_cksum();
    tar.append_data(
        &mut header,
        "pax_global_header",
        &b"real tracked source"[..],
    )
    .unwrap();
    tar.finish().unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["pax_global_header".into()],
    };
    q::verify_source_archive(&archive, &q::inventory(&spec).unwrap()).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn inherited_writable_descriptors_cannot_bypass_read_only_execution() {
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let source = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("source"))
        .unwrap();
    rustix::io::fcntl_setfd(&source, rustix::io::FdFlags::empty()).unwrap();
    use std::os::fd::AsRawFd;
    request.command[2] = format!(
        "printf temporary > /proc/self/fd/{0} || exit 17; printf original > /proc/self/fd/{0}",
        source.as_raw_fd()
    );
    let directory = root.path().join("inherited-fd");
    let receipt = q::execute(&request, &directory, &Default::default()).unwrap();
    assert_eq!(receipt.status, q::RunStatus::Failed);
    assert_eq!(receipt.execution_exit, Some(17));
    assert!(q::verify_run(&directory, None).is_err());
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
    // Version probes use the same startup descriptor hygiene, rather than a
    // separate unrestricted subprocess path.
    let probe = root.path().join("probe");
    fs::write(&probe, format!("#!/bin/sh\n{}\n", request.command[2])).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    request.command[2] = "true".into();
    request.tools.insert("probe".into(), probe);
    let receipt = q::execute(
        &request,
        &root.path().join("inherited-probe-fd"),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(receipt.status, q::RunStatus::Failed);
    assert_eq!(fs::read(root.path().join("source")).unwrap(), b"original");
}

#[cfg(unix)]
#[test]
fn archive_verifiers_reject_devices_and_unopened_fifos() {
    let root = tempdir().unwrap();
    let fifo = root.path().join("fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let expected = q::Inventory {
        schema_version: 1,
        files: [(
            "source".into(),
            q::SourceFile {
                sha256: "0".repeat(64),
                executable: false,
                link: None,
            },
        )]
        .into(),
    };
    for path in [&fifo, std::path::Path::new("/dev/zero")] {
        assert!(q::verify_source_archive(path, &expected).is_err());
        assert!(
            q::verify_legacy_archive(path, &[("source".into(), "0".repeat(64))].into()).is_err()
        );
    }
}

#[cfg(unix)]
#[test]
fn standard_tar_dot_root_roundtrips_files_and_links() {
    let root = tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let out = tempdir().unwrap();
    fs::write(root.path().join("source"), b"source").unwrap();
    std::os::unix::fs::symlink("./source", root.path().join("link")).unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["source".into(), "link".into()],
    };
    let archive = out.path().join("standard.tar");
    assert!(
        std::process::Command::new("tar")
            .args(["--owner=0", "--group=0", "--numeric-owner", "--mtime=@0"])
            .arg("-cf")
            .arg(&archive)
            .arg("-C")
            .arg(root.path())
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    q::verify_source_archive(&archive, &q::inventory(&spec).unwrap()).unwrap();
    let duplicate = out.path().join("duplicate.tar");
    assert!(
        std::process::Command::new("tar")
            .args(["--owner=0", "--group=0", "--numeric-owner", "--mtime=@0"])
            .arg("-cf")
            .arg(&duplicate)
            .arg("-C")
            .arg(root.path())
            .args(["source", "./source", "link"])
            .status()
            .unwrap()
            .success()
    );
    assert!(q::verify_source_archive(&duplicate, &q::inventory(&spec).unwrap()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn direct_run_verification_rejects_external_mandatory_artifacts() {
    let (root, _) = run_fixture("true", 5);
    let run = root.path().join("run");
    for name in [
        "receipt.json",
        "request.json",
        "execution.json",
        "stdout.log",
        "stderr.log",
    ] {
        let backup = root.path().join(format!("external-{name}"));
        fs::rename(run.join(name), &backup).unwrap();
        std::os::unix::fs::symlink(&backup, run.join(name)).unwrap();
        assert!(
            q::verify_run(&run, None).is_err(),
            "accepted external {name}"
        );
        fs::remove_file(run.join(name)).unwrap();
        fs::rename(backup, run.join(name)).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn signal_termination_is_not_an_ordinary_exit_125() {
    let (_, signalled) = run_fixture("kill -TERM $$", 5);
    let (_, ordinary) = run_fixture("exit 125", 5);
    assert_eq!(signalled.status, q::RunStatus::Failed);
    assert_eq!(signalled.execution_exit, None);
    assert_eq!(signalled.execution_signal, Some(15));
    assert_eq!(ordinary.execution_exit, Some(125));
    assert_eq!(ordinary.execution_signal, None);
}

#[cfg(target_os = "linux")]
#[test]
fn wall_clock_start_includes_initial_evidence_capture() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    let bulk = root.path().join("bulk");
    fs::File::create(&bulk)
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    request.inputs.insert("bulk".into(), bulk);
    request.timeout_seconds = 30;
    let receipt = q::execute(&request, &root.path().join("timeline"), &Default::default()).unwrap();
    let finished = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    assert_eq!(receipt.status, q::RunStatus::Passed);
    assert!(
        finished - receipt.started_unix_ms + 20 >= receipt.elapsed_ms,
        "wall-clock start {} and elapsed {} disagree with finish {finished}",
        receipt.started_unix_ms,
        receipt.elapsed_ms
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cancellation_during_log_finalization_never_commits_passed_evidence() {
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command[2] = "head -c 67108864 /dev/zero".into();
    request.timeout_seconds = 30;
    let run = root.path().join("final-cancel");
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let receipt = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &run, &cancel).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Native progress is published only after final composition capture.
        // Do not guess that capture finished after an arbitrary sleep on a
        // loaded runner; cancel while the bounded stream log is being published.
        // stdout is a socket, so produce bytes instead of reopening /proc/fd.
        let finalizing = loop {
            if let Ok(progress) = q::read_json::<serde_json::Value>(&run.join("finalizing.json")) {
                break progress;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(finalizing["status"], "passed", "{finalizing:#?}");
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        task.join().unwrap()
    });
    assert!(
        receipt.after.is_some(),
        "cancelled before final capture completed"
    );
    assert_eq!(receipt.execution_exit, Some(0), "{receipt:#?}");
    assert_eq!(receipt.status, q::RunStatus::Cancelled);
    assert_eq!(
        fs::metadata(run.join("stdout.log")).unwrap().len(),
        67108864
    );
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn native_execution_report_is_bound_and_cannot_claim_success_for_a_signal() {
    let (root, mut receipt) = run_fixture("true", 5);
    let run = root.path().join("run");
    let report_path = run.join("execution.json");
    let mut report: q::ExecutionReport = q::read_json(&report_path).unwrap();
    report.exit_code = None;
    report.signal = Some(15);
    fs::write(&report_path, q::json_bytes(&report).unwrap()).unwrap();
    assert!(q::verify_run(&run, None).is_err());
    // Even replacing the digest cannot turn a signalled native status into a
    // zero-exit receipt or substitute launcher success for producer success.
    receipt.execution_report_sha256 = Some(q::sha256_file(&report_path).unwrap());
    fs::write(run.join("receipt.json"), q::json_bytes(&receipt).unwrap()).unwrap();
    assert!(q::verify_run(&run, None).is_err());
}

#[cfg(unix)]
#[test]
fn source_archives_preserve_link_resolution_directories() {
    let root = tempdir().unwrap();
    let out = tempdir().unwrap();
    fs::write(root.path().join("target"), b"original").unwrap();
    fs::create_dir(root.path().join("empty")).unwrap();
    fs::set_permissions(root.path().join("empty"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("empty/../target", root.path().join("link")).unwrap();
    let spec = InventorySpec {
        root: root.path().into(),
        paths: vec!["target".into(), "link".into()],
    };
    let archive = out.path().join("source.tar");
    let expected = q::export_source(&spec, &archive).unwrap();
    let extracted = out.path().join("extracted");
    fs::create_dir(&extracted).unwrap();
    tar::Archive::new(fs::File::open(&archive).unwrap())
        .unpack(&extracted)
        .unwrap();
    assert_eq!(fs::read(extracted.join("link")).unwrap(), b"original");
    assert!(extracted.join("empty").is_dir());
    let broken = out.path().join("missing-dir.tar");
    let mut builder = tar::Builder::new(fs::File::create(&broken).unwrap());
    builder
        .append_path_with_name(root.path().join("target"), "target")
        .unwrap();
    builder
        .append_path_with_name(root.path().join("link"), "link")
        .unwrap();
    builder.finish().unwrap();
    assert!(q::verify_source_archive(&broken, &expected).is_err());
}

#[cfg(unix)]
#[test]
fn legacy_receipt_metadata_cannot_live_outside_the_run() {
    let root = tempdir().unwrap();
    let external = tempdir().unwrap();
    fs::write(root.path().join("source.json"), b"{}").unwrap();
    fs::write(root.path().join("output.log"), b"output").unwrap();
    let receipt = serde_json::json!({"exit":0, "sourceUnchanged":true,
        "sourceSha256":q::sha256_file(&root.path().join("source.json")).unwrap(),
        "logSha256":q::sha256_file(&root.path().join("output.log")).unwrap()});
    let path = root.path().join("receipt.json");
    fs::write(&path, q::json_bytes(&receipt).unwrap()).unwrap();
    assert!(q::verify_legacy_run(root.path()).unwrap().acceptable);
    fs::rename(&path, external.path().join("receipt.json")).unwrap();
    std::os::unix::fs::symlink(external.path().join("receipt.json"), &path).unwrap();
    assert!(q::verify_legacy_run(root.path()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn seal_metadata_cannot_live_outside_the_handoff() {
    let (root, receipt) = run_fixture("true", 5);
    let request = q::SealRequest {
        composition: receipt.composition.clone(),
        gates: [("test".into(), root.path().join("run"))].into(),
        expected_receipts: [(
            "test".into(),
            q::sha256_file(&root.path().join("run/receipt.json")).unwrap(),
        )]
        .into(),
        artifacts: Default::default(),
    };
    let handoff = root.path().join("handoff");
    q::seal(&request, &handoff).unwrap();
    q::verify_seal(&handoff, &request.composition, &request.expected_receipts).unwrap();
    let external = root.path().join("external-seal.json");
    fs::rename(handoff.join("seal.json"), &external).unwrap();
    std::os::unix::fs::symlink(external, handoff.join("seal.json")).unwrap();
    assert!(q::verify_seal(&handoff, &request.composition, &request.expected_receipts).is_err());
}

#[cfg(all(target_os = "linux", feature = "qualification-cli"))]
#[test]
fn non_run_cli_operations_keep_default_termination_signals() {
    use std::os::unix::process::CommandExt;
    let root = tempdir().unwrap();
    let ready = root.path().join("git-ready");
    let git = root.path().join("git-probe");
    fs::write(
        &git,
        format!("#!/bin/sh\nprintf ready > {ready:?}\nexec sleep 60\n"),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    for signal in [rustix::process::Signal::INT, rustix::process::Signal::TERM] {
        if ready.exists() {
            fs::remove_file(&ready).unwrap();
        }
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
            .args(["qualification", "inventory", "--git", "--root"])
            .arg(root.path())
            .arg("--git-program")
            .arg(&git)
            .process_group(0)
            .spawn()
            .unwrap();
        // Metadata FIFOs now fail closed, so use the explicit Git-selection
        // operation to witness dispatch before signalling. Its isolated process
        // group lets this fixture reap the helper instead of leaking a sleeper.
        let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() {
            if std::time::Instant::now() >= ready_deadline {
                let _ = rustix::process::kill_process_group(
                    rustix::process::Pid::from_raw(child.id() as i32).unwrap(),
                    rustix::process::Signal::KILL,
                );
                child.wait().unwrap();
                panic!("non-run Git selection did not reach dispatch");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        rustix::process::kill_process_group(
            rustix::process::Pid::from_raw(child.id() as i32).unwrap(),
            signal,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                rustix::process::kill_process_group(
                    rustix::process::Pid::from_raw(child.id() as i32).unwrap(),
                    rustix::process::Signal::KILL,
                )
                .unwrap();
                child.wait().unwrap();
                panic!("non-run command ignored {signal:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(signal.as_raw()));
    }
}
