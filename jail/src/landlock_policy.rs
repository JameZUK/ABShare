//! The Landlock part of the jail.

use crate::args::Config;
use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetStatus, Scope,
};
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags};
use std::os::fd::OwnedFd;
use std::path::Path;

/// The oldest ABI accepted: 6 adds signal and abstract-socket scoping (Linux 6.12).
/// Anything older is refused rather than half-applied.
const MIN_ABI: ABI = ABI::V6;

fn read_rights(dir: bool) -> BitFlags<AccessFs> {
    if dir { AccessFs::ReadFile | AccessFs::ReadDir } else { AccessFs::ReadFile.into() }
}

fn write_rights() -> BitFlags<AccessFs> {
    AccessFs::ReadFile
        | AccessFs::ReadDir
        | AccessFs::WriteFile
        | AccessFs::MakeReg
        | AccessFs::RemoveFile
        | AccessFs::Truncate
}

/// Opens a path for a rule. `strict` (the inputs, from the library): no symbolic link
/// anywhere in it, so a path swapped for a link after the server checked it can't widen
/// what ffmpeg may read (review M1). Library directories (`--lib`) may contain links
/// (`/lib64`), and the job directory is in ABShare's own private data directory (under
/// systemd's DynamicUser that path is itself a link), so they're opened normally.
fn open(path: &Path, strict: bool) -> Result<(OwnedFd, bool), String> {
    let flags = OFlags::PATH | OFlags::CLOEXEC;
    let fd = if strict {
        let how = ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS;
        rustix::fs::openat2(CWD, path, flags | OFlags::NOFOLLOW, Mode::empty(), how)
    } else {
        rustix::fs::open(path, flags, Mode::empty())
    }
    .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let dir =
        rustix::fs::fstat(&fd).map(|st| rustix::fs::FileType::from_raw_mode(st.st_mode).is_dir()).unwrap_or(false);
    Ok((fd, dir))
}

fn rule(fd: OwnedFd, rights: BitFlags<AccessFs>) -> PathBeneath<OwnedFd> {
    PathBeneath::new(fd, rights)
}

pub fn apply(c: &Config) -> Result<(), String> {
    let err = |e: landlock::RulesetError| format!("landlock: {e} (is `landlock` in the kernel's lsm= list?)");
    let mut rules = Vec::new();
    for p in &c.read {
        let (fd, dir) = open(p, true)?;
        rules.push(rule(fd, read_rights(dir)));
    }
    for p in &c.lib {
        let (fd, dir) = open(p, false)?;
        rules.push(rule(fd, read_rights(dir) | AccessFs::Execute));
    }
    for p in &c.write {
        let (fd, dir) = open(p, false)?;
        if !dir {
            return Err(format!("--write must be a directory: {}", p.display()));
        }
        rules.push(rule(fd, write_rights()));
    }
    let (fd, _) = open(&c.program, false)?;
    rules.push(rule(fd, AccessFs::ReadFile | AccessFs::Execute));

    let status = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(MIN_ABI))
        .map_err(err)?
        .handle_access(AccessNet::BindTcp | AccessNet::ConnectTcp)
        .map_err(err)?
        .scope(Scope::AbstractUnixSocket | Scope::Signal)
        .map_err(err)?
        .create()
        .map_err(err)?
        .no_new_privs(true)
        .add_rules(rules.into_iter().map(Ok::<_, landlock::RulesetError>))
        .map_err(err)?
        .restrict_self()
        .map_err(err)?;
    if status.ruleset != RulesetStatus::FullyEnforced || !status.no_new_privs {
        return Err("landlock: ruleset not fully enforced".into());
    }
    Ok(())
}
