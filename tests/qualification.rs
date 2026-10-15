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
fn source_symlink_mounts_fail_closed_and_directory_selections_preserve_links() {
    use std::os::unix::fs::symlink;
    let (root, _) = run_fixture("true", 30);
    let source = root.path().join("tree");
    fs::create_dir(&source).unwrap();
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
        "truncate -s 512M /proc/self/fd/1; truncate -s 1T /proc/self/fd/1; exit 0",
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
        while fs::read(root.path().join("cancel/stdout.log")).unwrap_or_default() != b"ready" {
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
fn huge_sparse_post_execution_inputs_respect_the_deadline() {
    let (root, _) = run_fixture("true", 5);
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
    request.command[2] = "printf started; sleep 0.2".into();
    request.timeout_seconds = 1;
    let directory = root.path().join("growth");
    let started = std::time::Instant::now();
    let receipt = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &directory, &Default::default()).unwrap());
        while fs::read(directory.join("stdout.log")).unwrap_or_default() != b"started" {
            assert!(started.elapsed() < std::time::Duration::from_secs(3));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        fs::OpenOptions::new()
            .write(true)
            .open(root.path().join("input"))
            .unwrap()
            .set_len(1 << 40)
            .unwrap();
        task.join().unwrap()
    });
    assert_eq!(receipt.status, q::RunStatus::TimedOut);
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert!(q::verify_run(&directory, None).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn transient_source_input_and_alias_writes_are_blocked_but_work_is_writable() {
    let (root, _) = run_fixture("true", 5);
    fs::hard_link(root.path().join("source"), root.path().join("alias")).unwrap();
    let mut request: q::RunRequest = q::read_json(&root.path().join("run/request.json")).unwrap();
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
    header.set_entry_type(tar::EntryType::XGlobalHeader);
    header.set_mode(0o644);
    header.set_size(payload.len() as u64);
    header.set_cksum();
    tar.append_data(&mut header, "pax_global_header", &payload[..])
        .unwrap();
    tar.append_path_with_name(root.path().join("pax_global_header"), "pax_global_header")
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
    request.command[2] = "truncate -s 512M /proc/self/fd/1".into();
    request.timeout_seconds = 30;
    let run = root.path().join("final-cancel");
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let receipt = std::thread::scope(|scope| {
        let task = scope.spawn(|| q::execute(&request, &run, &cancel).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Native progress is published only after final composition capture.
        // Do not guess that capture finished after an arbitrary sleep on a
        // loaded runner; cancel while the bounded sparse log is being hashed.
        while q::read_json::<serde_json::Value>(&run.join("finalizing.json")).is_err() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        task.join().unwrap()
    });
    assert!(
        receipt.after.is_some(),
        "cancelled before final capture completed"
    );
    assert_eq!(receipt.status, q::RunStatus::Cancelled);
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
    use std::os::unix::fs::OpenOptionsExt;
    let root = tempdir().unwrap();
    let fifo = root.path().join("metadata");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    for signal in [rustix::process::Signal::INT, rustix::process::Signal::TERM] {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_canix-toolbelt"))
            .args(["qualification", "parents"])
            .arg(&fifo)
            .arg("app")
            .spawn()
            .unwrap();
        // Opening a writer proves the command reached its blocking metadata
        // read after dispatch and signal setup, rather than signalling startup.
        let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let writer = loop {
            match fs::OpenOptions::new()
                .write(true)
                .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                .open(&fifo)
            {
                Ok(file) => break file,
                Err(_) => {
                    assert!(std::time::Instant::now() < ready_deadline);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        };
        rustix::process::kill_process(
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
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("non-run command ignored {signal:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(signal.as_raw()));
        drop(writer);
    }
}
