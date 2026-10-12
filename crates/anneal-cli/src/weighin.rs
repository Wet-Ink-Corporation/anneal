//! The weigh-in bridge: anneal parks one-way calls in weigh-in and reads its rulings back, and
//! never stores a decision itself.
//!
//! - `locate`: the CLI, from `$WEIGHIN_BIN`, then `weighin` on `PATH`.
//! - `batch`: the raise and deferral a park pipes to `weighin batch`.
//! - `receipt`: the item `weighin batch` minted (or found already raised) for a park.
//! - `items`: the decided items of `weighin open --all --json --top 4096`, for D6; a listing
//!   that long may be cut, so it is refused.
//!
//! The CLI runs through the [`Runner`] port; [`ProcessRunner`] spawns it with the input piped
//! to stdin, each output stream capped at [`MAX_OUTPUT_BYTES`] (one byte more is read, so a
//! longer one is refused rather than read in part) and a timeout (H-13). Raising is idempotent
//! in weigh-in, so a park run again names the same item. See `docs/orchestrate.md`.

mod batch;
mod items;
mod locate;
mod receipt;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Duration;

use crate::process::{self, WaitError};
pub(crate) use batch::ParkRequest;
pub(crate) use items::Decided;
use items::ListingError;
pub(crate) use locate::{BIN_VAR, locate};
use receipt::ReceiptError;

/// The prefix of every weigh-in item id.
const PREFIX: &str = "wi-";

/// Most characters after the prefix (weigh-in mints six hex digits).
const MAX_ID_CHARS: usize = 32;

/// How long a park's `weighin batch` may take.
pub(crate) const PARK_TIMEOUT: Duration = Duration::from_secs(10);

/// Most bytes kept of each of the CLI's output streams.
pub(crate) const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Most characters of the CLI's stderr kept in a failure's reason.
const MAX_STDERR_CHARS: usize = 200;

/// The `--top` asked of `weighin open --all --json`, whose own default (12) would cut the
/// listing short; a listing this long may have been cut, so it is refused.
pub(crate) const MAX_LISTED_ITEMS: usize = 4096;

/// A weigh-in item id: `wi-` and 1 to 32 lower-case ASCII letters or digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemId(String);

impl ItemId {
    /// The id as weigh-in prints it.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A word that is not a weigh-in item id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a weigh-in item id: expected wi- and 1 to {MAX_ID_CHARS} of a-z 0-9")]
pub(crate) struct NotAnItemId;

impl FromStr for ItemId {
    type Err = NotAnItemId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rest = text.strip_prefix(PREFIX).ok_or(NotAnItemId)?;
        let valid = (1..=MAX_ID_CHARS).contains(&rest.len())
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(NotAnItemId)
        }
    }
}

/// One run of the CLI.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Call<'a> {
    /// The program.
    pub(crate) program: &'a Path,
    /// Its arguments.
    pub(crate) args: &'a [&'a str],
    /// What it reads on stdin (empty for nothing).
    pub(crate) input: &'a [u8],
    /// Its working directory.
    pub(crate) cwd: &'a Path,
    /// How long it may take.
    pub(crate) timeout: Duration,
}

/// How a run ended: its exit code (`None` when a signal ended it) and its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ran {
    /// The exit code, or `None` when it was killed by a signal.
    pub(crate) code: Option<i32>,
    /// Its stdout, at most [`MAX_OUTPUT_BYTES`] and one byte.
    pub(crate) stdout: Vec<u8>,
    /// Its stderr, at most [`MAX_OUTPUT_BYTES`] and one byte.
    pub(crate) stderr: Vec<u8>,
}

