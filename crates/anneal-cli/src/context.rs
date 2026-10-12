//! Context usage, shared by `anneal statusline` (which writes it) and the context-monitor and
//! bootstrap hooks (which read it). Hooks receive no usage field, so the status line, the one
//! place Claude Code reports it, saves each session's latest reading in the anneal home.
//!
//! - [`usage`]: the auto-compact window, the thresholds and the crossing rule.
//! - [`sessions`]: the small per-session state files (at most 64 sessions, newest last).
//! - [`note`]: the handoff note `.continue-here`: its bounded read, the record of the notes the
//!   handoff hook wrote (`handoff-notes.json` in the anneal home), and its git exclusion.
//!
//! The files, all in the anneal home (`~/.anneal`, or `$ANNEAL_HOME`):
//! [`USAGE_FILE`] (written by the status line), [`MONITOR_FILE`] (the level each session last
//! reached, written by the monitor) and [`HINT_FILE`] (the bootstrap's one-time setup hint).

pub(crate) mod note;
pub(crate) mod sessions;
pub(crate) mod usage;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use usage::{Level, WindowSource};

/// The status line's readings, one per session.
pub(crate) const USAGE_FILE: &str = "context-usage.json";

/// The `kind` of [`USAGE_FILE`].
pub(crate) const USAGE_KIND: &str = "anneal-context-usage";

/// The level each session last reached, so a threshold fires once per crossing.
pub(crate) const MONITOR_FILE: &str = "context-monitor.json";

/// The `kind` of [`MONITOR_FILE`].
pub(crate) const MONITOR_KIND: &str = "anneal-context-monitor";

/// Present once the bootstrap has shown the status-line setup hint.
pub(crate) const HINT_FILE: &str = "statusline-hint.json";

/// The `kind` of [`HINT_FILE`].
pub(crate) const HINT_KIND: &str = "anneal-statusline-hint";

/// `name` in the anneal home `home`.
pub(crate) fn in_home(home: &Path, name: &str) -> PathBuf {
    home.join(name)
}

/// One session's latest reading, as the status line saved it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    /// Tokens in the context window, or `None` while Claude Code reports no percentage (early
    /// in a session, and after a compaction until the next response).
    pub(crate) used_tokens: Option<f64>,
    /// The auto-compact window the thresholds are measured against, in tokens.
    pub(crate) window: u32,
    /// Where the window came from.
    pub(crate) window_source: WindowSource,
}

/// What the monitor remembers of a session: the level its last reading was at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fired {
    /// The level, or `None` below the warning.
    pub(crate) level: Option<Level>,
}
