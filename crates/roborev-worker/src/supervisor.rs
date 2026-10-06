//! The trusted Rust copy/hash/journal work also needs a killable process boundary:
//! a cooperative deadline cannot interrupt a blocked read or fsync.
use super::{Binding, Limits, Prepared, Tools, prepare_in_process};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_WIRE: u64 = 65_536;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    objects: PathBuf,
    state: PathBuf,
    binding: Binding,
    tools: Tools,
    limits: Limits,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", content = "value", deny_unknown_fields)]
enum Reply {
    Prepared(Prepared),
    Failed(String),
}

struct Helper {
    child: Child,
    reaped: bool,
}

impl Helper {
    fn exited(&self) -> Result<bool> {
        use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
        // WNOWAIT pins the unreaped group leader until terminate kills that group.
        let status = waitid(
            Id::Pid(nix::unistd::Pid::from_raw(self.child.id() as i32)),
            WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
        )?;
        Ok(status != WaitStatus::StillAlive)
    }

    fn terminate(&mut self) -> std::io::Result<std::process::ExitStatus> {
        // The process group is established before exec. Git's private PID
        // namespace additionally dies with its helper parent via Bubblewrap.
        if !self.reaped {
            // This leader remains unreaped, so its group cannot be recycled.
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.child.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = self.child.kill();
        }
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.terminate();
        }
    }
}

fn read_wire(input: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(MAX_WIRE + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_WIRE,
        "preparation helper exceeded its protocol bound"
    );
    Ok(bytes)
}

pub(super) fn prepare(
    objects: &Path,
    state: &Path,
    binding: &Binding,
    tools: &Tools,
    limits: &Limits,
) -> Result<Prepared> {
    let started = Instant::now();
    let request = serde_json::to_vec(&Request {
        objects: objects.into(),
        state: state.into(),
        binding: binding.clone(),
        tools: tools.clone(),
        limits: limits.clone(),
    })?;
    ensure!(
        request.len() as u64 <= MAX_WIRE,
        "oversized preparation helper request"
    );
    let mut command = Command::new(&tools.preparer);
    command
        .arg("__canix-roborev-prepare")
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::platform::configure_command(
        &mut command,
        limits.memory_bytes,
        limits.cpu_seconds,
        limits.snapshot_bytes,
        Some(nix::unistd::getpid()),
    );
    let mut helper = Helper {
        child: command
            .spawn()
            .context("start supervised preparation helper")?,
        reaped: false,
    };
    let mut stdin = helper.child.stdin.take().context("missing helper stdin")?;
    let stdout = helper
        .child
        .stdout
        .take()
        .context("missing helper stdout")?;
    let stderr = helper
        .child
        .stderr
        .take()
        .context("missing helper stderr")?;
    let writer = thread::spawn(move || stdin.write_all(&request));
    let output = thread::spawn(move || read_wire(stdout));
    let errors = thread::spawn(move || read_wire(stderr));
    let status = loop {
        if helper.exited()? {
            break helper.terminate()?;
        }
        if started.elapsed() >= Duration::from_secs(limits.wall_seconds) {
            let _ = helper.terminate();
            let _ = writer.join();
            let _ = output.join();
            let _ = errors.join();
            bail!(
                "whole preparation exceeded its time bound; retain the same request for diagnosis"
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Reap background descendants holding protocol pipes even on normal exit.
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("helper input writer failed"))??;
    let output = output
        .join()
        .map_err(|_| anyhow::anyhow!("helper output reader failed"))??;
    let _errors = errors
        .join()
        .map_err(|_| anyhow::anyhow!("helper error reader failed"))??;
    ensure!(
        status.success(),
        "preparation helper failed ({status}); diagnostics omitted"
    );
    ensure!(
        started.elapsed() < Duration::from_secs(limits.wall_seconds),
        "whole preparation exceeded its time bound"
    );
    match serde_json::from_slice(&output).context("invalid complete preparation helper reply")? {
        Reply::Prepared(prepared) => {
            ensure!(
                prepared.binding == *binding
                    && prepared.checkout == state.join(binding.key()).join("checkout")
                    && prepared.journal_path()
                        == state.join(binding.key()).join("preparation.json")
                    && super::is_hex(&prepared.snapshot_sha256, 64),
                "preparation helper returned a different identity"
            );
            Ok(prepared)
        }
        Reply::Failed(error) => bail!("{error}"),
    }
}

/// Private, bounded stdin/stdout protocol shared by the Toolbelt worker executable
/// and native qualification helper. It carries no authorization or agent action.
pub fn helper_main() -> Result<()> {
    let result = (|| -> Result<Prepared> {
        let request: Request = serde_json::from_slice(&read_wire(std::io::stdin().lock())?)?;
        prepare_in_process(
            &request.objects,
            &request.state,
            &request.binding,
            &request.tools,
            &request.limits,
        )
    })();
    let reply = match result {
        Ok(prepared) => Reply::Prepared(prepared),
        Err(error) => Reply::Failed(error.to_string()),
    };
    let bytes = serde_json::to_vec(&reply)?;
    ensure!(
        bytes.len() as u64 <= MAX_WIRE,
        "oversized preparation helper reply"
    );
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
