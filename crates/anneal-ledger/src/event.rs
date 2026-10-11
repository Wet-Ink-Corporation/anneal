//! The event registry: every event type the ledger accepts, and its payload.
//!
//! The registry is versioned ([`REGISTRY_VERSION`]). A new event type, or a new optional
//! payload field, is additive and bumps the version; renaming or removing a type or a field is
//! a ledger migration. Every payload field name is a persisted JSON key, and every optional
//! field is `#[serde(default)]`, so an event written before the field existed still reads.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::tag::TagKey;

/// The version of the event registry this build reads and writes. Bumped when an event type
/// or a payload field is added.
pub const REGISTRY_VERSION: u32 = 2;

/// The name of an event type, as stored in the event's `type` key. The set is closed: an event
/// whose `type` is not listed here is a schema error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    /// `test_run`
    TestRun,
    /// `task_done`
    TaskDone,
    /// `plan_approved`
    PlanApproved,
    /// `grill_confirmed`
    GrillConfirmed,
    /// `repro_red`
    ReproRed,
    /// `verdict_set`
    VerdictSet,
    /// `round_started`
    RoundStarted,
    /// `gate_blocked`
    GateBlocked,
    /// `stop_blocked`
    StopBlocked,
}

impl EventKind {
    /// Every registered type, in registry order.
    pub const ALL: [Self; 9] = [
        Self::TestRun,
        Self::TaskDone,
        Self::PlanApproved,
        Self::GrillConfirmed,
        Self::ReproRed,
        Self::VerdictSet,
        Self::RoundStarted,
        Self::GateBlocked,
        Self::StopBlocked,
    ];

    /// The type as stored: snake case.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestRun => "test_run",
            Self::TaskDone => "task_done",
            Self::PlanApproved => "plan_approved",
            Self::GrillConfirmed => "grill_confirmed",
            Self::ReproRed => "repro_red",
            Self::VerdictSet => "verdict_set",
            Self::RoundStarted => "round_started",
            Self::GateBlocked => "gate_blocked",
            Self::StopBlocked => "stop_blocked",
        }
    }

    /// The key of the subject tag events of this type carry.
    #[must_use]
    pub const fn subject(self) -> TagKey {
        match self {
            Self::TestRun | Self::TaskDone | Self::ReproRed => TagKey::Task,
            Self::PlanApproved | Self::GrillConfirmed => TagKey::Plan,
            Self::VerdictSet | Self::RoundStarted => TagKey::Pr,
            Self::GateBlocked | Self::StopBlocked => TagKey::Point,
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is not a registered event type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown event type {name:?} (registry v{REGISTRY_VERSION})")]
pub struct UnknownEventKind {
    /// The name that was given.
    pub name: String,
}

impl FromStr for EventKind {
    type Err = UnknownEventKind;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == name)
            .ok_or_else(|| UnknownEventKind {
                name: name.to_owned(),
            })
    }
}

/// An event: its type (the `type` key) and its payload, flattened into one JSON object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerEvent {
    /// A test command ran.
    TestRun(TestRun),
    /// A task was marked done.
    TaskDone(TaskDone),
    /// A plan was approved at a content hash.
    PlanApproved(PlanApproved),
    /// The human confirmed a plan's grilling.
    GrillConfirmed(GrillConfirmed),
    /// A reproduction test was seen failing before a fix.
    ReproRed(ReproRed),
    /// A review verdict was set on a pull request.
    VerdictSet(VerdictSet),
    /// A review round started on a pull request.
    RoundStarted(RoundStarted),
    /// A gate refused to let work proceed.
    GateBlocked(GateBlocked),
    /// The Stop hook refused to let the session stop.
    StopBlocked(StopBlocked),
}

impl LedgerEvent {
    /// The event's registered type.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::TestRun(_) => EventKind::TestRun,
            Self::TaskDone(_) => EventKind::TaskDone,
            Self::PlanApproved(_) => EventKind::PlanApproved,
            Self::GrillConfirmed(_) => EventKind::GrillConfirmed,
            Self::ReproRed(_) => EventKind::ReproRed,
            Self::VerdictSet(_) => EventKind::VerdictSet,
            Self::RoundStarted(_) => EventKind::RoundStarted,
            Self::GateBlocked(_) => EventKind::GateBlocked,
            Self::StopBlocked(_) => EventKind::StopBlocked,
        }
    }

    /// The value of the event's subject tag (its key is [`EventKind::subject`]), or `None` for
    /// a `test_run` that names no task. Borrowed except for a pull request number, which is
    /// formatted.
    #[must_use]
    pub fn subject_value(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::TestRun(TestRun { task, .. }) => task.as_deref().map(Cow::Borrowed),
            Self::TaskDone(TaskDone { task }) | Self::ReproRed(ReproRed { task, .. }) => {
                Some(Cow::Borrowed(task))
            }
            Self::PlanApproved(PlanApproved { plan, .. })
            | Self::GrillConfirmed(GrillConfirmed { plan, .. }) => Some(Cow::Borrowed(plan)),
            Self::VerdictSet(VerdictSet { pr, .. })
            | Self::RoundStarted(RoundStarted { pr, .. }) => Some(Cow::Owned(pr.to_string())),
            Self::GateBlocked(GateBlocked { point, .. })
            | Self::StopBlocked(StopBlocked { point, .. }) => Some(Cow::Borrowed(point)),
        }
    }
}

