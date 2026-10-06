use anyhow::{Context, Result, ensure};
use std::{
    io::{ErrorKind, Read},
    os::fd::AsFd,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) struct ChildGuard(pub Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn nonblocking(pipe: &impl AsFd) -> Result<()> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let flags = OFlag::from_bits_retain(fcntl(pipe, FcntlArg::F_GETFL)?);
    fcntl(pipe, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(())
}

/// Systemd control queries are bounded independently of backend runtime. A hung
/// manager cannot indefinitely hold a caller or accumulate unbounded diagnostics.
pub(super) fn output(command: &mut Command) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = ChildGuard(
        command
            .spawn()
            .context("start bounded controller command")?,
    );
    let mut stdout = child.0.stdout.take().context("missing control stdout")?;
    let mut stderr = child.0.stderr.take().context("missing control stderr")?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut out_done = false;
    let mut err_done = false;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        for (pipe, bytes, done) in [
            (&mut stdout as &mut dyn Read, &mut out, &mut out_done),
            (&mut stderr as &mut dyn Read, &mut err, &mut err_done),
        ] {
            if *done {
                continue;
            }
            let mut buffer = [0; 8192];
            match pipe.read(&mut buffer) {
                Ok(0) => *done = true,
                Ok(size) => {
                    ensure!(
                        bytes.len() + size <= 65536,
                        "controller command output exceeds budget"
                    );
                    bytes.extend_from_slice(&buffer[..size]);
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(error) => return Err(error.into()),
            }
        }
        if out_done
            && err_done
            && let Some(status) = child.0.try_wait()?
        {
            return Ok(Output {
                status,
                stdout: out,
                stderr: err,
            });
        }
        ensure!(
            Instant::now() < deadline,
            "controller command deadline exceeded"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
