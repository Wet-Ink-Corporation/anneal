//! `anneal hook guard`: the `PreToolUse` guard on Bash. It fails closed.
//!
//! It reads Claude Code's `PreToolUse` input on stdin, lexes the Bash command (`shell`) and
//! checks it against the deterministic denylist (`denylist`). A denied command gets Claude
//! Code's deny decision on stdout, whose reason is one `ERROR <reason>` line, and a
//! `gate_blocked` event in the ledger. An allowed one gets no output. Whatever the guard cannot
//! read (input that is not `PreToolUse` JSON, over 1 MiB, a command with an unclosed quote) is
//! denied; a failed write of the decision exits 2 (`cli` fails it closed). What the denylist
//! leaves undecided goes to `Fallback`, the extension point for Clef D1 (#30). See
//! `docs/guard.md`.

mod denylist;
mod shell;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, StoreError, Timestamp, TimestampError};
use serde::Deserialize;

use crate::hook::{HookEnv, StdinError, read_stdin, reason, write_deny};
use crate::paths::{self, PathError};
use denylist::{Context, Verdict};

/// The most stdin the guard reads; a larger input is denied (H-13). Commands carry heredoc
/// bodies, so the cap is well above a `PreToolUse` input's usual size.
pub(crate) const MAX_INPUT_BYTES: u64 = 1024 * 1024;

/// How long git may take to find the ledger (`anneal ledger` allows 10 s). With
/// [`BUSY_TIMEOUT`] this keeps recording a denial well inside the hook's 5 s `timeout`, after
/// which Claude Code blocks the call without the guard's reason; see `docs/guard.md`.
/// The preconditions hook, under the same 5 s budget, shares it.
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(1);

/// How long each ledger statement waits for another process's lock (the default is 10 s). A
/// locked ledger fails the first statement that meets the lock, so it costs about this once.
/// The preconditions hook shares it.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_millis(500);

/// The `point` of the guard's `gate_blocked` events, before the rule's id.
const POINT_PREFIX: &str = "guard.";

/// A denylist entry, or the reason a command could not be judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rule {
    /// `git push --force` (or a `+refspec`) to a protected branch.
    ForcePush,
    /// `rm -rf` outside the temporary and scratch directories.
    RecursiveForceDelete,
    /// `git add -f`.
    ForceAdd,
    /// A Bash write to lint or hook config.
    ConfigWrite,
    /// The command could not be read.
    Unparseable,
    /// The hook input could not be read.
    BadInput,
}

impl Rule {
    /// The rule's id, as shown in a denial and in the `gate_blocked` point.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::ForcePush => "force-push",
            Self::RecursiveForceDelete => "rm-rf",
            Self::ForceAdd => "git-add-force",
            Self::ConfigWrite => "config-write",
            Self::Unparseable => "unparseable",
            Self::BadInput => "bad-input",
        }
    }
}

/// Why a command is denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denial {
    /// The entry that denied it.
    pub(crate) rule: Rule,
    /// One line naming what was denied and why.
    pub(crate) reason: String,
}

impl Denial {
    pub(crate) fn new(rule: Rule, reason: impl Into<String>) -> Self {
        Self {
            rule,
            reason: reason.into(),
        }
    }
}

/// The guard's answer to a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Let it run: no output.
    Allow,
    /// Block it.
    Deny(Denial),
}

/// What decides a command the denylist leaves undecided.
///
/// **Extension point for Clef D1 (#30).** The denylist only denies or leaves a command
/// undecided; every undecided command comes here. Clef D1 plugs in as a new variant (say
/// `Clef`) whose `decide` asks Clef within the 2 s budget for Clef calls and maps its answer to
/// a [`Decision`], with `Allow` staying the answer when Clef is off. The match in `decide` is
/// exhaustive, so a new variant has to say what it does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Fallback {
    /// Allow it: the only fallback until Clef D1 lands.
    #[default]
    Allow,
}

impl Fallback {
    /// The decision for a command the denylist did not deny.
    pub(crate) const fn decide(self) -> Decision {
        match self {
            Self::Allow => Decision::Allow,
        }
    }
}

/// The fields of Claude Code's `PreToolUse` input the guard uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    /// Where the ledger is resolved; the process's working directory when absent.
    #[serde(default)]
    cwd: Option<PathBuf>,
    tool_name: String,
    #[serde(default, rename = "tool_input")]
    tool: ToolInput,
}

