//! Where anneal keeps state: the ledger store (normally in the git common dir) and
//! `~/.anneal` (decision log).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use serde::Serialize;

#[cfg(test)]
use crate::process::read_capped;
use crate::process::{self, WaitError};

/// How long a `git` call may take before it is killed (H-13).
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// The environment variable that overrides the anneal home directory.
pub const ANNEAL_HOME_VAR: &str = "ANNEAL_HOME";

/// The environment variable that overrides the ledger store's file.
pub const ANNEAL_STORE_VAR: &str = "ANNEAL_STORE";

/// The ledger file's name inside its directory.
const LEDGER_FILE: &str = "ledger.sqlite3";

/// Why a path could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    /// `git` could not be started or waited on.
    #[error("running git")]
    Git(#[source] io::Error),
    /// `git` did not finish within the timeout and was killed.
    #[error("git did not answer within {}s", timeout.as_secs())]
    GitTimeout {
        /// The limit that was exceeded.
        timeout: Duration,
    },
    /// `dir` is not inside a git repository.
    #[error("{dir} is not in a git repository: {stderr}", dir = dir.display())]
    NotARepository {
        /// The directory git was run in.
        dir: PathBuf,
        /// What git said, trimmed.
        stderr: String,
    },
    /// git failed in `dir` for a reason other than `dir` not being in a repository, such as a
    /// repository owned by someone else ("dubious ownership") or a broken `.git` file.
    #[error("git failed in {dir}: {stderr}", dir = dir.display())]
    GitFailed {
        /// The directory git was run in.
        dir: PathBuf,
        /// What git said, trimmed.
        stderr: String,
    },
    /// git printed a path that is not UTF-8.
    #[error("git printed a path that is not UTF-8")]
    NonUtf8Output(#[source] std::string::FromUtf8Error),
    /// None of `ANNEAL_HOME`, `HOME` or `USERPROFILE` is set.
    #[error("cannot find a home directory: set ANNEAL_HOME, HOME or USERPROFILE")]
    NoHome,
    /// The variable holds a relative path, which would move with the working directory.
    #[error("{var} must be an absolute path, got {value}", value = value.display())]
    RelativeHome {
        /// The variable that was read.
        var: &'static str,
        /// Its value.
        value: PathBuf,
    },
}

/// The git common dir for `cwd`: the main repository's `.git` directory, shared by every linked
/// worktree, and so the one place a ledger is seen from all of them.
///
/// Runs `git rev-parse --git-common-dir` in `cwd` and resolves a relative answer against it.
///
/// # Errors
/// [`PathError::NotARepository`] outside a repository; [`PathError::GitFailed`] when git fails
/// for another reason; [`PathError::Git`] or [`PathError::GitTimeout`] when git cannot run;
/// [`PathError::NonUtf8Output`] for a path that is not UTF-8.
pub fn git_common_dir(cwd: &Path) -> Result<PathBuf, PathError> {
    git_common_dir_within(cwd, GIT_TIMEOUT)
}

/// [`git_common_dir`] with git killed after `timeout`.
fn git_common_dir_within(cwd: &Path, timeout: Duration) -> Result<PathBuf, PathError> {
    let output = run_git(cwd, &["rev-parse", "--git-common-dir"], timeout)?;
    if !output.status.success() {
        return Err(git_failure(cwd, &output.stderr));
    }
    let printed = String::from_utf8(output.stdout).map_err(PathError::NonUtf8Output)?;
    Ok(cwd.join(printed.trim_end_matches(['\r', '\n'])))
}

/// What git says (untranslated: [`run_git`] runs it under `LC_ALL=C`) when a directory is in
/// no repository.
const NOT_A_REPOSITORY: &str = "not a git repository";

/// The error for a git call in `dir` that exited non-zero, from what it wrote on stderr:
/// [`PathError::NotARepository`] only when git says so, [`PathError::GitFailed`] otherwise.
pub(crate) fn git_failure(dir: &Path, stderr: &[u8]) -> PathError {
    let dir = dir.to_path_buf();
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    if stderr.contains(NOT_A_REPOSITORY) {
        PathError::NotARepository { dir, stderr }
    } else {
        PathError::GitFailed { dir, stderr }
    }
}

/// Runs git with a deadline, killing it if the deadline passes. Its messages are not
/// translated, so [`git_failure`] can read them.
fn run_git(cwd: &Path, args: &[&str], timeout: Duration) -> Result<Output, PathError> {
    run_git_capped(cwd, args, timeout, MAX_CAPTURE_BYTES)
}

/// [`run_git`] keeping at most `cap` bytes of each output stream (the rest is read and
/// dropped). A caller that must not read a truncated answer passes one byte more than it
/// accepts and checks the length.
///
/// # Errors
/// [`PathError::Git`] when git cannot be started or waited on, [`PathError::GitTimeout`] past
/// `timeout`. A non-zero exit is not an error here: see [`git_failure`].
pub(crate) fn run_git_capped(
    cwd: &Path,
    args: &[&str],
    timeout: Duration,
    cap: u64,
) -> Result<Output, PathError> {
    let mut child = Command::new("git")
        .args(args)
        .env("LC_ALL", "C")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(PathError::Git)?;
    wait_capped(&mut child, timeout, cap)
}

/// Most bytes kept from each of git's output streams (H-13).
const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

/// [`process::wait_with_deadline`] for git: at most [`MAX_CAPTURE_BYTES`] per stream, and a
/// timeout is [`PathError::GitTimeout`].
#[cfg(test)]
fn wait_with_deadline(child: &mut Child, timeout: Duration) -> Result<Output, PathError> {
    wait_capped(child, timeout, MAX_CAPTURE_BYTES)
}

/// [`process::wait_with_deadline`] for git: at most `cap` bytes per stream, and a timeout is
/// [`PathError::GitTimeout`].
fn wait_capped(child: &mut Child, timeout: Duration, cap: u64) -> Result<Output, PathError> {
    process::wait_with_deadline(child, timeout, cap, "git").map_err(|error| match error {
        WaitError::Io(source) => PathError::Git(source),
        WaitError::Timeout => PathError::GitTimeout { timeout },
    })
}

/// Where the ledger store was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreSource {
    /// `--store <file>`.
    Flag,
    /// `$ANNEAL_STORE`.
    Env,
    /// `<git-common-dir>/anneal/ledger.sqlite3`.
    GitCommonDir,
    /// `<folder>/.anneal/ledger.sqlite3`, outside git.
    Folder,
}

