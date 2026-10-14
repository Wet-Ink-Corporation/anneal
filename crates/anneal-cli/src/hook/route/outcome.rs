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
//! The state is one file per session in [`PENDING_DIR`], in the decision log's directory:
//! `<hex>.json`, where `<hex>` is the 64-digit SHA-256 of the session id (so the name is
//! bounded, safe on every file system, and holds no id), with the decision's id, its top phase
//! and the offset. Sessions never write the same file, so parallel sessions lose nothing. A
//! decision is claimed before it is labelled, by renaming its file (atomic: only one claim can
//! succeed), so it is never labelled twice. At most [`MAX_SESSIONS`] pending files are kept;
//! past that the oldest are dropped. Claimed files are bounded apart, so dropping pending files
//! never reaches a claim in flight; past [`MAX_SESSIONS`] claims (mostly left by hooks killed
//! while labelling) the oldest are dropped too, by when the decision was remembered (a rename
//! keeps the file's time), so a claim in flight may be among them; a claim found gone is
//! nothing to label, never an error. Not labelled: a session's last decision (no next prompt), one whose
//! transcript was rewritten (it is shorter, or no longer ends a line at the remembered length)
//! or grew by more than [`MAX_WINDOW_BYTES`] before the next prompt, and one dropped as the
//! oldest.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// The directory, in the decision log's directory, holding one pending file per session.
const PENDING_DIR: &str = "route-pending";

/// A pending file's extension.
const PENDING_EXTENSION: &str = "json";

/// A claimed file's extension, while it is being labelled.
const CLAIMED_EXTENSION: &str = "claimed";

/// A pending file's `kind`, so another file of the same name is not taken for one.
const KIND: &str = "anneal-route-pending";

/// A pending file's format version.
const FORMAT_VERSION: u32 = 1;

/// How many sessions are remembered; the oldest files past it are dropped (H-13).
const MAX_SESSIONS: usize = 64;

/// Most entries of the pending directory looked at when dropping the oldest (H-13).
const MAX_LISTED: usize = 1024;

/// Most transcript bytes read between a decision and the next prompt (H-13).
const MAX_WINDOW_BYTES: u64 = 16 * 1024 * 1024;

/// On disk: `{"kind": "anneal-route-pending", "version": 1, "id", "phase", "offset"}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    kind: String,
    version: u32,
    id: String,
    phase: String,
    offset: u64,
}

/// Why a decision could not be remembered or labelled.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OutcomeError {
    /// A pending file could not be read or written.
    #[error(transparent)]
    File(#[from] BaselineError),
    /// The pending directory or a file in it could not be listed, claimed or removed.
    #[error("{op} {path}", path = path.display())]
    Dir {
        /// What was being done.
        op: &'static str,
        /// The directory or file.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// A pending file is not ours, or not of this version.
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
/// [`OutcomeError`] when the transcript's length cannot be read, the session's file cannot be
/// written, or the oldest files cannot be dropped.
pub(crate) fn remember(
    clef: &Prepared,
    session: Session<'_>,
    id: &DecisionId,
    phase: &str,
) -> Result<(), OutcomeError> {
    let offset = match transcript::transcript_len(session.transcript) {
        Ok(len) => len,
        // A session's first prompt can come before its transcript is written.
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(OutcomeError::Transcript {
                path: session.transcript.to_path_buf(),
                source,
            });
        }
    };
    let dir = clef.log_dir().join(PENDING_DIR);
    let path = pending_path(&dir, session.id);
    let pending = Pending {
        kind: KIND.to_owned(),
        version: FORMAT_VERSION,
        id: id.to_string(),
        phase: phase.to_owned(),
        offset,
    };
    baseline::write_snapshot_json(&path, &pending)?;
    drop_oldest(&dir, &path)
}

/// Labels `session`'s remembered decision, if any, from what its transcript gained since.
///
/// # Errors
/// [`OutcomeError`] when the session's file, the transcript or the log cannot be read or
/// written.
pub(crate) fn settle(
    clef: &Prepared,
    playbook: &Playbook,
    session: Session<'_>,
    clock: &impl Clock,
) -> Result<(), OutcomeError> {
    let Some(pending) = claim(&clef.log_dir().join(PENDING_DIR), session.id)? else {
        return Ok(());
    };
    // From one byte before the offset: the transcript remembered ends a line there.
    let from = pending.offset.saturating_sub(1);
    let read = transcript::read_since(session.transcript, from, MAX_WINDOW_BYTES.saturating_add(1));
    let window = match read {
        Ok(Some(window)) => window,
        // Shrunk, overgrown or gone: what happened in between cannot be told.
        Ok(None) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(OutcomeError::Transcript {
                path: session.transcript.to_path_buf(),
                source,
            });
        }
    };
    let since = match (pending.offset, window.split_first()) {
        (0, _) => window.as_slice(),
        (_, Some((b'\n', since))) => since,
        // Rewritten since (no line ends at the offset any more): it cannot be told either.
        _ => return Ok(()),
    };
    let id: DecisionId = pending.id.parse().map_err(OutcomeError::Invalid)?;
    let label: OutcomeLabel = observed(playbook, &pending.phase, since)
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

/// `session`'s pending file in `dir`: `<hex>.json`, `<hex>` the SHA-256 of its id.
fn pending_path(dir: &Path, session: &str) -> PathBuf {
    let hash = content_hash::sha256(session.as_bytes());
    let hex = hash.strip_prefix("sha256:").unwrap_or(&hash);
    dir.join(hex).with_extension(PENDING_EXTENSION)
}

/// Takes `session`'s pending decision out of `dir`, if any: its file is renamed to a name of
/// this process's own, which only one claimant can do, then read and removed.
fn claim(dir: &Path, session: &str) -> Result<Option<Pending>, OutcomeError> {
    let path = pending_path(dir, session);
    let claimed = path.with_extension(format!("{}.{CLAIMED_EXTENSION}", std::process::id()));
    match fs::rename(&path, &claimed) {
        Ok(()) => {}
        // Nothing pending, or another hook claimed it first.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(OutcomeError::Dir {
                op: "claiming",
                path,
                source,
            });
        }
    }
    take(&claimed)
}