/// The Bash tool's input; other tools' fields are ignored.
#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    #[serde(default)]
    command: Option<String>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    /// stdin could not be read, or held more than [`MAX_INPUT_BYTES`].
    #[error(transparent)]
    Stdin(#[from] StdinError),
    /// stdin was not a JSON object of the expected shape.
    #[error("the input is not PreToolUse JSON")]
    Json(#[source] serde_json::Error),
    /// A Bash tool call with no `tool_input.command`.
    #[error("the Bash input has no command")]
    NoCommand,
}

/// Why a denial could not be recorded in the ledger.
#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error("reading the working directory")]
    CurrentDir(#[source] io::Error),
    #[error("finding the ledger")]
    Locate(#[source] PathError),
    #[error("reading the clock")]
    Clock(#[source] TimestampError),
    #[error("appending gate_blocked")]
    Append(#[source] StoreError),
}

/// Runs the guard: reads the input from `env.stdin`, decides, and on a denial records it in
/// the ledger (resolved from the input's `cwd`, or `env.current_dir`, and `env.lookup`) and
/// writes the deny decision to `out`. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure denies.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let input = read_input(env.stdin);
    let decision = decide(&input, &Context::new(&env.lookup));
    let Decision::Deny(denial) = decision else {
        return Ok(());
    };
    let recorded = record(&denial, input.ok(), env.current_dir, &env.lookup);
    write_denial(out, &denial, recorded.err().as_ref())
}

/// Reads and parses at most [`MAX_INPUT_BYTES`] of `stdin`.
fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The decision for `input`: the denylist's denial, else the fallback's answer.
fn decide(input: &Result<Input, InputError>, context: &Context) -> Decision {
    let verdict = match input {
        Ok(input) if input.tool_name != "Bash" => return Decision::Allow,
        Ok(Input {
            tool: ToolInput {
                command: Some(command),
            },
            ..
        }) => denylist::check(command, context),
        Ok(_) => bad_input(&InputError::NoCommand),
        Err(error) => bad_input(error),
    };
    match verdict {
        Verdict::Deny(denial) => Decision::Deny(denial),
        Verdict::Undecided => Fallback::default().decide(),
    }
}

fn bad_input(error: &InputError) -> Verdict {
    Verdict::Deny(Denial::new(
        Rule::BadInput,
        format!("the hook input cannot be read: {}", reason(error)),
    ))
}

/// Appends a `gate_blocked` event for `denial`, tagged with the input's session, to the ledger
/// seen from the input's `cwd` (or `current_dir` when there is none).
fn record(
    denial: &Denial,
    input: Option<Input>,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<(), RecordError> {
    let (session_id, cwd) = input.map_or((None, None), |input| (input.session_id, input.cwd));
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => current_dir().map_err(RecordError::CurrentDir)?,
    };
    let location =
        paths::ledger_store_within(None, lookup, &cwd, GIT_TIMEOUT).map_err(RecordError::Locate)?;
    let event = LedgerEvent::GateBlocked(GateBlocked {
        point: format!("{POINT_PREFIX}{}", denial.rule.id()),
        // The reason is duplicated: the ledger keeps one copy, the deny decision shows the other.
        reason: denial.reason.clone(),
    });
    let mut envelope =
        EventEnvelope::new(event, "hook", Timestamp::now().map_err(RecordError::Clock)?);
    envelope.session_id = session_id;
    Store::open_with_busy_timeout(&location.path, BUSY_TIMEOUT)
        .and_then(|ledger| ledger.append(&envelope))
        .map(drop)
        .map_err(RecordError::Append)
}

/// Writes the deny decision for `denial` to `out`; its reason is one `ERROR` line, which notes
/// `unrecorded` when the ledger could not take the event.
fn write_denial<W: Write>(
    out: W,
    denial: &Denial,
    unrecorded: Option<&RecordError>,
) -> io::Result<()> {
    let line = match unrecorded {
        None => format!("ERROR guard {}: {}", denial.rule.id(), denial.reason),
        Some(error) => format!(
            "ERROR guard {}: {} (the block was not recorded in the ledger: {})",
            denial.rule.id(),
            denial.reason,
            reason(error)
        ),
    };
    write_deny(out, &line)
}

#[cfg(test)]
mod tests;
