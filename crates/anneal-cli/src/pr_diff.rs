//! The diff a pull request makes, `git diff <base>...<head>`.
//!
//! It is taken from the merge base, so commits the base gained since do not count, and read by
//! the bounded git calls `verdict`, `ship`, `brief build` and the merge gate share: commit
//! resolution, the default base, ancestry (`is_ancestor`), the diff's `git patch-id
//! --verbatim`, its `--numstat` and its text (`diff_text`). Each call takes a timeout and caps
//! what it keeps (H-13).
//!
//! The diff's options are fixed so the same change always gives the same patch id: no colour,
//! external diff, text conversion or rename detection, every submodule change shown whatever
//! `diff.ignoreSubmodules` says, three lines of context, `a/` and `b/` prefixes, and `--binary`
//! so a binary file's content is hashed rather than one "Binary files differ" line.

use std::io;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::exit_codes::{self, Code};
use crate::glob::GlobError;
use crate::paths::{self, PathError};
use crate::process::{self, WaitError};
use crate::repo_config::{FileError, GateConfigError};
use crate::ship::skip_rule::NumstatError;

/// The base refs tried, in order, when none is given (as `floor-guard` does).
pub(crate) const DEFAULT_BASES: [&str; 2] = ["origin/main", "main"];

/// Most bytes kept from a `rev-parse` or `patch-id` call.
const SMALL_OUTPUT_BYTES: u64 = 64 * 1024;

/// Most bytes of `--numstat` output read; more is an error, never a partial list.
pub(crate) const MAX_NUMSTAT_BYTES: u64 = 16 * 1024 * 1024;

/// The options of every diff read here.
const DIFF_OPTIONS: [&str; 11] = [
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--no-renames",
    "--ignore-submodules=none",
    "--binary",
    "-U3",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--end-of-options",
];

/// Why a pull request's change could not be read: its worktree, a commit, its base, its diff
/// or the repository's gate settings. Shared by `verdict`, `ship` and the merge gate.
#[derive(Debug, thiserror::Error)]
pub enum PrError {
    /// The working directory is not in a git worktree.
    #[error("anneal {command} needs a git worktree")]
    NotAWorktree {
        /// The command, as typed.
        command: &'static str,
        /// What git said.
        #[source]
        source: PathError,
    },
    /// A commit flag names no commit.
    #[error("{flag} {rev}: names no commit in this repository (fetch it first?)")]
    UnknownCommit {
        /// The flag.
        flag: &'static str,
        /// The revision as given.
        rev: String,
    },
    /// No base was given and neither default names a commit.
    #[error("no base: neither origin/main nor main names a commit; pass --base <ref>")]
    NoBase,
    /// The base names no commit.
    #[error("base {base}: names no commit in this repository; pass --base <ref>")]
    UnknownBase {
        /// The base as given or recorded.
        base: String,
    },
    /// The repository's gate settings could not be read.
    #[error(transparent)]
    Config(#[from] GateConfigError),
    /// A default sensitive glob does not compile (a test keeps them valid).
    #[error("a default sensitive glob")]
    DefaultGlob(#[source] GlobError),
    /// git could not read the diff.
    #[error(transparent)]
    Diff(#[from] DiffError),
    /// `git diff --numstat` printed a record it does not document.
    #[error(transparent)]
    Numstat(#[from] NumstatError),
}

impl PrError {
    /// The documented exit code: a usage error (2) for what the caller can fix (the worktree,
    /// a commit, the base, the config file's content), else an internal error (70).
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::NotAWorktree { .. }
            | Self::UnknownCommit { .. }
            | Self::NoBase
            | Self::UnknownBase { .. }
            | Self::Config(
                GateConfigError::File(FileError::NotAFile | FileError::TooLarge)
                | GateConfigError::Json(_)
                | GateConfigError::TooManyGlobs
                | GateConfigError::Glob { .. },
            ) => exit_codes::USAGE,
            Self::Config(GateConfigError::File(FileError::Read(_)))
            | Self::DefaultGlob(_)
            | Self::Diff(_)
            | Self::Numstat(_) => exit_codes::INTERNAL,
        }
    }
}

/// Why the diff could not be read.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    /// git could not be started, waited on, or finished in time.
    #[error(transparent)]
    Git(#[from] PathError),
    /// A git call exited non-zero.
    #[error("git {what} failed: {stderr}")]
    Failed {
        /// The subcommand.
        what: &'static str,
        /// What git wrote on stderr, trimmed.
        stderr: String,
    },
    /// A git call printed more than its cap.
    #[error("git {what} printed more than {max} bytes")]
    TooLarge {
        /// The subcommand.
        what: &'static str,
        /// The cap, in bytes.
        max: u64,
    },
    /// git printed something that is not what it documents.
    #[error("git {what} printed {output:?}, which is not what it documents")]
    Unexpected {
        /// The subcommand.
        what: &'static str,
        /// The output, lossy UTF-8, trimmed.
        output: String,
    },
    /// The caller's time budget ran out before the next git call.
    #[error("no time left for git in a {budget:?} budget")]
    OutOfTime {
        /// The whole budget.
        budget: Duration,
    },
}

/// How long each git call may take: a fixed limit per call, and for a caller with a deadline
/// (a hook) also what is left of its budget.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    per_call: Duration,
    deadline: Option<(Instant, Duration)>,
}

/// The least time worth giving a git call; with less left the budget is spent.
const MIN_CALL: Duration = Duration::from_millis(100);

impl Budget {
    /// `per_call` for each call, with no overall deadline (a command).
    pub(crate) const fn per_call(per_call: Duration) -> Self {
        Self {
            per_call,
            deadline: None,
        }
    }

