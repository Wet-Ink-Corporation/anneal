//! The auto-compact window, the two thresholds measured against it, and the rule that fires
//! each threshold once per crossing.
//!
//! Thresholds are absolute tokens of the auto-compact window: [`WARN_PERCENT`] and
//! [`STOP_PERCENT`] of it. The window is `CLAUDE_CODE_AUTO_COMPACT_WINDOW` when set (read as
//! Claude Code reads it), else the model's context window from the status-line input, else
//! [`DEFAULT_WINDOW`]. The status line's `used_percentage` is always of the model's full
//! context window, so the tokens used are that percentage of the model window, not of the
//! auto-compact one.

use std::ffi::OsStr;

use serde::{Deserialize, Serialize};

/// The variable that sets the auto-compact window, in tokens.
pub(crate) const WINDOW_VAR: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";

/// The window when neither the variable nor the model says otherwise, in tokens.
pub(crate) const DEFAULT_WINDOW: u32 = 200_000;

/// The smallest window the variable can set (Claude Code clamps to it).
const MIN_ENV_WINDOW: u32 = 100_000;

/// The largest window the variable can set (Claude Code clamps to it).
const MAX_ENV_WINDOW: u32 = 1_000_000;

/// The share of the window, in percent, at which the monitor warns.
pub(crate) const WARN_PERCENT: u32 = 60;

/// The share of the window, in percent, at which the monitor stops the agent.
pub(crate) const STOP_PERCENT: u32 = 75;

/// Where a window came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WindowSource {
    /// `CLAUDE_CODE_AUTO_COMPACT_WINDOW`.
    Env,
    /// The model's context window, from the status-line input.
    Model,
    /// [`DEFAULT_WINDOW`].
    Default,
}

/// The auto-compact window, in tokens, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    /// The window's size in tokens.
    pub(crate) tokens: u32,
    /// Where the size came from.
    pub(crate) source: WindowSource,
}

/// A threshold that usage has reached. The order is the order of the thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Level {
    /// [`WARN_PERCENT`] of the window or more.
    Warn,
    /// [`STOP_PERCENT`] of the window or more.
    Stop,
}

/// One status-line reading: the tokens used, when Claude Code reported a percentage, and the
/// window they are measured against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Reading {
    /// Tokens in the context window; `None` without a usable percentage.
    pub(crate) used_tokens: Option<f64>,
    /// The auto-compact window.
    pub(crate) window: Window,
}

/// The auto-compact window from the variable's value `env` and the model's context window
/// `model`, both in tokens.
///
/// The variable is read as Claude Code reads it: its leading digits (`500k` is 500), clamped to
/// 100,000..=1,000,000, and never past the model's window. A value with no leading digits is
/// ignored.
pub(crate) fn window(env: Option<&OsStr>, model: Option<u32>) -> Window {
    let model = model.filter(|tokens| *tokens > 0);
    if let Some(set) = env.and_then(leading_number) {
        let clamped = set.clamp(MIN_ENV_WINDOW, MAX_ENV_WINDOW);
        return Window {
            tokens: model.map_or(clamped, |model| clamped.min(model)),
            source: WindowSource::Env,
        };
    }
    match model {
        Some(tokens) => Window {
            tokens,
            source: WindowSource::Model,
        },
        None => Window {
            tokens: DEFAULT_WINDOW,
            source: WindowSource::Default,
        },
    }
}

/// The number the leading ASCII digits of `value` spell, saturating at `u32::MAX`; `None` when
/// it does not start with a digit.
fn leading_number(value: &OsStr) -> Option<u32> {
    let bytes = value.as_encoded_bytes();
    let digits = bytes.iter().take_while(|byte| byte.is_ascii_digit());
    let mut number: Option<u32> = None;
    for digit in digits {
        let value = u32::from(digit.saturating_sub(b'0'));
        number = Some(number.unwrap_or(0).saturating_mul(10).saturating_add(value));
    }
    number
}

/// The reading for a status-line input whose context window reports `used_percentage` of a
/// model window of `model_window` tokens, with the variable's value `env`. A percentage over
/// 100 reads as 100; a negative or non-finite one as no reading.
pub(crate) fn reading(
    used_percentage: Option<f64>,
    model_window: Option<u64>,
    env: Option<&OsStr>,
) -> Reading {
    let model = model_window
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0);
    let measured_against = f64::from(model.unwrap_or(DEFAULT_WINDOW));
    let used_tokens = used_percentage
        .filter(|percent| percent.is_finite() && *percent >= 0.0)
        .map(|percent| percent.min(100.0) / 100.0 * measured_against);
    Reading {
        used_tokens,
        window: window(env, model),
    }
}

impl Window {
    /// The warning threshold, in tokens: [`WARN_PERCENT`] of the window.
    pub(crate) const fn warn_at(self) -> u32 {
        share(self.tokens, WARN_PERCENT)
    }

    /// The stop threshold, in tokens: [`STOP_PERCENT`] of the window.
    pub(crate) const fn stop_at(self) -> u32 {
        share(self.tokens, STOP_PERCENT)
    }

    /// The highest threshold `used_tokens` has reached, if any.
    pub(crate) fn level(self, used_tokens: f64) -> Option<Level> {
        if used_tokens >= f64::from(self.stop_at()) {
            Some(Level::Stop)
        } else if used_tokens >= f64::from(self.warn_at()) {
            Some(Level::Warn)
        } else {
            None
        }
    }
}

/// `percent` (at most 100) of `tokens`, rounded down, without overflow.
const fn share(tokens: u32, percent: u32) -> u32 {
    (tokens / 100) * percent + (tokens % 100) * percent / 100
}

/// What fires when usage moves from `previous` to `current`: `current`, when it is a higher
/// threshold than `previous`; nothing otherwise. Remembering `current` as the next `previous`
/// makes each threshold fire once per upward crossing, and re-arms it once usage falls below.
pub(crate) fn crossing(previous: Option<Level>, current: Option<Level>) -> Option<Level> {
    current.filter(|level| previous.is_none_or(|previous| *level > previous))
}

#[cfg(test)]
mod tests;