/// Whether a test run passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    /// Every test passed.
    Green,
    /// At least one test failed.
    Red,
}

/// `test_run`: a test command ran. Subject tag `task:` when `task` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRun {
    /// The task the run was for, if any.
    #[serde(default)]
    pub task: Option<String>,
    /// The command line that ran.
    pub command: String,
    /// Whether it passed.
    pub outcome: TestOutcome,
}

/// `task_done`: a task was marked done. Subject tag `task:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDone {
    /// The task id.
    pub task: String,
}

/// `plan_approved`: a plan was approved at a content hash. Subject tag `plan:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanApproved {
    /// The plan id (its path relative to the repository root).
    pub plan: String,
    /// The hash of the plan's content that was approved.
    pub hash: String,
}

/// `grill_confirmed`: the human confirmed a plan's grilling. Subject tag `plan:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrillConfirmed {
    /// The plan id: for `anneal grill confirm`, the design tree's path as given.
    pub plan: String,
    /// The hash of the design tree that was confirmed, `sha256:<64 hex digits>` (registry v2).
    #[serde(default)]
    pub hash: Option<String>,
    /// The human's confirmation, word for word (registry v2).
    #[serde(default)]
    pub quote: Option<String>,
}

/// `repro_red`: a reproduction test was seen failing before its fix. Subject tag `task:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproRed {
    /// The task id.
    pub task: String,
    /// The test that failed.
    pub test: String,
}

/// A review verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The change may merge.
    Approved,
    /// The change needs work.
    ChangesRequested,
    /// Every finding was noise.
    NoiseOnly,
}

/// `verdict_set`: a review verdict was set on a pull request. Subject tag `pr:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictSet {
    /// The pull request number.
    pub pr: u64,
    /// The verdict.
    pub verdict: Verdict,
}

/// `round_started`: a review round started on a pull request. Subject tag `pr:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundStarted {
    /// The pull request number.
    pub pr: u64,
    /// The round number, from 1.
    pub round: u32,
}

/// `gate_blocked`: a gate refused to let work proceed. Subject tag `point:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateBlocked {
    /// The gate (decision point) that blocked.
    pub point: String,
    /// Why, in one line.
    pub reason: String,
}

/// `stop_blocked`: the Stop hook refused to let the session stop. Subject tag `point:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopBlocked {
    /// The check that blocked the stop.
    pub point: String,
    /// Why, in one line.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::{EventKind, GrillConfirmed, LedgerEvent, UnknownEventKind};
    use serde_json::json;

    #[test]
    fn grill_confirmed_carries_the_tree_hash_and_the_humans_words() {
        let value = json!({
            "type": "grill_confirmed",
            "plan": "design-tree.json",
            "hash": "sha256:00ff",
            "quote": "yes, that is what I want",
        });

        let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");

        assert_eq!(
            event,
            LedgerEvent::GrillConfirmed(GrillConfirmed {
                plan: "design-tree.json".to_owned(),
                hash: Some("sha256:00ff".to_owned()),
                quote: Some("yes, that is what I want".to_owned()),
            }),
            "hash and quote are read"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            value,
            "and written back under the same keys"
        );
    }

    #[test]
    fn a_grill_confirmed_written_before_hash_and_quote_still_reads() {
        let event: LedgerEvent =
            serde_json::from_value(json!({"type": "grill_confirmed", "plan": "p.md"}))
                .expect("registry v1 shape");

        assert_eq!(
            event,
            LedgerEvent::GrillConfirmed(GrillConfirmed {
                plan: "p.md".to_owned(),
                hash: None,
                quote: None,
            }),
            "the added fields default to none"
        );
    }

    #[test]
    fn every_kind_parses_from_its_stored_name() {
        for kind in EventKind::ALL {
            assert_eq!(kind.as_str().parse::<EventKind>(), Ok(kind), "{kind}");
        }
    }

    #[test]
    fn an_unregistered_name_is_unknown() {
        assert_eq!(
            "task_started".parse::<EventKind>(),
            Err(UnknownEventKind {
                name: "task_started".to_owned()
            }),
            "only registered names parse"
        );
    }
}
