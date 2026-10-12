//! The handoff note, [`CONTINUE_HERE`] at a worktree's root: the `handoff` hook writes it, the
//! bootstrap injects it at the next session start.
//!
//! Three things keep a planted or stray file from becoming the next session's instructions,
//! and the note itself from looking like work:
//!
//! - [`read`] reads only a regular file (a symlink is refused, not followed) of at most
//!   [`MAX_NOTE_BYTES`], checking the length before reading and reading at most one byte more.
//! - [`record`] keeps, in the anneal home, the SHA-256 of the note's head (everything before
//!   its `## Notes` line) for its worktree; [`is_recorded`] is true only for a note whose head
//!   the hook wrote, so a hand-written or committed `.continue-here` is never injected. The
//!   notes section is the agent's to edit, so it is not hashed; the bootstrap scans it.
//! - [`exclude`] adds `/.continue-here` to the repository's `info/exclude` (once), so the note
//!   is never an untracked change for `anneal mode`, the Stop hook or `git status`.
//!
//! The record file holds at most 64 worktrees, the newest last; concurrent writers are last
//! writer wins, so a lost record only means that note is not injected.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::baseline::{self, BaselineError};
use crate::content_hash;
use crate::context::sessions::{SessionFile, SessionsError};
use crate::mode::scope;
use crate::paths::{self, PathError};

/// The handoff note's name, at the worktree root.
pub(crate) const CONTINUE_HERE: &str = ".continue-here";

/// The heading whose section, and everything after it, is the agent's notes.
pub(crate) const NOTES_HEADING: &str = "## Notes";

/// The largest note read, in bytes; the hook writes its facts and at most 8 KiB of notes, far
/// less (H-13).
pub(crate) const MAX_NOTE_BYTES: u64 = 32 * 1024;

/// The record of the notes the hook wrote, in the anneal home.
const RECORD_FILE: &str = "handoff-notes.json";

/// The `kind` of [`RECORD_FILE`].
const RECORD_KIND: &str = "anneal-handoff-notes";

/// The line [`exclude`] adds.
const EXCLUDE_LINE: &str = "/.continue-here";

/// What the hook wrote for one worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    /// `sha256:` and the hex SHA-256 of the note's head.
    head: String,
}

/// Why a note could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    /// Something other than a regular file is there; it is not opened.
    #[error("{path} is not a regular file", path = path.display())]
    NotAFile {
        /// The note's path.
        path: PathBuf,
    },
    /// The file is larger than [`MAX_NOTE_BYTES`].
    #[error("{path} is larger than {MAX_NOTE_BYTES} bytes", path = path.display())]
    TooLarge {
        /// The note's path.
        path: PathBuf,
    },
    /// Its metadata or contents could not be read.
    #[error("reading {path}", path = path.display())]
    Io {
        /// The note's path.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: io::Error,
    },
}

/// Why the note could not be excluded from git.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExcludeError {
    /// The repository's git directory could not be found.
    #[error(transparent)]
    Git(#[from] PathError),
    /// `info/exclude` could not be read.
    #[error(transparent)]
    Read(#[from] BaselineError),
    /// `info/exclude` could not be written.
    #[error("writing {path}", path = path.display())]
    Write {
        /// `info/exclude`.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: io::Error,
    },
}

/// The note at `path`, its invalid UTF-8 replaced; `None` when nothing is there.
///
/// # Errors
/// [`ReadError`] when something other than a regular file is at `path`, it is larger than
/// [`MAX_NOTE_BYTES`], or it cannot be read.
pub(crate) fn read(path: &Path) -> Result<Option<String>, ReadError> {
    let io_error = |source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    };
    let too_large = || ReadError::TooLarge {
        path: path.to_path_buf(),
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    if !metadata.is_file() {
        return Err(ReadError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > MAX_NOTE_BYTES {
        return Err(too_large());
    }
    let file = File::open(path).map_err(io_error)?;
    // A directory or device swapped in after the check above is refused by what was opened.
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(ReadError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    let mut bytes = Vec::new();
    file.take(MAX_NOTE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if !u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_NOTE_BYTES) {
        return Err(too_large());
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Where the notes section of `text` starts: its first [`NOTES_HEADING`] at a line start.
pub(crate) fn notes_start(text: &str) -> Option<usize> {
    text.match_indices(NOTES_HEADING)
        .map(|(at, _)| at)
        .find(|at| *at == 0 || text.get(..*at).is_some_and(|before| before.ends_with('\n')))
}

/// The hash of the head of `text`: all before its notes section, or all of it.
fn head_hash(text: &str) -> String {
    let head = notes_start(text)
        .and_then(|at| text.get(..at))
        .unwrap_or(text);
    content_hash::sha256(head.as_bytes())
}

/// The record file in the anneal home `home`.
fn record_path(home: &Path) -> PathBuf {
    super::in_home(home, RECORD_FILE)
}

/// Records, in the anneal home `home`, that the hook wrote `text` as the note of the worktree
/// at `root`.
///
/// # Errors
/// [`SessionsError`] when the record file cannot be read or written.
pub(crate) fn record(home: &Path, root: &str, text: &str) -> Result<(), SessionsError> {
    let path = record_path(home);
    let mut file = SessionFile::<Written>::load(&path, RECORD_KIND)?;
    file.set(
        &scope::worktree_key(root),
        Written {
            head: head_hash(text),
        },
    );
    file.save(&path)
}

/// Whether `text` has the head the hook last recorded for the worktree at `root`.
///
/// # Errors
/// [`SessionsError`] when the record file cannot be read.
pub(crate) fn is_recorded(home: &Path, root: &str, text: &str) -> Result<bool, SessionsError> {
    let file = SessionFile::<Written>::load(&record_path(home), RECORD_KIND)?;
    Ok(file
        .get(&scope::worktree_key(root))
        .is_some_and(|written| written.head == head_hash(text)))
}

/// Adds `/.continue-here` to the `info/exclude` of the repository of the worktree at `root`,
/// unless a line there already names it, with git killed after `timeout`.
///
/// # Errors
/// [`ExcludeError`] when git cannot find the repository, or the file cannot be read or
/// written.
pub(crate) fn exclude(root: &Path, timeout: Duration) -> Result<(), ExcludeError> {
    let info = paths::git_common_dir_within(root, timeout)?.join("info");
    let path = info.join("exclude");
    let separator = match baseline::read_state_bytes(&path)? {
        None => "",
        Some(existing) if is_listed(&existing) => return Ok(()),
        Some(existing) if existing.is_empty() || existing.ends_with(b"\n") => "",
        Some(_unterminated) => "\n",
    };
    append_line(&info, &path, separator).map_err(|source| ExcludeError::Write { path, source })
}

/// Whether a line of the exclude file `existing` already names the note.
fn is_listed(existing: &[u8]) -> bool {
    String::from_utf8_lossy(existing)
        .lines()
        .any(|line| matches!(line.trim(), EXCLUDE_LINE | CONTINUE_HERE))
}

/// Appends [`EXCLUDE_LINE`], after `separator`, to `path` in the directory `info`, creating
/// both as needed.
fn append_line(info: &Path, path: &Path, separator: &str) -> io::Result<()> {
    fs::create_dir_all(info)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{separator}{EXCLUDE_LINE}")
}

#[cfg(test)]
mod tests;
