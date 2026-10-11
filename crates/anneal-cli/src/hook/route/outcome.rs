//! D3's outcome label: which phase the session actually took after a decision.
//!
//! When a phase decision is answered, [`remember`] keeps it for the session with the
//! transcript's length at that moment. At the session's next prompt, [`settle`] reads what the
//! transcript gained since then and labels the decision with the phase of the first entry
//! skill the model invoked through the `Skill` tool, its top phase when that phase's own skill
//! was among them. With no entry skill invoked, the label is the top phase if it has no skill
//! (nothing was there to invoke), else the playbook's no-skill phase. So the label is the top
//! phase exactly when the hinted skill was invoked, and `calibrate` scores it as such.
//!
//! The state is [`PENDING_FILE`] in the decision log's directory: per session a hash of its id,
//! the decision's id, its top phase and the offset, newest last, at most [`MAX_SESSIONS`]. A
//! decision is forgotten before it is labelled, so it is never labelled twice. Not labelled:
//! a session's last decision (no next prompt), one whose transcript was rewritten (it is
//! shorter, or no longer ends a line at the remembered length) or grew by more than
//! [`MAX_WINDOW_BYTES`] before the next prompt, and one whose entry was
//! lost to another session's hook writing the file at the same moment.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::baseline::{self, BaselineError};
use crate::content_hash;
use crate::decide::clock::Clock;
use crate::decide::id::{DecisionId, InvalidLabel, OutcomeLabel};
use crate::decide::in_process::{LabelError, Prepared};
use crate::hook::route::playbook::{Phase, Playbook};
use crate::hook::transcript;

/// The session a decision belongs to: its id and its transcript.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Session<'a> {
    /// The session's id (stored only as a hash).
    pub(crate) id: &'a str,
    /// The session's transcript, read for the skills invoked.
    pub(crate) transcript: &'a Path,
}

/// The state file's name in the decision log's directory.
const PENDING_FILE: &str = "route-pending.json";

/// The state file's `kind`, so another file of the same name is not overwritten.
const KIND: &str = "anneal-route-pending";

/// The state file's format version.
const FORMAT_VERSION: u32 = 1;

/// How many sessions the file remembers; older ones are dropped (H-13).
const MAX_SESSIONS: usize = 64;

/// Most transcript bytes read between a decision and the next prompt (H-13).
const MAX_WINDOW_BYTES: u64 = 16 * 1024 * 1024;

/// One session's decision awaiting its label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    session: String,
    id: String,
    phase: String,
    offset: u64,
}

/// On disk: `{"kind": "anneal-route-pending", "version": 1, "sessions": [...]}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingFile {
    kind: String,
    version: u32,
    sessions: Vec<Entry>,
}

