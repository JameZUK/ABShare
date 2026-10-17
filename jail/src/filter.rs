//! The seccomp part of the jail.
//!
//! Three filters are stacked:
//! 1. a hand-written guard that kills the process on any system call from another ABI: a
//!    foreign architecture, or x86-64's x32 numbers (bit 0x40000000), which the rule-based
//!    filters below would otherwise never match (review J1);
//! 2. `clone3` fails with ENOSYS, so the C library falls back to `clone`, whose flags a
//!    filter can see;
//! 3. the system calls a media decoder never needs and an attacker would want are refused
//!    with EPERM: among them every socket, and every new process (threads only).
//!
//! This is a deny-list. An allow-list depends on the exact ffmpeg build, so it comes when
//! ABShare's own minimal ffmpeg is fixed (Phase 1); Landlock already confines files,
//! execution, TCP and signals.

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen as Len, SeccompCmpOp as Op, SeccompCondition as Cond, SeccompFilter,
    SeccompRule, TargetArch,
};
use std::collections::BTreeMap;

#[cfg(target_arch = "x86_64")]
const ARCH: TargetArch = TargetArch::x86_64;
#[cfg(target_arch = "aarch64")]
const ARCH: TargetArch = TargetArch::aarch64;

/// Refused outright.
const DENIED: &[i64] = &[
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_chroot,
    libc::SYS_unshare,
    libc::SYS_setns,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    libc::SYS_keyctl,
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_memfd_create,
    libc::SYS_execveat,
    libc::SYS_personality,
    libc::SYS_acct,
    libc::SYS_swapon,
    libc::SYS_swapoff,
    libc::SYS_reboot,
    libc::SYS_settimeofday,
    libc::SYS_clock_settime,
    libc::SYS_clock_adjtime,
    libc::SYS_adjtimex,
    libc::SYS_quotactl,
    libc::SYS_name_to_handle_at,
    libc::SYS_open_by_handle_at,
    libc::SYS_open_tree,
    libc::SYS_move_mount,
    libc::SYS_fsopen,
    libc::SYS_fsconfig,
    libc::SYS_fsmount,
    libc::SYS_fspick,
    libc::SYS_mount_setattr,
    // No sockets of any family (ffmpeg reads files only): nothing can reach a UNIX socket on
    // the host, such as a user D-Bus or a container runtime (review J2).
    libc::SYS_socket,
    libc::SYS_socketpair,
    libc::SYS_connect,
    // No new processes: a child could outlive the wall-clock kill (review J6).
    libc::SYS_fork,
    libc::SYS_vfork,
];

/// Namespace flags `clone` may not carry.
const CLONE_NS_FLAGS: &[u64] = &[
    libc::CLONE_NEWUSER as u64,
    libc::CLONE_NEWNS as u64,
    libc::CLONE_NEWNET as u64,
    libc::CLONE_NEWPID as u64,
    libc::CLONE_NEWIPC as u64,
    libc::CLONE_NEWUTS as u64,
    libc::CLONE_NEWCGROUP as u64,
];

pub struct Filters {
    abi: BpfProgram,
    clone3: BpfProgram,
    deny: BpfProgram,
}

fn cond(arg: u8, len: Len, op: Op, value: u64) -> Result<Cond, String> {
    Cond::new(arg, len, op, value).map_err(|e| format!("seccomp: {e}"))
}

fn rule(conds: Vec<Cond>) -> Result<SeccompRule, String> {
    SeccompRule::new(conds).map_err(|e| format!("seccomp: {e}"))
}

fn build(rules: BTreeMap<i64, Vec<SeccompRule>>, action: SeccompAction) -> Result<BpfProgram, String> {
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, action, ARCH).map_err(|e| format!("seccomp: {e}"))?;
    filter.try_into().map_err(|e: seccompiler::BackendError| format!("seccomp: {e}"))
}

pub fn compile() -> Result<Filters, String> {
    let clone3 = build(BTreeMap::from([(libc::SYS_clone3, vec![])]), SeccompAction::Errno(libc::ENOSYS as u32))?;

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = DENIED.iter().map(|&nr| (nr, vec![])).collect();
    // clone(flags, ...) carrying any namespace flag, or making a process rather than a thread.
    let thread = libc::CLONE_THREAD as u64;
    let mut clone_rules: Vec<SeccompRule> = CLONE_NS_FLAGS
        .iter()
        .map(|&f| rule(vec![cond(0, Len::Qword, Op::MaskedEq(f), f)?]))
        .collect::<Result<_, _>>()?;
    clone_rules.push(rule(vec![cond(0, Len::Qword, Op::MaskedEq(thread), 0)?])?);
    rules.insert(libc::SYS_clone, clone_rules);
    let deny = build(rules, SeccompAction::Errno(libc::EPERM as u32))?;
    Ok(Filters { abi: abi_guard(), clone3, deny })
}

/// Kills the process on a system call from any ABI but the native one. Raw BPF:
/// `arch == native && nr < 0x40000000 ? ALLOW : KILL_PROCESS`.
fn abi_guard() -> BpfProgram {
    use seccompiler::sock_filter;
    const LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const JGE_K: u16 = 0x35; // BPF_JMP | BPF_JGE | BPF_K
    const RET_K: u16 = 0x06; // BPF_RET | BPF_K
    const KILL_PROCESS: u32 = 0x8000_0000;
    const ALLOW: u32 = 0x7fff_0000;
    const X32_BIT: u32 = 0x4000_0000;
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;
    let i = |code, jt, jf, k| sock_filter { code, jt, jf, k };
    vec![
        i(LD_W_ABS, 0, 0, 4),         // A = seccomp_data.arch
        i(JEQ_K, 1, 0, AUDIT_ARCH),   // native? skip the kill
        i(RET_K, 0, 0, KILL_PROCESS), //
        i(LD_W_ABS, 0, 0, 0),         // A = seccomp_data.nr
        i(JGE_K, 0, 1, X32_BIT),      // x32 number? kill : allow
        i(RET_K, 0, 0, KILL_PROCESS), //
        i(RET_K, 0, 0, ALLOW),        //
    ]
}

pub fn apply(f: &Filters) -> Result<(), String> {
    seccompiler::apply_filter(&f.abi).map_err(|e| format!("seccomp: {e}"))?;
    seccompiler::apply_filter(&f.clone3).map_err(|e| format!("seccomp: {e}"))?;
    seccompiler::apply_filter(&f.deny).map_err(|e| format!("seccomp: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_filters_compile_for_this_architecture() {
        assert!(super::compile().is_ok());
    }
}
