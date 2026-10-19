//! The sections `anneal status` reads from the ledger: `phase`, `plan` and `tasks` of the
//! worktree, and the ledger-wide `gates` and `ledger_tail`. Each reads through the module that
//! owns the data; a failure is the one-line reason the caller turns into a finding.

use std::path::Path;

use anneal_ledger::event::{GateBlocked, Mode, StopBlocked};
use anneal_ledger::{EventKind, Filter, LedgerEvent, Recorded, Store};
use serde::Serialize;

use crate::error_chain::reason;
use crate::hook::completeness::playbook::RULES;
use crate::hook::handoff::facts::{self, MAX_TITLE_CHARS};
use crate::lint;
use crate::mode::{self, Active, BuildMode, Entered, scope};
use crate::plan::{self, Approval, ApprovalRecord, Status};

/// How many of the newest blocking events `gates` lists (H-13).
pub(super) const GATES_SHOWN: usize = 10;

/// The worktree the status is about: the ledger and the root git printed.
#[derive(Debug, Clone, Copy)]
pub(super) struct Worktree<'a> {
    /// The ledger.
    pub(super) store: &'a Store,
    /// The worktree's top-level directory, as git prints it.
    pub(super) root: &'a str,
}

/// What build mode names: the plan, relative to the worktree root, and the position build
/// mode was entered at (tasks count only when done after it).
#[derive(Debug, Clone, Copy)]
pub(super) struct BuildPlan<'a> {
    plan: &'a str,
    since: u64,
}

impl<'a> BuildPlan<'a> {
    /// The plan of `entered` when it is build mode with a plan.
    pub(super) fn of(entered: &'a Entered) -> Option<Self> {
        match &entered.mode {
            Active::Build(BuildMode { plan, .. }) => Some(Self {
                plan,
                since: entered.position,
            }),
            Active::IncompleteBuild | Active::Diagnose => None,
        }
    }
}

/// `phase`: the worktree's anneal mode and the playbook it runs.
#[derive(Debug, Serialize)]
pub(crate) struct Phase {
    /// `build` or `diagnose`.
    mode: &'static str,
    /// The playbook of the mode: the events its stop rules require.
    playbook: Playbook,
    /// The `mode_set` event's position.
    event: u64,
    /// Build mode's plan, relative to the worktree root.
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    /// Build mode's design tree, relative to the worktree root.
    #[serde(skip_serializing_if = "Option::is_none")]
    tree: Option<String>,
    /// Whether the working tree was clean when build mode was entered.
    #[serde(skip_serializing_if = "Option::is_none")]
    clean: Option<bool>,
}

/// The active playbook: its name and the events its rules require since the mode was entered
/// (`hook::completeness::playbook::RULES`).
#[derive(Debug, Serialize)]
struct Playbook {
    name: &'static str,
    requires: Vec<&'static str>,
}

impl Phase {
    /// The phase of `entered`, which it consumes.
    pub(super) fn of(entered: Entered) -> Self {
        let (mode, plan, tree, clean) = match entered.mode {
            Active::Build(BuildMode { plan, tree, clean }) => {
                (Mode::Build, Some(plan), Some(tree), Some(clean))
            }
            Active::IncompleteBuild => (Mode::Build, None, None, None),
            Active::Diagnose => (Mode::Diagnose, None, None, None),
        };
        let name = facts::mode_name(mode);
        let requires = RULES
            .iter()
            .filter(|rule| rule.mode == mode)
            .map(|rule| rule.event.as_str())
            .collect();
        Self {
            mode: name,
            playbook: Playbook { name, requires },
            event: entered.position,
            plan,
            tree,
            clean,
        }
    }

    /// The mode, for the text form.
    pub(super) const fn mode(&self) -> &'static str {
        self.mode
    }

    /// The plan, for the text form.
    pub(super) fn plan(&self) -> Option<&str> {
        self.plan.as_deref()
    }
}

/// `plan`: the build plan's approval at its current bytes.
#[derive(Debug, Serialize)]
pub(crate) struct Plan {
    /// The plan id: its path relative to the worktree root.
    id: String,
    /// `approved`, `stale` (its bytes changed since the approval) or `none`.
    status: PlanStatus,
    /// The `sha256:` hash of the plan's current bytes.
    hash: String,
    /// The latest approval, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    approved: Option<Approved>,
}

impl Plan {
    /// The id, for the text form.
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    /// The status, for the text form.
    pub(super) const fn status(&self) -> &'static str {
        match self.status {
            PlanStatus::Approved => "approved",
            PlanStatus::Stale => "stale",
            PlanStatus::NotApproved => "none",
        }
    }
}

/// Whether the plan is approved at its current bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PlanStatus {
    /// The latest approval has the plan's hash.
    Approved,
    /// The latest approval has another hash.
    Stale,
    /// No approval.
    #[serde(rename = "none")]
    NotApproved,
}

/// The latest approval: the hash it approved, its ledger position and when.
#[derive(Debug, Serialize)]
struct Approved {
    hash: String,
    event: u64,
    ts: String,
}

/// The plan's approval and its bytes, read once for `plan` and `tasks`.
#[derive(Debug)]
pub(super) struct PlanRead {
    /// The `plan` section.
    pub(super) approval: Plan,
    /// The plan's bytes.
    pub(super) bytes: Vec<u8>,
}

/// `tasks`: the build plan's tasks done since build mode was entered, and in all.
#[derive(Debug, Serialize)]
pub(crate) struct Tasks {
    done: usize,
    total: usize,
    /// The first task not done, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<NextTask>,
}

