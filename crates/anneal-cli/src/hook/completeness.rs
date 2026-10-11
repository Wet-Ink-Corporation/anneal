//! `anneal hook completeness`: the `Stop` hook. It fails open.
//!
//! It reads Claude Code's `Stop` input on stdin (at most 4 MiB; it uses `session_id`,
//! `transcript_path`, `cwd`, `stop_hook_active` and `last_assistant_message`, or the
//! transcript's last assistant message when that is absent) and checks the worktree of `cwd`:
//!
//! - **hard gates**, which block (`decision: block`): a success claim in the reply ("done",
//!   "all tests pass", ...: `claim`) while a source file changed after the worktree's latest
//!   green `test_run` (`edits`), and a `[DEBUG-<id>]` tag left in the worktree;
//! - **nudges**, which are only context (`additionalContext`): the events the active playbook
//!   requires and the ledger lacks (`playbook`).
//!
//! It blocks at most [`counter::MAX_CONSECUTIVE_BLOCKS`] times in a row per session (`counter`,
//! reset by a tool call in the transcript or a stop with no hard gate); past that the gates
//! become context. When `stop_hook_active` is set (the stop already follows a block) it does
//! nothing. Each block is recorded as `stop_blocked` in an existing ledger; it never creates
//! one. Anything it cannot check is one `ERROR` line as the `systemMessage`, and the stop goes
//! on. See `docs/hooks.md`.

mod check;
mod claim;
mod counter;
mod edits;
mod playbook;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anneal_ledger::event::StopBlocked;
use anneal_ledger::{EventEnvelope, LedgerEvent, Timestamp};
use serde::Deserialize;

use crate::error_chain::reason;
use crate::hook::stop_output::StopOutput;
use crate::hook::transcript::{self, Activity};
use crate::hook::{HookEnv, StdinError, read_stdin};
use crate::paths;

use check::Findings;
use counter::Counter;

/// Most bytes of hook input read: the reply rides in it.
const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024;

/// How long each git call may take. The hook's `timeout` is 30 s: four git calls, the
/// leftover scan's reads and about ten 500 ms ledger statements fit well inside it.
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// How much of the transcript's end is read for the last assistant message.
const TAIL_BYTES: u64 = 1024 * 1024;

/// How much transcript growth since the last block is searched for a tool call; more counts
/// as one.
const GROWTH_BYTES: u64 = 16 * 1024 * 1024;

/// The `point` of the hook's `stop_blocked` events, before the gate's id.
const POINT_PREFIX: &str = "completeness.";

/// The fields of Claude Code's `Stop` input the hook uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<PathBuf>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    stop_hook_active: bool,
    #[serde(default)]
    last_assistant_message: Option<String>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("the input is not Stop JSON")]
    Json(#[source] serde_json::Error),
}

/// Runs the hook: reads the input from `env.stdin`, checks the worktree of its `cwd` (or
/// `env.current_dir`), and writes the decision to `out`. Pass `&mut out` to keep the writer.
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
    let HookEnv {
        stdin,
        current_dir,
        lookup,
    } = env;
    let failed = |what: &str, error: &dyn std::error::Error| {
        StopOutput::new("Stop").error(&format!("completeness: {what}: {}", reason(error)))
    };
    let input = match read_stdin(stdin, MAX_INPUT_BYTES)
        .map_err(InputError::from)
        .and_then(|bytes| serde_json::from_slice::<Input>(&bytes).map_err(InputError::Json))
    {
        Ok(input) => input,
        Err(error) => return failed("the hook input cannot be read", &error).write(out),
    };
    // The stop already follows a block: let it end, as the platform asks.
    if input.stop_hook_active {
        return Ok(());
    }
    let cwd = match input.cwd {
        Some(cwd) => cwd,
        None => match current_dir() {
            Ok(cwd) => cwd,
            Err(error) => return failed("the working directory cannot be read", &error).write(out),
        },
    };
    let transcript = input.transcript_path.as_deref();
    let mut notes = Vec::new();
    let reply = input
        .last_assistant_message
        .or_else(|| last_message(transcript, &mut notes));
    let mut found = check::run(&cwd, reply.as_deref(), &lookup);
    found.notes.append(&mut notes);
    let session = Session {
        id: input.session_id.as_deref(),
        transcript,
    };
    let blocked = cap(&mut found, &session, &lookup);
    if blocked {
        record(&mut found, session.id);
    }
    output(found, blocked).write(out)
}

/// The text of the transcript's last assistant message, for a Claude Code that does not send
/// `last_assistant_message`.
fn last_message(transcript: Option<&Path>, notes: &mut Vec<String>) -> Option<String> {
    match transcript::read_tail(transcript?, TAIL_BYTES) {
        Ok(tail) => transcript::last_assistant_text(&tail),
        Err(error) => {
            notes.push(format!(
                "the transcript could not be read: {}",
                reason(&error)
            ));
            None
        }
    }
}

