use super::budget::Budget;
use super::files::CopiedObjects;
use super::{Limits, Tools};
use anyhow::{Context, Result, ensure};
use std::{
    io::{BufRead, BufReader, Read},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Component, Path},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

const MAX_OUTPUT: u64 = 65_536;

struct ConfinedChild(Child);

impl Drop for ConfinedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub(super) fn validated_tools(tools: &Tools) -> Result<Tools> {
    let immutable = |path: &Path| -> Result<_> {
        ensure!(
            path.is_absolute() && path.starts_with("/nix/store"),
            "preparation tool must be an explicit immutable Nix-store executable"
        );
        let path = path.canonicalize().context("locate preparation tool")?;
        let meta = path.metadata()?;
        ensure!(
            path.starts_with("/nix/store")
                && meta.is_file()
                && meta.permissions().mode() & 0o111 != 0,
            "invalid immutable preparation executable"
        );
        Ok(path)
    };
    Ok(Tools {
        git: immutable(&tools.git)?,
        bubblewrap: immutable(&tools.bubblewrap)?,
        preparer: {
            // Native test binaries are Cargo artifacts. Production accepts only
            // an explicit immutable store executable and has no test fallback.
            #[cfg(feature = "roborev-preparation-tests")]
            {
                let path = tools.preparer.canonicalize()?;
                ensure!(
                    path.is_file() && path.metadata()?.permissions().mode() & 0o111 != 0,
                    "invalid native preparation helper"
                );
                path
            }
            #[cfg(not(feature = "roborev-preparation-tests"))]
            {
                immutable(&tools.preparer)?
            }
        },
    })
}

pub(super) fn check_expansion(
    tools: &Tools,
    checkout: &Path,
    limits: &Limits,
    budget: &Budget,
    head: &str,
    copied: &CopiedObjects,
) -> Result<()> {
    let (object_bytes, object_entries, hash_length) = (copied.bytes, copied.entries, head.len());
    let inventory_limits = limits.clone();
    run_with_stdout(
        tools,
        checkout,
        limits,
        budget,
        &["ls-tree", "-rtlz", "--full-tree", head],
        move |pipe| {
            check_inventory(
                pipe,
                &inventory_limits,
                hash_length,
                object_bytes,
                object_entries,
            )
        },
    )
}

fn check_inventory(
    pipe: Box<dyn Read + Send>,
    limits: &Limits,
    hash_length: usize,
    object_bytes: u64,
    object_entries: usize,
) -> Result<()> {
    // Bound each raw NUL-delimited record rather than truncating the inventory.
    // Metadata is ASCII; committed path bytes need not be UTF-8. Retain the
    // complete-success/EOF requirement before allowing any checkout writes.
    let mut input = BufReader::new(pipe);
    let mut bytes = object_bytes + 16_384;
    let mut entries = object_entries + 512;
    loop {
        let mut record = Vec::new();
        let size = Read::by_ref(&mut input)
            .take(4353)
            .read_until(0, &mut record)?;
        if size == 0 {
            break;
        }
        ensure!(
            size <= 4352 && record.last() == Some(&0),
            "incomplete or oversized tree inventory record before checkout"
        );
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("invalid tree inventory before checkout")?;
        let metadata =
            std::str::from_utf8(&record[..tab]).context("invalid tree inventory metadata")?;
        let name = &record[tab + 1..record.len() - 1];
        let fields: Vec<_> = metadata.split_ascii_whitespace().collect();
        ensure!(
            fields.len() == 4 && super::is_hex(fields[2], hash_length),
            "invalid tree identity before checkout"
        );
        let path = Path::new(std::ffi::OsStr::from_bytes(name));
        ensure!(
            !name.is_empty()
                && name.len() <= 4096
                && path.components().count() <= 128
                && path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "unsupported tree path before checkout"
        );
        let size = match (fields[0], fields[1], fields[3]) {
            ("040000", "tree", "-") | ("160000", "commit", "-") => 0,
            ("100644" | "100755" | "120000", "blob", size) => size
                .parse::<u64>()
                .context("invalid blob size before checkout")?,
            _ => anyhow::bail!("unsupported tree entry before checkout"),
        };
        bytes = bytes
            .checked_add(size)
            .and_then(|bytes| bytes.checked_add(128 + record.len() as u64 * 2))
            .context("tree size overflow before checkout")?;
        entries += 1;
        ensure!(
            bytes <= limits.snapshot_bytes && entries <= limits.entries,
            "tree expansion exceeds preparation bound before checkout"
        );
    }
    ensure!(
        bytes <= limits.snapshot_bytes && entries <= limits.entries,
        "Git metadata exceeds preparation bound before checkout"
    );
    Ok(())
}

