//! The sections `anneal status` reads from the anneal home: `clef`, from `decide`'s
//! configuration and its answered-decision counts (`decisions/counts.json`), and `context`,
//! the latest snapshot `anneal statusline` saved (`context-usage.json`).

use std::ffi::OsString;
use std::path::Path;

use serde::Serialize;

use crate::context::sessions::SessionFile;
use crate::context::usage::{Level, Window, WindowSource};
use crate::context::{self, Snapshot};
use crate::decide::client::effective_mode;
use crate::decide::config::{ClefConfig, Mode};
use crate::decide::log::DecisionLog;
use crate::decide::point::Point;
use crate::error_chain::reason;

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
) -> Result<Option<Clef>, String> {
    let config = ClefConfig::from_env(lookup).map_err(|error| reason(&error))?;
    let answered = DecisionLog::in_home(home)
        .answered_by_point()
        .map_err(|error| reason(&error))?;
    let configured = config.account().is_some();
    if !configured && answered.is_empty() {
        return Ok(None);
    }
    let points = Point::ALL
        .into_iter()
        .map(|point| {
            let answered = answered.get(&point).copied().unwrap_or(0);
            let configured_mode = config.mode(point);
            let mode = match configured_mode {
                _ if !configured => Mode::Off,
                other => effective_mode(other, answered),
            };
            PointStatus {
                point,
                answered,
                configured_mode,
                mode,
            }
        })
        .collect();
    Ok(Some(Clef { configured, points }))
}

/// `context`: the newest session's context usage, as the status line saved it.
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

/// `context`, or `None` when the status line has saved no reading.
pub(super) fn context(home: &Path) -> Result<Option<Context>, String> {
    let path = context::in_home(home, context::USAGE_FILE);
    let file = SessionFile::<Snapshot>::load(&path, context::USAGE_KIND)
        .map_err(|error| reason(&error))?;
    Ok(file.newest().map(|snapshot| {
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
    }))
}
