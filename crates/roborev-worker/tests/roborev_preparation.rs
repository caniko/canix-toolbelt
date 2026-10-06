#![cfg(all(feature = "roborev-preparation-tests", target_os = "linux"))]

use canix_toolbelt_roborev_worker::{Binding, Limits, Tools, prepare_local};
use nix::fcntl::{Flock, FlockArg};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    root: tempfile::TempDir,
    tools: Tools,
    binding: Binding,
    source: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new(&self.tools.git)
            .env_clear()
            .env("HOME", self.root.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&self.source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn new() -> Self {
        Self::new_format("sha1")
    }

    fn new_format(format: &str) -> Self {
        Self::new_with_files(format, 0)
    }

    fn new_with_files(format: &str, extra_files: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let tools = Tools {
            preparer: std::env::var_os("CANIX_TEST_PREPARER")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_BIN_EXE_roborev-preparation-fixture"))
                }),
            git: PathBuf::from(
                std::env::var("CANIX_TEST_GIT").expect("explicit test Git required"),
            ),
            bubblewrap: PathBuf::from(
                std::env::var("CANIX_TEST_BWRAP").expect("explicit test Bubblewrap required"),
            ),
        };
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new(&tools.git)
                .env_clear()
                .env("PATH", "/run/current-system/sw/bin")
                .env("HOME", root.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_AUTHOR_NAME", "Disposable Fixture")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
                .env("GIT_COMMITTER_NAME", "Disposable Fixture")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&[
            "init",
            "--quiet",
            "--initial-branch=trunk",
            &format!("--object-format={format}"),
        ]);
        fs::write(source.join("file.txt"), "ancestor\n").unwrap();
        git(&["add", "file.txt"]);
        git(&["commit", "--quiet", "-m", "ancestor"]);
        git(&["checkout", "--quiet", "-b", "feature"]);
        fs::write(source.join("file.txt"), "head\n").unwrap();
        for index in 0..extra_files {
            fs::write(
                source.join(format!("budget-{index}.txt")),
                "x".repeat(16_384),
            )
            .unwrap();
        }
        fs::write(
            source.join(".gitattributes"),
            "*.txt filter=host-command text eol=crlf ident\n",
        )
        .unwrap();
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(
            source.join("nested/.gitattributes"),
            "*.txt text eol=crlf ident\n",
        )
        .unwrap();
        fs::write(source.join("nested/raw.txt"), "nested $Id$\n").unwrap();
        symlink("/protected/synthetic-sentinel", source.join("outside-link")).unwrap();
        fs::write(
            source.join(".roborev.toml"),
            "default_agent = 'untrusted'\n",
        )
        .unwrap();
        git(&["add", "."]);
        git(&["commit", "--quiet", "-m", "source"]);
        let head = git(&["rev-parse", "HEAD"]);
        git(&["checkout", "--quiet", "trunk"]);
        fs::write(source.join("target-only.txt"), "target advanced\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "--quiet", "-m", "target"]);
        let base = git(&["rev-parse", "HEAD"]);
        if format == "sha256" {
            git(&["repack", "--quiet", "-ad"]);
        }
        let binding = Binding {
            request_url: "https://forge.invalid/owner/repo/pull/1".into(),
            request_id: "request-one".into(),
            authorized_request_sha256: "a".repeat(64),
            execution_policy_sha256: "b".repeat(64),
            base,
            head,
        };
        let state = root.path().join("state");
        Self {
            root,
            tools,
            binding,
            source,
            state,
        }
    }

    fn prepare(&self) -> anyhow::Result<canix_toolbelt_roborev_worker::Prepared> {
        prepare_local(
            &self.source.join(".git/objects"),
            &self.state,
            &self.binding,
            &self.tools,
            &Limits::default(),
        )
    }
}

#[test]
fn expanded_checkout_is_rejected_before_files_are_written() {
    let fixture = Fixture::new_with_files("sha1", 8);
    let limits = Limits {
        input_bytes: 65_536,
        snapshot_bytes: 65_536,
        ..Limits::default()
    };
    let error = prepare_local(
        &fixture.source.join(".git/objects"),
        &fixture.state,
        &fixture.binding,
        &fixture.tools,
        &limits,
    )
    .unwrap_err()
    .to_string();
    let entry = fixture.state.join(fixture.binding.key());
    assert!(error.contains("before checkout"), "{error}");
    assert!(!entry.join("checkout/budget-0.txt").exists());
    assert!(!entry.join("checkout/.git/index").exists());
    let journal: serde_json::Value =
        serde_json::from_slice(&fs::read(entry.join("preparation.json")).unwrap()).unwrap();
    assert!(journal["snapshot_sha256"].is_null());
}

