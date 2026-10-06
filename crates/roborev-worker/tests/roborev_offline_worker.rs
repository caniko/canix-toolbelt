#![cfg(all(feature = "roborev-worker-tests", target_os = "linux"))]

use canix_toolbelt_roborev_worker::{
    Binding,
    execution::{ExecutionBinding, ExecutionFence, ExecutionState},
    offline::{Executable, OfflineSpec, WorkerLimits, run_offline},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Fixture(Option<tempfile::TempDir>);

impl Fixture {
    fn path(&self) -> &Path {
        self.0.as_ref().unwrap().path()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::env::var_os("CANIX_TEST_RETAIN_WORKER_EVIDENCE").is_some() {
            println!(
                "retained-worker-fixture={}",
                self.0.take().unwrap().keep().display()
            );
        }
    }
}

fn private() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    Fixture(Some(root))
}

fn retain(root: Fixture) {
    drop(root);
}

fn tool(variable: &str) -> Executable {
    Executable::capture(Path::new(&std::env::var(variable).expect(variable))).unwrap()
}

fn spec(root: &Path, script: &str) -> OfflineSpec {
    let checkout = root.join("checkout");
    fs::create_dir(&checkout).unwrap();
    fs::write(checkout.join("source.txt"), b"frozen source\n").unwrap();
    fs::set_permissions(
        checkout.join("source.txt"),
        fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    fs::set_permissions(&checkout, fs::Permissions::from_mode(0o500)).unwrap();
    OfflineSpec::capture(
        tool("CANIX_TEST_SYSTEMD_RUN"),
        tool("CANIX_TEST_SYSTEMCTL"),
        tool("CANIX_TEST_BWRAP"),
        Executable::capture(Path::new(env!("CARGO_BIN_EXE_roborev-worker-fixture"))).unwrap(),
        tool("CANIX_TEST_PYTHON"),
        vec!["/control/probe.py".into()],
        std::env::var("CANIX_TEST_STORE_PATHS")
            .expect("explicit runtime closure")
            .lines()
            .map(PathBuf::from)
            .collect(),
        checkout,
        BTreeMap::from([("probe.py".into(), script.as_bytes().to_vec())]),
        b"synthetic offline input\n".to_vec(),
        WorkerLimits::default(),
    )
    .unwrap()
}

fn fence(root: &Path, spec: &OfflineSpec) -> ExecutionFence {
    let state = root.join("fences");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let identity = format!("{:x}", Sha256::digest(root.as_os_str().as_encoded_bytes()));
    ExecutionFence::register(
        &state,
        &ExecutionBinding {
            request: Binding {
                request_url: "https://forge.invalid/owner/repo/pull/1".into(),
                request_id: identity.clone(),
                authorized_request_sha256: "a".repeat(64),
                execution_policy_sha256: spec.policy_digest().unwrap(),
                base: "b".repeat(40),
                head: "c".repeat(40),
            },
            controller_id: "d".repeat(64),
            daemon_identity_sha256: "e".repeat(64),
            job_id: 1,
            job_uuid: "01234567-89ab-4cde-8fab-0123456789ab".into(),
            execution_id: identity,
            input_manifest_sha256: spec.input_digest().unwrap(),
            backend_manifest_sha256: spec.backend_digest().unwrap(),
        },
    )
    .unwrap()
}

#[test]
fn actual_worker_is_verified_before_execution_and_preserves_immutable_inputs() {
    let root = private();
    let protected = root.path().join("protected");
    fs::write(&protected, b"protected synthetic sentinel").unwrap();
    let script = format!(
        r#"
import ctypes, json, os, socket
assert open('/repo/source.txt').read() == 'frozen source\n'
denied = []
for path in [{protected:?}, '/sys/fs/cgroup', '/run/user/1000/bus', '/repo/source.txt']:
    try:
        if path == '/repo/source.txt':
            open(path, 'w').write('mutated')
        else:
            open(path, 'rb').read()
    except OSError:
        denied.append(path)
assert len(denied) == 4, denied
assert ctypes.CDLL(None, use_errno=True).unshare(0x10000000) == -1
with socket.socket() as s:
    s.settimeout(0.1)
    assert s.connect_ex(('198.51.100.1', 443)) != 0
print(json.dumps({{'denied': len(denied), 'private_home': os.environ['HOME']}}))
"#,
        protected = protected.to_string_lossy()
    );
    let spec = spec(root.path(), &script);
    let fence = fence(root.path(), &spec);
    let receipt =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap();
    assert_eq!(receipt.boundary.capabilities, [0; 5]);
    assert!(receipt.boundary.no_new_privileges);
    assert!(receipt.cgroup_empty);
    assert!(receipt.backend_success);
    assert_eq!(
        fs::read(&protected).unwrap(),
        b"protected synthetic sentinel"
    );
    assert_eq!(
        fs::read(spec.checkout.join("source.txt")).unwrap(),
        b"frozen source\n"
    );
    let stdout = fs::read(root.path().join("result/stdout")).unwrap();
    let output: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(output["denied"], 4);
    assert_eq!(fence.state().unwrap(), ExecutionState::Unknown);
    assert!(fence.reserve().is_err());
    retain(root);
}

#[test]
fn changed_inputs_or_execution_binding_cannot_start_a_worker() {
    for change in ["input", "policy"] {
        let root = private();
        let mut spec = spec(root.path(), "print('must not run')");
        let fence = fence(root.path(), &spec);
        if change == "input" {
            fs::set_permissions(
                spec.checkout.join("source.txt"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
            fs::write(spec.checkout.join("source.txt"), b"changed").unwrap();
            fs::set_permissions(
                spec.checkout.join("source.txt"),
                fs::Permissions::from_mode(0o400),
            )
            .unwrap();
        } else {
            spec.limits.tasks += 1;
        }
        assert!(run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).is_err());
        assert!(!root.path().join("result/stdout").exists());
        assert_eq!(fence.state().unwrap(), ExecutionState::Unknown);
        assert!(fence.reserve().is_err());
        retain(root);
    }
}

#[test]
fn output_flood_and_detached_descendants_are_cancelled_as_one_cgroup() {
    let root = private();
    let spec = spec(
        root.path(),
        r#"
import os, subprocess, sys
subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], start_new_session=True)
while True:
    os.write(1, b'x' * 4096)
"#,
    );
    let fence = fence(root.path(), &spec);
    let error =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap_err();
    assert!(format!("{error:#}").contains("output"), "{error:#}");
    let cleanup: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("result/cleanup.json")).unwrap())
            .unwrap();
    assert_eq!(cleanup["cgroup_empty"], true);
    assert!(!root.path().join("result/receipt.json").exists());
    assert!(fence.reserve().is_err());
    retain(root);
}

