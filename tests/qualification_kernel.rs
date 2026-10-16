#![cfg(all(target_os = "linux", feature = "qualification"))]

use canix_toolbelt::qualification as q;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, time::Instant};

#[test]
fn kernel_observation_covers_escape_flags_async_io_faults_and_thread_exec() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("probe.c");
    let probe = root.path().join("probe");
    fs::write(
        &source,
        include_bytes!("fixtures/qualification-kernel-probe.c"),
    )
    .unwrap();
    let build = Command::new("cc")
        .args(["-pthread", "-Wall", "-Wextra", "-Werror", "-O2"])
        .arg(&source)
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let program = |name: &str| -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join(name))
            .find(|path| path.is_file())
            .unwrap()
    };
    let mut request = q::RunRequest {
        cwd: root.path().into(),
        sources: [(
            "probe".into(),
            q::InventorySpec {
                root: root.path().into(),
                paths: vec!["probe.c".into()],
            },
        )]
        .into(),
        inputs: BTreeMap::new(),
        command: vec![probe.to_str().unwrap().into(), String::new()],
        argument_zero: None,
        environment: BTreeMap::new(),
        tools: BTreeMap::new(),
        tool_argument_zeros: BTreeMap::new(),
        runtime_paths: Vec::new(),
        timeout_seconds: 30,
        containment: Some(q::ContainmentPrograms {
            unshare: program("unshare"),
            executor: env!("CARGO_BIN_EXE_canix-toolbelt-qualification-executor").into(),
            setpriv: program("setpriv"),
        }),
    };
    let mut modes = vec![
        "clone-untraced",
        "clone3",
        "async-io",
        "mmap-quota",
        "thread-exec",
        "syscalls",
    ];
    if cfg!(target_arch = "x86_64") {
        modes.push("x32-syscall");
    }
    for mode in modes {
        request.command[1] = mode.into();
        let directory = root.path().join(mode);
        let receipt = q::execute(&request, &directory, &Default::default()).unwrap();
        let stdout = fs::read_to_string(directory.join("stdout.log")).unwrap();
        let stderr = fs::read_to_string(directory.join("stderr.log")).unwrap();
        if mode == "x32-syscall" {
            assert_eq!(receipt.status, q::RunStatus::Failed, "{receipt:#?}");
            assert_eq!(receipt.execution_signal, Some(libc::SIGSYS));
            assert!(stdout.is_empty());
            assert!(q::verify_run(&directory, None).is_err());
            continue;
        }
        assert_eq!(
            stdout.trim(),
            if mode == "thread-exec" {
                "thread-exec"
            } else {
                "probe-complete"
            },
            "{mode}: {stderr}"
        );
        if mode == "mmap-quota" {
            assert_eq!(receipt.status, q::RunStatus::Failed, "{mode}: {receipt:#?}");
            assert!(stderr.contains("producer encountered a work-tree quota failure"));
            assert!(q::verify_run(&directory, None).is_err());
        } else {
            assert_eq!(
                receipt.status,
                q::RunStatus::Passed,
                "{mode}: {receipt:#?}; {stderr}"
            );
            q::verify_run(&directory, None).unwrap();
        }
        if mode == "syscalls" {
            let started = Instant::now();
            assert!(
                Command::new(&probe)
                    .arg(mode)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            eprintln!(
                "10,000 getpid syscalls: direct={}us; complete contained execution={}ms (includes capture, namespaces, observation, retention and publication)",
                started.elapsed().as_micros(),
                receipt.elapsed_ms
            );
        }
    }
}
