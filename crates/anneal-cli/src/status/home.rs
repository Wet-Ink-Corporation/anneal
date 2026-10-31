//! The sections `anneal status` reads from the anneal home: `clef`, from `decide`'s
//! configuration and its answered-decision counts (`decisions/counts.json`), and `context`,
//! the fresher of the latest snapshots `anneal statusline` (`context-usage.json`) and the mod
//! (`context-usage-mod.json`) saved.

use std::ffi::OsString;
use std::path::Path;

use serde::Serialize;

use super::Unreadable;
use crate::context::usage::{Level, Window, WindowSource};
use crate::context::{self, Fresher, Snapshot};
use crate::decide::config::Mode;
use crate::decide::log::DecisionLog;
use crate::decide::modes::Modes;
use crate::decide::point::Point;

/// `clef`: each decision point's answered decisions and its mode.
#[derive(Debug, Serialize)]
pub(crate) struct Clef {
    /// Whether an account is configured; without one every point is off.
    configured: bool,
    /// D1 to D7, in order.
    points: Vec<PointStatus>,
}

impl Clef {
    /// Each point's name, answered count and effective mode, for the text form.
    pub(super) fn points(&self) -> impl Iterator<Item = (Point, u64, Mode)> + '_ {
        self.points
            .iter()
            .map(|point| (point.point, point.answered, point.mode))
    }
}

/// One decision point.
#[derive(Debug, Serialize)]
struct PointStatus {
    point: Point,
    /// Answered decisions logged for it (`counts.json`).
    answered: u64,
    /// The mode configured (`CLAUDE_PLUGIN_OPTION_CLEF_MODE`).
    configured_mode: Mode,
    /// The mode in effect: `off` without an account, `shadow` while a configured `enforce`
    /// has fewer than 50 answered decisions, else the configured one.
    mode: Mode,
}

/// `clef`, or `None` when Clef has no account and no decision was ever answered.
pub(super) fn clef(
    home: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<Option<Clef>, Unreadable> {
    let modes = Modes::read(&DecisionLog::in_home(home), lookup)?;
    let configured = modes.is_configured();
    if !configured && !modes.any_answered() {
        return Ok(None);
    }
    let points = Point::ALL
        .into_iter()
        .map(|point| PointStatus {
            point,
            answered: modes.answered(point),
            configured_mode: modes.configured(point),
            mode: modes.effective(point),
        })
        .collect();
    Ok(Some(Clef { configured, points }))
}

/// `context`: the newest session's context usage, as the status line or the mod saved it.
#[derive(Debug, Serialize)]
pub(crate) struct Context {
    /// Tokens in the context window; `None` before Claude Code reports usage.
    used_tokens: Option<f64>,
    /// The auto-compact window, in tokens.
    window: u32,
    /// Where the window came from: `env`, `model` or `default`.
    window_source: WindowSource,
    /// `used_tokens` as a percentage of `window`, to one decimal.
    percent: Option<f64>,
    /// The context monitor's threshold reached: `warn` (60%), `stop` (75%) or none.
    level: Option<Level>,
}

impl Context {
    /// Used tokens, window and percent, for the text form.
    pub(super) const fn parts(&self) -> (Option<f64>, u32, Option<f64>) {
        (self.used_tokens, self.window, self.percent)
    }
}

/// `context` as read: the section, and why each usage file that could not be read could not.
pub(super) struct ContextRead {
    /// The section, or `None` when no readable usage file has a reading.
    pub(super) context: Option<Context>,
    /// Why each unreadable usage file could not be read, the status line's first.
    pub(super) unreadable: Vec<Unreadable>,
}

/// `context`: the fresher of the status line's and the mod's newest snapshots
/// ([`context::Sources::newest`]); a usage file that cannot be read leaves the other's shown.
pub(super) fn context(home: &Path) -> ContextRead {
    let Fresher { saved, unreadable } = context::Sources::load(home).newest();
    ContextRead {
        context: saved.map(|saved| section(saved.reading)),
        unreadable: unreadable.into_iter().map(Unreadable::Sessions).collect(),
    }
}

/// The section for `snapshot`.
fn section(snapshot: Snapshot) -> Context {
    let window = Window {
        tokens: snapshot.window,
        source: snapshot.window_source,
    };
    let percent = snapshot
        .used_tokens
        .map(|used| (used / f64::from(snapshot.window.max(1)) * 1000.0).round() / 10.0);
    Context {
        used_tokens: snapshot.used_tokens,
        window: snapshot.window,
        window_source: snapshot.window_source,
        percent,
        level: snapshot.used_tokens.and_then(|used| window.level(used)),
    }
}