pub(super) fn run(
    tools: &Tools,
    checkout: &Path,
    limits: &Limits,
    budget: &Budget,
    args: &[&str],
) -> Result<String> {
    run_with_stdout(tools, checkout, limits, budget, args, |pipe| {
        let output = read_bounded(pipe)?;
        String::from_utf8(output).context("non-UTF-8 preparation Git identity")
    })
}

fn read_bounded(pipe: Box<dyn Read + Send>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(MAX_OUTPUT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_OUTPUT,
        "preparation Git exceeded its output bound"
    );
    Ok(bytes)
}

fn run_with_stdout<T: Send + 'static>(
    tools: &Tools,
    checkout: &Path,
    limits: &Limits,
    budget: &Budget,
    args: &[&str],
    read_stdout: impl FnOnce(Box<dyn Read + Send>) -> Result<T> + Send + 'static,
) -> Result<T> {
    budget.check()?;
    let mut command = Command::new(&tools.bubblewrap);
    command
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "--unshare-all",
            "--unshare-user",
            "--unshare-cgroup",
            // Git owns/reaps its children. Running it as namespace PID 1 keeps
            // their CPU charges on the wait chain back to the Rust helper.
            "--as-pid-1",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--ro-bind",
            "/nix/store",
            "/nix/store",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/home/review",
            "--bind",
        ])
        .arg(checkout)
        .arg("/snapshot")
        .args([
            "--clearenv",
            "--setenv",
            "HOME",
            "/home/review",
            "--setenv",
            "TMPDIR",
            "/tmp",
            "--setenv",
            "GIT_CONFIG_NOSYSTEM",
            "1",
            "--setenv",
            "GIT_CONFIG_GLOBAL",
            "/dev/null",
            "--setenv",
            "GIT_TERMINAL_PROMPT",
            "0",
            "--setenv",
            "GIT_NO_REPLACE_OBJECTS",
            "1",
            "--setenv",
            "GIT_ATTR_NOSYSTEM",
            "1",
            "--setenv",
            "GIT_LFS_SKIP_SMUDGE",
            "1",
            "--chdir",
            "/snapshot",
            "--",
        ])
        .arg(&tools.git)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.allow=never",
            "-c",
            "credential.helper=",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "core.autocrlf=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
        ])
        .args(args);
    let (memory, cpu, size) = (
        limits.memory_bytes,
        budget.remaining_cpu_seconds()?.max(1),
        limits.snapshot_bytes,
    );
    super::platform::configure_command(&mut command, memory, cpu, size, None);
    let mut child = ConfinedChild(command.spawn().context("start confined preparation Git")?);
    let stdout = child
        .0
        .stdout
        .take()
        .context("missing preparation stdout")?;
    let stderr = child
        .0
        .stderr
        .take()
        .context("missing preparation stderr")?;
    let output = thread::spawn(move || read_stdout(Box::new(stdout)));
    let errors = thread::spawn(move || read_bounded(Box::new(stderr)));
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if let Err(error) = budget.check() {
            let _ = child.0.kill();
            let _ = child.0.wait();
            let _ = output.join();
            let _ = errors.join();
            return Err(error);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = output
        .join()
        .map_err(|_| anyhow::anyhow!("preparation output reader failed"))??;
    let _errors = errors
        .join()
        .map_err(|_| anyhow::anyhow!("preparation error reader failed"))??;
    budget.check()?;
    // Do not return attacker-controlled Git diagnostics as controller instructions
    // or leak raw repository content through a generic error report.
    ensure!(
        status.success(),
        "confined preparation Git failed at {} ({status})",
        args.first().unwrap_or(&"verification")
    );
    Ok(output)
}

