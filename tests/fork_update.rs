//! Fixture-backed check that updates only move branches nobody else owns.
//!
//! Each fixture is a disposable repository with global Git configuration
//! disabled, so managed identity, signing and hook policy cannot leak in.

use canix_toolbelt::fork::{SKIPPED, UP_TO_DATE, UPDATED, update};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "init.defaultBranch=main"])
        .args(arguments)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn text<'a>(path: &'a Path, value: &str) -> &'a Path {
    std::fs::write(path, value).expect("fixture file is writable");
    path
}

fn seeded_upstream(root: &Path) -> PathBuf {
    let seed = root.join("seed");
    git(root, &["init", "-q", seed.to_str().unwrap()]);
    git(
        &seed,
        &["add", text(&seed.join("file"), "one\n").to_str().unwrap()],
    );
    git(&seed, &["commit", "-q", "-m", "one"]);
    let upstream = root.join("upstream.git");
    git(
        root,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            upstream.to_str().unwrap(),
        ],
    );
    upstream
}

fn advance(root: &Path, upstream: &Path) {
    let seed = root.join("seed");
    text(&seed.join("file"), "two\n");
    git(&seed, &["commit", "-q", "-am", "two"]);
    git(&seed, &["push", "-q", upstream.to_str().unwrap(), "main"]);
}

fn fork_clone(root: &Path, upstream: &Path) -> PathBuf {
    let fork = root.join("fork");
    git(
        root,
        &[
            "clone",
            "-q",
            upstream.to_str().unwrap(),
            fork.to_str().unwrap(),
        ],
    );
    git(&fork, &["remote", "rename", "origin", "upstream"]);
    git(&fork, &["remote", "set-head", "upstream", "main"]);
    fork
}

#[test]
fn fast_forwards_a_clean_fork_without_local_commits() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let upstream = seeded_upstream(root);
    let fork = fork_clone(root, &upstream);
    advance(root, &upstream);

    let outcome = update(&fork);

    assert_eq!(outcome.action, UPDATED, "{outcome:?}");
    assert_eq!(
        git(&fork, &["rev-parse", "HEAD"]),
        git(&upstream, &["rev-parse", "main"])
    );
}

#[test]
fn refuses_a_fork_that_carries_local_commits() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let upstream = seeded_upstream(root);
    let fork = fork_clone(root, &upstream);
    git(&fork, &["commit", "-q", "--allow-empty", "-m", "local"]);
    let local = git(&fork, &["rev-parse", "HEAD"]);
    advance(root, &upstream);

    let outcome = update(&fork);

    assert_eq!(outcome.action, SKIPPED, "{outcome:?}");
    assert_eq!(outcome.detail.as_deref(), Some("1 local commits"));
    assert_eq!(git(&fork, &["rev-parse", "HEAD"]), local);
}

#[test]
fn leaves_a_dirty_worktree_alone() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let upstream = seeded_upstream(root);
    let fork = fork_clone(root, &upstream);
    advance(root, &upstream);
    text(&fork.join("file"), "edited\n");

    let outcome = update(&fork);

    assert_eq!(outcome.action, SKIPPED, "{outcome:?}");
    assert_eq!(outcome.detail.as_deref(), Some("uncommitted changes"));
    assert_eq!(
        std::fs::read_to_string(fork.join("file")).unwrap(),
        "edited\n"
    );
}

#[test]
fn reports_an_up_to_date_fork_without_touching_it() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let upstream = seeded_upstream(root);
    let fork = fork_clone(root, &upstream);

    let outcome = update(&fork);

    assert_eq!(outcome.action, UP_TO_DATE, "{outcome:?}");
    assert_eq!(outcome.detail.as_deref(), Some("main"));
}
