//! A small state file keyed by session: `{"kind": ..., "version": 1, "sessions": [...]}`, each
//! entry a hash of the session's id and its value, newest last, at most [`MAX_SESSIONS`] of
//! them (H-13). The `kind` names the file's owner, so another file of the same name is refused
//! rather than overwritten.
//!
//! Concurrent sessions may write the same file. A save is atomic but a load-then-save is not
//! locked, so the last writer wins and another session's update made in between is lost. What
//! a loss costs depends on the file: a lost status-line reading is replaced at that session's
//! next refresh; a lost monitor level means that session's threshold can fire once more (a
//! repeated warning or stop); a lost handoff record means that note is not injected.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::baseline::{self, BaselineError};
use crate::content_hash;

/// How many sessions a file remembers; older ones are dropped.
pub(crate) const MAX_SESSIONS: usize = 64;

/// The files' format version.
const FORMAT_VERSION: u32 = 1;

/// The sessions of one state file, oldest first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionFile<T> {
    kind: &'static str,
    entries: Vec<Entry<T>>,
}

/// One session: the hash of its id and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry<T> {
    session: String,
    value: T,
}

/// On disk.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnDisk<T> {
    kind: String,
    version: u32,
    sessions: Vec<Entry<T>>,
}

/// The same shape, borrowing the entries for writing.
#[derive(Debug, Serialize)]
struct OnDiskRef<'a, T> {
    kind: &'a str,
    version: u32,
    sessions: &'a [Entry<T>],
}

/// Why a state file could not be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionsError {
    /// The file could not be read or written, or something other than a file is there.
    #[error(transparent)]
    File(#[from] BaselineError),
    /// The file is not JSON of the expected shape.
    #[error("{path} is not a {kind} file", path = path.display())]
    Format {
        /// The file.
        path: PathBuf,
        /// The kind it should be.
        kind: &'static str,
        /// What did not parse.
        #[source]
        source: serde_json::Error,
    },
    /// The file belongs to something else, or to another format version.
    #[error(
        "{path} is {kind} version {version}, not {expected} version {FORMAT_VERSION}",
        path = path.display()
    )]
    Foreign {
        /// The file.
        path: PathBuf,
        /// Its `kind`.
        kind: String,
        /// Its `version`.
        version: u32,
        /// The kind it should be.
        expected: &'static str,
    },
}

impl<T: Serialize + DeserializeOwned> SessionFile<T> {
    /// Loads the `kind` file at `path`; a missing file has no sessions.
    ///
    /// # Errors
    /// [`SessionsError`] when something other than a regular file is at `path` (it is not
    /// opened), or the file cannot be read, does not parse, or is another kind or version.
    pub(crate) fn load(path: &Path, kind: &'static str) -> Result<Self, SessionsError> {
        let Some(bytes) = baseline::read_state_bytes(path)? else {
            return Ok(Self {
                kind,
                entries: Vec::new(),
            });
        };
        let file: OnDisk<T> =
            serde_json::from_slice(&bytes).map_err(|source| SessionsError::Format {
                path: path.to_path_buf(),
                kind,
                source,
            })?;
        if file.kind != kind || file.version != FORMAT_VERSION {
            return Err(SessionsError::Foreign {
                path: path.to_path_buf(),
                kind: file.kind,
                version: file.version,
                expected: kind,
            });
        }
        let mut loaded = Self {
            kind,
            entries: file.sessions,
        };
        loaded.trim();
        Ok(loaded)
    }

    /// Saves the file to `path`, atomically, creating its directory.
    ///
    /// # Errors
    /// [`SessionsError::File`] when it cannot be written.
    pub(crate) fn save(&self, path: &Path) -> Result<(), SessionsError> {
        let file = OnDiskRef {
            kind: self.kind,
            version: FORMAT_VERSION,
            sessions: &self.entries,
        };
        Ok(baseline::write_snapshot_json(path, &file)?)
    }
}

impl<T> SessionFile<T> {
    /// `session`'s value, if it has one.
    pub(crate) fn get(&self, session: &str) -> Option<&T> {
        let key = content_hash::sha256(session.as_bytes());
        self.entries
            .iter()
            .find(|entry| entry.session == key)
            .map(|entry| &entry.value)
    }

    /// Sets `session`'s value: it becomes the newest entry, and the oldest are dropped past
    /// [`MAX_SESSIONS`].
    pub(crate) fn set(&mut self, session: &str, value: T) {
        let key = content_hash::sha256(session.as_bytes());
        self.entries.retain(|entry| entry.session != key);
        self.entries.push(Entry {
            session: key,
            value,
        });
        self.trim();
    }

    /// The newest session's value: the one set last.
    pub(crate) fn newest(&self) -> Option<&T> {
        self.entries.last().map(|entry| &entry.value)
    }

    /// How many sessions the file holds.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Drops the oldest entries past [`MAX_SESSIONS`].
    fn trim(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_SESSIONS);
        self.entries.drain(..excess);
    }
}

#[cfg(test)]
mod tests;
