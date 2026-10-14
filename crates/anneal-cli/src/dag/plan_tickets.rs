//! `dag check --plan`: whether the tracker's tickets agree with a plan's tasks.
//!
//! A ticket answers the task whose id is the ticket's id, exactly (a markdown ticket's id is
//! its file stem, so `tickets/T1.md` answers `### T1: ...`); titles are not compared. Three
//! kinds of disagreement, each a finding: a task with no ticket, a ticket with no task, and a
//! task whose ticket lists other blockers (compared as sets of ids, as written). A ticket's
//! blocker that is no task of the plan is left out of the comparison: a blocker from outside
//! the plan belongs in the tracker alone (`plugin/skills/plan/references/slice.md`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::dag::ticket::{Ticket, TicketId, id_order};
use crate::envelope::Finding;
use crate::lint::plan::{self as plan_lint, Rule, Task};
use crate::lint::read_whole_bytes;
use crate::lint::skills::LintError;

/// A plan's tasks as `lint plan` reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanTasks {
    /// The plan as named on the command line.
    file: String,
    /// The tasks with well-formed headings, each id once.
    tasks: Vec<Task>,
    /// `lint plan`'s `tasks` and `task-fields` findings: each may hide a task or a blocker.
    unread: usize,
}

impl PlanTasks {
    /// Reads the plan `file`, relative to `cwd` unless absolute, with `lint plan`'s reader and
    /// cap.
    ///
    /// # Errors
    /// [`LintError::MissingPath`] or [`LintError::WrongKind`] when the plan is missing or a
    /// directory, [`LintError::TooLarge`] when it is over the read cap, [`LintError::Io`] when it
    /// cannot be read.
    pub(crate) fn read(file: &Path, cwd: &Path) -> Result<Self, LintError> {
        let bytes = read_whole_bytes(&cwd.join(file))?;
        let outcome = plan_lint::examine(file, &bytes);
        let report = outcome.report;
        let unread = report
            .findings
            .iter()
            .filter(|finding| matches!(finding.rule, Rule::Tasks | Rule::TaskFields))
            .count();
        Ok(Self {
            file: outcome.file,
            tasks: report.plan.tasks,
            unread,
        })
    }

    /// How the in-scope `tickets` agree with this plan.
    pub(crate) fn against<'a>(
        &'a self,
        tickets: impl IntoIterator<Item = &'a Ticket>,
    ) -> Agreement<'a> {
        Agreement {
            file: &self.file,
            tasks: self.tasks.len(),
            mismatches: compare(&self.tasks, tickets),
            unread: self.unread,
        }
    }
}

/// `data.plan` of `dag check --plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Agreement<'a> {
    /// The plan as named.
    file: &'a str,
    /// How many tasks were read.
    tasks: usize,
    /// Where the tickets disagree with the tasks.
    #[serde(flatten)]
    mismatches: Mismatches<'a>,
    /// `lint plan`'s findings about the plan's tasks.
    unread: usize,
}

impl Agreement<'_> {
    /// Whether the tickets agree with a plan that was read in full.
    pub(crate) fn holds(&self) -> bool {
        self.unread == 0 && self.mismatches.is_empty()
    }

    /// One finding when the plan was read in part, then one per mismatch.
    pub(crate) fn findings(&self) -> Vec<Finding> {
        let file = self.file;
        let unread = (self.unread > 0).then(|| {
            Finding::new(
                format!("dag/plan/unread:{file}"),
                format!(
                    "`lint plan` finds {} problem(s) in the plan's tasks, so a task or blocker \
                     may be missing from the comparison: run `anneal lint plan {file}`",
                    self.unread
                ),
            )
        });
        unread
            .into_iter()
            .chain(self.mismatches.findings())
            .collect()
    }
}

