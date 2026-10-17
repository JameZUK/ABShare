//! The Landlock part of the jail.

use crate::args::Config;
use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetStatus, Scope,
};
use std::path::Path;

/// The oldest ABI accepted: 6 adds signal and abstract-socket scoping (Linux 6.12).
/// Anything older is refused rather than half-applied.
const MIN_ABI: ABI = ABI::V6;

fn read_rights(path: &Path) -> BitFlags<AccessFs> {
    if path.is_dir() { AccessFs::ReadFile | AccessFs::ReadDir } else { AccessFs::ReadFile.into() }
}

fn lib_rights(path: &Path) -> BitFlags<AccessFs> {
    read_rights(path) | AccessFs::Execute
}

fn write_rights() -> BitFlags<AccessFs> {
    AccessFs::ReadFile
        | AccessFs::ReadDir
        | AccessFs::WriteFile
        | AccessFs::MakeReg
        | AccessFs::RemoveFile
        | AccessFs::Truncate
}

fn rule(path: &Path, rights: BitFlags<AccessFs>) -> Result<PathBeneath<PathFd>, String> {
    let fd = PathFd::new(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    Ok(PathBeneath::new(fd, rights))
}

pub fn apply(c: &Config) -> Result<(), String> {
    let err = |e: landlock::RulesetError| format!("landlock: {e} (is `landlock` in the kernel's lsm= list?)");
    let mut rules = Vec::new();
    for p in &c.read {
        rules.push(rule(p, read_rights(p))?);
    }
    for p in &c.lib {
        rules.push(rule(p, lib_rights(p))?);
    }
    for p in &c.write {
        if !p.is_dir() {
            return Err(format!("--write must be a directory: {}", p.display()));
        }
        rules.push(rule(p, write_rights())?);
    }
    rules.push(rule(&c.program, AccessFs::ReadFile | AccessFs::Execute)?);

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
