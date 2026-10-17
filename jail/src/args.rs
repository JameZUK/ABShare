//! Command-line parsing for `abshare-jail`. Pure: no I/O.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "usage: abshare-jail [--read PATH]... [--lib PATH]... [--write DIR]... \
[--mem-mib N] [--cpu-secs N] [--fsize-mib N] [--nofile N] -- /abs/path/program [ARGS]...";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Files or directories the program may read (no execute).
    pub read: Vec<PathBuf>,
    /// Directories or files the program may read and map as code (shared libraries).
    pub lib: Vec<PathBuf>,
    /// Directories the program may create, write, truncate and remove files in (no execute).
    pub write: Vec<PathBuf>,
    pub mem_mib: u64,
    pub cpu_secs: u64,
    pub fsize_mib: u64,
    pub nofile: u64,
    /// Absolute path of the program; the only file it may execute.
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    MissingProgram,
    RelativePath(PathBuf),
    UnknownOption(String),
    MissingValue(&'static str),
    BadNumber(&'static str),
    OutOfRange(&'static str),
    ProgramInWritableDir,
    NotUtf8Option,
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingProgram => f.write_str("missing `-- PROGRAM`"),
            Self::RelativePath(p) => write!(f, "path must be absolute: {}", p.display()),
            Self::UnknownOption(o) => write!(f, "unknown option {o}"),
            Self::MissingValue(o) => write!(f, "{o} needs a value"),
            Self::BadNumber(o) => write!(f, "{o} needs a whole number"),
            Self::OutOfRange(o) => write!(f, "{o} is out of range"),
            Self::ProgramInWritableDir => f.write_str("the program may not live under a --write directory"),
            Self::NotUtf8Option => f.write_str("options must be UTF-8"),
        }
    }
}

const fn bounds(name: &str) -> (u64, u64) {
    match name.as_bytes() {
        b"--mem-mib" => (16, 16_384),
        b"--cpu-secs" => (1, 86_400),
        b"--fsize-mib" => (1, 65_536),
        _ => (8, 4_096), // --nofile
    }
}

pub fn parse(argv: impl IntoIterator<Item = OsString>) -> Result<Config, ArgError> {
    let mut it = argv.into_iter().skip(1);
    let mut c = Config {
        read: Vec::new(),
        lib: Vec::new(),
        write: Vec::new(),
        mem_mib: 1_024,
        cpu_secs: 3_600,
        fsize_mib: 4_096,
        nofile: 64,
        program: PathBuf::new(),
        args: Vec::new(),
    };
    loop {
        let Some(arg) = it.next() else { return Err(ArgError::MissingProgram) };
        let opt = arg.to_str().ok_or(ArgError::NotUtf8Option)?;
        match opt {
            "--" => break,
            "--read" | "--lib" | "--write" => {
                let name: &'static str = match opt {
                    "--read" => "--read",
                    "--lib" => "--lib",
                    _ => "--write",
                };
                let p = PathBuf::from(it.next().ok_or(ArgError::MissingValue(name))?);
                if !p.is_absolute() {
                    return Err(ArgError::RelativePath(p));
                }
                match name {
                    "--read" => c.read.push(p),
                    "--lib" => c.lib.push(p),
                    _ => c.write.push(p),
                }
            }
            "--mem-mib" | "--cpu-secs" | "--fsize-mib" | "--nofile" => {
                let name: &'static str = match opt {
                    "--mem-mib" => "--mem-mib",
                    "--cpu-secs" => "--cpu-secs",
                    "--fsize-mib" => "--fsize-mib",
                    _ => "--nofile",
                };
                let v = it.next().ok_or(ArgError::MissingValue(name))?;
                let n: u64 = v.to_str().and_then(|s| s.parse().ok()).ok_or(ArgError::BadNumber(name))?;
                let (lo, hi) = bounds(name);
                if !(lo..=hi).contains(&n) {
                    return Err(ArgError::OutOfRange(name));
                }
                match name {
                    "--mem-mib" => c.mem_mib = n,
                    "--cpu-secs" => c.cpu_secs = n,
                    "--fsize-mib" => c.fsize_mib = n,
                    _ => c.nofile = n,
                }
            }
            other => return Err(ArgError::UnknownOption(other.to_owned())),
        }
    }
    let program = PathBuf::from(it.next().ok_or(ArgError::MissingProgram)?);
    if !program.is_absolute() {
        return Err(ArgError::RelativePath(program));
    }
    if c.write.iter().any(|w| program.starts_with(w)) {
        return Err(ArgError::ProgramInWritableDir);
    }
    c.program = program;
    c.args = it.collect();
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Config, ArgError> {
        parse(std::iter::once("abshare-jail").chain(args.iter().copied()).map(OsString::from))
    }

    #[test]
    fn a_full_command_line_parses() {
        let c = p(&["--read", "/in/a.m4b", "--write", "/job", "--mem-mib", "512", "--", "/usr/bin/ffmpeg", "-i", "x"])
            .expect("parse");
        assert_eq!(c.read, [PathBuf::from("/in/a.m4b")]);
        assert_eq!(c.write, [PathBuf::from("/job")]);
        assert_eq!(c.mem_mib, 512);
        assert_eq!(c.program, PathBuf::from("/usr/bin/ffmpeg"));
        assert_eq!(c.args, [OsString::from("-i"), OsString::from("x")]);
    }

    #[test]
    fn without_a_separator_nothing_runs() {
        assert_eq!(p(&["--read", "/in"]), Err(ArgError::MissingProgram));
        assert_eq!(p(&["--"]), Err(ArgError::MissingProgram));
        assert_eq!(p(&[]), Err(ArgError::MissingProgram));
    }

    #[test]
    fn relative_paths_are_refused() {
        assert!(matches!(p(&["--read", "in", "--", "/bin/x"]), Err(ArgError::RelativePath(_))));
        assert!(matches!(p(&["--", "ffmpeg"]), Err(ArgError::RelativePath(_))));
    }

    #[test]
    fn a_program_inside_a_writable_directory_is_refused() {
        assert_eq!(p(&["--write", "/job", "--", "/job/ffmpeg"]), Err(ArgError::ProgramInWritableDir));
    }

    #[test]
    fn unknown_options_and_bad_numbers_are_refused() {
        assert!(matches!(p(&["--net", "--", "/bin/x"]), Err(ArgError::UnknownOption(_))));
        assert_eq!(p(&["--mem-mib", "lots", "--", "/bin/x"]), Err(ArgError::BadNumber("--mem-mib")));
        assert_eq!(p(&["--mem-mib", "0", "--", "/bin/x"]), Err(ArgError::OutOfRange("--mem-mib")));
        assert_eq!(p(&["--nofile", "100000", "--", "/bin/x"]), Err(ArgError::OutOfRange("--nofile")));
        assert_eq!(p(&["--cpu-secs"]), Err(ArgError::MissingValue("--cpu-secs")));
    }

    #[test]
    fn options_after_the_separator_belong_to_the_program() {
        let c = p(&["--", "/bin/x", "--write", "/etc"]).expect("parse");
        assert!(c.write.is_empty());
        assert_eq!(c.args.len(), 2);
    }
}