impl Tasks {
    /// Done and total, for the text form.
    pub(super) const fn counts(&self) -> (usize, usize) {
        (self.done, self.total)
    }

    /// The next task's id, for the text form.
    pub(super) fn next(&self) -> Option<&str> {
        self.next.as_ref().map(|next| next.id.as_str())
    }
}

/// The first task of the plan not done.
#[derive(Debug, Serialize)]
struct NextTask {
    id: String,
    /// Its title, cut to the handoff note's length.
    title: String,
}

impl Worktree<'_> {
    /// The worktree's active mode (`mode::active`).
    pub(super) fn mode(&self) -> Result<Option<Entered>, String> {
        mode::active(self.store, self.root).map_err(|error| reason(&error))
    }

    /// The build plan's bytes and its approval (`plan::approval_record`).
    pub(super) fn plan(&self, build: &BuildPlan<'_>) -> Result<PlanRead, String> {
        let path = Path::new(self.root).join(build.plan);
        let bytes = lint::read_whole_bytes(&path)
            .map_err(|error| format!("the plan {}: {}", build.plan, reason(&error)))?;
        let Approval {
            status,
            hash,
            latest,
        } = plan::approval_record(self.store, build.plan, &bytes)
            .map_err(|error| reason(&error))?;
        let status = match status {
            Status::Current => PlanStatus::Approved,
            Status::Stale => PlanStatus::Stale,
            Status::NotApproved => PlanStatus::NotApproved,
        };
        let approved = latest.map(|ApprovalRecord { hash, position, ts }| Approved {
            hash,
            event: position,
            ts,
        });
        let approval = Plan {
            id: build.plan.to_owned(),
            status,
            hash,
            approved,
        };
        Ok(PlanRead { approval, bytes })
    }

    /// The build plan's tasks in `bytes`, counted against the `task_done` events of this
    /// worktree since build mode was entered (`hook::handoff::facts`).
    pub(super) fn tasks(&self, build: &BuildPlan<'_>, bytes: &[u8]) -> Result<Tasks, String> {
        let tasks = facts::plan_tasks(build.plan, bytes);
        let key = scope::worktree_key(self.root);
        let done = facts::done_since(self.store, build.since, Some(&key))?;
        let finished = facts::finished(&tasks, &done);
        let total = tasks.len();
        let next = facts::next_task(&tasks, &done).map(|(index, _)| index);
        // The plan's tasks are this function's own: the next one moves into the section.
        let next = next
            .and_then(|index| tasks.into_iter().nth(index))
            .map(|task| NextTask {
                title: task.title.chars().take(MAX_TITLE_CHARS).collect(),
                id: task.id,
            });
        Ok(Tasks {
            done: finished,
            total,
            next,
        })
    }
}

/// One gate that blocked: a `gate_blocked` (a guard or precondition denied a tool call) or a
/// `stop_blocked` (the Stop hook refused a stop).
#[derive(Debug, Serialize)]
pub(crate) struct Gate {
    /// The gate, as its event names it (`guard.rm-rf`, `completeness.claim`).
    id: String,
    /// `gate_blocked` or `stop_blocked`.
    kind: &'static str,
    /// Why it blocked, in one line.
    reason: String,
    /// The event's ledger position.
    event: u64,
    /// When it blocked.
    ts: String,
}

impl Gate {
    /// The id, reason and event, for the text form.
    pub(super) fn parts(&self) -> (&str, &str, u64) {
        (&self.id, &self.reason, self.event)
    }
}

/// `gates`: the newest [`GATES_SHOWN`] blocking events, oldest first.
pub(super) fn gates(store: &Store) -> Result<Vec<Gate>, String> {
    let filter = Filter {
        types: vec![EventKind::GateBlocked, EventKind::StopBlocked],
        tags: Vec::new(),
    };
    let recorded = store
        .tail(&filter, GATES_SHOWN)
        .map_err(|error| reason(&error))?;
    Ok(recorded.into_iter().filter_map(gate).collect())
}

/// The gate a blocking event records; `None` for any other event.
fn gate(recorded: Recorded) -> Option<Gate> {
    let kind = recorded.event.event.kind().as_str();
    let (LedgerEvent::GateBlocked(GateBlocked { point, reason })
    | LedgerEvent::StopBlocked(StopBlocked { point, reason })) = recorded.event.event
    else {
        return None;
    };
    Some(Gate {
        id: point,
        kind,
        reason,
        event: recorded.position,
        ts: recorded.event.ts.as_str().to_owned(),
    })
}

/// One ledger event of `ledger_tail`.
#[derive(Debug, Serialize)]
pub(crate) struct TailEvent {
    /// Its ledger position.
    event: u64,
    /// Its registered type.
    #[serde(rename = "type")]
    kind: &'static str,
    /// Its type and what it is about, one line (`task_done task:T1`).
    summary: String,
    /// Who wrote it.
    actor: String,
    /// When.
    ts: String,
}

impl TailEvent {
    /// The position and summary, for the text form.
    pub(super) fn parts(&self) -> (u64, &str) {
        (self.event, &self.summary)
    }
}

/// `ledger_tail`: the newest `n` events, oldest first.
pub(super) fn tail(store: &Store, n: usize) -> Result<Vec<TailEvent>, String> {
    let recorded = store
        .tail(&Filter::default(), n)
        .map_err(|error| reason(&error))?;
    Ok(recorded
        .into_iter()
        .map(|recorded| TailEvent {
            event: recorded.position,
            kind: recorded.event.event.kind().as_str(),
            summary: facts::event_word(&recorded),
            ts: recorded.event.ts.as_str().to_owned(),
            actor: recorded.event.actor,
        })
        .collect())
}
