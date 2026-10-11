//! Live state: the ledger tail, injected only when it changed for the session.
//!
//! What each session last saw is kept beside the ledger in [`SESSIONS_FILE`]: per session a
//! hash of its id and a hash of the state text it was given, newest last, at most
//! [`MAX_SESSIONS`] of them.

use std::path::{Path, PathBuf};

use anneal_ledger::{Filter, Recorded, Store, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::baseline::{self, BaselineError};

/// How many of the newest ledger events the live state shows.
pub(crate) const TAIL_EVENTS: usize = 5;

/// How many sessions the sessions file remembers; older ones are dropped (H-13).
pub(crate) const MAX_SESSIONS: usize = 64;

/// The sessions file's name, beside the ledger file.
const SESSIONS_FILE: &str = "bootstrap-sessions.json";

/// The sessions file's `kind`, so another file of the same name is not overwritten.
const KIND: &str = "anneal-bootstrap-sessions";

/// The sessions file's format version.
const FORMAT_VERSION: u32 = 1;

/// How many hex digits of SHA-256 identify a session or a state.
const HASH_HEX_DIGITS: usize = 16;

/// The live state section for the ledger at `store`: its newest events, oldest first. `None`
/// when there is no ledger file or it holds no events; the bootstrap never creates a ledger.
///
/// # Errors
/// [`StoreError`] when the ledger cannot be opened or read.
pub(crate) fn live_state(store: &Path) -> Result<Option<String>, StoreError> {
    if !store.is_file() {
        return Ok(None);
    }
    let events = Store::open(store)?.tail(&Filter::default(), TAIL_EVENTS)?;
    if events.is_empty() {
        return Ok(None);
    }
    let lines: String = events.iter().map(event_line).collect();
    Ok(Some(format!(
        "## Live state\n\nLedger tail (newest last):\n\n{lines}"
    )))
}

/// The live state section when the ledger could not be read: one line saying why.
#[must_use]
pub(crate) fn unavailable(reason: &str) -> String {
    format!("## Live state\n\nThe ledger could not be read: {reason}\n")
}

/// `- #<position> <ts> <type> <subject key>:<subject>`, one line: a line break in the subject
/// is folded into a space. An event with no subject (a `test_run` for no task) has no
/// `<subject key>:<subject>` part.
fn event_line(recorded: &Recorded) -> String {
    let envelope = &recorded.event;
    let kind = envelope.event.kind();
    let subject = envelope
        .event
        .subject_value()
        .map(|value| {
            let value = value.replace(['\r', '\n'], " ");
            format!(" {}:{value}", kind.subject().as_str())
        })
        .unwrap_or_default();
    format!(
        "- #{} {} {}{subject}\n",
        recorded.position,
        envelope.ts.as_str(),
        kind.as_str()
    )
}

/// The sessions file for the ledger at `store`.
#[must_use]
pub(crate) fn sessions_path(store: &Path) -> PathBuf {
    store.with_file_name(SESSIONS_FILE)
}

/// What each recent session was last given, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Sessions {
    seen: Vec<Seen>,
}

/// One session: hashes of its id and of the state it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seen {
    session: String,
    state: String,
}

/// On disk: `{"kind": "anneal-bootstrap-sessions", "version": 1, "sessions": [...]}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionsFile {
    kind: String,
    version: u32,
    sessions: Vec<Seen>,
}

/// The same shape as [`SessionsFile`], borrowing the sessions for writing.
#[derive(Debug, Serialize)]
struct SessionsFileRef<'a> {
    kind: &'a str,
    version: u32,
    sessions: &'a [Seen],
}

/// Why the sessions file could not be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionsError {
    /// The file could not be read or written.
    #[error(transparent)]
    File(#[from] BaselineError),
    /// The file is not JSON of the expected shape.
    #[error("{path} is not a bootstrap sessions file", path = path.display())]
    Format {
        /// The sessions file.
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
        /// The sessions file.
        path: PathBuf,
        /// Its `kind`.
        kind: String,
        /// Its `version`.
        version: u32,
    },
}

