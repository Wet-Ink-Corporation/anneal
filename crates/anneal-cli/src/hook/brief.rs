//! `anneal hook brief`: the `PreToolUse` gate on the Agent tool. It fails closed.
//!
//! No brief, no spawn: an Agent call's prompt must name, on one `Brief: <path>` line, a brief
//! under `.anneal/briefs/` of the worktree (`reference`) that passes `lint brief`. And a
//! subagent spawns nothing: a call whose input carries `agent_id` (it runs inside a subagent)
//! is denied unless its `agent_type` is one of the orchestrate leads (`orchestrate.leads` in
//! `.anneal/config.json`, default `orchestrate-lead`). Any other tool is allowed silently.
//!
//! A denial is Claude Code's deny decision whose reason is one `ERROR brief:` line, plus a
//! `gate_blocked` event (`brief.<check>`) in an existing ledger; the hook never creates one.
//! Input it cannot read, a worktree, setting or brief it cannot read, deny. git and the ledger
//! get [`HOOK_BUDGET`] from the hook's start, each git call at most [`GIT_TIMEOUT`]. See
//! `docs/hooks.md` and `docs/brief.md`.

mod reference;

use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use serde::Deserialize;

use crate::error_chain::reason;
use crate::existing_ledger::{self, LedgerError};
use crate::hook::guard::{BUSY_TIMEOUT, GIT_TIMEOUT};
use crate::hook::{HookEnv, StdinError, read_stdin, write_deny};
use crate::lint::brief::{BriefFinding, Rule, check};
use crate::paths::{self, PathError};
use crate::pr_diff::Budget;
use crate::repo_config;
use reference::Refusal;

/// The most stdin the hook reads: an Agent prompt can be long (H-13).
const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024;

/// The most bytes of a brief read; a larger one is denied.
const MAX_BRIEF_BYTES: u64 = 256 * 1024;

/// What git and the ledger reads may start within, from the hook's start: the 5 s hook
/// `timeout` less the record's lock wait and process start-up.
const HOOK_BUDGET: Duration = Duration::from_millis(3_500);

/// The tool the hook judges.
const AGENT_TOOL: &str = "Agent";

/// What a denial is about; its id names the `gate_blocked` point (`brief.<id>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    /// The hook input could not be read.
    BadInput,
    /// A subagent that is not an orchestrate lead tried to spawn.
    Nested,
    /// The prompt names no brief, or more than one.
    NoBrief,
    /// The brief's path is refused or names nothing.
    Path,
    /// The brief fails `lint brief`, is too large or is not a file.
    Invalid,
    /// The worktree or the settings could not be read.
    Unjudged,
}

impl Check {
    const fn id(self) -> &'static str {
        match self {
            Self::BadInput => "bad-input",
            Self::Nested => "nested",
            Self::NoBrief => "no-brief",
            Self::Path => "path",
            Self::Invalid => "invalid",
            Self::Unjudged => "unjudged",
        }
    }
}

/// Why a spawn is denied.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Denial {
    check: Check,
    /// One line, starting `brief: `.
    reason: String,
}

impl Denial {
    fn new(check: Check, reason: &str) -> Self {
        Self {
            check,
            reason: format!("brief: {reason}"),
        }
    }

    fn unjudged(what: &str, error: &dyn Error) -> Self {
        Self::new(
            Check::Unjudged,
            &format!("{what} ({}), so the spawn cannot be checked", reason(error)),
        )
    }
}

/// The fields of Claude Code's `PreToolUse` input the hook uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    tool_name: String,
    #[serde(default, rename = "tool_input")]
    tool: ToolInput,
    /// Set when the call runs inside a subagent.
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_type: Option<String>,
}

/// The Agent tool's input; other fields are ignored.
#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    #[serde(default)]
    prompt: Option<String>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("the input is not PreToolUse JSON")]
    Json(#[source] serde_json::Error),
}

/// Why a denial could not be recorded in the ledger.
#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error(transparent)]
    Ledger(LedgerError),
    #[error("reading the clock")]
    Clock(#[source] anneal_ledger::TimestampError),
    #[error("appending gate_blocked")]
    Append(#[source] anneal_ledger::StoreError),
}

/// Runs the gate: reads the input from `env.stdin`, judges an Agent call, and on a denial
/// records `gate_blocked` (in the ledger seen from the input's `cwd`, or `env.current_dir`)
/// and writes the deny decision to `out`. Pass `&mut out` to keep the writer.
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
    let budget = Budget::within(Instant::now(), HOOK_BUDGET, GIT_TIMEOUT);
    let HookEnv {
        stdin,
        current_dir,
        lookup,
    } = env;
    let (denial, cwd, session) = match read_input(stdin) {
        Ok(mut input) => {
            let session = input.session_id.take();
            let cwd = input.cwd.take().map_or_else(current_dir, Ok);
            let Some(denial) = judge(input, cwd.as_deref(), budget) else {
                return Ok(());
            };
            (denial, cwd, session)
        }
        Err(error) => {
            let denial = Denial::new(
                Check::BadInput,
                &format!("the hook input cannot be read: {}", reason(&error)),
            );
            (denial, current_dir(), None)
        }
    };
    let ledger = cwd.map_err(LedgerError::CurrentDir).and_then(|cwd| {
        existing_ledger::open_existing(
            &cwd,
            lookup,
            budget.cap(GIT_TIMEOUT),
            budget.cap(BUSY_TIMEOUT),
        )
    });
    let unrecorded = match ledger {
        Ok(Some(store)) => record(&store, &denial, session).err(),
        Ok(None) => None,
        Err(error) => Some(RecordError::Ledger(error)),
    };
    let line = match unrecorded {
        None => format!("ERROR {}", denial.reason),
        Some(error) => format!(
            "ERROR {} (the block was not recorded in the ledger: {})",
            denial.reason,
            reason(&error)
        ),
    };
    write_deny(out, &line)
}

