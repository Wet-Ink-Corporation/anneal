//! Where anneal keeps state: the git common dir (ledger home) and `~/.anneal` (decision log).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
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
    let started = Instant::now();
    loop {
        if child.try_wait().map_err(PathError::Git)?.is_some() {
            return child.wait_with_output().map_err(PathError::Git);
        }
        if started.elapsed() >= timeout {
            child.kill().map_err(PathError::Git)?;
            child.wait().map_err(PathError::Git)?;
            return Err(PathError::GitTimeout { timeout });
        }
        thread::sleep(POLL_INTERVAL);
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
    use super::{PathError, anneal_home_from};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

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