impl Sessions {
    /// Loads the sessions file at `path`; a missing file is no sessions.
    ///
    /// # Errors
    /// [`SessionsError`] when the file cannot be read, does not parse, or is another kind or
    /// version of file.
    pub(crate) fn load(path: &Path) -> Result<Self, SessionsError> {
        let Some(bytes) = baseline::read_snapshot_bytes(path)? else {
            return Ok(Self::default());
        };
        let file: SessionsFile =
            serde_json::from_slice(&bytes).map_err(|source| SessionsError::Format {
                path: path.to_path_buf(),
                source,
            })?;
        if file.kind != KIND || file.version != FORMAT_VERSION {
            return Err(SessionsError::Foreign {
                path: path.to_path_buf(),
                kind: file.kind,
                version: file.version,
            });
        }
        let mut sessions = Self {
            seen: file.sessions,
        };
        sessions.trim();
        Ok(sessions)
    }

    /// Saves the sessions to `path`, atomically.
    ///
    /// # Errors
    /// [`SessionsError::File`] when the file cannot be written.
    pub(crate) fn save(&self, path: &Path) -> Result<(), SessionsError> {
        let file = SessionsFileRef {
            kind: KIND,
            version: FORMAT_VERSION,
            sessions: &self.seen,
        };
        Ok(baseline::write_snapshot_json(path, &file)?)
    }

    /// True when `session` was last given exactly `state`.
    #[must_use]
    pub(crate) fn is_current(&self, session: &str, state: &str) -> bool {
        self.seen.contains(&Seen::new(session, state))
    }

    /// Records that `session` was given `state`: it becomes the newest entry, and the oldest
    /// are dropped past [`MAX_SESSIONS`].
    pub(crate) fn record(&mut self, session: &str, state: &str) {
        let seen = Seen::new(session, state);
        self.seen.retain(|entry| entry.session != seen.session);
        self.seen.push(seen);
        self.trim();
    }

    /// How many sessions are remembered.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.seen.len()
    }

    /// Drops the oldest entries past [`MAX_SESSIONS`].
    fn trim(&mut self) {
        let excess = self.seen.len().saturating_sub(MAX_SESSIONS);
        self.seen.drain(..excess);
    }
}

impl Seen {
    fn new(session: &str, state: &str) -> Self {
        Self {
            session: short_hash(session),
            state: short_hash(state),
        }
    }
}

/// The first [`HASH_HEX_DIGITS`] hex digits of the SHA-256 of `text`.
fn short_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .take(HASH_HEX_DIGITS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{MAX_SESSIONS, Sessions};

    #[test]
    fn a_session_that_saw_this_state_is_current() {
        let mut sessions = Sessions::default();
        sessions.record("s1", "tail A");
        assert!(
            sessions.is_current("s1", "tail A"),
            "same session, same state"
        );
    }

    #[test]
    fn a_changed_state_or_another_session_is_not_current() {
        let mut sessions = Sessions::default();
        sessions.record("s1", "tail A");
        assert!(!sessions.is_current("s1", "tail B"), "the state changed");
        assert!(!sessions.is_current("s2", "tail A"), "s2 never saw it");
    }

    #[test]
    fn recording_keeps_the_newest_sessions_up_to_the_cap() {
        let mut sessions = Sessions::default();
        for n in 0..=MAX_SESSIONS {
            sessions.record(&format!("s{n}"), "tail");
        }
        sessions.record("s1", "tail");
        assert!(!sessions.is_current("s0", "tail"), "the oldest was dropped");
        assert!(sessions.is_current("s1", "tail"), "re-recorded, so newest");
        assert_eq!(sessions.len(), MAX_SESSIONS, "never more than the cap");
    }
}