    /// At most `per_call` for each call, and all of them within `budget` of `started`.
    pub(crate) const fn within(started: Instant, budget: Duration, per_call: Duration) -> Self {
        Self {
            per_call,
            deadline: Some((started, budget)),
        }
    }

    /// `limit`, or what is left of the budget when that is less (zero once it is spent): the
    /// longest a wait that is not a git call (a ledger lock) may take.
    pub(crate) fn cap(&self, limit: Duration) -> Duration {
        match self.deadline {
            Some((started, budget)) => limit.min(budget.saturating_sub(started.elapsed())),
            None => limit,
        }
    }

    /// The timeout for the next git call.
    ///
    /// # Errors
    /// [`DiffError::OutOfTime`] when less than 100 ms of the budget is left.
    pub(crate) fn next(&self) -> Result<Duration, DiffError> {
        let Some((started, budget)) = self.deadline else {
            return Ok(self.per_call);
        };
        let left = budget.saturating_sub(started.elapsed());
        if left < MIN_CALL {
            return Err(DiffError::OutOfTime { budget });
        }
        Ok(left.min(self.per_call))
    }
}

/// The full id of the commit `rev` names in the repository at `root`; `None` when it names
/// none.
///
/// # Errors
/// The [`DiffError`] when git cannot answer.
pub(crate) fn resolve_commit(
    root: &Path,
    rev: &str,
    timeout: Duration,
) -> Result<Option<String>, DiffError> {
    let commit = format!("{rev}^{{commit}}");
    let args = [
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &commit,
    ];
    let output = paths::run_git_capped(root, &args, timeout, SMALL_OUTPUT_BYTES)?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let id = text.trim();
    if is_object_id(id) {
        Ok(Some(id.to_owned()))
    } else {
        Err(DiffError::Unexpected {
            what: "rev-parse",
            output: id.to_owned(),
        })
    }
}

/// The first of [`DEFAULT_BASES`] that names a commit; `None` when neither does.
///
/// # Errors
/// The [`DiffError`] when git cannot answer.
pub(crate) fn default_base(
    root: &Path,
    timeout: Duration,
) -> Result<Option<&'static str>, DiffError> {
    let started = Instant::now();
    for base in DEFAULT_BASES {
        let left = timeout.saturating_sub(started.elapsed());
        if resolve_commit(root, base, left)?.is_some() {
            return Ok(Some(base));
        }
    }
    Ok(None)
}

