//! The completeness hook's own block cap: at most [`MAX_CONSECUTIVE_BLOCKS`] blocks in a row
//! per session.
//!
//! The counts live in the anneal home directory (`~/.anneal`, or `$ANNEAL_HOME`) in
//! [`COUNTER_FILE`]: per session a hash of its id, its consecutive blocks and the transcript's
//! length at the last block (where the next look for a tool call starts), newest last, at most
//! [`MAX_SESSIONS`] of them. The platform's own cap is 8 (`CLAUDE_CODE_STOP_HOOK_BLOCK_CAP`);
//! this one is stricter.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::baseline::{self, BaselineError};
use crate::content_hash;

/// Most blocks in a row for one session; past it, hard gates become context.
pub(crate) const MAX_CONSECUTIVE_BLOCKS: u32 = 2;

/// How many sessions the file remembers; older ones are dropped (H-13).
pub(crate) const MAX_SESSIONS: usize = 64;

/// The counter file's name in the anneal home directory.
const COUNTER_FILE: &str = "stop-blocks.json";

/// The file's `kind`, so another file of the same name is not overwritten.
const KIND: &str = "anneal-stop-blocks";

/// The file's format version.
const FORMAT_VERSION: u32 = 1;

/// The counter file in the anneal home directory `home`.
pub(crate) fn counter_path(home: &Path) -> PathBuf {
    home.join(COUNTER_FILE)
}

/// Each recent session's consecutive blocks, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Counter {
    entries: Vec<Entry>,
}

/// One session: a hash of its id, its blocks in a row and the transcript offset at the last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    session: String,
    blocks: u32,
    offset: u64,
}

/// On disk: `{"kind": "anneal-stop-blocks", "version": 1, "sessions": [...]}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterFile {
    kind: String,
    version: u32,
    sessions: Vec<Entry>,
}

/// The same shape, borrowing the entries for writing.
#[derive(Debug, Serialize)]
struct CounterFileRef<'a> {
    kind: &'a str,
    version: u32,
    sessions: &'a [Entry],
}

/// Why the counter file could not be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CounterError {
    /// The file could not be read or written.
    #[error(transparent)]
    File(#[from] BaselineError),
    /// The file is not JSON of the expected shape.
    #[error("{path} is not a stop-block counter file", path = path.display())]
    Format {
        /// The file.
        path: PathBuf,
        /// What did not parse.
        #[source]
        source: serde_json::Error,
    },
    /// The file belongs to something else, or to another format version.
    #[error(
        "{path} is {kind} version {version}, not {KIND} version {FORMAT_VERSION}",
        path = path.display()
    )]
    Foreign {
        /// The file.
        path: PathBuf,
        /// Its `kind`.
        kind: String,
        /// Its `version`.
        version: u32,
    },
}

impl Counter {
    /// Loads the counter at `path`; a missing file is an empty counter.
    ///
    /// # Errors
    /// [`CounterError`] when something other than a regular file is at `path`, or the file
    /// cannot be read, does not parse, or is another kind or version of file.
    pub(crate) fn load(path: &Path) -> Result<Self, CounterError> {
        let Some(bytes) = baseline::read_state_bytes(path)? else {
            return Ok(Self::default());
        };
        let file: CounterFile =
            serde_json::from_slice(&bytes).map_err(|source| CounterError::Format {
                path: path.to_path_buf(),
                source,
            })?;
        if file.kind != KIND || file.version != FORMAT_VERSION {
            return Err(CounterError::Foreign {
                path: path.to_path_buf(),
                kind: file.kind,
                version: file.version,
            });
        }
        let mut counter = Self {
            entries: file.sessions,
        };
        counter.trim();
        Ok(counter)
    }

    /// Saves the counter to `path`, atomically, creating its directory.
    ///
    /// # Errors
    /// [`CounterError::File`] when the file cannot be written.
    pub(crate) fn save(&self, path: &Path) -> Result<(), CounterError> {
        let file = CounterFileRef {
            kind: KIND,
            version: FORMAT_VERSION,
            sessions: &self.entries,
        };
        Ok(baseline::write_snapshot_json(path, &file)?)
    }

    /// The transcript offset at `session`'s last block, when it has blocks in a row.
    pub(crate) fn since(&self, session: &str) -> Option<u64> {
        self.entry(session).map(|entry| entry.offset)
    }

    /// True while `session` has fewer than [`MAX_CONSECUTIVE_BLOCKS`] blocks in a row.
    pub(crate) fn may_block(&self, session: &str) -> bool {
        self.entry(session)
            .is_none_or(|entry| entry.blocks < MAX_CONSECUTIVE_BLOCKS)
    }