#[test]
fn timed_out_execution_preserves_unknown_and_reaps_descendants() {
    let root = private();
    let mut spec = spec(root.path(), "import time; time.sleep(60)");
    spec.limits.wall_seconds = 3;
    let fence = fence(root.path(), &spec);
    let error =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap_err();
    assert!(
        format!("{error:#}").contains("deadline")
            || format!("{error:#}").contains("backend or worker unit failed"),
        "{error:#}"
    );
    let cleanup: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("result/cleanup.json")).unwrap())
            .unwrap();
    assert_eq!(cleanup["cgroup_empty"], true);
    assert!(fence.reserve().is_err());
    retain(root);
}

#[test]
fn scratch_inodes_bytes_and_tasks_have_aggregate_kernel_limits() {
    let root = private();
    let mut spec = spec(
        root.path(),
        r#"
import errno, json, os, subprocess, sys
created = 0
try:
    while True:
        open('/work/cache/f' + str(created), 'w').close()
        created += 1
except OSError as e:
    assert e.errno == errno.ENOSPC, e
assert created < 64
for i in range(created):
    os.unlink('/work/cache/f' + str(i))
total = 0
try:
    with open('/work/cache/big', 'wb') as f:
        while True:
            f.write(b'x' * 4096)
            f.flush()
            total += 4096
except OSError as e:
    assert e.errno == errno.ENOSPC, e
assert 0 < total <= 1048576
children = []
try:
    while True:
        children.append(subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']))
except OSError as e:
    assert e.errno == errno.EAGAIN, e
finally:
    for child in children:
        child.kill()
    for child in children:
        child.wait()
assert 0 < len(children) < 8
print(json.dumps({'files': created, 'bytes': total, 'children': len(children)}))
"#,
    );
    spec.limits.scratch_inodes = 64;
    spec.limits.scratch_bytes = 1024 * 1024;
    spec.limits.tasks = 8;
    let fence = fence(root.path(), &spec);
    let receipt =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap();
    assert!(receipt.cgroup_empty);
    assert_eq!(receipt.boundary.scratch_inodes, 64);
    retain(root);
}

#[test]
fn successful_output_is_fully_drained_before_a_durable_receipt() {
    let root = private();
    let spec = spec(
        root.path(),
        "import os; os.write(1, b'x' * 300000); os.write(2, b'y' * 200000)",
    );
    let fence = fence(root.path(), &spec);
    let receipt =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap();
    assert_eq!(
        fs::read(root.path().join("result/stdout")).unwrap(),
        vec![b'x'; 300000]
    );
    assert_eq!(
        fs::read(root.path().join("result/stderr")).unwrap(),
        vec![b'y'; 200000]
    );
    assert!(receipt.cgroup_empty);
    retain(root);
}

#[test]
fn pinned_opencode_uses_a_private_standalone_backend_and_synthetic_provider() {
    let root = private();
    let pinned = tool("CANIX_TEST_OPENCODE");
    assert_eq!(
        pinned.sha256,
        std::env::var("CANIX_TEST_OPENCODE_SHA256").unwrap()
    );
    let mut spec = spec(
        root.path(),
        include_str!("fixtures/roborev-synthetic-provider.py"),
    );
    spec.trusted_files.insert(
        "synthetic.json".into(),
        serde_json::to_vec(&serde_json::json!({
            "executable": pinned.path, "sha256": pinned.sha256
        }))
        .unwrap(),
    );
    fs::set_permissions(&spec.checkout, fs::Permissions::from_mode(0o700)).unwrap();
    for (name, bytes) in [
        ("AGENTS.md", b"HOSTILE_POLICY_SENTINEL".as_slice()),
        (
            "opencode.json",
            b"{\"plugins\":[\"HOSTILE_POLICY_SENTINEL\"]}".as_slice(),
        ),
    ] {
        fs::write(spec.checkout.join(name), bytes).unwrap();
        fs::set_permissions(spec.checkout.join(name), fs::Permissions::from_mode(0o400)).unwrap();
    }
    fs::set_permissions(&spec.checkout, fs::Permissions::from_mode(0o500)).unwrap();
    spec.limits.wall_seconds = 90;
    spec.limits.tasks = 128;
    spec.limits.memory_bytes = 2 * 1024 * 1024 * 1024;
    spec.limits.scratch_bytes = 128 * 1024 * 1024;
    // Refresh the captured input identity after constructing the hostile fixture.
    spec = OfflineSpec::capture(
        spec.systemd_run,
        spec.systemctl,
        spec.bubblewrap,
        spec.helper,
        spec.backend,
        spec.arguments,
        spec.store_paths,
        spec.checkout,
        spec.trusted_files,
        spec.prompt,
        spec.limits,
    )
    .unwrap();
    let fence = fence(root.path(), &spec);
    let receipt =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("result/stdout")).unwrap()).unwrap();
    assert_eq!(output["standalone"], true);
    assert_eq!(output["requests"].as_array().unwrap().len(), 1);
    assert!(receipt.cgroup_empty);
    assert!(fence.reserve().is_err());
    retain(root);
}

