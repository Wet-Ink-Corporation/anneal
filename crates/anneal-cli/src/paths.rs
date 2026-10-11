//! Where anneal keeps state: the git common dir (ledger home) and `~/.anneal` (decision log).

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

/// How long a `git` call may take before it is killed (H-13).
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a running `git` call is checked for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The environment variable that overrides the anneal home directory.
pub const ANNEAL_HOME_VAR: &str = "ANNEAL_HOME";

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
/// [`PathError::NotARepository`] outside a repository; [`PathError::Git`] or
/// [`PathError::GitTimeout`] when git cannot run; [`PathError::NonUtf8Output`] for a path
/// that is not UTF-8.
pub fn git_common_dir(cwd: &Path) -> Result<PathBuf, PathError> {
    let output = run_git(cwd, &["rev-parse", "--git-common-dir"], GIT_TIMEOUT)?;
    if !output.status.success() {
        return Err(PathError::NotARepository {
            dir: cwd.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let printed = String::from_utf8(output.stdout).map_err(PathError::NonUtf8Output)?;
    Ok(cwd.join(printed.trim_end_matches(['\r', '\n'])))
}

/// Runs git with a deadline, killing it if the deadline passes.
fn run_git(cwd: &Path, args: &[&str], timeout: Duration) -> Result<Output, PathError> {
    let mut child = Command::new("git")
        .args(args)
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
        .spawn_scoped(scope, move || {
            let mut kept = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.by_ref()
                    .take(MAX_CAPTURE_BYTES)
                    .read_to_end(&mut kept)?;
                io::copy(&mut pipe, &mut io::sink())?;
            }
            Ok(kept)
        })
}

fn join_reader(reader: Reader<'_>) -> Result<Vec<u8>, PathError> {
    reader
        .join()
        .map_err(|_panicked| PathError::Git(io::Error::other("an output reader thread panicked")))?
        .map_err(PathError::Git)
}

/// Polls `child` until it exits; past `timeout` kills and reaps it.
fn wait_or_kill(child: &mut Child, timeout: Duration) -> Result<ExitStatus, PathError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            // Kill it: the reader threads would otherwise wait on its pipes forever.
            Err(error) => return Err(kill_after(child, error)),
        }
        if started.elapsed() >= timeout {
            child.kill().map_err(PathError::Git)?;
            child.wait().map_err(PathError::Git)?;
            return Err(PathError::GitTimeout { timeout });
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
mod tests {
    use super::{PathError, anneal_home_from, wait_with_deadline};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    /// `git hash-object --stdin`: needs no repository and waits until stdin closes.
    fn hash_object() -> Child {
        Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("git runs")
    }

    #[test]
    #[cfg_attr(miri, ignore = "spawns git")]
    fn a_child_past_its_deadline_is_killed_and_reaped() {
        // The test keeps the child's stdin open, so git never finishes on its own.
        let mut child = hash_object();

        let result = wait_with_deadline(&mut child, Duration::from_millis(50));

        assert!(
            matches!(
                result,
                Err(PathError::GitTimeout { timeout }) if timeout == Duration::from_millis(50)
            ),
            "got {result:?}"
        );
        assert!(
            child.try_wait().expect("status is available").is_some(),
            "the killed child was reaped"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "spawns git")]
    fn a_child_that_finishes_in_time_returns_its_output() {
        let mut child = hash_object();
        let mut stdin = child.stdin.take().expect("stdin is piped");
        stdin.write_all(b"hello\n").expect("write to git");
        drop(stdin);

        let output = wait_with_deadline(&mut child, Duration::from_secs(30)).expect("finishes");

        assert!(output.status.success(), "git succeeded: {output:?}");
        assert_eq!(
            output.stdout, b"ce013625030ba8dba906f756967f9e9ca394464a\n",
            "the blob id of \"hello\\n\""
        );
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn absolute(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn anneal_home_override_wins() {
        let over = absolute("over");
        let home = absolute("home");
        let lookup = env(&[
            ("ANNEAL_HOME", over.to_str().expect("UTF-8")),
            ("HOME", home.to_str().expect("UTF-8")),
        ]);
        assert_eq!(
            anneal_home_from(lookup).expect("resolves"),
            over,
            "override"
        );
    }

    #[test]
    fn home_then_userprofile_get_dot_anneal() {
        let home = absolute("home");
        let profile = absolute("profile");
        let both = env(&[
            ("ANNEAL_HOME", ""),
            ("HOME", home.to_str().expect("UTF-8")),
            ("USERPROFILE", profile.to_str().expect("UTF-8")),
        ]);
        let windows = env(&[("USERPROFILE", profile.to_str().expect("UTF-8"))]);

        assert_eq!(
            anneal_home_from(both).expect("resolves"),
            home.join(".anneal"),
            "HOME first"
        );
        assert_eq!(
            anneal_home_from(windows).expect("resolves"),
            profile.join(".anneal"),
            "USERPROFILE when HOME is unset"
        );
    }

    #[test]
    fn no_home_and_relative_home_are_errors() {
        let none = anneal_home_from(env(&[])).expect_err("nothing set");
        let relative = anneal_home_from(env(&[("ANNEAL_HOME", "rel/dir")])).expect_err("relative");

        assert!(matches!(none, PathError::NoHome), "got {none:?}");
        assert!(
            matches!(
                relative,
                PathError::RelativeHome {
                    var: "ANNEAL_HOME",
                    ..
                }
            ),
            "got {relative:?}"
        );
    }
}
