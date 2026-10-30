//! What the handoff note says about the worktree, each part read from the ledger, the plan or
//! the Stop hook's checks; what could not be read is a problem, never a failure.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use anneal_ledger::event::{Mode, ModeSet, TaskDone};
use anneal_ledger::{EventKind, Filter, LedgerEvent, LedgerTag, Recorded, Store, TagKey};

use crate::error_chain::reason;
use crate::hook::completeness::check;
use crate::lint;
use crate::lint::spec::Source;
use crate::mode::{self, Active, BuildMode, Entered};

/// How many of the newest ledger events the summary names.
const SUMMARY_EVENTS: usize = 5;

/// How many of the worktree's newest `task_done` events are read for the task count (H-13).
const TASKS_READ: usize = 1024;

/// The most characters of a task title the note (and `anneal status`) keeps.
pub(crate) const MAX_TITLE_CHARS: usize = 200;

/// The worktree's state for the handoff note.
#[derive(Debug)]
pub(super) struct Facts {
    /// The anneal mode, and what it was entered with.
    pub(super) phase: String,
    /// The current task of the build plan: "task N of M".
    pub(super) task: String,
    /// What the Stop hook would hold against stopping now: its gates and nudges.
    pub(super) gates: Vec<String>,
    /// The newest ledger events, oldest first, as one line.
    pub(super) ledger: String,
    /// What could not be read.
    pub(super) problems: Vec<String>,
}

/// The facts of the worktree whose top-level directory git prints as `root`.
pub(super) fn gather(root: &str, lookup: impl Fn(&str) -> Option<OsString>) -> Facts {
    let check::Findings {
        gates,
        nudges,
        notes,
        store,
        worktree,
    } = check::run(Path::new(root), None, lookup);
    let gates = gates
        .into_iter()
        .map(|(_, line)| line)
        .chain(nudges)
        .collect();
    let mut problems = notes;
    let Some(store) = &store else {
        return Facts {
            phase: phase(None),
            task: NO_PLAN.to_owned(),
            gates,
            ledger: "empty".to_owned(),
            problems,
        };
    };
    let entered = mode::active(store, root).unwrap_or_else(|error| {
        problems.push(format!("the mode: {}", reason(&error)));
        None
    });
    let here = Worktree {
        store,
        root,
        key: worktree.as_deref(),
    };
    let task = match &entered {
        Some(Entered {
            mode: Active::Build(build),
            position,
        }) => here.task(build, *position),
        _ => Ok(NO_PLAN.to_owned()),
    };
    let task = task.unwrap_or_else(|problem| {
        problems.push(problem);
        "unknown".to_owned()
    });
    let ledger = summary(store).unwrap_or_else(|problem| {
        problems.push(problem);
        "unknown".to_owned()
    });
    Facts {
        phase: phase(entered.as_ref()),
        task,
        gates,
        ledger,
        problems,
    }
}

/// The worktree the note is about: its ledger, its root and its key.
struct Worktree<'a> {
    store: &'a Store,
    root: &'a str,
    /// The worktree key its events are tagged with, when the ledger can tag it.
    key: Option<&'a str>,
}

/// The task line when the mode has no plan.
const NO_PLAN: &str = "no plan (only build mode has one)";

/// The phase line for the worktree's active mode.
fn phase(entered: Option<&Entered>) -> String {
    match entered.map(|entered| &entered.mode) {
        None => "no anneal mode".to_owned(),
        Some(Active::Build(BuildMode { plan, tree, .. })) => {
            format!("build mode (plan `{plan}`, design tree `{tree}`)")
        }
        Some(Active::IncompleteBuild) => {
            "build mode, recorded without a plan or design tree".to_owned()
        }
        Some(Active::Diagnose) => "diagnose mode".to_owned(),
    }
}

impl Worktree<'_> {
    /// "`T2` (2 of 3): <title>; 1 done": the first task of the plan with no `task_done` in the
    /// worktree since build mode was entered at `since`.
    fn task(&self, build: &BuildMode, since: u64) -> Result<String, String> {
        let plan = self.plan_tasks(build)?;
        let done = done_since(self.store, since, self.key)?;
        Ok(task_line(&plan, &done))
    }

    /// The tasks of the build plan, in file order.
    fn plan_tasks(&self, build: &BuildMode) -> Result<Vec<lint::plan::Task>, String> {
        let path = Path::new(self.root).join(&build.plan);
        let bytes = lint::read_whole_bytes(&path)
            .map_err(|error| format!("the plan {}: {}", build.plan, reason(&error)))?;
        Ok(plan_tasks(&build.plan, &bytes))
    }
}

