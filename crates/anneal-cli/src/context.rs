//! Context usage, shared by its writers (`anneal statusline`, and `anneal snapshot write` for
//! the TypeScript mod) and its readers (the context-monitor and bootstrap hooks). Hooks receive
//! no usage field, so each writer saves each session's latest reading in the anneal home.
//!
//! - [`usage`]: the auto-compact window, the thresholds and the crossing rule.
//! - [`sessions`]: the small per-session state files (at most 64 sessions, newest last).
//! - [`note`]: the handoff note `.continue-here`: its bounded read, the record of the notes the
//!   handoff hook wrote (`handoff-notes.json` in the anneal home), and its git exclusion.
//!
//! The files, all in the anneal home (`~/.anneal`, or `$ANNEAL_HOME`):
//! [`USAGE_FILE`] (written by the status line) and [`MOD_USAGE_FILE`] (written for the mod),
//! one per [`Source`] so neither writer races the other; [`MONITOR_FILE`] (the level each
//! session last reached, written by the monitor) and [`HINT_FILE`] (the bootstrap's one-time
//! setup hint). Of a session's two snapshots the monitor reads the fresher ([`freshest`]).

pub(crate) mod note;
pub(crate) mod sessions;
pub(crate) mod usage;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use sessions::{SessionFile, SessionsError};
use usage::{Level, WindowSource};

/// The status line's snapshots, one per session.
pub(crate) const USAGE_FILE: &str = "context-usage.json";

/// The mod's snapshots, one per session, in [`USAGE_FILE`]'s format.
pub(crate) const MOD_USAGE_FILE: &str = "context-usage-mod.json";

/// The `kind` of [`USAGE_FILE`] and [`MOD_USAGE_FILE`].
pub(crate) const USAGE_KIND: &str = "anneal-context-usage";

/// The level each session last reached, so a threshold fires once per crossing.
pub(crate) const MONITOR_FILE: &str = "context-monitor.json";

/// The `kind` of [`MONITOR_FILE`].
pub(crate) const MONITOR_KIND: &str = "anneal-context-monitor";

/// Present once the bootstrap has shown the status-line setup hint.
pub(crate) const HINT_FILE: &str = "statusline-hint.json";

/// The `kind` of [`HINT_FILE`].
pub(crate) const HINT_KIND: &str = "anneal-statusline-hint";

/// What a writer of usage (`anneal statusline`, `anneal snapshot write`) reads: its input,
/// the environment (the auto-compact window variable and the anneal home) and the clock that
/// stamps the snapshot.
pub(crate) struct WriterEnv<'c, R, L, C> {
    /// The input, on stdin.
    pub(crate) stdin: R,
    /// The environment.
    pub(crate) lookup: L,
    /// The wall clock.
    pub(crate) clock: &'c C,
}

/// `name` in the anneal home `home`.
pub(crate) fn in_home(home: &Path, name: &str) -> PathBuf {
    home.join(name)
}

/// One session's latest reading. Unknown fields are ignored, so a snapshot that gained a field
/// (as [`Saved`]'s stamp) still reads.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    /// Tokens in the context window, or `None` while Claude Code reports no percentage (early
    /// in a session, and after a compaction until the next response).
    pub(crate) used_tokens: Option<f64>,
    /// The auto-compact window the thresholds are measured against, in tokens.
    pub(crate) window: u32,
    /// Where the window came from.
    pub(crate) window_source: WindowSource,
}

/// Who saved a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    /// `anneal statusline`; also every snapshot saved before snapshots named their source.
    #[default]
    Statusline,
    /// The TypeScript mod, through `anneal snapshot write`.
    Mod,
}

impl Source {
    /// The file this source's snapshots live in.
    const fn file(self) -> &'static str {
        match self {
            Self::Statusline => USAGE_FILE,
            Self::Mod => MOD_USAGE_FILE,
        }
    }
}

/// A snapshot as saved: the reading, who saved it and when.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Saved {
    /// The reading.
    #[serde(flatten)]
    pub(crate) reading: Snapshot,
    /// Who saved it; [`Source::Statusline`] when the snapshot predates the field.
    #[serde(default)]
    pub(crate) source: Source,
    /// When it was saved, in milliseconds since the Unix epoch; `None` when the snapshot
    /// predates the field, which is older than any stamped one.
    #[serde(default)]
    pub(crate) written_at_ms: Option<u64>,
}

impl Saved {
    /// `reading`, saved by `source` at `now_ms`.
    pub(crate) const fn stamped(reading: Snapshot, source: Source, now_ms: u64) -> Self {
        Self {
            reading,
            source,
            written_at_ms: Some(now_ms),
        }
    }
}

/// Saves `saved` as `session`'s snapshot in its source's file in the anneal home `home`.
///
/// # Errors
/// [`SessionsError`] when the file cannot be read, is not a usage file, or cannot be written.
pub(crate) fn save(home: &Path, session: &str, saved: Saved) -> Result<(), SessionsError> {
    let path = in_home(home, saved.source.file());
    let mut file = SessionFile::<Saved>::load(&path, USAGE_KIND)?;
    file.set(session, saved);
    file.save(&path)
}

/// `session`'s fresher snapshot of the two sources in the anneal home `home`: the later
/// `written_at_ms`, the mod's on a tie; `None` when neither has one.
///
/// # Errors
/// [`SessionsError`] when either file cannot be read or is not a usage file.
pub(crate) fn freshest(home: &Path, session: &str) -> Result<Option<Saved>, SessionsError> {
    let [status_line, from_mod] =
        [Source::Statusline, Source::Mod].map(|source| load(home, source));
    let status_line = status_line?.get(session).copied();
    let from_mod = from_mod?.get(session).copied();
    Ok(match (status_line, from_mod) {
        (Some(status_line), Some(from_mod))
            if status_line.written_at_ms > from_mod.written_at_ms =>
        {
            Some(status_line)
        }
        (status_line, None) => status_line,
        (_, from_mod) => from_mod,
    })
}

/// True when any mod snapshot in the anneal home `home` was saved less than `window_ms`
/// milliseconds before `now_ms` (or after it, by another clock).
///
/// # Errors
/// [`SessionsError`] when the mod's file cannot be read or is not a usage file.
pub(crate) fn mod_written_within(
    home: &Path,
    now_ms: u64,
    window_ms: u64,
) -> Result<bool, SessionsError> {
    let file = load(home, Source::Mod)?;
    Ok(file
        .values()
        .filter_map(|saved| saved.written_at_ms)
        .any(|at| now_ms.saturating_sub(at) < window_ms))
}

/// `source`'s file in the anneal home `home`.
fn load(home: &Path, source: Source) -> Result<SessionFile<Saved>, SessionsError> {
    SessionFile::load(&in_home(home, source.file()), USAGE_KIND)
}

/// What the monitor remembers of a session: the level its last reading was at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fired {
    /// The level, or `None` below the warning.
    pub(crate) level: Option<Level>,
}

#[cfg(test)]
mod tests;