    /// Counts a block of `session`, made when its transcript was `offset` bytes long: it
    /// becomes the newest entry, and the oldest are dropped past [`MAX_SESSIONS`].
    pub(crate) fn record_block(&mut self, session: &str, offset: u64) {
        let key = key(session);
        let blocks = self
            .entries
            .iter()
            .find(|entry| entry.session == key)
            .map_or(0, |entry| entry.blocks);
        self.entries.retain(|entry| entry.session != key);
        self.entries.push(Entry {
            session: key,
            blocks: blocks.saturating_add(1),
            offset,
        });
        self.trim();
    }

    /// Forgets `session`'s blocks; true when it had any.
    pub(crate) fn reset(&mut self, session: &str) -> bool {
        let key = key(session);
        let before = self.entries.len();
        self.entries.retain(|entry| entry.session != key);
        self.entries.len() != before
    }

    fn entry(&self, session: &str) -> Option<&Entry> {
        let key = key(session);
        self.entries.iter().find(|entry| entry.session == key)
    }

    /// Drops the oldest entries past [`MAX_SESSIONS`].
    fn trim(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_SESSIONS);
        self.entries.drain(..excess);
    }
}

/// What a session is stored as: the `sha256:` hash of its id, so the file holds no ids.
fn key(session: &str) -> String {
    content_hash::sha256(session.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{
        BaselineError, Counter, CounterError, MAX_CONSECUTIVE_BLOCKS, MAX_SESSIONS, counter_path,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn a_session_may_be_blocked_twice_in_a_row_and_not_a_third_time() {
        let mut counter = Counter::default();

        let mut allowed = Vec::new();
        for offset in [10, 20, 30] {
            let may = counter.may_block("s1");
            allowed.push(may);
            if may {
                counter.record_block("s1", offset);
            }
        }

        assert_eq!(
            MAX_CONSECUTIVE_BLOCKS, 2,
            "the cap is 2, stricter than the platform's 8"
        );
        assert_eq!(allowed, [true, true, false], "the third block is refused");
        assert_eq!(
            counter.since("s1"),
            Some(20),
            "the offset of the last block"
        );
    }

    #[test]
    fn a_reset_session_may_be_blocked_again_and_other_sessions_are_separate() {
        let mut counter = Counter::default();
        counter.record_block("s1", 1);
        counter.record_block("s1", 2);

        assert!(counter.may_block("s2"), "another session has its own count");
        assert!(counter.reset("s1"), "s1 had an entry");
        assert!(
            counter.may_block("s1"),
            "reset by a tool call or a clean stop"
        );
        assert!(!counter.reset("s1"), "nothing left to reset");
    }

    #[test]
    fn the_counter_round_trips_through_its_file_and_forgets_the_oldest_sessions() -> TestResult {
        let home = tempfile::tempdir()?;
        let path = counter_path(home.path());
        let mut counter = Counter::default();
        for n in 0..=MAX_SESSIONS {
            counter.record_block(&format!("s{n}"), 5);
        }
        counter.record_block("s64", 6);

        counter.save(&path)?;
        let loaded = Counter::load(&path)?;

        assert_eq!(loaded, counter, "what was saved is read back");
        assert_eq!(loaded.since("s0"), None, "the oldest session is dropped");
        assert_eq!(loaded.since("s64"), Some(6), "the newest is kept");
        assert!(!loaded.may_block("s64"), "with its two blocks");
        Ok(())
    }

    #[test]
    fn a_missing_file_is_an_empty_counter_and_a_foreign_one_is_an_error() -> TestResult {
        let home = tempfile::tempdir()?;
        let path = counter_path(home.path());

        assert_eq!(Counter::load(&path)?, Counter::default(), "nothing yet");
        std::fs::write(&path, br#"{"kind": "x", "version": 1, "sessions": []}"#)?;
        assert!(
            matches!(Counter::load(&path), Err(CounterError::Foreign { kind, .. }) if kind == "x"),
            "another kind of file is refused, so it is not overwritten"
        );
        Ok(())
    }

    #[test]
    fn a_directory_at_the_counter_path_is_refused_without_being_opened() -> TestResult {
        let home = tempfile::tempdir()?;
        let path = counter_path(home.path());
        std::fs::create_dir(&path)?;

        let loaded = Counter::load(&path);

        assert!(
            matches!(
                &loaded,
                Err(CounterError::File(BaselineError::NotAFile { path: refused })) if *refused == path
            ),
            "a directory is not a counter file: {loaded:?}"
        );
        Ok(())
    }
}