/// The tasks of the plan `file` whose content is `bytes`, in file order, as `lint plan` reads
/// them; shared with `anneal status`.
pub(crate) fn plan_tasks(file: &str, bytes: &[u8]) -> Vec<lint::plan::Task> {
    let text = String::from_utf8_lossy(bytes);
    lint::plan::check(Source { file, text: &text }).plan.tasks
}

/// How many of `tasks` are in `done`.
pub(crate) fn finished(tasks: &[lint::plan::Task], done: &BTreeSet<String>) -> usize {
    tasks.iter().filter(|task| done.contains(&task.id)).count()
}

/// The first of `tasks` not in `done`, with its index in the plan.
pub(crate) fn next_task<'t>(
    tasks: &'t [lint::plan::Task],
    done: &BTreeSet<String>,
) -> Option<(usize, &'t lint::plan::Task)> {
    tasks
        .iter()
        .enumerate()
        .find(|(_, task)| !done.contains(&task.id))
}

/// The task line for the plan's `tasks` when `done` are done.
fn task_line(tasks: &[lint::plan::Task], done: &BTreeSet<String>) -> String {
    let finished = finished(tasks, done);
    let total = tasks.len();
    let next = next_task(tasks, done);
    match next {
        Some((index, next)) => {
            let title: String = next.title.chars().take(MAX_TITLE_CHARS).collect();
            format!(
                "`{}` ({} of {total}): {title}; {finished} done",
                next.id,
                index.saturating_add(1)
            )
        }
        None if total == 0 => "the plan lists no task".to_owned(),
        None => format!("all {total} done"),
    }
}

/// The tasks with a `task_done` tagged with `worktree` after position `since`; shared with
/// `anneal status`.
pub(crate) fn done_since(
    store: &Store,
    since: u64,
    worktree: Option<&str>,
) -> Result<BTreeSet<String>, String> {
    let Some(tag) = worktree.and_then(|key| LedgerTag::new(TagKey::Worktree, key).ok()) else {
        return Ok(BTreeSet::new());
    };
    let filter = Filter {
        types: vec![EventKind::TaskDone],
        tags: vec![tag],
    };
    let recorded = store
        .tail(&filter, TASKS_READ)
        .map_err(|error| format!("the done tasks: {}", reason(&error)))?;
    Ok(recorded
        .into_iter()
        .filter(|recorded| recorded.position > since)
        .filter_map(|recorded| match recorded.event.event {
            LedgerEvent::TaskDone(TaskDone { task }) => Some(task),
            _ => None,
        })
        .collect())
}

/// The newest [`SUMMARY_EVENTS`] ledger events, oldest first: `<type> <subject>` each.
fn summary(store: &Store) -> Result<String, String> {
    let events = store
        .tail(&Filter::default(), SUMMARY_EVENTS)
        .map_err(|error| format!("the ledger tail: {}", reason(&error)))?;
    if events.is_empty() {
        return Ok("empty".to_owned());
    }
    Ok(events.iter().map(event_word).collect::<Vec<_>>().join("; "))
}

/// `task_done task:T1`, `mode_set build`: an event's type and what it is about, one line.
pub(crate) fn event_word(recorded: &Recorded) -> String {
    let event = &recorded.event.event;
    let kind = event.kind();
    let about = match event {
        LedgerEvent::ModeSet(ModeSet { mode, .. }) => Some(mode_name(*mode).to_owned()),
        _ => event
            .subject_value()
            .map(|value| format!("{}:{value}", kind.subject().as_str())),
    };
    let word = match about {
        Some(about) => format!("{} {about}", kind.as_str()),
        None => kind.as_str().to_owned(),
    };
    word.replace(['\r', '\n'], " ")
}

/// The mode as typed after `anneal mode`.
pub(crate) const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Build => "build",
        Mode::Diagnose => "diagnose",
        Mode::Off => "off",
    }
}