/// Reads and parses at most [`MAX_INPUT_BYTES`] of `stdin`.
fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// Judges `input` run from `cwd`: `None` to allow, else the denial.
fn judge(input: Input, cwd: Result<&Path, &io::Error>, budget: Budget) -> Option<Denial> {
    if input.tool_name != AGENT_TOOL {
        return None;
    }
    let cwd = match cwd {
        Ok(cwd) => cwd,
        Err(error) => {
            return Some(Denial::unjudged(
                "the working directory cannot be read",
                error,
            ));
        }
    };
    let root = match worktree_root(cwd, budget) {
        Ok(root) => root,
        Err(denial) => return Some(denial),
    };
    if input.agent_id.as_deref().is_some_and(|id| !id.is_empty())
        && let Some(denial) = nested(&root, input.agent_type.as_deref())
    {
        return Some(denial);
    }
    let Some(prompt) = input.tool.prompt else {
        return Some(refused(Check::NoBrief, &Refusal::None));
    };
    validate(&root, &prompt).err()
}

/// The worktree root seen from `cwd`, within `budget`.
fn worktree_root(cwd: &Path, budget: Budget) -> Result<PathBuf, Denial> {
    let timeout = budget
        .next()
        .map_err(|error| Denial::unjudged("no time left to find the worktree", &error))?;
    match paths::worktree_root_within(cwd, timeout) {
        Ok(root) => Ok(PathBuf::from(root)),
        Err(PathError::NotARepository { .. }) => Err(Denial::new(
            Check::Unjudged,
            "not in a git worktree, so there is no .anneal/briefs/ to read a brief from",
        )),
        Err(error) => Err(Denial::unjudged("the worktree cannot be found", &error)),
    }
}

/// The denial for a spawn from inside a subagent of type `agent_type`, unless it is an
/// orchestrate lead of the worktree `root`.
fn nested(root: &Path, agent_type: Option<&str>) -> Option<Denial> {
    let leads = match repo_config::orchestrate_leads(root) {
        Ok(leads) => leads,
        Err(error) => {
            return Some(Denial::unjudged(
                "the orchestrate leads cannot be read",
                &error,
            ));
        }
    };
    let agent = agent_type.unwrap_or("");
    if leads.iter().any(|lead| lead == agent) {
        return None;
    }
    Some(Denial::new(
        Check::Nested,
        &format!(
            "a subagent ({agent:?}) may not spawn subagents; only the orchestrate leads may \
             (orchestrate.leads in .anneal/config.json: {})",
            leads.join(", ")
        ),
    ))
}

/// Checks that `prompt` names a valid brief in the worktree `root`.
fn validate(root: &Path, prompt: &str) -> Result<(), Denial> {
    let text = reference::find(prompt).map_err(|refusal| refused(Check::NoBrief, &refusal))?;
    let path = reference::resolve(root, text).map_err(|refusal| refused(Check::Path, &refusal))?;
    let brief = read_brief(&path)
        .map_err(|why| Denial::new(Check::Invalid, &format!("the brief {text} {why}")))?;
    let findings = check(&brief);
    if findings.is_empty() {
        return Ok(());
    }
    Err(Denial::new(
        Check::Invalid,
        &format!(
            "the brief {text} fails lint brief: {}; fill them (anneal lint brief {text})",
            summary(&findings)
        ),
    ))
}

/// The denial for a `refusal` of the brief reference.
fn refused(check: Check, refusal: &Refusal) -> Denial {
    Denial::new(check, &refusal.reason())
}

/// The fields each rule found, as `missing: Goal, Invariants; empty: Verification`.
fn summary(findings: &[BriefFinding]) -> String {
    [
        (Rule::Missing, "missing"),
        (Rule::Empty, "empty"),
        (Rule::Repeated, "repeated"),
    ]
    .into_iter()
    .filter_map(|(rule, label)| {
        let fields: Vec<&str> = findings
            .iter()
            .filter(|finding| finding.rule == rule)
            .map(|finding| finding.field)
            .collect();
        (!fields.is_empty()).then(|| format!("{label}: {}", fields.join(", ")))
    })
    .collect::<Vec<String>>()
    .join("; ")
}

/// The text of the brief at `path` (lossy UTF-8), a regular file of at most
/// [`MAX_BRIEF_BYTES`]; else why not, as the end of a sentence.
fn read_brief(path: &Path) -> Result<String, String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err("is not a regular file".to_owned()),
        Err(error) => return Err(format!("cannot be read: {error}")),
    }
    let file = File::open(path).map_err(|error| format!("cannot be read: {error}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_BRIEF_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot be read: {error}"))?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_BRIEF_BYTES) {
        return Err(format!("is larger than {MAX_BRIEF_BYTES} bytes"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Appends the `gate_blocked` for `denial` to `store`.
fn record(store: &Store, denial: &Denial, session: Option<String>) -> Result<(), RecordError> {
    let event = LedgerEvent::GateBlocked(GateBlocked {
        point: format!("brief.{}", denial.check.id()),
        // Duplicated on purpose: the event keeps the reason and the deny line still writes it.
        reason: denial.reason.clone(),
    });
    let mut envelope =
        EventEnvelope::new(event, "hook", Timestamp::now().map_err(RecordError::Clock)?);
    envelope.session_id = session;
    store
        .append(&envelope)
        .map(|_| ())
        .map_err(RecordError::Append)
}
