//! `anneal hook guard`: the `PreToolUse` guard on Bash. It fails closed.
//!
//! It reads Claude Code's `PreToolUse` input on stdin, lexes the Bash command (`shell`) and
//! checks it against the deterministic denylist (`denylist`). A denied command gets Claude
//! Code's deny decision on stdout, whose reason is one `ERROR <reason>` line, and a
//! `gate_blocked` event in the ledger. An allowed one gets no output. Whatever the guard cannot
//! read (input that is not `PreToolUse` JSON, over 1 MiB, a command with an unclosed quote) is
//! denied; a failed write of the decision exits 2 (`cli` fails it closed). A Bash command the
//! denylist leaves undecided gets `Fallback`'s answer unless Clef D1 (`clef`, #30), enforced,
//! denies or asks; its command is redacted (`redact`) before Clef or the log see it. See
//! `docs/guard.md`.

mod clef;
mod denylist;
pub(crate) mod redact;
mod shell;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, StoreError, Timestamp, TimestampError};
use serde::Deserialize;

use crate::decide::clock::{Clock, SystemClock};
use crate::decide::transport::Transport;
use crate::decide::transport::http::UreqTransport;
use crate::hook::{HOOK_BUDGET, HookEnv, StdinError, read_stdin, reason, write_ask, write_deny};
use crate::paths::{self, PathError};
/// What [`run_with`] reaches Clef and time through, so its callers can name it.
pub(crate) use clef::Ports;
use denylist::Verdict;
/// The merge gate reads commands as the guard does.
pub(crate) use denylist::{Context, MergeScan, PrMerge, Target, find_pr_merge, read_target};

/// The most stdin the guard reads; a larger input is denied (H-13). Commands carry heredoc
/// bodies, so the cap is well above a `PreToolUse` input's usual size.
pub(crate) const MAX_INPUT_BYTES: u64 = 1024 * 1024;

/// How long git may take to find the ledger (`anneal ledger` allows 10 s). With
/// [`RECORD_LOCK_RESERVE`] this keeps recording a denial well inside the hook's 5 s `timeout`,
/// after which Claude Code blocks the call without the guard's reason; see `docs/guard.md`.
/// The preconditions hook, under the same 5 s budget, shares it.
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(1);

/// How long each ledger statement of the other hooks waits for another process's lock (the
/// default is 10 s). The guard's own record waits [`RECORD_BUSY_TIMEOUT`] per statement instead.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_millis(500);

/// The least time worth giving git to find the ledger; with less, the record is skipped and the
/// deny says so.
const MIN_GIT_TIMEOUT: Duration = Duration::from_millis(200);

/// How long each statement of the guard's own ledger record waits for another process's lock.
/// Shorter than [`BUSY_TIMEOUT`] so that [`RECORD_LOCK_WAITS`] of them fit in the budget; an
/// append holds the lock for one small insert, so a writer that is not stuck still gets through.
const RECORD_BUSY_TIMEOUT: Duration = Duration::from_millis(100);

/// The most statements of one record that can each meet another process's lock, so wait up to
/// [`RECORD_BUSY_TIMEOUT`]: SQLite's timeout is per statement, not per record. Opening an
/// existing ledger runs nine (the schema version, the journal mode and the contents, setting the
/// journal mode, `synchronous` and `foreign_keys`, the stamp, then happenstance's two identity
/// reads) and the append two (`BEGIN IMMEDIATE` and `COMMIT`; the statements between them hold
/// the lock); a new ledger adds four (`ATTACH`, `BEGIN IMMEDIATE`, `COMMIT`, `DETACH`). That is
/// 15, rounded up to 16.
const RECORD_LOCK_WAITS: u32 = 16;

/// What [`git_budget`] holds back for the record's lock waits: every one of them, in full.
const RECORD_LOCK_RESERVE: Duration = RECORD_BUSY_TIMEOUT.saturating_mul(RECORD_LOCK_WAITS);

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
    /// `anneal verdict set` or `anneal decide label` with `--actor human` from a model (#62).
    HumanOnly,
    /// The command could not be read.
    Unparseable,
    /// The hook input could not be read.
    BadInput,
    /// Clef D1, enforced, rates the command destructive or irreversible.
    Destructive,
}

impl Rule {
    /// The rule's id, as shown in a denial and in the `gate_blocked` point.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::ForcePush => "force-push",
            Self::RecursiveForceDelete => "rm-rf",
            Self::ForceAdd => "git-add-force",
            Self::ConfigWrite => "config-write",
            Self::HumanOnly => "human-only",
            Self::Unparseable => "unparseable",
            Self::BadInput => "bad-input",
            Self::Destructive => "destructive",
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
    /// Ask the human (Clef D1, enforced): the reason, one line.
    Ask(String),
    /// Block it.
    Deny(Denial),
}