#[test]
fn packed_sha256_objects_preserve_exact_commit_and_checkout_identity() {
    let fixture = Fixture::new_format("sha256");
    assert_eq!(fixture.binding.head.len(), 64);
    assert!(
        fs::read_dir(fixture.source.join(".git/objects/pack"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "pack"))
    );
    let prepared = fixture.prepare().unwrap();
    assert_eq!(
        fs::read_to_string(prepared.checkout.join("file.txt")).unwrap(),
        "head\n"
    );
    assert_eq!(
        fs::read_to_string(prepared.checkout.join(".git/HEAD"))
            .unwrap()
            .trim(),
        fixture.binding.head
    );
    assert_eq!(prepared, fixture.prepare().unwrap());
}

#[test]
fn repeated_anchor_loss_replacement_and_state_relocation_cannot_resume() {
    for case in ["deleted", "replaced", "relocated"] {
        let mut fixture = Fixture::new();
        let prepared = fixture.prepare().unwrap();
        let before = fs::read(prepared.journal_path()).unwrap();
        let lock = fixture
            .state
            .join(fixture.binding.key())
            .join("request.lock");
        match case {
            "deleted" => fs::remove_file(&lock).unwrap(),
            "replaced" => {
                fs::rename(&lock, lock.with_extension("retained")).unwrap();
                fs::write(&lock, b"").unwrap();
                fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
            }
            "relocated" => {
                let relocated = fixture.root.path().join("relocated-state");
                fs::rename(&fixture.state, &relocated).unwrap();
                fixture.state = relocated;
            }
            _ => unreachable!(),
        }
        for attempt in 0..3 {
            assert!(
                fixture.prepare().is_err(),
                "{case}: retry {attempt} accepted a changed anchor/path"
            );
            let journal = fixture
                .state
                .join(fixture.binding.key())
                .join("preparation.json");
            assert_eq!(fs::read(journal).unwrap(), before);
        }
    }
}

#[test]
fn noncommit_ids_cannot_substitute_for_either_raw_commit() {
    for format in ["sha1", "sha256"] {
        for coordinate in ["base", "head"] {
            for kind in ["tag", "tree", "blob"] {
                let mut fixture = Fixture::new_format(format);
                let commit = if coordinate == "base" {
                    &fixture.binding.base
                } else {
                    &fixture.binding.head
                };
                let object = match kind {
                    "tag" => {
                        fixture.git(&[
                            "tag",
                            "-a",
                            "coordinate",
                            "-m",
                            "disposable annotated tag",
                            commit,
                        ]);
                        fixture.git(&["rev-parse", "refs/tags/coordinate"])
                    }
                    "tree" => fixture.git(&["rev-parse", &format!("{commit}^{{tree}}")]),
                    "blob" => fixture.git(&["rev-parse", &format!("{commit}:file.txt")]),
                    _ => unreachable!(),
                };
                if coordinate == "base" {
                    fixture.binding.base = object;
                } else {
                    fixture.binding.head = object;
                }
                let error = fixture.prepare().unwrap_err().to_string();
                assert!(
                    error.contains("raw commit"),
                    "{format}/{coordinate}/{kind}: {error}"
                );
                let entry = fixture.state.join(fixture.binding.key());
                assert!(!entry.join("checkout/.git/index").exists());
                assert!(!entry.join("checkout/file.txt").exists());
            }
        }
    }
}

#[test]
fn complete_inventory_accepts_many_files_within_declared_limits() {
    let fixture = Fixture::new_with_files("sha1", 1500);
    let prepared = fixture.prepare().unwrap();
    assert_eq!(
        fs::read(prepared.checkout.join("budget-1499.txt"))
            .unwrap()
            .len(),
        16_384
    );
    assert_eq!(prepared, fixture.prepare().unwrap());
}

