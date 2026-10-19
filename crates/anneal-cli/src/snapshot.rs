//! `anneal snapshot write --json`: saves a context-usage snapshot the TypeScript mod read
//! (`$.session.usage()`), so the format of the file the context monitor reads stays the CLI's.
//!
//! It reads one JSON object on stdin (at most 64 KiB):
//!
//! - `session_id`: the session, not blank;
//! - `window`: tokens, more than 0. Without `window_source` it is the model's context window,
//!   and the auto-compact window is resolved from it as `anneal statusline` resolves it
//!   (`CLAUDE_CODE_AUTO_COMPACT_WINDOW`, clamped to the model's window); with `window_source`
//!   (`env`, `model` or `default`) it is the auto-compact window already, saved as given, so a
//!   snapshot in the saved shape round-trips;
//! - `used_tokens` (optional, `null` for none yet): tokens in the context, finite and not
//!   negative;
//! - `used_percentage` (optional): 0 to 100, of `window`; it gives the tokens only when
//!   `used_tokens` is absent or `null`.
//!
//! The snapshot is saved for the session in the anneal home's `context-usage-mod.json`,
//! stamped `source: mod` with the time, beside the status line's `context-usage.json`; the
//! context monitor reads the fresher of the two. Any other field, or a field out of range, is
//! a usage error (2) that writes nothing. Debouncing (at most one write every 2 s) is the
//! mod's job. See `docs/handoff.md`.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::time::SystemTimeError;

use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::command::CommandName;
use crate::context::sessions::SessionsError;
use crate::context::usage::{self, WINDOW_VAR, WindowSource};
use crate::context::{self, Saved, Snapshot, Source, WriterEnv};
use crate::decide::clock::Clock;
use crate::envelope::Envelope;
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::hook::{StdinError, read_stdin};
use crate::paths::{self, PathError};

/// The most stdin read; a snapshot is a few hundred bytes (H-13).
const MAX_INPUT_BYTES: u64 = 64 * 1024;

/// `anneal snapshot`.
#[derive(Debug, Args)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    action: SnapshotAction,
}

/// The `anneal snapshot` subcommands.
#[derive(Debug, Subcommand)]
pub enum SnapshotAction {
    /// Save the mod's context-usage snapshot, read on stdin, for the context monitor
    Write(WriteArgs),
}

/// `anneal snapshot write`.
#[derive(Debug, Args)]
pub struct WriteArgs {
    /// Read the snapshot on stdin as JSON (the one input format, named so another can follow)
    #[arg(long, required = true)]
    json: bool,
}

/// The snapshot read on stdin.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    session_id: String,
    window: u32,
    #[serde(default)]
    window_source: Option<WindowSource>,
    #[serde(default)]
    used_tokens: Option<f64>,
    #[serde(default)]
    used_percentage: Option<f64>,
}

/// Why `anneal snapshot write` saved nothing.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// stdin could not be read.
    #[error("snapshot write: reading the snapshot on stdin")]
    Read(#[source] io::Error),
    /// stdin held more than the cap.
    #[error("snapshot write: the snapshot is larger than {max} bytes")]
    TooLarge {
        /// The cap, in bytes.
        max: u64,
    },
    /// stdin is not a JSON snapshot of the documented shape.
    #[error("snapshot write: the snapshot is not JSON of the expected shape")]
    Json(#[source] serde_json::Error),
    /// A field is out of its range.
    #[error("snapshot write: {field} {problem}")]
    Invalid {
        /// The field.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// The system clock is before 1970, so the snapshot cannot be stamped.
    #[error("snapshot write: reading the clock")]
    Clock(#[source] SystemTimeError),
    /// The anneal home could not be found.
    #[error("snapshot write: finding the anneal home")]
    Home(#[source] PathError),
    /// The mod's usage file could not be read or written.
    #[error("snapshot write: saving the snapshot")]
    File(#[source] FileError),
}

/// Why the mod's usage file could not be read or written; opaque, since the file's format is
/// crate-private.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct FileError(SessionsError);

impl From<StdinError> for SnapshotError {
    fn from(error: StdinError) -> Self {
        match error {
            StdinError::Read(source) => Self::Read(source),
            StdinError::TooLarge { max } => Self::TooLarge { max },
        }
    }
}

impl SnapshotError {
    /// The exit code: input that is too large, not the shape or out of range is a usage error
    /// (2); stdin, the clock, the anneal home or the file failing is internal (70).
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::TooLarge { .. } | Self::Json(_) | Self::Invalid { .. } => exit_codes::USAGE,
            Self::Read(_) | Self::Clock(_) | Self::Home(_) | Self::File(_) => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal snapshot <action>`: reads the snapshot from `env.stdin`, the auto-compact
/// window variable and the anneal home through `env.lookup`, stamps it with `env.clock`'s
/// time, and writes the envelope, whose `data` is the snapshot as saved, to `out`.
///
/// # Errors
/// [`CliError::Snapshot`] when the input is invalid or the snapshot cannot be saved, and
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn run<R, L, C, O>(
    args: &SnapshotArgs,
    env: WriterEnv<'_, R, L, C>,
    out: O,
) -> Result<Code, CliError>
where
    R: Read,
    L: Fn(&str) -> Option<OsString>,
    C: Clock,
    O: Write,
{
    let SnapshotAction::Write(WriteArgs { json: _ }) = &args.action;
    let WriterEnv {
        stdin,
        lookup,
        clock,
    } = env;
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES).map_err(SnapshotError::from)?;
    let input: Input = serde_json::from_slice(&bytes).map_err(SnapshotError::Json)?;
    let snapshot = reading(&input, lookup(WINDOW_VAR).as_deref())?;
    let now_ms = clock.unix_millis().map_err(SnapshotError::Clock)?;
    let home = paths::anneal_home_from(lookup).map_err(SnapshotError::Home)?;
    let saved = Saved::stamped(snapshot, Source::Mod, now_ms);
    context::save(&home, &input.session_id, saved)
        .map_err(|error| SnapshotError::File(FileError(error)))?;
    Envelope::new(
        CommandName::Snapshot,
        Some("write"),
        exit_codes::SNAPSHOT_WRITTEN,
        saved,
    )
    .emit(out)
    .map_err(CliError::Output)
}

/// The reading `input` gives, with the auto-compact window variable's value `env`.
fn reading(input: &Input, env: Option<&OsStr>) -> Result<Snapshot, SnapshotError> {
    let invalid = |field, problem| SnapshotError::Invalid { field, problem };
    if input.session_id.trim().is_empty() {
        return Err(invalid("session_id", "is blank"));
    }
    if input.window == 0 {
        return Err(invalid("window", "must be more than 0 tokens"));
    }
    if input
        .used_tokens
        .is_some_and(|tokens| !tokens.is_finite() || tokens < 0.0)
    {
        return Err(invalid(
            "used_tokens",
            "must be a number of tokens, 0 or more",
        ));
    }
    if input
        .used_percentage
        .is_some_and(|percent| !(0.0..=100.0).contains(&percent))
    {
        return Err(invalid("used_percentage", "must be from 0 to 100"));
    }
    let window = match input.window_source {
        Some(source) => usage::Window {
            tokens: input.window,
            source,
        },
        None => usage::window(env, Some(input.window)),
    };
    let of_window = |percent: f64| percent / 100.0 * f64::from(input.window);
    Ok(Snapshot {
        used_tokens: input
            .used_tokens
            .or_else(|| input.used_percentage.map(of_window)),
        window: window.tokens,
        window_source: window.source,
    })
}

#[cfg(test)]
mod tests;