/// What decides a command the denylist leaves undecided, before Clef D1 is asked.
///
/// The denylist only denies or leaves a command undecided; every undecided command comes here,
/// and its answer stands unless D1, in enforce mode, decides otherwise (`clef`). With Clef off
/// or in shadow it is the guard's answer. The match in `decide` is exhaustive, so a new variant
/// has to say what it does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Fallback {
    /// Allow it.
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
    #[error("no time left in the hook's budget")]
    NoTime,
}

/// Runs the guard: reads the input from `env.stdin`, decides (the denylist, then Clef D1 for a
/// Bash command it left undecided), and on a denial records it in the ledger (resolved from the
/// input's `cwd`, or `env.current_dir`, and `env.lookup`) and writes the deny decision to
/// `out`; an ask writes the ask decision. Clef is reached over HTTPS through
/// [`UreqTransport`]. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every failure of the denylist's input denies.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let clock = SystemClock::new();
    let ports = Ports {
        transport: &UreqTransport::new(),
        clock: &clock,
    };
    run_with(env, &ports, out)
}

/// [`run`], reaching Clef through `ports`.
///
/// # Errors
/// As [`run`].
pub(crate) fn run_with<R, D, L, W, T, C>(
    env: HookEnv<R, D, L>,
    ports: &Ports<'_, T, C>,
    out: W,
) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
    T: Transport,
    C: Clock,
{
    let started = ports.clock.monotonic();
    let input = read_input(env.stdin);
    let decision = match decide(&input, &Context::new(&env.lookup)) {
        Decision::Allow => bash_command(&input).map_or(Decision::Allow, |command| {
            clef::judge(command, &env.lookup, ports)
        }),
        decided @ (Decision::Ask(_) | Decision::Deny(_)) => decided,
    };
    match decision {
        Decision::Allow => Ok(()),
        Decision::Ask(reason) => write_ask(out, &reason),
        Decision::Deny(denial) => {
            let spent = ports.clock.monotonic().saturating_sub(started);
            let recorded = match git_budget(HOOK_BUDGET.saturating_sub(spent)) {
                Some(git_timeout) => record(&denial, input.ok(), |cwd| {
                    ledger_path(cwd, env.current_dir, &env.lookup, git_timeout)
                }),
                None => Err(RecordError::NoTime),
            };
            write_denial(out, &denial, recorded.err().as_ref())
        }
    }
}

/// How long git may take to find the ledger when `remaining` of [`HOOK_BUDGET`] is left: what
/// is left after every lock wait the record can meet ([`RECORD_LOCK_RESERVE`]), at most
/// [`GIT_TIMEOUT`]; `None` when that is under [`MIN_GIT_TIMEOUT`], and the record is skipped so
/// the deny still arrives in time. The guard's budget counts from when it starts reading its
/// input, and Clef D1 takes up to 2 s of it, so a deny after D1 can leave less than
/// [`GIT_TIMEOUT`] plus [`RECORD_LOCK_RESERVE`] for the record, which then gets only what remains.
fn git_budget(remaining: Duration) -> Option<Duration> {
    remaining
        .checked_sub(RECORD_LOCK_RESERVE)
        .filter(|git| *git >= MIN_GIT_TIMEOUT)
        .map(|git| git.min(GIT_TIMEOUT))
}

/// The command of a Bash call, if `input` is one.
fn bash_command(input: &Result<Input, InputError>) -> Option<&str> {
    match input {
        Ok(Input {
            tool_name,
            tool: ToolInput {
                command: Some(command),
            },
            ..
        }) if tool_name == "Bash" => Some(command),
        Ok(_) | Err(_) => None,
    }
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

/// The ledger seen from `cwd` (or `current_dir` when there is none), git given `git_timeout`.
fn ledger_path(
    cwd: Option<PathBuf>,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
    git_timeout: Duration,
) -> Result<PathBuf, RecordError> {
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => current_dir().map_err(RecordError::CurrentDir)?,
    };
    paths::ledger_store_within(None, lookup, &cwd, git_timeout)
        .map(|location| location.path)
        .map_err(RecordError::Locate)
}

/// Appends a `gate_blocked` event for `denial`, tagged with the input's session, to the ledger
/// `ledger` finds from the input's `cwd`.
fn record(
    denial: &Denial,
    input: Option<Input>,
    ledger: impl FnOnce(Option<PathBuf>) -> Result<PathBuf, RecordError>,
) -> Result<(), RecordError> {
    let (session_id, cwd) = input.map_or((None, None), |input| (input.session_id, input.cwd));
    let path = ledger(cwd)?;
    let event = LedgerEvent::GateBlocked(GateBlocked {
        point: format!("{POINT_PREFIX}{}", denial.rule.id()),
        // The reason is duplicated: the ledger keeps one copy, the deny decision shows the other.
        reason: denial.reason.clone(),
    });
    let mut envelope =
        EventEnvelope::new(event, "hook", Timestamp::now().map_err(RecordError::Clock)?);
    envelope.session_id = session_id;
    Store::open_with_busy_timeout(&path, RECORD_BUSY_TIMEOUT)
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
