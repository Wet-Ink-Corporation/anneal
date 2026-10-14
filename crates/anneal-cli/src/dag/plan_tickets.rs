//! `dag check --plan`: whether the tracker's tickets agree with a plan's tasks.
//!
//! A ticket answers the task whose id is the ticket's id, exactly (a markdown ticket's id is
//! its file stem, so `tickets/T1.md` answers `### T1: ...`); titles are not compared. Three
//! kinds of disagreement, each a finding: a task with no ticket, a ticket with no task, and a
//! task whose ticket lists other blockers (compared as sets of ids, as written). A blocker
//! from outside the plan belongs in the tracker alone (`plugin/skills/plan/references/slice.md`),
//! so it is never a disagreement: only blockers that are tasks of the plan are compared, on
//! both sides; a ticket that is no task but blocks a task's ticket, directly or through other
//! such tickets, is an outside blocker rather than a ticket with no task; and a plan that
//! names an outside blocker itself is one `lint plan` reports, so it counts as unread.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::dag::ticket::{Ticket, TicketId, id_order};
use crate::envelope::Finding;
use crate::lint::plan::{self as plan_lint, MAX_FINDINGS, PlanFinding, Rule, Task};
use crate::lint::read_whole_bytes;
use crate::lint::skills::LintError;
use crate::lint::spec::Source;

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
        let name = file.display().to_string();
        let text = String::from_utf8_lossy(&bytes);
        // The pure check, not `examine`: the spec is not this command's concern, and looking
        // for it would resolve its path against the process's directory rather than `cwd`.
        let report = plan_lint::check(Source {
            file: &name,
            text: &text,
        });
        let tasks = report.plan.tasks;
        let planned: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
        let hiding = report
            .findings
            .iter()
            .filter(|finding| may_hide(finding, &planned))
            .count();
        // A finding `lint plan` dropped past its cap may be any of them.
        let unread = hiding.saturating_add(report.omitted);
        Ok(Self {
            file: name,
            tasks,
            unread,
        })
    }

    /// How the in-scope `tickets` agree with this plan, at most [`MAX_FINDINGS`] mismatches
    /// listed.
    pub(crate) fn against<'a>(
        &'a self,
        tickets: impl IntoIterator<Item = &'a Ticket>,
    ) -> Agreement<'a> {
        let mut mismatches = compare(&self.tasks, tickets);
        let omitted = mismatches.keep(MAX_FINDINGS);
        Agreement {
            file: &self.file,
            tasks: self.tasks.len(),
            mismatches,
            unread: self.unread,
            omitted: (omitted > 0).then_some(omitted),
        }
    }
}

/// Whether a `lint plan` finding may hide a task or a blocker from the comparison: a heading
/// that is not a task or a repeated id (`tasks`), a missing or malformed field
/// (`task-fields`), or a blocker that is no task of the plan (`blockers`), which the
/// comparison leaves out as it does on the ticket's side.
fn may_hide(finding: &PlanFinding, planned: &HashSet<&str>) -> bool {
    match finding.rule {
        Rule::Tasks | Rule::TaskFields => true,
        Rule::Blockers => finding
            .subject
            .as_deref()
            .is_some_and(|blocker| !planned.contains(blocker)),
        Rule::Spec | Rule::Sections => false,
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
    /// `lint plan`'s findings that may hide a task or blocker, plus those it dropped.
    unread: usize,
    /// How many mismatches past [`MAX_FINDINGS`] were left out; present only when some were.
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted: Option<usize>,
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
                    "`lint plan` finds {} problem(s) in the plan's tasks or blockers, so a task \
                     or blocker may be missing from the comparison: run `anneal lint plan {file}`",
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
    /// Ticket ids no task has, outside blockers excluded.
    pub(crate) no_task: Vec<&'a TicketId>,
    /// Tasks whose ticket lists other blockers.
    pub(crate) blockers: Vec<Blockers<'a>>,
}

/// A task and its ticket that list different blockers, each list sorted and without repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Blockers<'a> {
    /// The task's and the ticket's id.
    pub(crate) id: &'a str,
    /// The plan's `Blocked by:` ids that are tasks of the plan.
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
        // A blocker outside the plan belongs in the tracker alone (`slice.md`), so only the
        // blockers that are tasks of the plan are compared, on both sides; `lint plan` reports
        // the plan's own outside blockers, which `unread` counts.
        let plan = id_set(
            task.blocked_by
                .iter()
                .map(String::as_str)
                .filter(|id| planned.contains(id)),
        );
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
    let outside = outside_blockers(&tickets, &planned);
    let mut no_task: Vec<&TicketId> = tickets
        .values()
        .map(|ticket| &ticket.id)
        .filter(|id| !planned.contains(id.as_str()) && !outside.contains(id.as_str()))
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

/// The ids of the tickets that are no task of the plan but block one: blockers of a task's
/// ticket, then, transitively, blockers of those (a release blocked by another release), the
/// walk stopping at the plan's own tasks. Each ticket is visited once, so the walk is bounded
/// by the tickets read.
fn outside_blockers<'a>(
    tickets: &HashMap<&'a str, &'a Ticket>,
    planned: &HashSet<&str>,
) -> HashSet<&'a str> {
    let mut outside = HashSet::new();
    let mut next: Vec<&TicketId> = planned
        .iter()
        .filter_map(|id| tickets.get(id))
        .flat_map(|ticket| &ticket.blocked_by)
        .collect();
    while let Some(id) = next.pop() {
        if planned.contains(id.as_str()) {
            continue;
        }
        // A blocker that is no ticket dangles: the graph check reports it.
        let Some(&ticket) = tickets.get(id.as_str()) else {
            continue;
        };
        if outside.insert(ticket.id.as_str()) {
            next.extend(&ticket.blocked_by);
        }
    }
    outside
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

    /// Keeps the first `cap` disagreements in finding order (tasks with no ticket, then
    /// tickets with no task, then differing blockers) and returns how many were dropped.
    fn keep(&mut self, cap: usize) -> usize {
        let mut left = cap;
        let mut dropped = 0_usize;
        let mut cut = |len: usize| {
            let kept = len.min(left);
            left = left.saturating_sub(kept);
            dropped = dropped.saturating_add(len.saturating_sub(kept));
            kept
        };
        let no_ticket = cut(self.no_ticket.len());
        let no_task = cut(self.no_task.len());
        let blockers = cut(self.blockers.len());
        self.no_ticket.truncate(no_ticket);
        self.no_task.truncate(no_task);
        self.blockers.truncate(blockers);
        dropped
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
