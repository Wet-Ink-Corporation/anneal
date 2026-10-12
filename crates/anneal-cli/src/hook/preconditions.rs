//! `anneal hook preconditions`: the `PreToolUse` gate on Edit and Write. It fails closed.
//!
//! It reads Claude Code's `PreToolUse` input on stdin and finds the edited file's own
//! worktree (from the file's longest existing ancestor directory, not the session's working
//! directory) and that worktree's anneal mode in the ledger (`mode::active`). A file in no
//! repository, no ledger, no mode or mode `off` is allowed silently, and it never creates a
//! ledger; the one exception is `.anneal/config.json`, anneal's own settings, which it denies
//! in every mode and none (`Check::Settings`). In a mode it allows test files, the build's
//! plan and design tree, scratch directories and prose documents (the spec among them when it
//! is one) (`mode::scope`), and judges every other file as source:
//!
//! - build mode (`build`): the plan's spec passes `lint spec`, the design tree is confirmed at
//!   its current hash, the tree was clean at entry and the plan is approved and current;
//! - diagnose mode: a `repro_red` tagged with the worktree's key (`anneal repro record`) was
//!   recorded after the mode was entered.
//!
//! A denial is Claude Code's deny decision whose reason is one `ERROR` line naming the missing
//! precondition and the command that satisfies it, plus a `gate_blocked` event. Input it cannot
//! read, and anything it cannot judge once a ledger exists (the ledger, git, the file system),
//! is denied. See `docs/preconditions.md`.

mod build;

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{
    EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, Store, StoreError, TagKey, Timestamp,
    TimestampError,
};
use serde::Deserialize;

use crate::error_chain::reason;
use crate::existing_ledger::{self, LedgerError};
use crate::hook::guard::{BUSY_TIMEOUT, GIT_TIMEOUT, MAX_INPUT_BYTES};
use crate::hook::{HookEnv, StdinError, read_stdin, write_deny};
use crate::mode::scope::{self, PathKind};
use crate::mode::{self, Active};
use crate::paths::{self, PathError};

/// The tools whose calls are judged; any other is allowed.
const GATED_TOOLS: [&str; 2] = ["Edit", "Write"];

/// The `point` of the hook's `gate_blocked` events, before the check's id.
const POINT_PREFIX: &str = "preconditions.";

/// The directory and file name of anneal's own settings, `.anneal/config.json`.
const SETTINGS: [&str; 2] = [".anneal", "config.json"];

/// The reason an agent's edit of `.anneal/config.json` is denied.
const SETTINGS_DENIED: &str = "preconditions: .anneal/config.json is anneal's own settings (the \
                               orchestrate leads, the merge gate's keys, check_fast), which an \
                               agent may not edit; edit it yourself";

/// The precondition a denial names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// Build: the plan links a spec that passes `lint spec`.
    Spec,
    /// Build: the design tree is confirmed at its current hash.
    Grill,
    /// Build: the working tree was clean when build mode was entered.
    Clean,
    /// Build: the plan can be read, and is approved and current.
    Plan,
    /// Build mode was recorded without a plan or design tree.
    Mode,
    /// Diagnose: a red reproduction was recorded after the mode was entered.
    Repro,
    /// The hook input could not be read.
    BadInput,
    /// The edit could not be judged: the ledger, git or the file system failed.
    Unjudged,
    /// Any mode, or none: the file is `.anneal/config.json`, anneal's own settings.
    Settings,
}

impl Check {
    /// The check's id, as in the `gate_blocked` point.
    const fn id(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::Grill => "grill",
            Self::Clean => "clean",
            Self::Plan => "plan",
            Self::Mode => "mode",
            Self::Repro => "repro",
            Self::BadInput => "bad-input",
            Self::Unjudged => "unjudged",
            Self::Settings => "settings",
        }
    }
}

/// Why an edit is denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denial {
    /// The precondition that is missing.
    check: Check,
    /// One line naming it and the command that satisfies it.
    reason: String,
}

