//! Where anneal keeps state: the ledger store (normally in the git common dir) and
//! `~/.anneal` (decision log).

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long a `git` call may take before it is killed (H-13).
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a running `git` call is checked for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

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
fn git_failure(dir: &Path, stderr: &[u8]) -> PathError {
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
    let mut child = Command::new("git")
        .args(args)
        .env("LC_ALL", "C")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(PathError::Git)?;
    wait_with_deadline(&mut child, timeout)
}

/// Waits for `child`, draining its piped stdout and stderr on named scoped threads so a chatty
/// child cannot block on a full pipe. Each stream keeps at most [`MAX_CAPTURE_BYTES`]; the rest
/// is read and dropped. Past `timeout` the child is killed and reaped, and the error is
/// [`PathError::GitTimeout`]. Every reader thread is joined before this returns.
///
/// Limit: the readers finish only when every holder of the pipes' write ends has exited. If
/// killing the child fails, or the child started a grandchild that inherited its stdout or
/// stderr, joining the readers blocks until those processes exit, past `timeout`. Std offers
/// no way to abandon a blocked read without detaching the thread (H-13), so this stays a
/// documented limit: use it only for commands, such as `git rev-parse`, that start no
/// long-lived processes.
fn wait_with_deadline(child: &mut Child, timeout: Duration) -> Result<Output, PathError> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    thread::scope(|scope| {
        let out_reader = match spawn_reader(scope, "git-stdout", stdout) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let err_reader = match spawn_reader(scope, "git-stderr", stderr) {
            Ok(handle) => handle,
            Err(error) => return Err(kill_after(child, error)),
        };
        let status = wait_or_kill(child, timeout)?;
        Ok(Output {
            status,
            stdout: join_reader(out_reader)?,
            stderr: join_reader(err_reader)?,
        })
    })
}

/// Most bytes kept from each of a child's output streams (H-13).
const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

type Reader<'scope> = ScopedJoinHandle<'scope, io::Result<Vec<u8>>>;

fn spawn_reader<'scope, R: Read + Send + 'scope>(
    scope: &'scope Scope<'scope, '_>,
    name: &str,
    pipe: Option<R>,
) -> io::Result<Reader<'scope>> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn_scoped(scope, move || match pipe {
            Some(pipe) => read_capped(pipe, MAX_CAPTURE_BYTES),
            None => Ok(Vec::new()),
        })
}

/// Reads `pipe` to its end, keeping the first `cap` bytes and discarding the rest, so the
/// writer never blocks on a full pipe and memory stays bounded.
fn read_capped<R: Read>(mut pipe: R, cap: u64) -> io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    pipe.by_ref().take(cap).read_to_end(&mut kept)?;
    io::copy(&mut pipe, &mut io::sink())?;
    Ok(kept)
}

fn join_reader(reader: Reader<'_>) -> Result<Vec<u8>, PathError> {
    reader
        .join()
        .map_err(|_panicked| PathError::Git(io::Error::other("an output reader thread panicked")))?
        .map_err(PathError::Git)
}

/// Polls `child` until it exits; past `timeout` kills and reaps it. The deadline is checked
/// before each poll, so an exit first seen after the deadline is a timeout, and a zero
/// timeout always is.
fn wait_or_kill(child: &mut Child, timeout: Duration) -> Result<ExitStatus, PathError> {
    let started = Instant::now();
    loop {
        if started.elapsed() >= timeout {
            child.kill().map_err(PathError::Git)?;
            child.wait().map_err(PathError::Git)?;
            return Err(PathError::GitTimeout { timeout });
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            // Kill it: the reader threads would otherwise wait on its pipes forever.
            Err(error) => return Err(kill_after(child, error)),
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Kills and reaps `child` after a setup failure, so no reader waits on a live pipe. A failure
/// to kill is reported alongside the original error rather than dropped.
fn kill_after(child: &mut Child, error: io::Error) -> PathError {
    match child.kill().and_then(|()| child.wait().map(drop)) {
        Ok(()) => PathError::Git(error),
        Err(kill_error) => PathError::Git(io::Error::new(
            error.kind(),
            format!("{error}; killing git also failed: {kill_error}"),
        )),
    }
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