#[cfg(all(test, feature = "roborev-preparation-tests"))]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{fs, path::PathBuf, time::Instant};

    fn probe(script: &str, limits: &Limits) -> Result<String> {
        probe_with_budget(script, limits, &Budget::new(limits)?)
    }

    fn probe_with_budget(script: &str, limits: &Limits, budget: &Budget) -> Result<String> {
        let temporary = tempfile::tempdir()?;
        let checkout = temporary.path().join("checkout");
        fs::create_dir(&checkout)?;
        let protected = temporary.path().join("protected-synthetic-sentinel");
        fs::write(&protected, b"SYNTHETIC_PROTECTED_CONTENT")?;
        let protected_literal = serde_json::to_string(&protected)?;
        fs::write(
            checkout.join("probe.py"),
            format!("protected = {protected_literal}\n{script}"),
        )?;
        let tools = validated_tools(&Tools {
            git: PathBuf::from(std::env::var("CANIX_TEST_GIT")?),
            bubblewrap: PathBuf::from(std::env::var("CANIX_TEST_BWRAP")?),
            preparer: PathBuf::from(std::env::var("CANIX_TEST_GIT")?),
        })?;
        let python = std::env::var("CANIX_TEST_PYTHON")?;
        ensure!(
            python.starts_with("/nix/store/") && !python.contains(['\'', ' ', '\n']),
            "explicit test Python required"
        );
        // Trusted diagnostic alias, never a repository-provided command. It
        // exercises the very same private launcher used by preparation Git.
        run(
            &tools,
            &checkout,
            limits,
            budget,
            &[
                "-c",
                &format!("alias.probe=!{python} /snapshot/probe.py"),
                "probe",
            ],
        )
    }

    #[test]
    fn actual_preparation_namespace_denies_host_access_and_applies_process_limits() {
        let limits = Limits::default();
        let output = probe(
            r#"
import json, os, resource
print(json.dumps({
    'home': os.environ['HOME'],
    'hostHome': os.path.exists('/home/can'),
    'hostRuntime': os.path.exists('/run/current-system'),
    'protectedSynthetic': os.path.exists(protected),
    'networkNamespace': os.readlink('/proc/self/ns/net'),
    'interfaces': [line.split(':')[0].strip() for line in open('/proc/net/dev').readlines()[2:]],
    'memory': resource.getrlimit(resource.RLIMIT_AS),
    'cpu': resource.getrlimit(resource.RLIMIT_CPU),
    'file': resource.getrlimit(resource.RLIMIT_FSIZE),
    'core': resource.getrlimit(resource.RLIMIT_CORE),
}))
"#,
            &limits,
        )
        .unwrap();
        let facts: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(facts["home"], "/home/review");
        assert_eq!(facts["hostHome"], false);
        assert_eq!(facts["hostRuntime"], false);
        assert_eq!(facts["protectedSynthetic"], false);
        assert_eq!(facts["interfaces"], serde_json::json!(["lo"]));
        assert_ne!(
            facts["networkNamespace"].as_str().unwrap(),
            fs::read_link("/proc/self/ns/net")
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert_eq!(
            facts["memory"],
            serde_json::json!([limits.memory_bytes, limits.memory_bytes])
        );
        assert_eq!(
            facts["cpu"],
            serde_json::json!([limits.cpu_seconds, limits.cpu_seconds])
        );
        assert_eq!(
            facts["file"],
            serde_json::json!([limits.snapshot_bytes, limits.snapshot_bytes])
        );
        assert_eq!(facts["core"], serde_json::json!([0, 0]));
    }

    #[test]
    fn preparation_timeout_reaps_a_descendant_holding_the_output_pipes() {
        let limits = Limits {
            wall_seconds: 1,
            ..Limits::default()
        };
        let started = Instant::now();
        let error = probe(
            r#"
import subprocess, sys, time
subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
time.sleep(60)
"#,
            &limits,
        )
        .unwrap_err();
        assert!(error.to_string().contains("time bound"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn preparation_rejects_oversized_output_instead_of_truncating_identity() {
        let error = probe(
            "import sys\nsys.stdout.write('x' * 65537)\n",
            &Limits::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("output bound"), "{error}");
    }

    #[test]
    fn consecutive_git_commands_cannot_reset_the_preparation_time_budget() {
        let limits = Limits {
            wall_seconds: 1,
            ..Limits::default()
        };
        let budget = Budget::new(&limits).unwrap();
        probe_with_budget("import time\ntime.sleep(0.65)\n", &limits, &budget).unwrap();
        let error =
            probe_with_budget("import time\ntime.sleep(0.65)\n", &limits, &budget).unwrap_err();
        assert!(error.to_string().contains("time bound"), "{error}");
    }

    #[test]
    fn consecutive_git_commands_cannot_reset_the_cumulative_cpu_budget() {
        let limits = Limits {
            cpu_seconds: 1,
            ..Limits::default()
        };
        let budget = Budget::new(&limits).unwrap();
        let script = "import time\nstart = time.process_time()\nwhile time.process_time() - start < 0.65: pass\n";
        probe_with_budget(script, &limits, &budget).unwrap();
        let error = probe_with_budget(script, &limits, &budget).unwrap_err();
        assert!(error.to_string().contains("CPU bound"), "{error}");
    }

    #[test]
    fn streamed_inventory_requires_complete_bounded_raw_records() {
        let metadata = format!("100644 blob {} 1\t", "a".repeat(40));
        let mut raw = metadata.as_bytes().to_vec();
        raw.extend_from_slice(b"non-utf8-\xff\0");
        let check = |bytes| {
            check_inventory(
                Box::new(std::io::Cursor::new(bytes)),
                &Limits::default(),
                40,
                0,
                0,
            )
        };
        check(raw.clone()).unwrap();
        raw.pop();
        assert!(check(raw).unwrap_err().to_string().contains("incomplete"));
        let oversized = format!("{metadata}{}\0", "x".repeat(5000));
        assert!(
            check(oversized.into_bytes())
                .unwrap_err()
                .to_string()
                .contains("oversized")
        );
        let absolute = format!("{metadata}/outside\0");
        assert!(
            check(absolute.into_bytes())
                .unwrap_err()
                .to_string()
                .contains("unsupported tree path")
        );
    }
}