impl Denial {
    pub(crate) fn new(check: Check, reason: impl Into<String>) -> Self {
        Self {
            check,
            reason: reason.into(),
        }
    }
}

/// The denial for an edit that could not be judged because `what` failed with `error`.
pub(crate) fn unjudged(what: &str, error: &dyn Error) -> Denial {
    Denial::new(
        Check::Unjudged,
        format!(
            "preconditions: {what} ({}), so the edit cannot be checked; fix that, then try \
             again",
            reason(error)
        ),
    )
}

/// The fields of Claude Code's `PreToolUse` input the hook uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    /// Where the ledger and the worktree are found, and relative paths resolved; the process's
    /// working directory when absent.
    #[serde(default)]
    cwd: Option<PathBuf>,
    tool_name: String,
    #[serde(default, rename = "tool_input")]
    tool: ToolInput,
}

/// The Edit and Write tools' input; other fields are ignored.
#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    #[serde(default)]
    file_path: Option<PathBuf>,
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
}

/// Why a denial could not be recorded in the ledger.
#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error(transparent)]
    Ledger(LedgerError),
    #[error("reading the clock")]
    Clock(#[source] TimestampError),
    #[error("appending gate_blocked")]
    Append(#[source] StoreError),
}

/// The ledger a denial is recorded in: none when no ledger exists (the hook never creates one),
/// or why it could not be opened.
type Ledger = Result<Option<Store>, LedgerError>;

/// The hook's answer, and the ledger a denial is recorded in.
struct Verdict {
    denial: Option<Denial>,
    ledger: Ledger,
}

impl Verdict {
    const fn allow() -> Self {
        Self {
            denial: None,
            ledger: Ok(None),
        }
    }

    const fn deny(denial: Denial, ledger: Ledger) -> Self {
        Self {
            denial: Some(denial),
            ledger,
        }
    }
}

/// Runs the hook: reads the input from `env.stdin`, judges it against the worktree's mode
/// (found from the input's `cwd`, or `env.current_dir`, and `env.lookup`), and on a denial
/// records `gate_blocked` and writes the deny decision to `out`. Pass `&mut out` to keep the
/// writer.
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
    let HookEnv {
        stdin,
        current_dir,
        lookup,
    } = env;
    let (verdict, session) = match read_input(stdin) {
        Ok(mut input) => {
            let session = input.session_id.take();
            (judge(input, current_dir, &lookup), session)
        }
        Err(error) => (bad_input(&error, current_dir, &lookup), None),
    };
    let Verdict {
        denial: Some(denial),
        ledger,
    } = verdict
    else {
        return Ok(());
    };
    let unrecorded = match ledger {
        Ok(Some(store)) => record(&store, &denial, session).err(),
        Ok(None) => None,
        Err(error) => Some(RecordError::Ledger(error)),
    };
    write_denial(out, &denial, unrecorded.as_ref())
}

