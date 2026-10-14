//! When source last changed: the newest modification time among the worktree's changed source
//! files.
//!
//! The changed files are what `git status --porcelain=v1 -z --untracked-files=all` lists
//! (tracked changes, staged or not, and untracked files git does not ignore); source is what
//! the preconditions hook guards (`mode::scope::classify`: not a test, a prose document, a
//! scratch path). A deleted file has no time and is passed over; so are committed changes,
//! which git no longer lists.

use std::io;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use crate::mode::scope::{self, PathKind};
use crate::paths::{self, PathError};

/// Most bytes of `git status` read; more is an error, never a partial answer.
const MAX_STATUS_BYTES: u64 = 4 * 1024 * 1024;

/// Why the latest source edit could not be found.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EditsError {
    /// git could not list the changed files.
    #[error("listing the changed files")]
    Git(#[source] PathError),
    /// `git status` printed more than [`MAX_STATUS_BYTES`].
    #[error("git status printed more than {MAX_STATUS_BYTES} bytes")]
    TooLarge,
    /// A changed file's modification time could not be read.
    #[error("reading the modification time of {file}")]
    Time {
        /// The file, as git names it.
        file: String,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

/// The newest change among a worktree's changed source files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// Its modification time, in milliseconds since the Unix epoch.
    pub(crate) at_ms: i64,
    /// The file, relative to the worktree root, as git names it.
    pub(crate) file: String,
}

/// The newest change among the changed source files of the worktree at `root`; `None` when no
/// source file changed.
///
/// # Errors
/// [`EditsError`] when git cannot list the files or a listed file's time cannot be read.
pub(crate) fn latest_source_edit(
    root: &Path,
    git_timeout: Duration,
) -> Result<Option<Edit>, EditsError> {
    let args = [
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
    ];
    let output =
        paths::run_git_capped(root, &args, git_timeout, MAX_STATUS_BYTES.saturating_add(1))
            .map_err(EditsError::Git)?;
    if !output.status.success() {
        return Err(EditsError::Git(paths::git_failure(root, &output.stderr)));
    }
    if u64::try_from(output.stdout.len()).map_or(true, |len| len > MAX_STATUS_BYTES) {
        return Err(EditsError::TooLarge);
    }
    let mut latest: Option<(i64, &[u8])> = None;
    for raw in source_paths(&output.stdout) {
        #[cfg(unix)]
        let relative = path_of(raw);
        #[cfg(not(unix))]
        let Some(relative) = path_of(raw) else {
            continue;
        };
        if let Some(at) = modified(&root.join(relative), raw)?
            && latest.is_none_or(|(seen, _)| at > seen)
        {
            latest = Some((at, raw));
        }
    }
    Ok(latest.map(|(at_ms, raw)| Edit {
        at_ms,
        file: String::from_utf8_lossy(raw).into_owned(),
    }))
}

/// The modification time of `path` in Unix milliseconds; `None` when it is gone or not a
/// regular file.
fn modified(path: &Path, raw: &[u8]) -> Result<Option<i64>, EditsError> {
    let failed = |source| EditsError::Time {
        file: String::from_utf8_lossy(raw).into_owned(),
        source,
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let time = metadata.modified().map_err(failed)?;
    // A time before the epoch reads as the epoch: older than any test run.
    let since = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    Ok(Some(i64::try_from(since.as_millis()).unwrap_or(i64::MAX)))
}

/// The changed source paths in a `git status --porcelain=v1 -z` listing.
fn source_paths(listing: &[u8]) -> Vec<&[u8]> {
    changed(listing)
        .filter(|raw| {
            let path = scope::normalise(&String::from_utf8_lossy(raw));
            scope::classify(&path, &[]) == PathKind::Source
        })
        .collect()
}

/// Each path in a `git status --porcelain=v1 -z` listing: `XY <path>` entries, a rename or
/// copy followed by its old path, which is skipped.
fn changed(listing: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut fields = listing.split(|byte| *byte == 0).filter(|f| !f.is_empty());
    std::iter::from_fn(move || {
        let entry = fields.next()?;
        let (status, path) = entry.split_at_checked(3)?;
        if status
            .iter()
            .take(2)
            .any(|code| matches!(code, b'R' | b'C'))
        {
            // The old path follows as its own field.
            fields.next();
        }
        Some(path)
    })
}

/// The relative path a raw file name from git names. On unix any bytes are a path.
#[cfg(unix)]
fn path_of(raw: &[u8]) -> &Path {
    use std::os::unix::ffi::OsStrExt;
    Path::new(std::ffi::OsStr::from_bytes(raw))
}

/// The relative path a raw file name from git names: elsewhere only UTF-8 names a file this
/// process can open.
#[cfg(not(unix))]
fn path_of(raw: &[u8]) -> Option<&Path> {
    std::str::from_utf8(raw).map(Path::new).ok()
}

#[cfg(test)]
mod tests {
    use super::{changed, source_paths};

    #[test]
    fn porcelain_entries_give_each_current_path_once_renames_by_their_new_name() {
        let listing = b" M src/lib.rs\0?? new file.rs\0R  src/new.rs\0src/old.rs\0 D gone.rs\0";

        let paths: Vec<&[u8]> = changed(listing).collect();

        assert_eq!(
            paths,
            [
                b"src/lib.rs".as_slice(),
                b"new file.rs",
                b"src/new.rs",
                b"gone.rs"
            ],
            "the old name of a rename is not a path; spaces are kept"
        );
    }

    #[test]
    fn only_source_paths_are_kept() {
        let listing = b" M src/lib.rs\0 M tests/it/a.rs\0 M src/x/tests.rs\0 M README.md\0\
                        ?? .temper/plans/p.md\0 M Cargo.toml\0";

        assert_eq!(
            source_paths(listing),
            [b"src/lib.rs".as_slice(), b"Cargo.toml"],
            "tests, docs and scratch are not source"
        );
    }
}