#[test]
fn whole_preparation_supervision_bounds_blocked_work_and_reaps_pipe_descendants() {
    for case in ["blocked", "exited"] {
        let fixture = Fixture::new();
        let python = std::env::var("CANIX_TEST_PYTHON").unwrap();
        let helper = fixture.root.path().join("synthetic-blocked-helper");
        let marker = fixture.root.path().join("descendant-pid");
        fs::write(&helper, format!(
            "#!{python}\nimport json, subprocess, sys, time\njson.load(sys.stdin)\np = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\nopen({}, 'w').write(str(p.pid))\n{}\n",
            serde_json::to_string(&marker).unwrap(),
            if case == "blocked" { "time.sleep(60)" } else { "print('{}')" },
        )).unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let mut tools = fixture.tools.clone();
        tools.preparer = helper;
        let limits = Limits {
            wall_seconds: 1,
            ..Limits::default()
        };
        let started = std::time::Instant::now();
        let error = prepare_local(
            &fixture.source.join(".git/objects"),
            &fixture.state,
            &fixture.binding,
            &tools,
            &limits,
        )
        .unwrap_err()
        .to_string();
        if case == "blocked" {
            assert!(
                error.contains("whole preparation exceeded its time bound"),
                "{error}"
            );
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{case}: unbounded pipe/child wait"
        );
        let pid = fs::read_to_string(marker).unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            assert!(
                stat.rsplit_once(')')
                    .unwrap()
                    .1
                    .trim_start()
                    .starts_with('Z'),
                "{case}: descendant is still running"
            );
        }
    }
}

#[test]
fn stalled_real_object_copy_times_out_and_preserves_incomplete_request() {
    let fixture = Fixture::new();
    let python = std::env::var("CANIX_TEST_PYTHON").unwrap();
    let helper = fixture.root.path().join("blocked-copy-helper");
    let marker = fixture.root.path().join("copy-stalled.json");
    fs::write(
        &helper,
        format!(
            "#!{python}\n{}\n",
            include_str!("fixtures/roborev-blocked-io.py")
                .replace(
                    "\"__FIXTURE_HELPER__\"",
                    &serde_json::to_string(&fixture.tools.preparer).unwrap()
                )
                .replace(
                    "\"__FIXTURE_MARKER__\"",
                    &serde_json::to_string(&marker).unwrap()
                )
        ),
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut tools = fixture.tools.clone();
    tools.preparer = helper;
    let limits = Limits {
        wall_seconds: 3,
        ..Limits::default()
    };
    let started = std::time::Instant::now();
    let prepare = || {
        prepare_local(
            &fixture.source.join(".git/objects"),
            &fixture.state,
            &fixture.binding,
            &tools,
            &limits,
        )
    };
    let error = std::thread::scope(|scope| {
        let first = scope.spawn(prepare);
        while !marker.exists() && started.elapsed() < std::time::Duration::from_secs(2) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "real helper did not reach source copying");
        let contention = prepare().unwrap_err().to_string();
        assert!(
            contention.contains("another preparer owns this request"),
            "{contention}"
        );
        first.join().unwrap().unwrap_err().to_string()
    });
    assert!(
        error.contains("whole preparation exceeded its time bound"),
        "{error}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
    let observed: serde_json::Value = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
    assert_eq!(observed["phase"], "source-object-copy");
    let pid = observed["pid"].as_u64().unwrap();
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        assert!(
            stat.rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('Z'),
            "stalled copier still running"
        );
    }
    let entry = fixture.state.join(fixture.binding.key());
    let journal = fs::read(entry.join("preparation.json")).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&journal).unwrap();
    assert!(state["snapshot_sha256"].is_null());
    assert!(!entry.join("checkout/.git/index").exists());
    for _ in 0..2 {
        assert!(
            prepare()
                .unwrap_err()
                .to_string()
                .contains("preparation is incomplete")
        );
        assert_eq!(fs::read(entry.join("preparation.json")).unwrap(), journal);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            let Ok(meta) = fs::symlink_metadata(path) else {
                return;
            };
            if meta.is_dir() {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                for entry in fs::read_dir(path).unwrap() {
                    writable(&entry.unwrap().path());
                }
            }
        }
        writable(self.root.path());
    }
}