/// Why a decision could not be remembered or labelled.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OutcomeError {
    /// The state file could not be read or written.
    #[error(transparent)]
    File(#[from] BaselineError),
    /// The state file is not ours, or not of this version.
    #[error("{path} is not {KIND} version {FORMAT_VERSION}", path = path.display())]
    Foreign {
        /// The file.
        path: PathBuf,
        /// What did not parse, if it did not.
        #[source]
        source: Option<serde_json::Error>,
    },
    /// The transcript could not be read.
    #[error("reading the transcript {path}", path = path.display())]
    Transcript {
        /// The transcript.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// A remembered id or phase is not one `decide` accepts.
    #[error("a remembered decision")]
    Invalid(#[source] InvalidLabel),
    /// The label could not be appended.
    #[error(transparent)]
    Label(#[from] LabelError),
}

/// Remembers the decision `id`, whose top phase is `phase`, for `session` to label at its next
/// prompt.
///
/// # Errors
/// [`OutcomeError`] when the transcript's length cannot be read or the state file cannot be
/// read or written.
pub(crate) fn remember(
    clef: &Prepared,
    session: Session<'_>,
    id: &DecisionId,
    phase: &str,
) -> Result<(), OutcomeError> {
    let Session {
        id: session,
        transcript,
    } = session;
    let offset = match transcript::transcript_len(transcript) {
        Ok(len) => len,
        // A session's first prompt can come before its transcript is written.
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(OutcomeError::Transcript {
                path: transcript.to_path_buf(),
                source,
            });
        }
    };
    let path = clef.log_dir().join(PENDING_FILE);
    let mut entries = load(&path)?;
    let key = key(session);
    entries.retain(|entry| entry.session != key);
    entries.push(Entry {
        session: key,
        id: id.to_string(),
        phase: phase.to_owned(),
        offset,
    });
    let excess = entries.len().saturating_sub(MAX_SESSIONS);
    entries.drain(..excess);
    save(&path, entries)
}

/// Labels `session`'s remembered decision, if any, from what its transcript gained since.
///
/// # Errors
/// [`OutcomeError`] when the state file, the transcript or the log cannot be read or written.
pub(crate) fn settle(
    clef: &Prepared,
    playbook: &Playbook,
    session: Session<'_>,
    clock: &impl Clock,
) -> Result<(), OutcomeError> {
    let Session {
        id: session,
        transcript,
    } = session;
    let path = clef.log_dir().join(PENDING_FILE);
    let mut entries = load(&path)?;
    let key = key(session);
    let Some(at) = entries.iter().position(|entry| entry.session == key) else {
        return Ok(());
    };
    let entry = entries.remove(at);
    // Forgotten first: a failure below leaves it unlabelled, never labelled twice.
    save(&path, entries)?;
    // From one byte before the offset: the transcript remembered ends a line there.
    let from = entry.offset.saturating_sub(1);
    let window = match transcript::read_since(transcript, from, MAX_WINDOW_BYTES.saturating_add(1))
    {
        Ok(Some(window)) => window,
        // Shrunk, overgrown or gone: what happened in between cannot be told.
        Ok(None) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(OutcomeError::Transcript {
                path: transcript.to_path_buf(),
                source,
            });
        }
    };
    let since = match (entry.offset, window.split_first()) {
        (0, _) => window.as_slice(),
        (_, Some((b'\n', since))) => since,
        // Rewritten since (no line ends at the offset any more): it cannot be told either.
        _ => return Ok(()),
    };
    let id: DecisionId = entry.id.parse().map_err(OutcomeError::Invalid)?;
    let label: OutcomeLabel = observed(playbook, &entry.phase, since)
        .parse()
        .map_err(OutcomeError::Invalid)?;
    Ok(clef.label(&id, &label, clock)?)
}

/// The phase the session took after a decision whose top phase was `top`, from the transcript
/// bytes written since; see the module documentation.
fn observed<'p>(playbook: &'p Playbook, top: &'p str, since: &[u8]) -> &'p str {
    let invoked: Vec<&str> = transcript::skills_invoked(since)
        .iter()
        .filter_map(|skill| playbook.phase_of_skill(skill))
        .map(Phase::name)
        .collect();
    if invoked.contains(&top) {
        return top;
    }
    if let Some(first) = invoked.first() {
        return first;
    }
    match playbook.phase(top) {
        Some(phase) if phase.entry_skill().is_none() => top,
        Some(_) | None => playbook.no_skill_phase(),
    }
}

/// The entries in `path`, oldest first; a missing file has none.
fn load(path: &Path) -> Result<Vec<Entry>, OutcomeError> {
    let Some(bytes) = baseline::read_state_bytes(path)? else {
        return Ok(Vec::new());
    };
    let foreign = |source| OutcomeError::Foreign {
        path: path.to_path_buf(),
        source,
    };
    let file: PendingFile = serde_json::from_slice(&bytes).map_err(|e| foreign(Some(e)))?;
    if file.kind != KIND || file.version != FORMAT_VERSION {
        return Err(foreign(None));
    }
    Ok(file.sessions)
}

/// Writes `entries` to `path`, atomically, creating its directory.
fn save(path: &Path, entries: Vec<Entry>) -> Result<(), OutcomeError> {
    let file = PendingFile {
        kind: KIND.to_owned(),
        version: FORMAT_VERSION,
        sessions: entries,
    };
    Ok(baseline::write_snapshot_json(path, &file)?)
}

/// What a session is stored as: the `sha256:` hash of its id, so the file holds no ids.
fn key(session: &str) -> String {
    content_hash::sha256(session.as_bytes())
}