/// Reads and parses at most [`MAX_INPUT_BYTES`] of `stdin`.
fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The denial for unreadable input, recorded in the ledger seen from the process's working
/// directory when one already exists there (the input names no `cwd` that can be read).
fn bad_input(
    error: &InputError,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Verdict {
    let denial = Denial::new(
        Check::BadInput,
        format!(
            "preconditions: the hook input cannot be read: {}",
            reason(error)
        ),
    );
    let ledger = current_dir()
        .map_err(LedgerError::CurrentDir)
        .and_then(|cwd| open_existing(&cwd, lookup));
    Verdict::deny(denial, ledger)
}

/// Judges `input`: allow, or the first missing precondition of the worktree's mode.
fn judge(
    input: Input,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Verdict {
    if !GATED_TOOLS.contains(&input.tool_name.as_str()) {
        return Verdict::allow();
    }
    let cwd = match input.cwd {
        Some(cwd) => cwd,
        None => match current_dir() {
            Ok(cwd) => cwd,
            Err(error) => {
                let denial = unjudged("the working directory cannot be read", &error);
                return Verdict::deny(denial, Err(LedgerError::CurrentDir(error)));
            }
        },
    };
    let Some(file_path) = input.tool.file_path else {
        let denial = Denial::new(
            Check::BadInput,
            format!(
                "preconditions: the hook input cannot be read: the {} input has no \
                 tool_input.file_path",
                input.tool_name
            ),
        );
        return Verdict::deny(denial, open_existing(&cwd, lookup));
    };
    let target = cwd.join(file_path);
    // The edited file's own worktree decides, wherever the session runs: a session in a
    // sibling worktree, or above the repository, writes into a gated worktree all the same.
    let Some(anchor) = existing_ancestor(&target) else {
        let denial = Denial::new(
            Check::Unjudged,
            format!(
                "preconditions: no directory of {} exists, so the edit cannot be checked",
                target.display()
            ),
        );
        return Verdict::deny(denial, Ok(None));
    };
    // Before any ledger or mode: the settings name the agents trusted to spawn and the merge
    // gate's rules, so an agent may never write them, in a mode or not.
    if is_settings(&target, &anchor) {
        let denial = Denial::new(Check::Settings, SETTINGS_DENIED);
        return Verdict::deny(denial, open_existing(&anchor, lookup));
    }
    let ledger = match open_existing(&anchor, lookup) {
        Ok(ledger) => ledger,
        Err(error) => {
            let denial = unjudged("the ledger cannot be read", &error);
            return Verdict::deny(denial, Err(error));
        }
    };
    let Some(store) = ledger else {
        // No ledger: no mode was ever entered in the edited file's repository.
        return Verdict::allow();
    };
    match judge_in_mode(&store, &anchor, &target) {
        Some(denial) => Verdict::deny(denial, Ok(Some(store))),
        None => Verdict::allow(),
    }
}

/// The longest existing directory among `target` (absolute) and its ancestors, after `.` and
/// `..` are taken lexically: where git is asked which worktree the edit is in.
fn existing_ancestor(target: &Path) -> Option<PathBuf> {
    let lexical = PathBuf::from(scope::normalise(&target.to_string_lossy()));
    lexical
        .ancestors()
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
}

/// Whether `target` (absolute), whose longest existing ancestor directory is `anchor`, is a
/// `.anneal/config.json`: as written after `.` and `..`, as the file resolves when it exists,
/// or as `anchor` resolves with the rest of the path after it (a symlinked directory). Any
/// directory's counts, not only the worktree root's, since this is judged before git is asked.
/// A path that does not resolve is judged as written: the edit tool would meet the same
/// failure.
fn is_settings(target: &Path, anchor: &Path) -> bool {
    let lexical = PathBuf::from(scope::normalise(&target.to_string_lossy()));
    let through_anchor = || {
        let rest = lexical.strip_prefix(anchor).ok()?;
        Some(std::fs::canonicalize(anchor).ok()?.join(rest))
    };
    names_settings(&lexical)
        || std::fs::canonicalize(&lexical).is_ok_and(|real| names_settings(&real))
        || through_anchor().is_some_and(|real| names_settings(&real))
}

/// Whether `path` ends in `.anneal/config.json`, in any ASCII letter case (APFS and NTFS are
/// case-insensitive by default) and with what Windows ignores in a name taken off: trailing
/// dots and spaces, and an alternate data stream (`config.json::$DATA`).
fn names_settings(path: &Path) -> bool {
    let names = [path.parent().and_then(Path::file_name), path.file_name()];
    names.iter().zip(SETTINGS).all(|(name, wanted)| {
        name.is_some_and(|name| {
            let name = name.to_string_lossy();
            let stem = name.split_once(':').map_or(&*name, |(stem, _stream)| stem);
            stem.trim_end_matches(['.', ' '])
                .eq_ignore_ascii_case(wanted)
        })
    })
}

/// The denial for an edit of `target` (absolute), whose longest existing ancestor directory is
/// `anchor`, given the ledger `store`: `None` when `target` is in no repository, with no mode
/// for its worktree, for a path the mode allows, or when the mode's preconditions hold.
fn judge_in_mode(store: &Store, anchor: &Path, target: &Path) -> Option<Denial> {
    let root = match paths::worktree_root_within(anchor, GIT_TIMEOUT) {
        Ok(root) => root,
        Err(PathError::NotARepository { .. }) => return None,
        Err(error) => return Some(unjudged("the worktree cannot be found", &error)),
    };
    let entered = match mode::active(store, &root) {
        Ok(entered) => entered?,
        Err(error) => return Some(unjudged("the ledger cannot be read", &error)),
    };
    let relative = match mode::worktree_relative(Path::new(&root), target) {
        Ok(Some(relative)) => relative,
        Ok(None) => {
            let denial = Denial::new(
                Check::Unjudged,
                format!(
                    "preconditions: {} is not inside its own worktree {root} once resolved, so \
                     the edit cannot be checked",
                    target.display()
                ),
            );
            return Some(denial);
        }
        Err(error) => return Some(unjudged("the edited path cannot be resolved", &error)),
    };
    match &entered.mode {
        Active::Build(build) => build::check(store, Path::new(&root), build, &relative),
        Active::IncompleteBuild => incomplete(&relative),
        Active::Diagnose => diagnose(store, &root, entered.position, &relative),
    }
}

/// The ledger seen from `cwd` when its file exists, within the hook's limits; `None` when it
/// does not (opening it would create it, and no ledger means no mode).
fn open_existing(
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<Option<Store>, LedgerError> {
    existing_ledger::open_existing(cwd, lookup, GIT_TIMEOUT, BUSY_TIMEOUT)
}

/// The denial for a source edit in a build mode recorded without a plan or design tree.
fn incomplete(relative: &str) -> Option<Denial> {
    (scope::classify(relative, &[]) == PathKind::Source).then(|| {
        Denial::new(
            Check::Mode,
            "build mode: the mode was recorded without a plan or design tree; run `anneal \
             mode build --plan <plan> --tree <design tree>`",
        )
    })
}

/// The denial for a source edit in the worktree whose top-level directory is `root`, in
/// diagnose mode entered at ledger position `since`, unless a `repro_red` tagged with that
/// worktree's key was recorded after it.
fn diagnose(store: &Store, root: &str, since: u64, relative: &str) -> Option<Denial> {
    if scope::classify(relative, &[]) != PathKind::Source {
        return None;
    }
    let missing = || {
        Denial::new(
            Check::Repro,
            "diagnose mode: no red reproduction is recorded in this worktree since diagnose \
             mode started; write a test that fails, then run `anneal repro record -- <test \
             command>`",
        )
    };
    // `anneal repro record` refuses a worktree the ledger cannot tag, so none has a repro.
    let Ok(worktree) = LedgerTag::new(TagKey::Worktree, &scope::worktree_key(root)) else {
        return Some(missing());
    };
    let repros = Filter {
        types: vec![EventKind::ReproRed],
        tags: vec![worktree],
    };
    match store.tail(&repros, 1) {
        Ok(latest) if latest.iter().any(|repro| repro.position > since) => None,
        Ok(_) => Some(missing()),
        Err(error) => Some(unjudged("the ledger cannot be read", &error)),
    }
}

/// Appends a `gate_blocked` event for `denial`, tagged with `session`, to `store`.
fn record(store: &Store, denial: &Denial, session: Option<String>) -> Result<(), RecordError> {
    let event = LedgerEvent::GateBlocked(GateBlocked {
        point: format!("{POINT_PREFIX}{}", denial.check.id()),
        // The reason is duplicated: the ledger keeps one copy, the deny decision shows the other.
        reason: denial.reason.clone(),
    });
    let mut envelope =
        EventEnvelope::new(event, "hook", Timestamp::now().map_err(RecordError::Clock)?);
    envelope.session_id = session;
    store
        .append(&envelope)
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
        None => format!("ERROR {}", denial.reason),
        Some(error) => format!(
            "ERROR {} (the block was not recorded in the ledger: {})",
            denial.reason,
            reason(error)
        ),
    };
    write_deny(out, &line)
}