#[test]
fn sanitized_preparation_keeps_divergent_commits_and_resumes_the_same_snapshot() {
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("source-policy-executed");
    let hook = fixture.source.join(".git/hooks/post-checkout");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = fs::read_to_string(fixture.source.join(".git/config")).unwrap();
    config.push_str(&format!("\n[filter \"host-command\"]\n smudge = touch '{}'\n required = true\n[core]\n fsmonitor = touch '{}'\n", marker.display(), marker.display()));
    fs::write(fixture.source.join(".git/config"), config).unwrap();
    let prepared = fixture.prepare().unwrap();
    assert_eq!(
        fs::read_to_string(prepared.checkout.join("file.txt")).unwrap(),
        "head\n"
    );
    assert!(!prepared.checkout.join("target-only.txt").exists());
    assert_eq!(
        fs::read_to_string(prepared.checkout.join("nested/raw.txt")).unwrap(),
        "nested $Id$\n"
    );
    assert_eq!(
        fs::read_link(prepared.checkout.join("outside-link")).unwrap(),
        PathBuf::from("/protected/synthetic-sentinel")
    );
    assert!(!prepared.checkout.join(".git/hooks").exists());
    assert!(!prepared.checkout.join(".git/refs/heads/trunk").exists());
    assert!(
        !fs::read_to_string(prepared.checkout.join(".git/config"))
            .unwrap()
            .contains("host-command")
    );
    assert!(!marker.exists());
    let before = fs::read(prepared.journal_path()).unwrap();
    assert_eq!(prepared, fixture.prepare().unwrap());
    assert_eq!(fs::read(prepared.journal_path()).unwrap(), before);
    let mut changed = fixture.binding.clone();
    changed.execution_policy_sha256 = "c".repeat(64);
    assert!(
        prepare_local(
            &fixture.source.join(".git/objects"),
            &fixture.state,
            &changed,
            &fixture.tools,
            &Limits::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(prepared.journal_path()).unwrap(), before);
    let original = fixture.source.join(".git/objects");
    let object = fs::read_dir(&original)
        .unwrap()
        .flat_map(|entry| {
            let path = entry.unwrap().path();
            if path.file_name().unwrap().len() == 2 {
                fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect()
            } else {
                Vec::new()
            }
        })
        .next()
        .unwrap();
    let copied = prepared
        .checkout
        .join(".git/objects")
        .join(object.strip_prefix(&original).unwrap());
    assert_ne!(
        fs::metadata(&object).unwrap().ino(),
        fs::metadata(copied).unwrap().ino()
    );
}

#[test]
fn unsafe_or_incomplete_objects_leave_a_persistent_preparation_tombstone() {
    for case in ["alternate", "symlink", "hardlink", "missing-head"] {
        let mut fixture = Fixture::new();
        let objects = fixture.source.join(".git/objects");
        match case {
            "alternate" => {
                fs::write(objects.join("info/alternates"), "/protected/objects\n").unwrap()
            }
            "symlink" => {
                fs::create_dir(objects.join("aa")).unwrap();
                symlink(
                    fixture.root.path().join("private-sentinel"),
                    objects.join("aa").join("b".repeat(38)),
                )
                .unwrap();
            }
            "hardlink" => {
                fs::create_dir(objects.join("aa")).unwrap();
                let protected = fixture.root.path().join("private-sentinel");
                fs::write(&protected, "SYNTHETIC_PROTECTED_CONTENT").unwrap();
                fs::hard_link(protected, objects.join("aa").join("b".repeat(38))).unwrap();
            }
            "missing-head" => fixture.binding.head = "f".repeat(40),
            _ => unreachable!(),
        }
        let error = fixture.prepare().unwrap_err().to_string();
        let expected = match case {
            "alternate" => "alternates are forbidden",
            "symlink" => "without following symlinks",
            "hardlink" => "unshared regular file",
            "missing-head" => "failed at fsck",
            _ => unreachable!(),
        };
        assert!(error.contains(expected), "{case}: {error}");
        let entry = fixture.state.join(fixture.binding.key());
        assert!(entry.join("request.lock").exists(), "{case}");
        assert!(entry.join("preparation.json").exists(), "{case}");
        let before = fs::read(entry.join("preparation.json")).unwrap();
        assert!(
            fixture
                .prepare()
                .unwrap_err()
                .to_string()
                .contains("incomplete"),
            "{case}"
        );
        assert_eq!(fs::read(entry.join("preparation.json")).unwrap(), before);
    }
}

#[test]
fn byte_and_entry_limits_fail_before_snapshot_acceptance() {
    for case in ["bytes", "entries"] {
        let fixture = Fixture::new();
        let mut limits = Limits::default();
        if case == "bytes" {
            limits.input_bytes = 1;
        } else {
            limits.entries = 1;
        }
        let error = prepare_local(
            &fixture.source.join(".git/objects"),
            &fixture.state,
            &fixture.binding,
            &fixture.tools,
            &limits,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("bound"), "{case}: {error}");
        let journal = fixture
            .state
            .join(fixture.binding.key())
            .join("preparation.json");
        let journal: serde_json::Value =
            serde_json::from_slice(&fs::read(journal).unwrap()).unwrap();
        assert!(journal["snapshot_sha256"].is_null());
    }
}

#[test]
fn changed_commits_authorization_limits_and_deleted_anchor_cannot_rebind_a_request() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare().unwrap();
    let before = fs::read(prepared.journal_path()).unwrap();
    for case in ["head", "base", "authorization", "limits"] {
        let mut binding = fixture.binding.clone();
        let mut limits = Limits::default();
        match case {
            "head" => binding.head = "f".repeat(40),
            "base" => binding.base = "f".repeat(40),
            "authorization" => binding.authorized_request_sha256 = "c".repeat(64),
            "limits" => limits.cpu_seconds += 1,
            _ => unreachable!(),
        }
        assert!(
            prepare_local(
                &fixture.source.join(".git/objects"),
                &fixture.state,
                &binding,
                &fixture.tools,
                &limits
            )
            .unwrap_err()
            .to_string()
            .contains("changed preparation identity")
        );
        assert_eq!(fs::read(prepared.journal_path()).unwrap(), before);
    }
    fs::remove_file(
        fixture
            .state
            .join(fixture.binding.key())
            .join("request.lock"),
    )
    .unwrap();
    assert!(
        fixture
            .prepare()
            .unwrap_err()
            .to_string()
            .contains("original lock anchor")
    );
    assert_eq!(fs::read(prepared.journal_path()).unwrap(), before);
}