/// Why the CLI could not be run to the end.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunError {
    /// It could not be started.
    #[error("starting weighin")]
    Spawn(#[source] io::Error),
    /// It could not be waited on, or ran past its timeout and was killed.
    #[error("running weighin")]
    Wait(#[source] WaitError),
}

/// The port the bridge runs the CLI through.
pub(crate) trait Runner {
    /// Runs `call` to the end.
    ///
    /// # Errors
    /// [`RunError`] when it cannot start, be waited on, or finish within its timeout.
    fn run(&self, call: &Call<'_>) -> Result<Ran, RunError>;
}

/// Runs the CLI as a child process (see the module documentation for its bounds).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProcessRunner;

impl Runner for ProcessRunner {
    fn run(&self, call: &Call<'_>) -> Result<Ran, RunError> {
        let mut child = Command::new(call.program)
            .args(call.args)
            .current_dir(call.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(RunError::Spawn)?;
        let cap = u64::try_from(MAX_OUTPUT_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let limits = process::Limits {
            timeout: call.timeout,
            cap,
        };
        let output = process::wait_with_input(&mut child, call.input, limits, "weighin")
            .map_err(RunError::Wait)?;
        Ok(Ran {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Why the bridge could not park a call or read weigh-in's rulings.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BridgeError {
    /// `$WEIGHIN_BIN` is unset or empty and no `PATH` directory holds `weighin`.
    #[error("no weighin: {BIN_VAR} is unset and none is on PATH")]
    NotFound,
    /// `$WEIGHIN_BIN` names no file.
    #[error("{BIN_VAR} names no file: {path}", path = path.display())]
    NotAFile {
        /// What it names.
        path: PathBuf,
    },
    /// The batch could not be encoded.
    #[error("encoding the batch")]
    Encode(#[source] serde_json::Error),
    /// The CLI could not be run to the end.
    #[error(transparent)]
    Run(#[from] RunError),
    /// The CLI failed.
    #[error("weighin exited with {code}: {stderr}", code = ExitCode(*code))]
    Exit {
        /// Its exit code; `None` when a signal ended it.
        code: Option<i32>,
        /// The first line of its stderr, cut to 200 characters.
        stderr: String,
    },
    /// An output stream was longer than the cap.
    #[error("weighin wrote more than {cap} bytes")]
    TooLarge {
        /// The cap.
        cap: usize,
    },
    /// The receipt does not show the park.
    #[error(transparent)]
    Receipt(#[from] ReceiptError),
    /// `open --all --json` did not parse.
    #[error("weighin open --all --json did not parse")]
    Listing(#[source] serde_json::Error),
    /// `open --all --json` listed as many items as it was asked for, so it may have left some
    /// out.
    #[error(
        "weighin open --all --json listed {cap} items or more; refusing a listing that may be cut"
    )]
    Truncated {
        /// The most items asked for.
        cap: usize,
    },
}

/// An exit code, or the signal that stands in for one.
struct ExitCode(Option<i32>);

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(code) => write!(f, "{code}"),
            None => f.write_str("a signal"),
        }
    }
}

/// Parks `request` through the CLI at `bin`, run in `cwd`: one `weighin batch` with the raise
/// and its deferral on stdin, within [`PARK_TIMEOUT`]. Returns the item weigh-in raised, or had
/// raised already.
///
/// # Errors
/// [`BridgeError`] when the CLI cannot run, fails, writes too much, or its receipt does not show
/// the item raised and deferred.
pub(crate) fn park(
    runner: &impl Runner,
    bin: &Path,
    cwd: &Path,
    request: &ParkRequest<'_>,
) -> Result<ItemId, BridgeError> {
    let batch = batch::json(request).map_err(BridgeError::Encode)?;
    let call = Call {
        program: bin,
        args: &["batch"],
        input: batch.as_bytes(),
        cwd,
        timeout: PARK_TIMEOUT,
    };
    let stdout = run_ok(runner, &call)?;
    Ok(receipt::parked_item(&String::from_utf8_lossy(&stdout))?)
}

/// The items weigh-in has decided, from `weighin open --all --json --top 4096` run in `cwd`
/// within `timeout`.
///
/// # Errors
/// [`BridgeError`] when the CLI cannot run, fails, writes too much, or its listing does not
/// parse; [`BridgeError::Truncated`] when it lists [`MAX_LISTED_ITEMS`] items or more, since
/// it may have left some out.
pub(crate) fn decided_items(
    runner: &impl Runner,
    bin: &Path,
    cwd: &Path,
    timeout: Duration,
) -> Result<Vec<Decided>, BridgeError> {
    let top = MAX_LISTED_ITEMS.to_string();
    let call = Call {
        program: bin,
        args: &["open", "--all", "--json", "--top", &top],
        input: &[],
        cwd,
        timeout,
    };
    let stdout = run_ok(runner, &call)?;
    items::decided(&stdout, MAX_LISTED_ITEMS).map_err(|error| match error {
        ListingError::Parse(source) => BridgeError::Listing(source),
        ListingError::Truncated => BridgeError::Truncated {
            cap: MAX_LISTED_ITEMS,
        },
    })
}

/// Runs `call` and returns its stdout when it exited 0 within the caps.
fn run_ok(runner: &impl Runner, call: &Call<'_>) -> Result<Vec<u8>, BridgeError> {
    let ran = runner.run(call)?;
    if ran.stdout.len() > MAX_OUTPUT_BYTES || ran.stderr.len() > MAX_OUTPUT_BYTES {
        return Err(BridgeError::TooLarge {
            cap: MAX_OUTPUT_BYTES,
        });
    }
    if ran.code != Some(0) {
        return Err(BridgeError::Exit {
            code: ran.code,
            stderr: first_line(&ran.stderr),
        });
    }
    Ok(ran.stdout)
}

/// The first non-blank line of `bytes`, trimmed and cut to [`MAX_STDERR_CHARS`] characters.
fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no message")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_STDERR_CHARS)
        .collect()
}

#[cfg(test)]
mod tests;
