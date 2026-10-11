//! The git calls behind `floor-guard`, each with a deadline and capped output (H-13), and the
//! bounded reads of working-tree files. Output over a cap is an error, never a partial check.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use super::FloorError;
use crate::paths::{self, PathError};

/// How long one git call may take.
pub(super) const GIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Most bytes read from `git diff` or `git ls-files`.
pub(super) const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
/// Most bytes read from one working-tree file.
pub(super) const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The base refs tried, in order, when `--base` is not given.
pub(super) const DEFAULT_BASES: [&str; 2] = ["origin/main", "main"];

/// Runs git in `cwd`, keeping one byte past `max` of stdout so an overflow is seen.
fn git(cwd: &Path, args: &[&str], max: u64) -> Result<Output, FloorError> {
    paths::run_git_capped(cwd, args, GIT_TIMEOUT, max.saturating_add(1)).map_err(FloorError::Git)
}

/// The stdout of a successful call, refusing more than `max` bytes.
fn checked(output: Output, what: &'static str, max: u64) -> Result<Vec<u8>, FloorError> {
    if !output.status.success() {
        return Err(FloorError::GitFailed {
            what,
            stderr: trimmed(&output.stderr),
        });
    }
    if u64::try_from(output.stdout.len()).map_or(true, |len| len > max) {
        return Err(FloorError::TooLarge { what, max });
    }
    Ok(output.stdout)
}

fn trimmed(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

/// The first line of `bytes` as UTF-8, without its line ending.
fn first_line(bytes: Vec<u8>) -> Result<String, FloorError> {
    let text = String::from_utf8(bytes)
        .map_err(|error| FloorError::Git(PathError::NonUtf8Output(error)))?;
    Ok(text.lines().next().unwrap_or("").to_owned())
}

/// The working tree's root for `cwd`.
pub(super) fn toplevel(cwd: &Path) -> Result<PathBuf, FloorError> {
    let output = git(cwd, &["rev-parse", "--show-toplevel"], MAX_FILE_BYTES)?;
    if !output.status.success() {
        return Err(FloorError::Git(paths::git_failure(cwd, &output.stderr)));
    }
    let root = first_line(checked(output, "rev-parse", MAX_FILE_BYTES)?)?;
    Ok(PathBuf::from(root))
}

/// The commit `base` names, or `None` when it names none.
pub(super) fn resolve(root: &Path, base: &str) -> Result<Option<String>, FloorError> {
    let commit = format!("{base}^{{commit}}");
    let args = [
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &commit,
    ];
    let output = git(root, &args, MAX_FILE_BYTES)?;
    if !output.status.success() {
        return Ok(None);
    }
    first_line(checked(output, "rev-parse", MAX_FILE_BYTES)?).map(Some)
}

/// The merge base of `HEAD` and `commit`.
pub(super) fn merge_base(root: &Path, base: &str, commit: &str) -> Result<String, FloorError> {
    let output = git(root, &["merge-base", "HEAD", commit], MAX_FILE_BYTES)?;
    if !output.status.success() {
        return Err(FloorError::NoMergeBase {
            // The error owns the ref it names.
            base: base.to_owned(),
            stderr: trimmed(&output.stderr),
        });
    }
    first_line(checked(output, "merge-base", MAX_FILE_BYTES)?)
}

/// `git diff -U0` of the working tree (committed and uncommitted changes) against `commit`,
/// with renames found, fixed `a/`/`b/` prefixes, and no external diff or text conversion.
pub(super) fn diff(root: &Path, commit: &str) -> Result<String, FloorError> {
    let args = [
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-U0",
        "-M",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        commit,
        "--",
    ];
    let output = git(root, &args, MAX_OUTPUT_BYTES)?;
    let bytes = checked(output, "diff", MAX_OUTPUT_BYTES)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The untracked files git does not ignore, relative to `root`.
pub(super) fn untracked(root: &Path) -> Result<Vec<String>, FloorError> {
    let args = ["ls-files", "--others", "--exclude-standard", "-z"];
    let output = git(root, &args, MAX_OUTPUT_BYTES)?;
    let bytes = checked(output, "ls-files", MAX_OUTPUT_BYTES)?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

/// A working-tree file as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FileRead {
    /// It is not there, or is not a regular file (a directory, a symlink, a pipe).
    Absent,
    /// It is over [`MAX_FILE_BYTES`].
    TooLarge,
    /// Its content (lossy UTF-8).
    Text(String),
}

/// Reads `path` (relative to `root`) up to [`MAX_FILE_BYTES`].
pub(super) fn read_file(root: &Path, path: &str) -> Result<FileRead, FloorError> {
    let full = root.join(path);
    let failed = |source: io::Error| FloorError::Read {
        // The error owns the path it names.
        path: PathBuf::from(path),
        source,
    };
    // Checked before opening: a pipe would block, a directory fails on Windows.
    let metadata = match std::fs::symlink_metadata(&full) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(FileRead::Absent),
        Err(error) => return Err(failed(error)),
    };
    if !metadata.is_file() {
        return Ok(FileRead::Absent);
    }
    let file = std::fs::File::open(&full).map_err(failed)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_FILE_BYTES) {
        return Ok(FileRead::TooLarge);
    }
    Ok(FileRead::Text(String::from_utf8_lossy(&bytes).into_owned()))
}
