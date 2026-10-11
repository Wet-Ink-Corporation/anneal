//! `anneal hook status`: the `SubagentStop` hook. It fails open and never blocks.
//!
//! It reads Claude Code's `SubagentStop` input on stdin (at most 4 MiB; it uses `session_id`,
//! `cwd`, `agent_id`, `agent_type`, `agent_transcript_path` and `last_assistant_message`) and
//! records one `subagent_status` event in an existing ledger (it never creates one): the status
//! block of the subagent's last message (`block`; a missing or malformed one is the event's
//! `violation`), and, from its transcript, its duration and token use. The input carries no
//! timing or token fields (Claude Code 2.1.296), so they come from the transcript, read whole
//! up to 32 MiB; a longer one is recorded without them. The hook writes nothing unless
//! something failed, which is one `ERROR` line as the `systemMessage`. See `docs/hooks.md`.

mod block;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anneal_ledger::event::SubagentStatus;
use anneal_ledger::{EventEnvelope, LedgerEvent, LedgerTag, TagKey, Timestamp};
use serde::Deserialize;

use crate::error_chain::reason;
use crate::existing_ledger;
use crate::hook::guard::BUSY_TIMEOUT;
use crate::hook::stop_output::StopOutput;
use crate::hook::transcript::{self, Usage};
use crate::hook::{HookEnv, StdinError, read_stdin};
use crate::mode::scope;
use crate::paths::{self, PathError};

/// Most bytes of hook input read: the last message rides in it.
const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024;

/// How long each git call may take, well inside the hook's 15 s `timeout`.
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Most bytes of a subagent transcript read for its timing and tokens (H-13).
const MAX_TRANSCRIPT_BYTES: u64 = 32 * 1024 * 1024;

/// The agent recorded when the input names none the ledger can tag.
const UNKNOWN_AGENT: &str = "unknown";

/// The fields of Claude Code's `SubagentStop` input the hook uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    agent_transcript_path: Option<PathBuf>,
    #[serde(default)]
    last_assistant_message: Option<String>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("the input is not SubagentStop JSON")]
    Json(#[source] serde_json::Error),
}

/// Runs the hook: reads the input from `env.stdin`, records the subagent's status in the
/// ledger seen from its `cwd` (or `env.current_dir`), and writes an `ERROR` line to `out` only
/// when something failed. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`.
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
    let mut notes = Vec::new();
    let input = read_stdin(stdin, MAX_INPUT_BYTES)
        .map_err(InputError::from)
        .and_then(|bytes| serde_json::from_slice::<Input>(&bytes).map_err(InputError::Json));
    match input {
        Ok(input) => record(input, current_dir, &lookup, &mut notes),
        Err(error) => notes.push(format!("the hook input cannot be read: {}", reason(&error))),
    }
    let mut output = StopOutput::new("SubagentStop");
    if !notes.is_empty() {
        output = output.error(&format!("status: {}", notes.join("; ")));
    }
    output.write(out)
}

/// Records `input`'s `subagent_status` in the ledger seen from its working directory, when one
/// exists, noting in `notes` whatever failed.
fn record(
    input: Input,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
    notes: &mut Vec<String>,
) {
    let cwd = match input.cwd {
        Some(cwd) => cwd,
        None => match current_dir() {
            Ok(cwd) => cwd,
            Err(error) => {
                notes.push(format!(
                    "the working directory cannot be read: {}",
                    reason(&error)
                ));
                return;
            }
        },
    };
    let store = match existing_ledger::open_existing(&cwd, lookup, GIT_TIMEOUT, BUSY_TIMEOUT) {
        Ok(Some(store)) => store,
        Ok(None) => return,
        Err(error) => {
            notes.push(format!("the ledger could not be read: {}", reason(&error)));
            return;
        }
    };
    let usage = input
        .agent_transcript_path
        .as_deref()
        .and_then(|path| read_usage(path, notes));
    let (duration_ms, tokens, transcript_text) = match usage {
        Some(Usage {
            duration_ms,
            tokens,
            last_text,
        }) => (duration_ms, tokens, last_text),
        None => (None, None, None),
    };
    let message = input.last_assistant_message.or(transcript_text);
    let parsed = block::parse(message.as_deref().unwrap_or_default());
    let event = LedgerEvent::SubagentStatus(SubagentStatus {
        agent: agent(input.agent_type),
        agent_id: input.agent_id,
        status: parsed.status,
        violation: parsed.violation,
        missing: parsed.missing,
        duration_ms,
        tokens,
    });
    let appended = Timestamp::now()
        .map_err(|error| reason(&error))
        .and_then(|ts| {
            let mut envelope = EventEnvelope::new(event, "hook", ts);
            envelope.session_id = input.session_id;
            envelope.worktree = worktree(&cwd, notes);
            store.append(&envelope).map_err(|error| reason(&error))
        });
    if let Err(why) = appended {
        notes.push(format!("the status was not recorded in the ledger: {why}"));
    }
}

/// The figures of the transcript at `path`; `None`, with a note, when it cannot be read, and
/// `None` without one when it is over [`MAX_TRANSCRIPT_BYTES`] (a known gap, not a failure).
fn read_usage(path: &Path, notes: &mut Vec<String>) -> Option<Usage> {
    match transcript::usage(path, MAX_TRANSCRIPT_BYTES) {
        Ok(usage) => usage,
        Err(error) => {
            notes.push(format!(
                "the subagent transcript could not be read: {}",
                reason(&error)
            ));
            None
        }
    }
}

/// The agent type to record: the input's when the ledger can tag it, else [`UNKNOWN_AGENT`].
fn agent(agent_type: Option<String>) -> String {
    agent_type
        .filter(|agent| LedgerTag::new(TagKey::Agent, agent).is_ok())
        .unwrap_or_else(|| UNKNOWN_AGENT.to_owned())
}

/// The worktree key of `cwd`'s worktree; `None` outside one, and, with a note, when git fails.
fn worktree(cwd: &Path, notes: &mut Vec<String>) -> Option<String> {
    match paths::worktree_root_within(cwd, GIT_TIMEOUT) {
        Ok(root) => Some(scope::worktree_key(&root)),
        Err(PathError::NotARepository { .. }) => None,
        Err(error) => {
            notes.push(format!(
                "the worktree could not be found: {}",
                reason(&error)
            ));
            None
        }
    }
}
