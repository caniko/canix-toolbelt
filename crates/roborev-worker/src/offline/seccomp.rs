//! Lock the final envelope after trusted mount setup. Namespace-changing syscall
//! arguments cannot be rewritten through a pointer race: legacy clone flags are
//! scalar, and clone3 returns ENOSYS so libc may use the filtered legacy call.
use anyhow::{Result, ensure};

// The classic BPF installation ABI requires a pointer. This private function
// constructs the entire bounded program itself; callers cannot supply filters.
#[allow(unsafe_code)]
pub(super) fn install() -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc000_00b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("offline worker seccomp is qualified only for x86_64 and aarch64");

    const LD: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const JEQ: u16 = 0x15;
    const JGE: u16 = 0x35;
    const JSET: u16 = 0x45;
    const RET: u16 = 0x06;
    const ALLOW: u32 = 0x7fff_0000;
    const ERRNO: u32 = 0x0005_0000;
    let op = |code, jt, jf, k| nix::libc::sock_filter { code, jt, jf, k };
    let mut program = vec![
        op(LD, 0, 0, 4),
        op(JEQ, 1, 0, ARCH),
        op(RET, 0, 0, 0x8000_0000),
        op(LD, 0, 0, 0),
        op(JGE, 0, 1, 0x4000_0000),
        op(RET, 0, 0, ERRNO | nix::libc::ENOSYS as u32),
    ];
    for syscall in [
        nix::libc::SYS_unshare,
        nix::libc::SYS_setns,
        nix::libc::SYS_mount,
        nix::libc::SYS_umount2,
        nix::libc::SYS_fsopen,
        nix::libc::SYS_fsmount,
        nix::libc::SYS_fspick,
        nix::libc::SYS_move_mount,
        nix::libc::SYS_open_tree,
        nix::libc::SYS_mount_setattr,
        nix::libc::SYS_pivot_root,
        nix::libc::SYS_chroot,
        nix::libc::SYS_bpf,
        nix::libc::SYS_ptrace,
        nix::libc::SYS_process_vm_readv,
        nix::libc::SYS_process_vm_writev,
        nix::libc::SYS_userfaultfd,
    ] {
        program.push(op(JEQ, 0, 1, syscall as u32));
        program.push(op(RET, 0, 0, ERRNO | nix::libc::EPERM as u32));
    }
    program.extend([
        op(JEQ, 0, 1, nix::libc::SYS_clone3 as u32),
        op(RET, 0, 0, ERRNO | nix::libc::ENOSYS as u32),
    ]);
    let namespaces = nix::libc::CLONE_NEWUSER
        | nix::libc::CLONE_NEWNS
        | nix::libc::CLONE_NEWNET
        | nix::libc::CLONE_NEWIPC
        | nix::libc::CLONE_NEWUTS
        | nix::libc::CLONE_NEWPID
        | nix::libc::CLONE_NEWCGROUP;
    program.extend([
        op(JEQ, 0, 3, nix::libc::SYS_clone as u32),
        op(LD, 0, 0, 16),
        op(JSET, 0, 1, namespaces as u32),
        op(RET, 0, 0, ERRNO | nix::libc::EPERM as u32),
        op(RET, 0, 0, ALLOW),
    ]);
    let filter = nix::libc::sock_fprog {
        len: program.len().try_into()?,
        filter: program.as_mut_ptr(),
    };
    // SAFETY: bounded valid classic BPF records, all buffers live while the
    // kernel copies them. NNP is already set, and filters cannot be removed.
    ensure!(
        unsafe { nix::libc::prctl(nix::libc::PR_SET_SECCOMP, 2, &filter, 0, 0) } == 0,
        "cannot install worker syscall confinement: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
