//! Private post-fork setup. Keep every operation inside pre_exec allocation-free.
use nix::{
    sys::{
        prctl::set_pdeathsig,
        resource::{Resource, setrlimit},
        signal::Signal,
    },
    unistd::{Pid, getppid, setpgid},
};
use std::{os::unix::process::CommandExt, process::Command};

// Command::pre_exec is necessarily unsafe; this wrapper accepts only scalar
// limits and an optional original parent. No arbitrary closure crosses it.
#[allow(unsafe_code)]
pub(crate) fn configure_command(
    command: &mut Command,
    memory: u64,
    cpu: u64,
    size: u64,
    parent: Option<Pid>,
) {
    // SAFETY: the child closure performs only allocation-free syscalls. It never
    // accesses inherited Rust locks, executes user callbacks or handles secrets.
    unsafe {
        command.pre_exec(move || {
            if let Some(parent) = parent {
                setpgid(Pid::from_raw(0), Pid::from_raw(0))?;
                set_pdeathsig(Signal::SIGKILL)?;
                if getppid() != parent {
                    return Err(std::io::Error::from_raw_os_error(nix::libc::ESRCH));
                }
            }
            setrlimit(Resource::RLIMIT_AS, memory, memory)?;
            setrlimit(Resource::RLIMIT_CPU, cpu, cpu)?;
            setrlimit(Resource::RLIMIT_FSIZE, size, size)?;
            setrlimit(Resource::RLIMIT_CORE, 0, 0)?;
            Ok(())
        });
    }
}
