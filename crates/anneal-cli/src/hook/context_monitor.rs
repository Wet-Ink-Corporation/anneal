//! `anneal hook context-monitor`: warns once at 60% and stops once at 75% of the auto-compact
//! window, per crossing. It fails open.
//!
//! Hooks receive no usage field, so it reads the reading `anneal statusline` saved for the
//! session in the anneal home (`context-usage.json`). Without one (no status line configured,
//! or none reported yet for this session) it is silent. The level each session last reached
//! is kept in `context-monitor.json`, so a threshold fires only when usage rises past it, and
//! fires again only after usage has fallen below it (after a compaction, say).
//!
//! It is declared under `PostToolBatch` (once per batch of tool calls) and also answers a
//! `PostToolUse` input as `PostToolUse`. A warning is `additionalContext`; a stop is
//! `decision: "block"`, which ends the agentic loop before the next model call with the reason
//! shown. Its own failures are one `ERROR` line as the `systemMessage`, never a block.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::Deserialize;

use crate::context::sessions::{SessionFile, SessionsError};
use crate::context::usage::{self, Level, Window};
use crate::context::{self, Fired, Snapshot};
use crate::error_chain::reason;
use crate::hook::HookEnv;
use crate::hook::stop_output::StopOutput;
use crate::paths;

/// The most stdin read. A batch's input carries its tool calls' results; only two fields are
/// kept, the rest are skipped as they stream past (H-13).
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// The fields of the `PostToolBatch` (or `PostToolUse`) input the hook uses.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    hook_event_name: Option<String>,
}

/// The event the output answers: the input's, `PostToolUse` or `PostToolBatch`.
fn event_of(input: Option<&Input>) -> &'static str {
    match input.and_then(|input| input.hook_event_name.as_deref()) {
        Some("PostToolUse") => "PostToolUse",
        _ => "PostToolBatch",
    }
}

/// Why the monitor could not judge the session.
#[derive(Debug, thiserror::Error)]
enum MonitorError {
    #[error("the hook input cannot be read")]
    Input(#[source] serde_json::Error),
    #[error("finding the anneal home")]
    Home(#[source] paths::PathError),
    #[error(transparent)]
    State(#[from] SessionsError),
}

/// Runs the hook: reads the input from `env.stdin`, the anneal home through `env.lookup`, and
/// writes the warning, the stop or nothing to `out`. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure is the `ERROR` line.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let input: Result<Input, _> = serde_json::from_reader(env.stdin.take(MAX_INPUT_BYTES));
    let output = match input {
        Ok(input) => {
            let event = event_of(Some(&input));
            match judge(&input, &env.lookup) {
                Ok(Some(crossed)) => fire(StopOutput::new(event), &crossed),
                Ok(None) => StopOutput::new(event),
                Err(error) => failed(event, &error),
            }
        }
        Err(error) => failed(event_of(None), &MonitorError::Input(error)),
    };
    output.write(out)
}

/// A threshold the session's usage has just risen past.
#[derive(Debug, Clone, Copy)]
struct Crossed {
    level: Level,
    used_tokens: f64,
    window: u32,
}

/// The threshold to fire for the input's session, with the reading that crossed it; `None`
/// when nothing fires. Records the session's new level.
fn judge(
    input: &Input,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<Option<Crossed>, MonitorError> {
    let Some(session) = input.session_id.as_deref() else {
        return Ok(None);
    };
    let home = paths::anneal_home_from(lookup).map_err(MonitorError::Home)?;
    let readings = SessionFile::<Snapshot>::load(
        &context::in_home(&home, context::USAGE_FILE),
        context::USAGE_KIND,
    )?;
    let Some(snapshot) = readings.get(session).copied() else {
        return Ok(None);
    };
    let window = Window {
        tokens: snapshot.window,
        source: snapshot.window_source,
    };
    let reached = snapshot
        .used_tokens
        .and_then(|used| window.level(used).map(|level| (level, used)));
    let current = reached.map(|(level, _)| level);
    let path = context::in_home(&home, context::MONITOR_FILE);
    let mut fired = SessionFile::<Fired>::load(&path, context::MONITOR_KIND)?;
    let previous = fired.get(session).and_then(|fired| fired.level);
    if previous == current {
        return Ok(None);
    }
    fired.set(session, Fired { level: current });
    // Fire only once the new level is kept, so a file that cannot be written never fires
    // the same crossing on every batch.
    fired.save(&path)?;
    // `crossing` fires `current` or nothing, and `reached` carries `current` with its tokens.
    Ok(usage::crossing(previous, current)
        .and(reached)
        .map(|(level, used_tokens)| Crossed {
            level,
            used_tokens,
            window: snapshot.window,
        }))
}

/// `output` with the warning as context, or the stop as a block.
fn fire(output: StopOutput, crossed: &Crossed) -> StopOutput {
    let Crossed {
        level,
        used_tokens: used,
        window,
    } = *crossed;
    let full = format!(
        "the context holds {:.0}k tokens, {:.0}% of the {}k-token auto-compact window",
        used / 1000.0,
        used / f64::from(window) * 100.0,
        window / 1000
    );
    match level {
        Level::Warn => output.context(format!(
            "anneal context-monitor: {full} (warning at {}%). At the next phase boundary, run \
             /handoff to choose: continue, clear, hand off, a subagent or compact.",
            usage::WARN_PERCENT
        )),
        Level::Stop => output.block(format!(
            "anneal context-monitor: {full} (stop at {}%). Stop and hand off now: run /handoff, \
             write what is in flight under `## Notes` in .continue-here, then /clear or \
             /compact.",
            usage::STOP_PERCENT
        )),
    }
}

/// The output for a monitor that could not judge: one `ERROR` line, never a block.
fn failed(event: &'static str, error: &MonitorError) -> StopOutput {
    StopOutput::new(event).error(&format!("context-monitor: {}", reason(error)))
}

#[cfg(test)]
mod tests;