#[test]
fn memory_exhaustion_fails_the_generation_and_reaps_its_cgroup() {
    let root = private();
    let mut spec = spec(
        root.path(),
        "blob = bytearray(256 * 1024 * 1024); print('must not complete')",
    );
    spec.limits.memory_bytes = 64 * 1024 * 1024;
    let fence = fence(root.path(), &spec);
    let error =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap_err();
    assert!(
        format!("{error:#}").contains("backend or worker unit failed"),
        "{error:#}"
    );
    let cleanup: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("result/cleanup.json")).unwrap())
            .unwrap();
    assert_eq!(cleanup["cgroup_empty"], true);
    assert!(fence.reserve().is_err());
    retain(root);
}

#[test]
fn non_page_aligned_scratch_budget_never_grows_during_kernel_setup() {
    let root = private();
    let mut spec = spec(root.path(), "print('bounded scratch')");
    spec.limits.scratch_bytes = 1024 * 1024 + 1;
    let fence = fence(root.path(), &spec);
    let receipt =
        run_offline(fence.reserve().unwrap(), &spec, &root.path().join("result")).unwrap();
    assert!(receipt.boundary.scratch_bytes <= spec.limits.scratch_bytes);
    assert_eq!(receipt.boundary.scratch_bytes, 1024 * 1024);
    retain(root);
}

#[test]
fn controller_death_does_not_release_replay_or_kernel_runtime_budget() {
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    let root = private();
    let mut spec = spec(root.path(), "import time; time.sleep(60)");
    spec.limits.wall_seconds = 4;
    let fence = fence(root.path(), &spec);
    fs::write(
        root.path().join("fence.json"),
        serde_json::to_vec(&fence).unwrap(),
    )
    .unwrap();
    fs::write(
        root.path().join("spec.json"),
        serde_json::to_vec(&spec).unwrap(),
    )
    .unwrap();
    let mut controller = Command::new(env!("CARGO_BIN_EXE_roborev-worker-fixture"))
        .arg("launch")
        .arg(root.path().join("fence.json"))
        .arg(root.path().join("spec.json"))
        .arg(root.path().join("result"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("result/boundary.json").exists() {
        assert!(
            controller.try_wait().unwrap().is_none(),
            "controller failed before admission"
        );
        assert!(
            Instant::now() < deadline,
            "controller did not reach admission"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let boundary: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("result/boundary.json")).unwrap())
            .unwrap();
    controller.kill().unwrap();
    controller.wait().unwrap();
    let cgroup = Path::new("/sys/fs/cgroup")
        .join(boundary["cgroup"].as_str().unwrap().trim_start_matches('/'));
    loop {
        match fs::read_to_string(cgroup.join("cgroup.events")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Ok(events) if events.lines().any(|line| line == "populated 0") => break,
            Ok(_) => assert!(
                Instant::now() < deadline,
                "worker survived its independent runtime budget"
            ),
            Err(error) => panic!("{error}"),
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fence.state().unwrap(), ExecutionState::Unknown);
    assert!(fence.reserve().is_err());
    assert!(!root.path().join("result/receipt.json").exists());
    let launch: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("result/launch.json")).unwrap()).unwrap();
    let status = Command::new(&spec.systemctl.path)
        .args(["--user", "reset-failed", launch["unit"].as_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    retain(root);
}