/// The session whose blocks are counted.
struct Session<'a> {
    id: Option<&'a str>,
    transcript: Option<&'a Path>,
}

/// Applies the block cap to `found`: true when its hard gates may block. Otherwise they move
/// to the nudges with the reason, and a stop with none resets the session's count.
fn cap(
    found: &mut Findings,
    session: &Session<'_>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> bool {
    let has_gates = !found.gates.is_empty();
    let path = match paths::anneal_home_from(lookup) {
        Ok(home) => counter::counter_path(&home),
        Err(error) if has_gates => return demote(found, "its block count cannot be kept", &error),
        Err(_unneeded) => return false,
    };
    let Some(id) = session.id else {
        if has_gates {
            demote_for(
                found,
                "the input names no session, so its blocks cannot be counted",
            );
        }
        return false;
    };
    let mut counter = match Counter::load(&path) {
        Ok(counter) => counter,
        Err(error) if has_gates => return demote(found, "its block count cannot be read", &error),
        Err(error) => {
            found.note("the block count could not be read", &error);
            return false;
        }
    };
    if !has_gates {
        if counter.reset(id)
            && let Err(error) = counter.save(&path)
        {
            found.note("the block count could not be reset", &error);
        }
        return false;
    }
    if let (Some(offset), Some(transcript)) = (counter.since(id), session.transcript)
        // A transcript that cannot be read shows no tool call: the count stands.
        && matches!(transcript::activity_since(transcript, offset, GROWTH_BYTES), Ok(Activity::ToolCall))
    {
        counter.reset(id);
    }
    if !counter.may_block(id) {
        demote_for(
            found,
            "it already blocked twice in a row with no tool call since, so it does not block again",
        );
        return false;
    }
    // Unreadable now means unreadable at the next stop too, where the count then stands.
    let offset = session
        .transcript
        .and_then(|transcript| transcript::transcript_len(transcript).ok())
        .unwrap_or(0);
    counter.record_block(id, offset);
    if let Err(error) = counter.save(&path) {
        return demote(found, "its block count cannot be saved", &error);
    }
    true
}

/// Moves the hard gates to the nudges because `why` failed with `error`; false (no block).
fn demote(found: &mut Findings, why: &str, error: &dyn std::error::Error) -> bool {
    found.note(why, error);
    demote_for(found, &format!("{why}, so it does not block"));
    false
}

/// Moves the hard gates to the nudges, saying `why` the hook did not block.
fn demote_for(found: &mut Findings, why: &str) {
    let open: Vec<String> = found.gates.drain(..).map(|(_, line)| line).collect();
    found.nudges.push(format!(
        "anneal completeness did not block: {why}. Still open: {}",
        open.join("; ")
    ));
}

/// Appends a `stop_blocked` event per hard gate to the ledger, when one exists.
fn record(found: &mut Findings, session: Option<&str>) {
    let Some(store) = &found.store else {
        return;
    };
    let mut failures = Vec::new();
    for (gate, line) in &found.gates {
        let event = LedgerEvent::StopBlocked(StopBlocked {
            point: format!("{POINT_PREFIX}{}", gate.id()),
            // The ledger keeps one copy of the line, the block shows the other.
            reason: line.clone(),
        });
        let appended = Timestamp::now()
            .map_err(|error| reason(&error))
            .and_then(|ts| {
                let mut envelope = EventEnvelope::new(event, "hook", ts);
                envelope.session_id = session.map(str::to_owned);
                envelope.worktree.clone_from(&found.worktree);
                store.append(&envelope).map_err(|error| reason(&error))
            });
        if let Err(why) = appended {
            failures.push(why);
        }
    }
    found.notes.extend(
        failures
            .into_iter()
            .map(|why| format!("the block was not recorded in the ledger: {why}")),
    );
}

/// The hook's output for `found`: a block with the hard gates when `blocked`, the nudges as
/// context, and the notes as one `ERROR` line.
fn output(found: Findings, blocked: bool) -> StopOutput {
    let mut output = StopOutput::new("Stop");
    if blocked {
        let lines: Vec<String> = found.gates.into_iter().map(|(_, line)| line).collect();
        output = output.block(format!("anneal completeness: {}", lines.join("; ")));
    }
    if !found.nudges.is_empty() {
        output = output.context(format!("anneal completeness: {}", found.nudges.join("; ")));
    }
    if !found.notes.is_empty() {
        output = output.error(&format!("completeness: {}", found.notes.join("; ")));
    }
    output
}