/// Where the tickets and the plan disagree, each list in [`TicketId`] order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub(crate) struct Mismatches<'a> {
    /// Task ids no ticket has.
    pub(crate) no_ticket: Vec<&'a str>,
    /// Ticket ids no task has.
    pub(crate) no_task: Vec<&'a TicketId>,
    /// Tasks whose ticket lists other blockers.
    pub(crate) blockers: Vec<Blockers<'a>>,
}

/// A task and its ticket that list different blockers, each list sorted and without repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Blockers<'a> {
    /// The task's and the ticket's id.
    pub(crate) id: &'a str,
    /// The plan's `Blocked by:` ids.
    pub(crate) plan: Vec<&'a str>,
    /// The ticket's `blocked_by` ids that are tasks of the plan.
    pub(crate) ticket: Vec<&'a str>,
}

/// Compares the plan's `tasks` with the tracker's in-scope `tickets`, whose ids are unique
/// (the graph refuses a repeated one before this runs).
pub(crate) fn compare<'a>(
    tasks: &'a [Task],
    tickets: impl IntoIterator<Item = &'a Ticket>,
) -> Mismatches<'a> {
    let tickets: HashMap<&str, &Ticket> = tickets
        .into_iter()
        .map(|ticket| (ticket.id.as_str(), ticket))
        .collect();
    let planned: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
    let mut no_ticket = Vec::new();
    let mut blockers = Vec::new();
    for task in tasks {
        let Some(ticket) = tickets.get(task.id.as_str()) else {
            no_ticket.push(task.id.as_str());
            continue;
        };
        let plan = id_set(task.blocked_by.iter().map(String::as_str));
        // A blocker outside the plan belongs in the tracker alone (`slice.md`), so only the
        // ticket's blockers that are tasks of the plan are compared.
        let listed = id_set(
            ticket
                .blocked_by
                .iter()
                .map(TicketId::as_str)
                .filter(|id| planned.contains(id)),
        );
        if plan != listed {
            blockers.push(Blockers {
                id: task.id.as_str(),
                plan,
                ticket: listed,
            });
        }
    }
    let mut no_task: Vec<&TicketId> = tickets
        .values()
        .map(|ticket| &ticket.id)
        .filter(|id| !planned.contains(id.as_str()))
        .collect();
    no_ticket.sort_by(|left, right| id_order(left, right));
    no_task.sort();
    blockers.sort_by(|left, right| id_order(left.id, right.id));
    Mismatches {
        no_ticket,
        no_task,
        blockers,
    }
}

/// `ids` in id order, each once.
fn id_set<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut ids: Vec<&str> = ids.collect();
    ids.sort_by(|left, right| id_order(left, right));
    ids.dedup();
    ids
}

impl Mismatches<'_> {
    /// Whether the tickets agree with the plan.
    pub(crate) fn is_empty(&self) -> bool {
        self.no_ticket.is_empty() && self.no_task.is_empty() && self.blockers.is_empty()
    }

    /// One finding per disagreement: tasks with no ticket, then tickets with no task, then
    /// differing blockers, each in id order.
    pub(crate) fn findings(&self) -> Vec<Finding> {
        let no_ticket = self.no_ticket.iter().map(|id| {
            Finding::new(
                format!("dag/plan/no-ticket:{id}"),
                format!("plan task {id} has no ticket: add a ticket with id {id}"),
            )
        });
        let no_task = self.no_task.iter().map(|id| {
            Finding::new(
                format!("dag/plan/no-task:{id}"),
                format!(
                    "ticket {id} is no task of the plan: add the task to the plan or remove the \
                     ticket"
                ),
            )
        });
        let blockers = self.blockers.iter().map(|Blockers { id, plan, ticket }| {
            Finding::new(
                format!("dag/plan/blockers:{id}"),
                format!(
                    "ticket {id} is blocked by [{}], but the plan's task is blocked by [{}]",
                    ticket.join(", "),
                    plan.join(", ")
                ),
            )
        });
        no_ticket.chain(no_task).chain(blockers).collect()
    }
}

#[cfg(test)]
mod tests;