/// Reads and removes the claimed file `claimed`. A file that is gone, read or not, is not an
/// error: another hook removes a claim only in [`drop_oldest`], past [`MAX_SESSIONS`] claims (a
/// reused process id may also replace a dead claim of the same name, harmlessly), so the decision is then simply not labelled (or labelled from what was read:
/// the claim was this hook's alone either way).
fn take(claimed: &Path) -> Result<Option<Pending>, OutcomeError> {
    let read = load(claimed);
    match fs::remove_file(claimed) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(OutcomeError::Dir {
                op: "removing",
                path: claimed.to_path_buf(),
                source,
            });
        }
    }
    read
}

/// The pending decision in `path`, or `None` when the file is gone.
fn load(path: &Path) -> Result<Option<Pending>, OutcomeError> {
    let foreign = |source| OutcomeError::Foreign {
        path: path.to_path_buf(),
        source,
    };
    let Some(bytes) = baseline::read_state_bytes(path)? else {
        return Ok(None);
    };
    let pending: Pending = serde_json::from_slice(&bytes).map_err(|e| foreign(Some(e)))?;
    if pending.kind != KIND || pending.version != FORMAT_VERSION {
        return Err(foreign(None));
    }
    Ok(Some(pending))
}

/// Removes the oldest pending files in `dir` past [`MAX_SESSIONS`], never `keep` (the file just
/// written), and the oldest claimed files past [`MAX_SESSIONS`] of their own. The two are
/// bounded apart so that a full store of pending files never reaches a claim another hook is
/// still reading. Past the second bound the oldest claims go, by when their decision was
/// remembered (a rename keeps the file's time): mostly those of hooks killed between claiming
/// and removing, but a claim in flight from a long-idle session may be among them, and [`take`]
/// treats one removed under it as nothing to label.
/// Another hook removing a file first is not an error.
fn drop_oldest(dir: &Path, keep: &Path) -> Result<(), OutcomeError> {
    let Listed { pending, claimed } = list(dir)?;
    remove_oldest(pending, Some(keep))?;
    remove_oldest(claimed, None)
}

/// The pending and claimed files of the pending directory, each with when it was modified.
struct Listed {
    pending: Vec<(SystemTime, PathBuf)>,
    claimed: Vec<(SystemTime, PathBuf)>,
}

/// Lists at most [`MAX_LISTED`] entries of `dir` into [`Listed`].
fn list(dir: &Path) -> Result<Listed, OutcomeError> {
    let dir_error = |op, source| OutcomeError::Dir {
        op,
        path: dir.to_path_buf(),
        source,
    };
    let mut listed = Listed {
        pending: Vec::new(),
        claimed: Vec::new(),
    };
    for entry in fs::read_dir(dir)
        .map_err(|e| dir_error("listing", e))?
        .take(MAX_LISTED)
    {
        let entry = entry.map_err(|e| dir_error("listing", e))?;
        let path = entry.path();
        let files = match path.extension() {
            Some(ext) if ext == PENDING_EXTENSION => &mut listed.pending,
            Some(ext) if ext == CLAIMED_EXTENSION => &mut listed.claimed,
            Some(_) | None => continue,
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            // Claimed or dropped by another hook since it was listed.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(dir_error("reading", error)),
        };
        if metadata.is_file() {
            let modified = metadata.modified().map_err(|e| dir_error("reading", e))?;
            files.push((modified, path));
        }
    }
    Ok(listed)
}

/// Removes the oldest of `files` past [`MAX_SESSIONS`], never `keep` when there is one.
fn remove_oldest(
    mut files: Vec<(SystemTime, PathBuf)>,
    keep: Option<&Path>,
) -> Result<(), OutcomeError> {
    let excess = files.len().saturating_sub(MAX_SESSIONS);
    files.sort();
    for (_, path) in files
        .into_iter()
        .filter(|(_, path)| keep != Some(path.as_path()))
        .take(excess)
    {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(OutcomeError::Dir {
                    op: "removing",
                    path,
                    source,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