/// `git patch-id --verbatim` of `git diff <base>...<head>`; `None` for an empty diff. The diff
/// streams from one git into the other through an OS pipe, so nothing is buffered here; both
/// are killed when `timeout` passes.
///
/// # Errors
/// The [`DiffError`] when either git cannot run, fails (an unknown base or head, no merge
/// base) or does not finish in time.
pub(crate) fn patch_id(
    root: &Path,
    base: &str,
    head: &str,
    timeout: Duration,
) -> Result<Option<String>, DiffError> {
    let started = Instant::now();
    let range = format!("{base}...{head}");
    let mut diff = git(root)
        .args(DIFF_OPTIONS)
        .args([range.as_str(), "--"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // The diff's own errors are not read: patch-id waits for its output, so a full stderr
        // pipe could stall both. A failing diff is seen by its exit status.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| DiffError::Git(PathError::Git(error)))?;
    let hashed = match diff.stdout.take() {
        Some(stream) => hash(root, Stdio::from(stream), timeout),
        None => Err(DiffError::Git(PathError::Git(io::Error::other(
            "git diff's output pipe was not opened",
        )))),
    };
    // Reap the diff in the time left; with none left (or after a failure) it is killed.
    let left = match &hashed {
        Ok(_) => timeout.saturating_sub(started.elapsed()),
        Err(_) => Duration::ZERO,
    };
    let diffed = wait(&mut diff, left);
    let output = hashed?;
    let status = diffed?;
    if !status.status.success() {
        return Err(DiffError::Failed {
            what: "diff",
            stderr: format!("{range} could not be diffed ({})", status.status),
        });
    }
    read_patch_id(&output)
}

/// Runs `git patch-id --verbatim` on `input` within `timeout`: whitespace is hashed as written
/// (it is code in Python, YAML and Makefiles). `--verbatim` implies `--stable`, and git refuses
/// the two together.
fn hash(root: &Path, input: Stdio, timeout: Duration) -> Result<Output, DiffError> {
    let mut patch = git(root)
        .args(["patch-id", "--verbatim"])
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| DiffError::Git(PathError::Git(error)))?;
    let output = wait(&mut patch, timeout)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(DiffError::Failed {
            what: "patch-id",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// The patch id `git patch-id` printed: the first word of its one line, `None` when it printed
/// nothing (an empty diff).
fn read_patch_id(output: &Output) -> Result<Option<String>, DiffError> {
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(line) = text.lines().next() else {
        return Ok(None);
    };
    match line.split_whitespace().next() {
        Some(id) if is_object_id(id) => Ok(Some(id.to_owned())),
        _ => Err(DiffError::Unexpected {
            what: "patch-id",
            output: line.trim().to_owned(),
        }),
    }
}

/// `git diff --numstat -z` of `<base>...<head>`, as git printed it, at most
/// [`MAX_NUMSTAT_BYTES`].
///
/// # Errors
/// The [`DiffError`] when git cannot run, fails, or prints more than the cap.
pub(crate) fn numstat(
    root: &Path,
    base: &str,
    head: &str,
    timeout: Duration,
) -> Result<Vec<u8>, DiffError> {
    let range = format!("{base}...{head}");
    let args = [
        "-c",
        "core.quotePath=false",
        "diff",
        "--numstat",
        "-z",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--ignore-submodules=none",
        "--end-of-options",
        &range,
        "--",
    ];
    let cap = MAX_NUMSTAT_BYTES.saturating_add(1);
    let output = paths::run_git_capped(root, &args, timeout, cap)?;
    if !output.status.success() {
        return Err(DiffError::Failed {
            what: "diff --numstat",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    if u64::try_from(output.stdout.len()).map_or(true, |len| len > MAX_NUMSTAT_BYTES) {
        return Err(DiffError::TooLarge {
            what: "diff --numstat",
            max: MAX_NUMSTAT_BYTES,
        });
    }
    Ok(output.stdout)
}

/// Whether the commit `ancestor` is an ancestor of `descendant` (a commit is its own
/// ancestor), by `git merge-base --is-ancestor`. Pass full commit ids, so no argument reads as
/// an option.
///
/// # Errors
/// The [`DiffError`] when git cannot run, does not finish in time, or fails (an unknown
/// commit).
pub(crate) fn is_ancestor(
    root: &Path,
    ancestor: &str,
    descendant: &str,
    timeout: Duration,
) -> Result<bool, DiffError> {
    let args = ["merge-base", "--is-ancestor", ancestor, descendant];
    let output = paths::run_git_capped(root, &args, timeout, SMALL_OUTPUT_BYTES)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(DiffError::Failed {
            what: "merge-base --is-ancestor",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }),
    }
}

/// How long a diff read may take and how much of it is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextLimits {
    /// git is killed after this.
    pub(crate) timeout: Duration,
    /// Most bytes of diff kept; a longer diff is not read as a partial one.
    pub(crate) max_bytes: u64,
}

/// The text of `git diff <base>...<head>` with the fixed options, except `--binary` (a binary
/// file is one "Binary files differ" line); `None` when it is longer than `limits.max_bytes`,
/// never a partial diff.
///
/// # Errors
/// The [`DiffError`] when git cannot run, fails or does not finish in time.
pub(crate) fn diff_text(
    root: &Path,
    base: &str,
    head: &str,
    limits: TextLimits,
) -> Result<Option<Vec<u8>>, DiffError> {
    let TextLimits {
        timeout,
        max_bytes: max,
    } = limits;
    let range = format!("{base}...{head}");
    let mut args: Vec<&str> = DIFF_OPTIONS
        .into_iter()
        .filter(|option| *option != "--binary")
        .collect();
    args.extend([range.as_str(), "--"]);
    let output = paths::run_git_capped(root, &args, timeout, max.saturating_add(1))?;
    if !output.status.success() {
        return Err(DiffError::Failed {
            what: "diff",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let within = u64::try_from(output.stdout.len()).is_ok_and(|len| len <= max);
    Ok(within.then_some(output.stdout))
}

/// git in `root`, its messages untranslated.
fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.env("LC_ALL", "C").current_dir(root);
    command
}

/// Waits for `child` up to `timeout` (killing it past that), keeping at most
/// [`SMALL_OUTPUT_BYTES`] of what it prints.
fn wait(child: &mut Child, timeout: Duration) -> Result<Output, DiffError> {
    process::wait_with_deadline(child, timeout, SMALL_OUTPUT_BYTES, "git").map_err(|error| {
        DiffError::Git(match error {
            WaitError::Io(source) => PathError::Git(source),
            WaitError::Timeout => PathError::GitTimeout { timeout },
        })
    })
}

/// Whether `text` is a full object id: 40 (SHA-1) or 64 (SHA-256) lower-case hex digits.
pub(crate) fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests;