/// The ledger store's file and how it was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoreLocation {
    /// The ledger file.
    pub path: PathBuf,
    /// Which step of the resolution order chose it.
    pub source: StoreSource,
}

/// The ledger store for a command run in `cwd`, in this order:
///
/// 1. `flag` (`--store <file>`), relative to `cwd`;
/// 2. `$ANNEAL_STORE` (read through `lookup`; empty counts as unset), relative to `cwd`;
/// 3. the anneal config file, once there is one (there is none yet, so this step is skipped);
/// 4. `<git-common-dir>/anneal/ledger.sqlite3`, the same file from every worktree;
/// 5. `<cwd>/.anneal/ledger.sqlite3` outside git.
///
/// # Errors
/// [`PathError::Git`], [`PathError::GitTimeout`], [`PathError::GitFailed`] or
/// [`PathError::NonUtf8Output`] when git cannot answer. Only git saying `cwd` is not in a
/// repository is step 5; any other git failure (such as "dubious ownership") is an error, so a
/// repository git refuses never gets a second, per-folder ledger.
pub fn ledger_store(
    flag: Option<&Path>,
    lookup: impl Fn(&str) -> Option<OsString>,
    cwd: &Path,
) -> Result<StoreLocation, PathError> {
    ledger_store_within(flag, lookup, cwd, GIT_TIMEOUT)
}

/// [`ledger_store`], with git given `git_timeout` instead of [`GIT_TIMEOUT`]: for callers with a
/// deadline, such as a hook.
///
/// # Errors
/// As [`ledger_store`]; [`PathError::GitTimeout`] carries `git_timeout`.
pub fn ledger_store_within(
    flag: Option<&Path>,
    lookup: impl Fn(&str) -> Option<OsString>,
    cwd: &Path,
    git_timeout: Duration,
) -> Result<StoreLocation, PathError> {
    let located = |path: PathBuf, source| StoreLocation { path, source };
    if let Some(path) = flag {
        return Ok(located(cwd.join(path), StoreSource::Flag));
    }
    if let Some(path) = lookup(ANNEAL_STORE_VAR).filter(|value| !value.is_empty()) {
        return Ok(located(cwd.join(path), StoreSource::Env));
    }
    match git_common_dir_within(cwd, git_timeout) {
        Ok(common) => Ok(located(
            common.join("anneal").join(LEDGER_FILE),
            StoreSource::GitCommonDir,
        )),
        Err(PathError::NotARepository { .. }) => Ok(located(
            cwd.join(".anneal").join(LEDGER_FILE),
            StoreSource::Folder,
        )),
        Err(other) => Err(other),
    }
}

/// The anneal home directory from the process environment; see [`anneal_home_from`].
///
/// # Errors
/// As [`anneal_home_from`].
pub fn anneal_home() -> Result<PathBuf, PathError> {
    anneal_home_from(|var| std::env::var_os(var))
}

/// The anneal home directory: `$ANNEAL_HOME` if set, else `$HOME/.anneal`, else
/// `%USERPROFILE%\.anneal`. Empty values count as unset. `lookup` reads one variable.
///
/// # Errors
/// [`PathError::NoHome`] when none is set; [`PathError::RelativeHome`] when the chosen value is
/// not absolute.
pub fn anneal_home_from(lookup: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, PathError> {
    let read = |var: &'static str| {
        lookup(var)
            .filter(|value| !value.is_empty())
            .map(|value| (var, PathBuf::from(value)))
    };
    let (var, path) = if let Some((var, home)) = read(ANNEAL_HOME_VAR) {
        (var, home)
    } else {
        let (var, home) = read("HOME")
            .or_else(|| read("USERPROFILE"))
            .ok_or(PathError::NoHome)?;
        (var, home.join(".anneal"))
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(PathError::RelativeHome { var, value: path })
    }
}

#[cfg(test)]
mod tests;
