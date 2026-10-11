//! `anneal hook fast-check`: the cheap checks, once per batch of edits (`PostToolBatch`). It
//! fails open.
//!
//! Claude Code fires `PostToolBatch` once after every call of a batch of parallel tool calls
//! has resolved, so ten parallel edits run these checks once. When the batch edited a file and
//! the session's worktree is in an anneal mode (`anneal mode build|diagnose`), it runs, within
//! a total budget under the hook's 15 s timeout:
//!
//! 1. `anneal floor-guard`: a lowered floor (exit 1) is the one result that blocks, as
//!    `{"decision": "block", "reason": "ERROR fast-check: ..."}`;
//! 2. the repository's `check:fast` command (`check_fast` in `.anneal/config.json`), 5 s;
//! 3. the glossary's avoided terms (`anneal lint terms`) on the changed files, when
//!    `CONTEXT.md` exists;
//! 4. `anneal lint spec` or `anneal lint plan` for each changed `.md` file under a `specs` or
//!    `plans` directory.
//!
//! Everything else that fails (a check, a timeout, the hook's own input) is a note for the
//! model in `additionalContext`, never a block. When every check passes it writes nothing.
//! The changed files are those the batch's `Edit`, `Write`, `MultiEdit` and `NotebookEdit` calls name;
//! an input that lists no calls falls back to `git status`. See `docs/hooks.md`.

mod batch;
mod checks;
mod config;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::hook::guard::GIT_TIMEOUT;
use crate::hook::preconditions::open_existing;
use crate::hook::{HookEnv, StdinError, read_stdin, reason};
use crate::mode;
use crate::paths::{self, PathError};
use batch::{Batch, Input, MAX_FILES};
use checks::{Anneal, Budget, Outcome};

/// The most stdin read: a batch of ten large Writes carries their contents. A larger input is
/// noted, and the changed files come from git.
const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;

/// The whole run, from reading stdin to the last check, under the entry's 15 s timeout.
const TOTAL_BUDGET: Duration = Duration::from_secs(13);
/// `anneal floor-guard`.
const FLOOR_GUARD_CAP: Duration = Duration::from_secs(6);
/// The repository's `check:fast`.
const CHECK_FAST_CAP: Duration = Duration::from_secs(5);
/// Each `anneal lint` run.
const LINT_CAP: Duration = Duration::from_secs(2);
/// `git status`, when the input lists no calls.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);
/// Most bytes of `git status` read.
const MAX_STATUS_BYTES: u64 = 1024 * 1024;

/// The glossary whose avoided terms are scanned for, at the worktree root.
const GLOSSARY: &str = "CONTEXT.md";

/// What the run found: at most one block, and notes.
#[derive(Debug, Default)]
struct Report {
    block: Option<String>,
    notes: Vec<String>,
}

impl Report {
    fn add(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Pass => {}
            Outcome::Block(reason) => self.block = Some(reason),
            Outcome::Note(note) => self.notes.push(note),
        }
    }

    fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }
}

/// Runs the hook: reads the `PostToolBatch` input from `env.stdin`, runs the checks for the
/// session's worktree (the input's `cwd`, or `env.current_dir`; the ledger found with
/// `env.lookup`), and writes the block or the notes to `out`, or nothing when all pass.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure is a note.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let budget = Budget::start(TOTAL_BUDGET);
    let report = check(env, budget);
    write_report(out, &report)
}

fn check<R, D, L>(env: HookEnv<R, D, L>, budget: Budget) -> Report
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
{
    let mut report = Report::default();
    let (input, unread) = match read_input(env.stdin) {
        Ok(input) => (input, None),
        Err(error) => (Input::default(), Some(error)),
    };
    let batch = input.batch();
    if batch == Batch::NoEdits {
        return report;
    }
    let cwd = match input.cwd {
        Some(cwd) => cwd,
        None => match (env.current_dir)() {
            Ok(cwd) => cwd,
            Err(error) => {
                report.note(format!(
                    "fast-check did not run: the working directory cannot be read ({error})"
                ));
                return report;
            }
        },
    };
    let root = match in_mode(&cwd, &env.lookup) {
        Ok(Some(root)) => root,
        Ok(None) => return report,
        Err(note) => {
            report.note(note);
            return report;
        }
    };
    if let Some(error) = unread {
        report.note(format!(
            "the hook input cannot be read ({}); the changed files come from git",
            reason(&error)
        ));
    }
    let files = changed_files(&root, &cwd, batch, &mut report);
    run_checks(&root, &files, budget, &mut report);
    report
}

/// Why the input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("the input is not PostToolBatch JSON")]
    Json(#[source] serde_json::Error),
}

fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The root of `cwd`'s worktree when it is in an anneal mode; `None` outside a repository,
/// with no ledger or no mode; the note when that cannot be told.
fn in_mode(
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<Option<PathBuf>, String> {
    let unknown = |what: &str, error: &dyn std::error::Error| {
        format!(
            "fast-check did not run: {what} ({}), so the anneal mode is unknown",
            reason(error)
        )
    };
    let root = match paths::worktree_root_within(cwd, GIT_TIMEOUT) {
        Ok(root) => root,
        Err(PathError::NotARepository { .. }) => return Ok(None),
        Err(error) => return Err(unknown("the worktree cannot be found", &error)),
    };
    let store = match open_existing(Path::new(&root), lookup) {
        Ok(Some(store)) => store,
        Ok(None) => return Ok(None),
        Err(error) => return Err(unknown("the ledger cannot be read", &error)),
    };
    match mode::active(&store, &root) {
        Ok(Some(_)) => Ok(Some(PathBuf::from(root))),
        Ok(None) => Ok(None),
        Err(error) => Err(unknown("the ledger cannot be read", &error)),
    }
}

/// The changed files inside the worktree `root`, relative to it and `/`-separated: the batch's
/// (named relative to `cwd`), or git's when the input lists none.
fn changed_files(root: &Path, cwd: &Path, batch: Batch, report: &mut Report) -> Vec<String> {
    let (named, omitted) = match batch {
        Batch::NoEdits => return Vec::new(),
        Batch::Files { files, omitted } => (files, omitted),
        Batch::Unlisted => match git_changes(root) {
            Ok(changes) => changes,
            Err(error) => {
                report.note(format!(
                    "the changed files cannot be listed ({}); only floor-guard and check_fast ran",
                    reason(&error)
                ));
                return Vec::new();
            }
        },
    };
    if omitted > 0 {
        report.note(format!(
            "{omitted} more changed files than the {MAX_FILES} scanned were not checked"
        ));
    }
    let mut files = Vec::with_capacity(named.len());
    for file in named {
        match mode::worktree_relative(root, &cwd.join(&file)) {
            Ok(Some(relative)) if !relative.is_empty() => files.push(relative),
            // Outside the worktree (or the root itself): not this worktree's to check.
            Ok(_) => {}
            Err(error) => report.note(format!("{file} cannot be resolved ({error}); not checked")),
        }
    }
    files
}

/// The changed and untracked files `git status` lists, at most [`MAX_FILES`], and how many
/// more there were.
fn git_changes(root: &Path) -> Result<(Vec<String>, usize), PathError> {
    let args = [
        "-c",
        "core.quotePath=false",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
    ];
    let output = paths::run_git_capped(root, &args, STATUS_TIMEOUT, MAX_STATUS_BYTES)?;
    if !output.status.success() {
        return Err(paths::git_failure(root, &output.stderr));
    }
    let mut files = batch::porcelain(&output.stdout);
    if u64::try_from(output.stdout.len()).map_or(true, |len| len >= MAX_STATUS_BYTES) {
        // The last record may be cut short.
        files.pop();
    }
    let omitted = files.len().saturating_sub(MAX_FILES);
    files.truncate(MAX_FILES);
    Ok((files, omitted))
}

/// Runs each check in order within `budget`.
fn run_checks(root: &Path, files: &[String], budget: Budget, report: &mut Report) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            report.note(format!(
                "fast-check did not run: the anneal binary cannot be found ({error})"
            ));
            return;
        }
    };
    let anneal = Anneal { exe: &exe, root };
    let granted = |what: &str, cap: Duration, report: &mut Report| {
        let grant = budget.grant(cap);
        if grant.is_none() {
            report.note(format!(
                "{what} did not run: the {} s budget was spent",
                budget.total().as_secs()
            ));
        }
        grant
    };
    if let Some(timeout) = granted("anneal floor-guard", FLOOR_GUARD_CAP, report) {
        report.add(checks::floor_guard(anneal, timeout));
    }
    match config::read(root) {
        Ok(Some(argv)) => {
            if let Some(timeout) = granted("check_fast", CHECK_FAST_CAP, report) {
                report.add(checks::check_fast(&argv, root, timeout));
            }
        }
        Ok(None) => {}
        Err(error) => report.note(format!("check_fast did not run: {}", reason(&error))),
    }
    let existing: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|file| root.join(file).is_file())
        .collect();
    if !existing.is_empty() && root.join(GLOSSARY).is_file() {
        let what = format!("avoided terms ({GLOSSARY})");
        if let Some(timeout) = granted(&what, LINT_CAP, report) {
            let mut args = vec!["lint", "terms", "--glossary", GLOSSARY, "--"];
            args.extend(&existing);
            report.add(checks::lint(anneal, &what, &args, timeout));
        }
    }
    for file in existing {
        let Some(target) = lint_target(file) else {
            continue;
        };
        let what = format!("anneal lint {target} {file}");
        if let Some(timeout) = granted(&what, LINT_CAP, report) {
            let args = ["lint", target, "--", file];
            report.add(checks::lint(anneal, &what, &args, timeout));
        }
    }
}

/// `spec` or `plan` for a `.md` file whose nearest `specs` or `plans` directory names it.
fn lint_target(file: &str) -> Option<&'static str> {
    let (dirs, name) = file.rsplit_once('/')?;
    if !name.to_ascii_lowercase().ends_with(".md") {
        return None;
    }
    dirs.rsplit('/').find_map(|dir| match dir {
        "specs" => Some("spec"),
        "plans" => Some("plan"),
        _ => None,
    })
}

/// `{"decision": "block", "reason": ..., "hookSpecificOutput": {...}}`, without the parts that
/// are empty.
#[derive(Debug, Serialize)]
struct Output<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    specific: Option<Specific>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Specific {
    hook_event_name: &'static str,
    additional_context: String,
}

/// Writes the report: nothing when it is empty.
fn write_report<W: Write>(mut out: W, report: &Report) -> io::Result<()> {
    if report.block.is_none() && report.notes.is_empty() {
        return Ok(());
    }
    let reason = report
        .block
        .as_deref()
        .map(|line| line.replace(['\r', '\n'], " "));
    let specific = (!report.notes.is_empty()).then(|| Specific {
        hook_event_name: "PostToolBatch",
        additional_context: format!("anneal fast-check:\n- {}", report.notes.join("\n- ")),
    });
    let output = Output {
        decision: reason.as_ref().map(|_| "block"),
        reason: reason.as_deref(),
        specific,
    };
    serde_json::to_writer(&mut out, &output)?;
    writeln!(out)
}

#[cfg(test)]
mod tests;
