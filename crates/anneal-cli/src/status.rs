//! `anneal status [--json] [--fields <sections>] [--tail <n>]`: the one read model every UI
//! renders.
//!
//! It folds the ledger and the anneal home's state files into one versioned document
//! (`schema: 1`), so a UI (the future cockpit) holds no logic of its own. Each section is read by the module that owns its data, never re-implemented here:
//!
//! - `phase`, `plan`, `tasks`, `gates` and `ledger_tail` from the ledger (`status::ledger`):
//!   `mode::active`, `plan::approval_record`, `hook::handoff::facts`' task count, the newest
//!   `gate_blocked`/`stop_blocked` events and `facts::event_word`;
//! - `verdict` from `verdict::pull_request` against `HEAD` (`status::head_verdict`);
//! - `clef` and `context` from the anneal home (`status::home`): `decide`'s configuration and
//!   `counts.json`, and the fresher of the snapshots `anneal statusline` and the mod save
//!   (`context::Sources`).
//!
//! A section whose source does not exist is absent; one whose source exists but cannot be read
//! is absent too, with one `status/unreadable:<section>` finding saying why, and the exit code
//! is unchanged. `context` has two sources: when one cannot be read, the other's reading is
//! still shown beside the finding. `--fields` limits the work to the sections named:
//! `--fields phase,plan,gates` runs no git beyond finding the ledger and the worktree. Exit 0
//! with a ledger; 2 without one, still with the document, and no ledger is created. See
//! `docs/status.md`.

mod head_verdict;
mod home;
mod ledger;
mod text;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anneal_ledger::Store;
use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::error_chain::reason;
use crate::existing_ledger::{self, LedgerError};
use crate::exit_codes::{self, Code};
use crate::paths::{self, GIT_TIMEOUT, PathError};

/// The document's schema version. Fields are only ever added within schema 1.
pub const SCHEMA: u32 = 1;

/// How long a ledger statement waits for another process's lock: a UI refreshes often and
/// should not hang on a busy writer (H-13).
const LEDGER_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// The most events `--tail` may ask for (H-13).
const MAX_TAIL: u16 = 1000;

/// `anneal status`.
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Print the JSON envelope, whose `data` is the status document, instead of the text form
    #[arg(long)]
    json: bool,
    /// Only these sections, comma-separated [default: all]
    #[arg(long, value_enum, value_delimiter = ',', value_name = "SECTIONS")]
    fields: Vec<Section>,
    /// How many of the newest ledger events `ledger_tail` lists (0 to 1000)
    #[arg(
        long,
        value_name = "N",
        default_value_t = 10,
        value_parser = clap::value_parser!(u16).range(..=i64::from(MAX_TAIL))
    )]
    tail: u16,
}

/// A section of the status document, as `--fields` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Section {
    /// The worktree's anneal mode and its playbook.
    Phase,
    /// The build plan's approval.
    Plan,
    /// The build plan's tasks done and in all.
    Tasks,
    /// The newest gates that blocked.
    Gates,
    /// The verdict for `HEAD`.
    Verdict,
    /// Clef's decision points: decisions answered and mode.
    Clef,
    /// The latest context-usage snapshot.
    Context,
    /// The newest ledger events.
    #[value(name = "ledger_tail", alias = "ledger-tail")]
    LedgerTail,
}

impl Section {
    /// The section's key in the document and in its findings.
    const fn key(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Plan => "plan",
            Self::Tasks => "tasks",
            Self::Gates => "gates",
            Self::Verdict => "verdict",
            Self::Clef => "clef",
            Self::Context => "context",
            Self::LedgerTail => "ledger_tail",
        }
    }
}

impl StatusArgs {
    /// The sections asked for.
    fn wanted(&self) -> Wanted<'_> {
        Wanted {
            fields: &self.fields,
        }
    }
}

/// The sections asked for: all of them when `--fields` names none.
#[derive(Debug, Clone, Copy)]
struct Wanted<'a> {
    fields: &'a [Section],
}

impl Wanted<'_> {
    fn has(self, section: Section) -> bool {
        self.fields.is_empty() || self.fields.contains(&section)
    }

    fn any(self, sections: &[Section]) -> bool {
        sections.iter().any(|section| self.has(*section))
    }
}

/// The status document: `data` of the envelope. Every section is optional and absent when it
/// has no data.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Document {
    schema: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<ledger::Phase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<ledger::Plan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tasks: Option<ledger::Tasks>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gates: Option<Vec<ledger::Gate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<head_verdict::HeadVerdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clef: Option<home::Clef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<home::Context>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ledger_tail: Option<Vec<ledger::TailEvent>>,
}

/// The document as it is gathered, and the findings for the sections that could not be read.
#[derive(Debug, Default)]
struct Gathered {
    document: Document,
    findings: Vec<Finding>,
}

impl Gathered {
    /// Keeps `read`'s value, or notes why `section` could not be read and leaves it absent.
    fn keep<T>(&mut self, section: Section, read: Result<Option<T>, String>) -> Option<T> {
        match read {
            Ok(value) => value,
            Err(why) => {
                self.unreadable(section.key(), &why);
                None
            }
        }
    }

    fn unreadable(&mut self, what: &str, why: &str) {
        self.findings.push(Finding::new(
            format!("status/unreadable:{what}"),
            format!("{what} could not be read: {why}"),
        ));
    }
}

