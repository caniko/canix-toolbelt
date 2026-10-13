//! Private PID-namespace init for qualification. Not an operator frontend.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
fn supervise() -> std::io::Result<i32> {
    use std::process::Command;
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
    use std::time::Duration;

    // A fresh PID namespace plus its own /proc is mandatory. Refuse direct use.
    if std::process::id() != 1
        || std::fs::read_to_string("/proc/self/stat")?
            .split_whitespace()
            .next()
            != Some("1")
    {
        return Err(std::io::Error::other(
            "executor requires a fresh PID namespace and proc mount",
        ));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, Arc::clone(&cancelled))?;
    }
    let mut args = std::env::args_os().skip(1);
    let program = args
        .next()
        .ok_or_else(|| std::io::Error::other("missing executable"))?;
    let mut child = Command::new(program).args(args).spawn()?;
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(125);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // After reaping the leader, orphans have been reparented to PID 1. Consult
    // kernel child accounting, not /proc, whose mount a privileged producer can
    // replace. A live child, zombie, or unknown wait error invalidates completion.
    // Exiting init makes the kernel kill/drain the whole namespace before the
    // launcher can reap us. wait() uses waitpid(-1), covering all process groups.
    match rustix::process::wait(rustix::process::WaitOptions::NOHANG) {
        Err(rustix::io::Errno::CHILD) => Ok(status.code().unwrap_or(125)),
        Ok(_) => Ok(125),
        Err(error) => Err(error.into()),
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    match supervise() {
        Ok(code) => return std::process::ExitCode::from(u8::try_from(code).unwrap_or(125)),
        Err(error) => eprintln!("qualification executor: {error}"),
    }
    #[cfg(not(target_os = "linux"))]
    eprintln!("qualification execution requires Linux PID namespaces");
    std::process::ExitCode::from(125)
}