#[test]
fn missing_state_changed_snapshot_and_competing_preparation_never_rematerialize() {
    for case in [
        "journal",
        "checkout",
        "content",
        "content-symlink",
        "mode",
        "lock",
    ] {
        let fixture = Fixture::new();
        let prepared = fixture.prepare().unwrap();
        let entry = fixture.state.join(fixture.binding.key());
        let held = match case {
            "journal" => {
                fs::remove_file(prepared.journal_path()).unwrap();
                None
            }
            "checkout" => {
                fs::rename(&prepared.checkout, entry.join("retained-checkout")).unwrap();
                None
            }
            "content" => {
                fs::set_permissions(
                    prepared.checkout.join("file.txt"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                fs::write(prepared.checkout.join("file.txt"), "changed\n").unwrap();
                fs::set_permissions(
                    prepared.checkout.join("file.txt"),
                    fs::Permissions::from_mode(0o400),
                )
                .unwrap();
                None
            }
            "content-symlink" => {
                fs::set_permissions(&prepared.checkout, fs::Permissions::from_mode(0o700)).unwrap();
                fs::remove_file(prepared.checkout.join("file.txt")).unwrap();
                symlink(
                    fixture.root.path().join("protected-sentinel"),
                    prepared.checkout.join("file.txt"),
                )
                .unwrap();
                fs::set_permissions(&prepared.checkout, fs::Permissions::from_mode(0o500)).unwrap();
                None
            }
            "mode" => {
                fs::set_permissions(
                    prepared.checkout.join("file.txt"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                None
            }
            "lock" => Some(
                Flock::lock(
                    fs::File::open(entry.join("request.lock")).unwrap(),
                    FlockArg::LockExclusiveNonblock,
                )
                .unwrap(),
            ),
            _ => unreachable!(),
        };
        let error = fixture.prepare().unwrap_err().to_string();
        if matches!(case, "content" | "content-symlink") {
            assert!(
                error.contains("prepared snapshot changed"),
                "{case}: {error}"
            );
        }
        assert!(entry.join("request.lock").exists(), "{case}");
        if case == "checkout" {
            assert!(!prepared.checkout.exists());
        }
        drop(held);
    }
}
