//! `anneal hook preconditions`: the `PreToolUse` gate on Edit, Write, `MultiEdit` and
//! `NotebookEdit`. It fails closed.
//!
//! It reads Claude Code's `PreToolUse` input on stdin and judges every file the call names
//! (`input`: `file_path`, the `notebook_path` of `NotebookEdit`, the `file_path` of each edit
//! of `MultiEdit`; the first denial is the answer). Each is judged everywhere it may be
//! written: where the OS writes it (`mode::resolve`: `..` after a symlink climbs from its
//! target, a dangling symlink leads to its target) and where a tool that takes `..` lexically
//! first writes it; either reading's denial denies. Each reading is judged in its own worktree
//! (from the longest existing ancestor directory of where it lands, not the session's working
//! directory) and that worktree's anneal mode in the ledger (`mode::active`), each worktree
//! found and read once per call (`worktrees`), all within [`JUDGE_BUDGET`], past which the edit
//! is denied. A file in no repository, no ledger, no mode or mode `off` is allowed
//! silently, and it never creates a ledger; the one exception is `.anneal/config.json`,
//! anneal's own settings, which it denies in every mode and none (`Check::Settings`). In a
//! mode it allows test files, the build's plan and design tree, scratch directories and prose
//! documents (the spec among them when it is one) (`mode::scope`), and judges every other file
//! as source:
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
mod input;
mod worktrees;

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{
    EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, Store, StoreError, TagKey, Timestamp,
    TimestampError,
};

use crate::error_chain::reason;
use crate::existing_ledger::LedgerError;
use crate::hook::guard::{BUSY_TIMEOUT, GIT_TIMEOUT, MAX_INPUT_BYTES};
use crate::hook::{HookEnv, StdinError, read_stdin, write_deny};
use crate::mode::scope;
use crate::mode::{self, Active, Entered};
use crate::pr_diff::Budget;

use self::input::{Input, Tool};
use self::worktrees::{Worktrees, open_within};

/// What git and the ledger may take, from the hook's start: the 5 s hook `timeout` less the
/// record's lock wait and process start-up, as the brief hook's. Every file of a call, and
/// every reading of it, is judged within it; past it the edit is denied. Not the shared
/// [`crate::hook::HOOK_BUDGET`]: this hook keeps its own.
const JUDGE_BUDGET: Duration = Duration::from_millis(3_500);

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
    #[error("no time was left to open it")]
    OutOfTime,
    #[error("reading the clock")]
    Clock(#[source] TimestampError),
    #[error("appending gate_blocked")]
    Append(#[source] StoreError),
}

/// The ledger a denial is recorded in: none when no ledger exists (the hook never creates one),
/// or why it could not be opened.
type Ledger = Result<Option<Store>, RecordError>;

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
    let budget = Budget::within(Instant::now(), JUDGE_BUDGET, GIT_TIMEOUT);
    let HookEnv {
        stdin,
        current_dir,
        lookup,
    } = env;
    let (verdict, session) = match read_input(stdin) {
        Ok(mut input) => {
            let session = input.session_id.take();
            (judge(input, current_dir, &lookup, budget), session)
        }
        Err(error) => (bad_input(&error, current_dir, &lookup, budget), None),
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
        Err(error) => Some(error),
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
    budget: Budget,
) -> Verdict {
    let denial = Denial::new(
        Check::BadInput,
        format!(
            "preconditions: the hook input cannot be read: {}",
            reason(error)
        ),
    );
    let ledger = match current_dir() {
        Ok(cwd) => open_within(&cwd, lookup, budget),
        Err(error) => Err(RecordError::Ledger(LedgerError::CurrentDir(error))),
    };
    Verdict::deny(denial, ledger)
}

/// Judges `input` within `budget`: allow, or the first missing precondition of the mode of a
/// worktree a file lands in.
fn judge(
    input: Input,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
    budget: Budget,
) -> Verdict {
    let Some(tool) = Tool::named(&input.tool_name) else {
        return Verdict::allow();
    };
    let cwd = match input.cwd {
        Some(cwd) => cwd,
        None => match current_dir() {
            Ok(cwd) => cwd,
            Err(error) => {
                let denial = unjudged("the working directory cannot be read", &error);
                let ledger = Err(RecordError::Ledger(LedgerError::CurrentDir(error)));
                return Verdict::deny(denial, ledger);
            }
        },
    };
    let paths = match input.tool.paths(tool, &input.tool_name) {
        Ok(paths) => paths,
        Err(denial) => return Verdict::deny(denial, open_within(&cwd, &lookup, budget)),
    };
    // Every file the call writes is judged, each worktree read once; the first denial is the
    // answer.
    let mut worktrees = Worktrees::new(&lookup, budget);
    for path in paths {
        if let Some(verdict) = judge_path(&cwd.join(path), &mut worktrees) {
            return verdict;
        }
    }
    Verdict::allow()
}

/// Judges an edit of `target` (absolute) in the worktrees it may land in: `None` to allow, or
/// the denial.
fn judge_path<L>(target: &Path, worktrees: &mut Worktrees<L>) -> Option<Verdict>
where
    L: Fn(&str) -> Option<OsString>,
{
    // Where the OS writes: `..` after a symlink, and a dangling symlink, lead elsewhere than
    // the name says. A path that cannot be resolved is judged unjudged once a mode is found.
    let resolved = mode::resolve::resolve(target);
    // Where a tool writes that takes `..` lexically before the OS sees the path: with
    // `src/link -> ../tmp/sub`, `src/link/../x.py` is `tmp/x.py` to the OS but `src/x.py` to
    // such a tool. Both readings are judged, and either's denial denies.
    let normalised = mode::resolve::resolve(&lexically_normalised(target));
    let lexical = lexical(target);
    // The edited file's own worktree decides, wherever the session runs: a session in a
    // sibling worktree, or above the repository, writes into a gated worktree all the same.
    // It is found from where the file lands, or from its name when that cannot be resolved.
    let anchor = match anchor_of(target, &resolved, &lexical) {
        Ok(anchor) => anchor,
        Err(denial) => return Some(Verdict::deny(denial, Ok(None))),
    };
    // Before any ledger or mode: the settings name the agents trusted to spawn and the merge
    // gate's rules, so an agent may never write them, in a mode or not.
    if is_settings(&lexical, &[&resolved, &normalised]) {
        let denial = Denial::new(Check::Settings, SETTINGS_DENIED);
        return Some(Verdict::deny(denial, worktrees.ledger_at(&anchor)));
    }
    if let Some(verdict) = worktrees.judge(&anchor, target, resolved.as_deref()) {
        return Some(verdict);
    }
    if same_place(&resolved, &normalised) {
        return None;
    }
    let anchor = match anchor_of(target, &normalised, &lexical) {
        Ok(anchor) => anchor,
        Err(denial) => return Some(Verdict::deny(denial, Ok(None))),
    };
    worktrees.judge(&anchor, target, normalised.as_deref())
}

/// The longest existing ancestor directory of `resolved`, where `target` lands, or of its
/// `lexical` name when it cannot be resolved; the denial when none exists.
fn anchor_of(
    target: &Path,
    resolved: &io::Result<PathBuf>,
    lexical: &Path,
) -> Result<PathBuf, Denial> {
    existing_ancestor(resolved.as_deref().unwrap_or(lexical)).ok_or_else(|| {
        Denial::new(
            Check::Unjudged,
            format!(
                "preconditions: no directory of {} exists, so the edit cannot be checked",
                target.display()
            ),
        )
    })
}

/// Whether two readings of a path are judged alike: the same place, or neither resolved (both
/// then judged from the path's lexical name).
fn same_place(one: &io::Result<PathBuf>, other: &io::Result<PathBuf>) -> bool {
    match (one, other) {
        (Ok(one), Ok(other)) => one == other,
        (Err(_), Err(_)) => true,
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => false,
    }
}

/// `target` (absolute) with `.` and `..` taken lexically.
fn lexical(target: &Path) -> PathBuf {
    PathBuf::from(scope::normalise(&target.to_string_lossy()))
}

/// `target` (absolute) as a tool that normalises paths before the OS sees them writes it: `.`
/// dropped and `..` the parent of the components before it, symlinks not followed. Unlike
/// [`lexical`] it keeps the platform's separators and prefixes, so the OS can resolve it.
fn lexically_normalised(target: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // At the root there is no parent: `/..` is `/`.
                normal.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normal.push(component.as_os_str());
            }
        }
    }
    normal
}