/// Runs `anneal status` in `cwd`, reading environment variables through `lookup`: 0 with a
/// ledger, 2 without one. It never creates a ledger.
///
/// # Errors
/// [`CliError::Path`] when git cannot say where the ledger is, [`CliError::Ledger`] when a
/// ledger file exists but cannot be opened, [`CliError::Output`] when `out` cannot be written.
/// Anything a section cannot read is a finding instead.
pub(crate) fn run<O: Write>(
    args: &StatusArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let store = existing_ledger::open_existing(cwd, &lookup, GIT_TIMEOUT, LEDGER_BUSY_TIMEOUT)
        .map_err(|error| match error {
            LedgerError::CurrentDir(source) => CliError::CurrentDir(source),
            LedgerError::Locate(source) => CliError::Path(source),
            LedgerError::Open(source) => CliError::Ledger(source),
        })?;
    let mut gathered = Gathered {
        document: Document {
            schema: SCHEMA,
            ..Document::default()
        },
        findings: Vec::new(),
    };
    let code = if let Some(store) = &store {
        from_ledger(store, cwd, args, &mut gathered);
        exit_codes::OK
    } else {
        gathered.findings.push(Finding::new(
            "status/no-ledger",
            "no ledger exists yet: nothing has been recorded here. fix: run an anneal command \
             that records, such as `anneal mode build`",
        ));
        exit_codes::STATUS_NO_LEDGER
    };
    from_home(&lookup, args.wanted(), &mut gathered);
    if args.json {
        Envelope::new(CommandName::Status, None, code, &gathered.document)
            .with_findings(&gathered.findings)
            .emit(out)
            .map_err(CliError::Output)
    } else {
        text::write(&gathered.document, &gathered.findings, out).map_err(CliError::Output)?;
        Ok(code)
    }
}

/// The sections read from the ledger, and the worktree-bound ones once its root is known.
fn from_ledger(store: &Store, cwd: &Path, args: &StatusArgs, gathered: &mut Gathered) {
    let wanted = args.wanted();
    if wanted.has(Section::Gates) {
        gathered.document.gates = gathered.keep(Section::Gates, ledger::gates(store).map(Some));
    }
    if wanted.has(Section::LedgerTail) {
        let read = ledger::tail(store, usize::from(args.tail)).map(Some);
        gathered.document.ledger_tail = gathered.keep(Section::LedgerTail, read);
    }
    let bound = [
        Section::Phase,
        Section::Plan,
        Section::Tasks,
        Section::Verdict,
    ];
    if !wanted.any(&bound) {
        return;
    }
    let root = match paths::worktree_root_within(cwd, GIT_TIMEOUT) {
        Ok(root) => root,
        // Outside a worktree there is no mode, no plan and no HEAD: those sections are absent.
        Err(PathError::NotARepository { .. }) => return,
        Err(error) => {
            gathered.unreadable("worktree", &reason(&error));
            return;
        }
    };
    let worktree = ledger::Worktree { store, root: &root };
    from_worktree(&worktree, wanted, gathered);
}

/// `phase`, `plan`, `tasks` and `verdict` of the worktree at `worktree.root`.
fn from_worktree(worktree: &ledger::Worktree<'_>, wanted: Wanted<'_>, gathered: &mut Gathered) {
    if wanted.any(&[Section::Phase, Section::Plan, Section::Tasks]) {
        let entered = gathered.keep(Section::Phase, worktree.mode());
        if let Some(build) = entered.as_ref().and_then(ledger::BuildPlan::of) {
            from_plan(worktree, &build, wanted, gathered);
        }
        if wanted.has(Section::Phase) {
            gathered.document.phase = entered.map(ledger::Phase::of);
        }
    }
    if wanted.has(Section::Verdict) {
        let read = head_verdict::read(worktree.store, Path::new(worktree.root));
        gathered.document.verdict = gathered.keep(Section::Verdict, read);
    }
}

/// `plan` and `tasks` of the build plan, whose bytes are read once for both.
fn from_plan(
    worktree: &ledger::Worktree<'_>,
    build: &ledger::BuildPlan<'_>,
    wanted: Wanted<'_>,
    gathered: &mut Gathered,
) {
    let ledger::PlanRead { approval, bytes } = match worktree.plan(build) {
        Ok(read) => read,
        Err(why) => {
            for section in [Section::Plan, Section::Tasks] {
                if wanted.has(section) {
                    gathered.unreadable(section.key(), &why);
                }
            }
            return;
        }
    };
    if wanted.has(Section::Plan) {
        gathered.document.plan = Some(approval);
    }
    if wanted.has(Section::Tasks) {
        let read = worktree.tasks(build, &bytes).map(Some);
        gathered.document.tasks = gathered.keep(Section::Tasks, read);
    }
}

/// `clef` and `context`, from the anneal home.
fn from_home(
    lookup: impl Fn(&str) -> Option<OsString>,
    wanted: Wanted<'_>,
    gathered: &mut Gathered,
) {
    if !wanted.any(&[Section::Clef, Section::Context]) {
        return;
    }
    let home = match paths::anneal_home_from(&lookup) {
        Ok(home) => home,
        Err(error) => {
            gathered.unreadable("home", &reason(&error));
            return;
        }
    };
    if wanted.has(Section::Clef) {
        gathered.document.clef = gathered.keep(Section::Clef, home::clef(&home, &lookup));
    }
    if wanted.has(Section::Context) {
        let home::ContextRead {
            context,
            unreadable,
        } = home::context(&home);
        if !unreadable.is_empty() {
            gathered.unreadable(Section::Context.key(), &unreadable.join("; "));
        }
        gathered.document.context = context;
    }
}

#[cfg(test)]
mod tests;
