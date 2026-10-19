//! `anneal statusline`: Claude Code's main status line, wired once in the user's settings
//! (plugins cannot set it).
//!
//! It reads the status-line input on stdin (at most 1 MiB), prints one line
//! (`anneal ctx 120k/200k 60%`: tokens used of the auto-compact window), and saves the reading
//! for the session in the anneal home's `context-usage.json`, stamped `source: statusline`
//! with the time, which the context-monitor hook reads (or the mod's snapshot, when fresher),
//! since hooks receive no usage field. The tokens used are
//! `context_window.used_percentage` of `context_window.context_window_size`; the window is
//! `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, the model's window, or 200,000 tokens (see
//! `docs/handoff.md`).
//!
//! It fails open like a hook: whatever goes wrong, it prints a line and exits 0, with one
//! `ERROR` line on stderr saying why.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::time::SystemTimeError;

use clap::Args;
use serde::Deserialize;

use crate::context::sessions::SessionsError;
use crate::context::usage::{self, Level, Reading, WINDOW_VAR};
use crate::context::{self, Saved, Snapshot, Source, WriterEnv};
use crate::decide::clock::{Clock, SystemClock};
use crate::error::CliError;
use crate::error_chain::reason;
use crate::exit_codes::{self, Code};
use crate::hook::{StdinError, read_stdin};
use crate::paths::{self, PathError};

/// The most stdin read; Claude Code's status-line input is a few KiB (H-13).
const MAX_INPUT_BYTES: u64 = 1024 * 1024;

/// Arguments of `anneal statusline`: none, it reads Claude Code's input on stdin.
#[derive(Debug, Args)]
pub struct StatuslineArgs;

/// The fields of the status-line input it uses; the rest are ignored.
#[derive(Debug, Default, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    context_window: Option<ContextWindow>,
}

/// `context_window`: both fields can be `null` early in a session.
#[derive(Debug, Default, Deserialize)]
struct ContextWindow {
    #[serde(default)]
    used_percentage: Option<f64>,
    #[serde(default)]
    context_window_size: Option<u64>,
}

/// Why the input could not be read.
#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("the status-line input is not JSON of the expected shape")]
    Json(#[source] serde_json::Error),
}

/// Why the reading could not be saved.
#[derive(Debug, thiserror::Error)]
enum SaveError {
    #[error("finding the anneal home")]
    Home(#[source] PathError),
    #[error("reading the clock")]
    Clock(#[source] SystemTimeError),
    #[error(transparent)]
    File(#[from] SessionsError),
}

/// Runs `anneal statusline`: reads the input from `stdin`, the auto-compact window variable
/// and the anneal home through `lookup`, writes the line to `out` and any failure as one
/// `ERROR` line to `err`.
///
/// The reading is stamped with the system clock's time.
///
/// # Errors
/// [`CliError::Output`] when `out` or `err` cannot be written; every other failure is in the
/// line and on `err`, and the code is still 0.
pub fn run<R, L, O, E>(stdin: R, lookup: L, out: O, err: E) -> Result<Code, CliError>
where
    R: Read,
    L: Fn(&str) -> Option<OsString>,
    O: Write,
    E: Write,
{
    let env = WriterEnv {
        stdin,
        lookup,
        clock: &SystemClock::new(),
    };
    run_with(env, out, err)
}

/// [`run`] with `env`'s input, environment and clock.
pub(crate) fn run_with<R, L, C, O, E>(
    env: WriterEnv<'_, R, L, C>,
    mut out: O,
    mut err: E,
) -> Result<Code, CliError>
where
    R: Read,
    L: Fn(&str) -> Option<OsString>,
    C: Clock,
    O: Write,
    E: Write,
{
    let WriterEnv {
        stdin,
        lookup,
        clock,
    } = env;
    let (line, problem) = match read_input(stdin) {
        Ok(input) => show(&input, &lookup, clock),
        Err(error) => (
            "anneal ctx - (input unreadable)".to_owned(),
            Some(reason(&error)),
        ),
    };
    writeln!(out, "{line}").map_err(CliError::Output)?;
    if let Some(problem) = problem {
        writeln!(err, "ERROR {}", problem.replace(['\r', '\n'], " ")).map_err(CliError::Output)?;
    }
    Ok(exit_codes::OK)
}

fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The line for `input`, and why its reading was not saved, if it was not.
fn show(
    input: &Input,
    lookup: impl Fn(&str) -> Option<OsString>,
    clock: &impl Clock,
) -> (String, Option<String>) {
    let window = input.context_window.as_ref();
    let reading = usage::reading(
        window.and_then(|window| window.used_percentage),
        window.and_then(|window| window.context_window_size),
        lookup(WINDOW_VAR).as_deref(),
    );
    let mut line = line(&reading);
    let Some(session) = input.session_id.as_deref() else {
        return (line, None);
    };
    match save(session, &reading, lookup, clock) {
        Ok(()) => (line, None),
        Err(error) => {
            line.push_str(" (usage not saved)");
            (
                line,
                Some(format!("saving the context usage: {}", reason(&error))),
            )
        }
    }
}

/// `anneal ctx <used>k/<window>k <percent>%`, with a hint at a threshold, or
/// `anneal ctx -/<window>k` before Claude Code reports usage.
fn line(reading: &Reading) -> String {
    let window = reading.window.tokens;
    let Some(used) = reading.used_tokens else {
        return format!("anneal ctx -/{}k", window / 1000);
    };
    let percent = used / f64::from(window) * 100.0;
    let hint = match reading.window.level(used) {
        None => "",
        Some(Level::Warn) => " (hand off soon)",
        Some(Level::Stop) => " (hand off now)",
    };
    format!(
        "anneal ctx {:.0}k/{}k {percent:.0}%{hint}",
        used / 1000.0,
        window / 1000
    )
}

/// Saves `reading` as `session`'s status-line snapshot in the anneal home, stamped with
/// `clock`'s time.
fn save(
    session: &str,
    reading: &Reading,
    lookup: impl Fn(&str) -> Option<OsString>,
    clock: &impl Clock,
) -> Result<(), SaveError> {
    let home = paths::anneal_home_from(lookup).map_err(SaveError::Home)?;
    let now_ms = clock.unix_millis().map_err(SaveError::Clock)?;
    let snapshot = Snapshot {
        used_tokens: reading.used_tokens,
        window: reading.window.tokens,
        window_source: reading.window.source,
    };
    let saved = Saved::stamped(snapshot, Source::Statusline, now_ms);
    Ok(context::save(&home, session, saved)?)
}

#[cfg(test)]
mod tests;
