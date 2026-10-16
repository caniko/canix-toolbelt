//! Private Linux syscall-result observer. A tmpfs cap bounds consumption but
//! cannot remember a handled ENOSPC after cleanup. PID 1 owns this latch; no
//! producer-controlled file or sampling interval establishes quota acceptance.
use std::{
    collections::BTreeSet,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

use super::execution::ExecutionReport;

// Linux UAPI, including musl targets whose libc bindings omit this request.
const GET_SYSCALL_INFO: u32 = 0x420e;

#[allow(unsafe_code)]
fn ptrace(request: u32, pid: libc::pid_t, data: usize) -> io::Result<()> {
    // SAFETY: requests only operate on our stopped namespace descendants. Data
    // is either an integer option/signal or points to the caller's live buffer.
    if unsafe {
        libc::ptrace(
            request as _,
            pid,
            std::ptr::null_mut::<libc::c_void>(),
            data as *mut libc::c_void,
        )
    } == -1
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[allow(unsafe_code)]
fn child_signals(block: bool, wait: bool) -> io::Result<()> {
    // SAFETY: sigemptyset initializes the set before use; all pointers refer to
    // stack values for the duration of these synchronous libc calls. This runs
    // in a single-threaded fresh namespace/exec adapter, never the host caller.
    unsafe {
        let mut set = std::mem::zeroed();
        if libc::sigemptyset(&mut set) != 0 || libc::sigaddset(&mut set, libc::SIGCHLD) != 0 {
            return Err(io::Error::last_os_error());
        }
        if wait {
            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000,
            };
            if libc::sigtimedwait(&set, std::ptr::null_mut(), &timeout) < 0 {
                let error = io::Error::last_os_error();
                if !matches!(error.raw_os_error(), Some(libc::EAGAIN | libc::EINTR)) {
                    return Err(error);
                }
            }
        } else if libc::sigprocmask(
            if block {
                libc::SIG_BLOCK
            } else {
                libc::SIG_UNBLOCK
            },
            &set,
            std::ptr::null_mut(),
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn prepare() -> io::Result<()> {
    child_signals(true, false)
}

pub fn ready() -> io::Result<()> {
    child_signals(false, false)?;
    prevent_untraced_children()?;
    // The trusted adapter stops before executing any producer byte. PID 1 seizes
    // it with inherited fork/clone tracing before releasing this group stop.
    rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::STOP)?;
    Ok(())
}

#[allow(unsafe_code)]
fn prevent_untraced_children() -> io::Result<()> {
    // CLONE_UNTRACED bypasses PTRACE_O_TRACE*. clone3 hides its flags behind a
    // mutable user pointer, so return ENOSYS and require the ordinary clone
    // fallback. Async I/O reports failures through completion queues rather
    // than syscall returns; require synchronous fallback for those APIs too.
    // This inherited filter cannot be removed by a producer.
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc00000b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Err(io::Error::other(
        "syscall observation requires native x86_64 or aarch64 Linux",
    ));
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
        let filter = [
            instruction(0x20, 0, 0, 4),          // BPF_LD W ABS: seccomp_data.arch
            instruction(0x15, 1, 0, arch),       // BPF_JMP JEQ
            instruction(0x06, 0, 0, 0x80000000), // SECCOMP_RET_KILL_PROCESS
            instruction(0x20, 0, 0, 0),          // seccomp_data.nr
            // x32 shares AUDIT_ARCH_X86_64 but has alternate syscall numbers
            // (including io_setup). Reject it rather than masking that ABI bit.
            instruction(0x45, 0, 1, 0x40000000),
            instruction(0x06, 0, 0, 0x80000000),
            instruction(0x15, 0, 1, libc::SYS_io_setup as u32),
            instruction(0x06, 0, 0, 0x00050000 | libc::ENOSYS as u32),
            instruction(0x15, 0, 1, libc::SYS_io_uring_setup as u32),
            instruction(0x06, 0, 0, 0x00050000 | libc::ENOSYS as u32),
            instruction(0x15, 0, 1, libc::SYS_clone3 as u32),
            instruction(0x06, 0, 0, 0x00050000 | libc::ENOSYS as u32),
            instruction(0x15, 0, 3, libc::SYS_clone as u32),
            instruction(0x20, 0, 0, 16), // seccomp_data.args[0], flags
            instruction(0x45, 0, 1, libc::CLONE_UNTRACED as u32), // BPF_JMP JSET
            instruction(0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
            instruction(0x06, 0, 0, 0x7fff0000), // SECCOMP_RET_ALLOW
        ];
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_ptr().cast_mut(),
        };
        // SAFETY: the kernel copies the initialized BPF program synchronously.
        // The adapter is single-threaded and setpriv already set no-new-privs.
        if unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2 as libc::c_ulong, &program) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[allow(unsafe_code)]
fn quota_error(pid: libc::pid_t) -> io::Result<bool> {
    let mut info = [0u8; 88];
    // SAFETY: GET_SYSCALL_INFO writes at most the supplied buffer length and
    // only reads the stopped tracee. The Linux UAPI's exit record has a fixed
    // 24-byte header followed by a native-endian i64 return and u8 error flag.
    let size = unsafe {
        libc::ptrace(
            GET_SYSCALL_INFO as _,
            pid,
            info.len() as *mut libc::c_void,
            info.as_mut_ptr().cast::<libc::c_void>(),
        )
    };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    match info[0] {
        1 if size >= 80 => return Ok(false), // PTRACE_SYSCALL_INFO_ENTRY
        2 if size >= 33 => {}                // PTRACE_SYSCALL_INFO_EXIT
        _ => return Err(io::Error::other("missing or short syscall observation")),
    }
    let result = i64::from_ne_bytes(info[24..32].try_into().unwrap());
    Ok(info[32] != 0
        && [libc::ENOSPC, libc::EDQUOT, libc::EFBIG]
            .iter()
            .any(|error| result == -i64::from(*error)))
}

#[allow(unsafe_code)]
fn wait_one() -> io::Result<Option<(libc::pid_t, i32)>> {
    let mut status = 0;
    // SAFETY: status points to a live integer; PID -1 selects only children of
    // this private PID 1. __WALL includes traced descendants and clone threads.
    let pid = unsafe {
        libc::waitpid(
            -1,
            &mut status,
            libc::WNOHANG | libc::WUNTRACED | libc::__WALL,
        )
    };
    if pid < 0 {
        Err(io::Error::last_os_error())
    } else if pid == 0 {
        Ok(None)
    } else {
        Ok(Some((pid, status)))
    }
}

pub fn watch(leader: libc::pid_t, cancelled: &AtomicBool) -> io::Result<ExecutionReport> {
    let mut traced = BTreeSet::new();
    let mut attached = false;
    let mut exhausted = false;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let (pid, status) = match wait_one() {
            Ok(Some(value)) => value,
            Ok(None) => {
                child_signals(true, true)?;
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            traced.remove(&pid);
            if pid != leader {
                continue;
            }
            if !attached {
                return Err(io::Error::other(
                    "producer exited before syscall observation",
                ));
            }
            if exhausted {
                return Err(io::Error::other(
                    "producer encountered a work-tree quota failure",
                ));
            }
            return Ok(ExecutionReport {
                work_sha256: None,
                exit_code: libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status)),
                signal: libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status)),
                descendants_drained: traced.is_empty(),
            });
        }
        if !libc::WIFSTOPPED(status) {
            return Err(io::Error::other("unrecognized producer wait status"));
        }
        let signal = libc::WSTOPSIG(status);
        let event = status >> 16;
        if !attached {
            if pid != leader || signal != libc::SIGSTOP {
                return Err(io::Error::other(
                    "producer did not stop for syscall observation",
                ));
            }
            let options = libc::PTRACE_O_TRACESYSGOOD
                | libc::PTRACE_O_TRACEFORK
                | libc::PTRACE_O_TRACEVFORK
                | libc::PTRACE_O_TRACECLONE
                | libc::PTRACE_O_TRACEEXEC
                | libc::PTRACE_O_EXITKILL;
            ptrace(libc::PTRACE_SEIZE as _, pid, options as usize)?;
            ptrace(libc::PTRACE_INTERRUPT as _, pid, 0)?;
            traced.insert(pid);
            attached = true;
            // Convert the adapter's initial group stop to normal seized tracing.
            rustix::process::kill_process(
                rustix::process::Pid::from_raw(pid)
                    .ok_or_else(|| io::Error::other("invalid producer PID"))?,
                rustix::process::Signal::CONT,
            )?;
            continue;
        }
        traced.insert(pid);
        if matches!(
            event,
            libc::PTRACE_EVENT_FORK
                | libc::PTRACE_EVENT_VFORK
                | libc::PTRACE_EVENT_CLONE
                | libc::PTRACE_EVENT_EXEC
        ) {
            let mut related: libc::c_ulong = 0;
            ptrace(
                libc::PTRACE_GETEVENTMSG as _,
                pid,
                (&mut related as *mut libc::c_ulong) as usize,
            )?;
            if event == libc::PTRACE_EVENT_EXEC {
                if related as libc::pid_t != pid {
                    traced.remove(&(related as libc::pid_t));
                }
            } else {
                traced.insert(related.try_into().map_err(io::Error::other)?);
            }
        }
        let delivered = if signal == (libc::SIGTRAP | 0x80) {
            exhausted |= quota_error(pid)?;
            0
        } else if event == libc::PTRACE_EVENT_STOP && signal != libc::SIGTRAP {
            // PTRACE_LISTEN preserves job-control stops and wakes on SIGCONT.
            ptrace(libc::PTRACE_LISTEN as _, pid, 0)?;
            continue;
        } else if event != 0 {
            0
        } else {
            // A tmpfs mmap allocation failure can arrive as SIGBUS rather than
            // an errno. It is non-acceptance even if the producer catches it.
            exhausted |= matches!(signal, libc::SIGBUS | libc::SIGXFSZ);
            signal
        };
        ptrace(libc::PTRACE_SYSCALL as _, pid, delivered as usize)?;
    }
}