/// The longest existing directory among `path` (absolute, without `.` or `..`) and its
/// ancestors: where git is asked which worktree the edit is in.
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
}

/// Whether the edited file, `lexical` as written after `.` and `..` and `readings` where it
/// may be written ([`mode::resolve::resolve`] of the path and of its lexical normalisation,
/// those that resolve), is a `.anneal/config.json`: as written, as any reading, as the file
/// resolves when it exists, or as its longest existing ancestor directory resolves with the
/// rest of the path after it (a symlinked directory). Any directory's counts, not only the
/// worktree root's, since this is judged before git is asked.
fn is_settings(lexical: &Path, readings: &[&io::Result<PathBuf>]) -> bool {
    let through_anchor = || {
        let anchor = existing_ancestor(lexical)?;
        let rest = lexical.strip_prefix(&anchor).ok()?;
        Some(std::fs::canonicalize(&anchor).ok()?.join(rest))
    };
    names_settings(lexical)
        || readings
            .iter()
            .any(|reading| reading.as_deref().is_ok_and(names_settings))
        || std::fs::canonicalize(lexical).is_ok_and(|real| names_settings(&real))
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

/// The denial for a gated edit in the worktree whose top-level directory is `root`, given its
/// ledger `store` and the mode `entered`: `None` when the mode's preconditions hold. They are
/// the same for every gated file of the worktree.
fn mode_denial(store: &Store, root: &str, entered: &Entered) -> Option<Denial> {
    match &entered.mode {
        Active::Build(build) => build::check(store, Path::new(root), build),
        Active::IncompleteBuild => Some(Denial::new(
            Check::Mode,
            "build mode: the mode was recorded without a plan or design tree; run `anneal \
             mode build --plan <plan> --tree <design tree>`",
        )),
        Active::Diagnose => diagnose(store, root, entered.position),
    }
}

/// `resolved`, where `target` is written, relative to the worktree whose top-level
/// directory is `root`; the denial when it cannot be resolved or lies outside that worktree.
fn relative_in(
    root: &str,
    target: &Path,
    resolved: Result<&Path, &io::Error>,
) -> Result<String, Denial> {
    let real = resolved.map_err(|error| unjudged("the edited path cannot be resolved", error))?;
    match mode::resolved_relative(Path::new(root), real) {
        Ok(Some(relative)) => Ok(relative),
        Ok(None) => Err(Denial::new(
            Check::Unjudged,
            format!(
                "preconditions: {} is not inside its own worktree {root} once resolved, so the \
                 edit cannot be checked",
                target.display()
            ),
        )),
        Err(error) => Err(unjudged("the worktree root cannot be resolved", &error)),
    }
}

/// The denial for a source edit in the worktree whose top-level directory is `root`, in
/// diagnose mode entered at ledger position `since`, unless a `repro_red` tagged with that
/// worktree's key was recorded after it.
fn diagnose(store: &Store, root: &str, since: u64) -> Option<Denial> {
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

#[cfg(test)]
mod tests;
