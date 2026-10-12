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
pub const REGISTRY_VERSION: u32 = 8;

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
    /// `mode_set`
    ModeSet,
    /// `subagent_status`
    SubagentStatus,
    /// `migrate.wave.started`
    MigrateWaveStarted,
    /// `migrate.wave.done`
    MigrateWaveDone,
    /// `migrate.contract`
    MigrateContract,
}

impl EventKind {
    /// Every registered type, in registry order.
    pub const ALL: [Self; 14] = [
        Self::TestRun,
        Self::TaskDone,
        Self::PlanApproved,
        Self::GrillConfirmed,
        Self::ReproRed,
        Self::VerdictSet,
        Self::RoundStarted,
        Self::GateBlocked,
        Self::StopBlocked,
        Self::ModeSet,
        Self::SubagentStatus,
        Self::MigrateWaveStarted,
        Self::MigrateWaveDone,
        Self::MigrateContract,
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
            Self::ModeSet => "mode_set",
            Self::SubagentStatus => "subagent_status",
            Self::MigrateWaveStarted => "migrate.wave.started",
            Self::MigrateWaveDone => "migrate.wave.done",
            Self::MigrateContract => "migrate.contract",
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
            // The payload names no worktree: the envelope's `worktree` is the subject.
            Self::ModeSet => TagKey::Worktree,
            Self::SubagentStatus => TagKey::Agent,
            Self::MigrateWaveStarted | Self::MigrateWaveDone | Self::MigrateContract => {
                TagKey::Migration
            }
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
    /// A worktree entered or left an anneal mode.
    ModeSet(ModeSet),
    /// A subagent stopped, with the status block its last message gave (or the violation).
    SubagentStatus(SubagentStatus),
    /// A wave of a migration started (registry v7).
    #[serde(rename = "migrate.wave.started")]
    MigrateWaveStarted(MigrateWave),
    /// A wave of a migration finished (registry v7).
    #[serde(rename = "migrate.wave.done")]
    MigrateWaveDone(MigrateWave),
    /// A migration's contract step ran: the old shape is removed (registry v7).
    #[serde(rename = "migrate.contract")]
    MigrateContract(MigrateContract),
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
            Self::ModeSet(_) => EventKind::ModeSet,
            Self::SubagentStatus(_) => EventKind::SubagentStatus,
            Self::MigrateWaveStarted(_) => EventKind::MigrateWaveStarted,
            Self::MigrateWaveDone(_) => EventKind::MigrateWaveDone,
            Self::MigrateContract(_) => EventKind::MigrateContract,
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
            Self::VerdictSet(VerdictSet { pr, .. }) => Some(Cow::Owned(pr.to_string())),
            // A ticket round has no pull request, so no subject tag.
            Self::RoundStarted(RoundStarted { pr, .. }) => pr.map(|pr| Cow::Owned(pr.to_string())),
            Self::GateBlocked(GateBlocked { point, .. })
            | Self::StopBlocked(StopBlocked { point, .. }) => Some(Cow::Borrowed(point)),
            Self::SubagentStatus(SubagentStatus { agent, .. }) => Some(Cow::Borrowed(agent)),
            Self::MigrateWaveStarted(MigrateWave { migration, .. })
            | Self::MigrateWaveDone(MigrateWave { migration, .. })
            | Self::MigrateContract(MigrateContract { migration }) => {
                Some(Cow::Borrowed(migration))
            }
            // Its subject is the envelope's `worktree`, tagged from the envelope.
            Self::ModeSet(_) => None,
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
    /// The command's exit code; `None` when it has none: it was killed (on a timeout, or by a
    /// signal) (registry v3).
    #[serde(default)]
    pub exit: Option<i32>,
    /// How long the command ran, in milliseconds, from a monotonic clock (registry v3).
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// The deadline in seconds, set only when the command ran past it and was killed
    /// (registry v3).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
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

/// A review verdict: a level a change reached (`tests_verified`, `reviewed`, `approved`, from
/// lowest to highest; the first two in registry v6), or a review's outcome
/// (`changes_requested`, `noise_only`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Level: the change's tests were verified green (registry v6).
    TestsVerified,
    /// Level: the change was reviewed (registry v6).
    Reviewed,
    /// Level: the change may merge.
    Approved,
    /// The change needs work.
    ChangesRequested,
    /// Every finding was noise.
    NoiseOnly,
}

/// Who set a verdict (registry v6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictSource {
    /// anneal's own checks in the session, such as `verify`.
    Local,
    /// The review plugin.
    ReviewPlugin,
    /// A human.
    Human,
}

/// `verdict_set`: a review verdict was set on a pull request. Subject tag `pr:`.
///
/// `sha`, `patch_id` and `base` say what the verdict is for: it is fresh for a head whose commit
/// is `sha`, or whose `git patch-id --verbatim` of the diff from `base` is `patch_id` (a
/// rebase that changed nothing else).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictSet {
    /// The pull request number.
    pub pr: u64,
    /// The verdict.
    pub verdict: Verdict,
    /// Who set it (registry v6).
    #[serde(default)]
    pub source: Option<VerdictSource>,
    /// The full commit id of the head it was set for (registry v6).
    #[serde(default)]
    pub sha: Option<String>,
    /// `git patch-id --verbatim` of `git diff <base>...<sha>`; `None` for an empty diff
    /// (registry v6).
    #[serde(default)]
    pub patch_id: Option<String>,
    /// The base ref the patch id was taken from, as given (registry v6).
    #[serde(default)]
    pub base: Option<String>,
}

/// `round_started`: a review or fix-loop round started, on a pull request or a ticket.
///
/// Subject tag `pr:` when `pr` is set; a ticket round has no subject tag and is found by its
/// type and `ticket` (`anneal round`). It names a pull request, a ticket or both, never
/// neither: it is built with [`RoundStarted::for_pr`] or [`RoundStarted::for_ticket`], and an
/// event naming neither does not read. A missing one is not written, so a pull request round
/// keeps registry v6's shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RoundStartedFields")]
pub struct RoundStarted {
    /// The pull request number; optional since registry v8, required before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pr: Option<u64>,
    /// The ticket the round is for, as given to `anneal round start --ticket` (registry v8).
    #[serde(skip_serializing_if = "Option::is_none")]
    ticket: Option<String>,
    /// The round number, from 1.
    round: u32,
}

impl RoundStarted {
    /// Round `round` of the review of pull request `pr`.
    #[must_use]
    pub const fn for_pr(pr: u64, round: u32) -> Self {
        Self {
            pr: Some(pr),
            ticket: None,
            round,
        }
    }

    /// Round `round` of the fix loop of `ticket`. The ticket is stored, so it is taken owned.
    #[must_use]
    pub fn for_ticket(ticket: impl Into<String>, round: u32) -> Self {
        Self {
            pr: None,
            ticket: Some(ticket.into()),
            round,
        }
    }

    /// The pull request, if the round is a review round.
    #[must_use]
    pub const fn pr(&self) -> Option<u64> {
        self.pr
    }

    /// The ticket, if the round is a fix-loop round.
    #[must_use]
    pub fn ticket(&self) -> Option<&str> {
        self.ticket.as_deref()
    }

    /// The round number, from 1.
    #[must_use]
    pub const fn round(&self) -> u32 {
        self.round
    }
}

/// `round_started` as stored, before it is checked to name a pull request or a ticket.
#[derive(Deserialize)]
struct RoundStartedFields {
    #[serde(default)]
    pr: Option<u64>,
    #[serde(default)]
    ticket: Option<String>,
    round: u32,
}

/// A `round_started` that names neither a pull request nor a (non-empty) ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("round_started names neither a pull request (pr) nor a ticket")]
pub struct RoundOfNothing;

impl TryFrom<RoundStartedFields> for RoundStarted {
    type Error = RoundOfNothing;

    fn try_from(fields: RoundStartedFields) -> Result<Self, Self::Error> {
        let RoundStartedFields { pr, ticket, round } = fields;
        let ticket = ticket.filter(|ticket| !ticket.is_empty());
        if pr.is_none() && ticket.is_none() {
            return Err(RoundOfNothing);
        }
        Ok(Self { pr, ticket, round })
    }
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

/// An anneal mode: what the preconditions hook enforces in a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Building from an approved plan: source edits need the define work.
    Build,
    /// Diagnosing a failure: source edits need a red reproduction first.
    Diagnose,
    /// No mode: the hook enforces nothing.
    Off,
}

/// `mode_set`: a worktree entered or left an anneal mode (registry v4). Its subject is the
/// envelope's `worktree` (tagged `worktree:`); the latest one for a worktree is its mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeSet {
    /// The mode entered.
    pub mode: Mode,
    /// Whether the working tree had no changes (untracked files included) at entry.
    pub clean: bool,
    /// For build mode, the plan's path relative to the worktree root, `/`-separated.
    #[serde(default)]
    pub plan: Option<String>,
    /// For build mode, the design tree's path relative to the worktree root, `/`-separated.
    #[serde(default)]
    pub tree: Option<String>,
}

/// The status a subagent reports on its `Status:` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// `DONE`: the work is finished and verified.
    Done,
    /// `DONE_WITH_CONCERNS`: finished, with concerns the caller should read.
    DoneWithConcerns,
    /// `NEEDS_CONTEXT`: it cannot go on without more context.
    NeedsContext,
    /// `BLOCKED`: it needs a decision it cannot take.
    Blocked,
}

/// Tokens a subagent used, summed over its transcript's assistant messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    /// Input tokens not read from or written to the cache.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Input tokens read from the prompt cache.
    pub cache_read: u64,
    /// Input tokens written to the prompt cache.
    pub cache_creation: u64,
}

/// `subagent_status`: a subagent stopped (registry v5). Subject tag `agent:` (its type).
///
/// `status` is `None` when the last message had no valid `Status:` line; `violation` then says
/// why, as it does when required sections are `missing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentStatus {
    /// The subagent's type, such as `Explore` or `temper:rust-reviewer`.
    pub agent: String,
    /// The subagent's id in its session.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The status it reported, when it reported a known one.
    #[serde(default)]
    pub status: Option<AgentStatus>,
    /// What is wrong with its status block, in one line, when anything is.
    #[serde(default)]
    pub violation: Option<String>,
    /// The required sections its status block lacks.
    #[serde(default)]
    pub missing: Vec<String>,
    /// From its transcript's first to last timestamp, in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// The tokens it used, from its transcript.
    #[serde(default)]
    pub tokens: Option<TokenUsage>,
}

/// `migrate.wave.started` and `migrate.wave.done`: a wave of an expand, migrate, contract
/// migration started or finished (registry v7). Subject tag `migration:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrateWave {
    /// The migration's id.
    pub migration: String,
    /// The wave's number within the migration.
    pub wave: u32,
}

/// `migrate.contract`: a migration's contract step ran, removing the old shape (registry v7).
/// Subject tag `migration:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrateContract {
    /// The migration's id.
    pub migration: String,
}

#[cfg(test)]
mod tests {
    use super::{
        AgentStatus, EventKind, GrillConfirmed, LedgerEvent, MigrateContract, MigrateWave, Mode,
        ModeSet, REGISTRY_VERSION, RoundStarted, SubagentStatus, TestOutcome, TestRun, TokenUsage,
        UnknownEventKind, Verdict, VerdictSet, VerdictSource,
    };
    use crate::tag::TagKey;
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
    fn test_run_carries_the_exit_code_duration_and_timeout() {
        let value = json!({
            "type": "test_run",
            "task": "T1",
            "command": "cargo test",
            "outcome": "red",
            "exit": null,
            "duration_ms": 1500,
            "timeout_secs": 1,
        });

        let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");

        assert_eq!(
            event,
            LedgerEvent::TestRun(TestRun {
                task: Some("T1".to_owned()),
                command: "cargo test".to_owned(),
                outcome: TestOutcome::Red,
                exit: None,
                duration_ms: Some(1500),
                timeout_secs: Some(1),
            }),
            "exit, duration and timeout are read"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            value,
            "and written back under the same keys"
        );
    }

    #[test]
    fn a_test_run_written_before_exit_and_duration_still_reads() {
        let event: LedgerEvent = serde_json::from_value(
            json!({"type": "test_run", "command": "cargo test", "outcome": "green"}),
        )
        .expect("registry v2 shape");

        assert_eq!(
            event,
            LedgerEvent::TestRun(TestRun {
                task: None,
                command: "cargo test".to_owned(),
                outcome: TestOutcome::Green,
                exit: None,
                duration_ms: None,
                timeout_secs: None,
            }),
            "the added fields default to none"
        );
    }

    #[test]
    fn mode_set_carries_the_mode_the_clean_flag_and_the_define_artifacts() {
        let value = json!({
            "type": "mode_set",
            "mode": "build",
            "clean": true,
            "plan": "docs/plans/x.md",
            "tree": "design-tree.json",
        });

        let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");

        assert_eq!(
            event,
            LedgerEvent::ModeSet(ModeSet {
                mode: Mode::Build,
                clean: true,
                plan: Some("docs/plans/x.md".to_owned()),
                tree: Some("design-tree.json".to_owned()),
            }),
            "mode, clean, plan and tree are read"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            value,
            "and written back under the same keys"
        );
        assert_eq!(event.kind().as_str(), "mode_set", "stored as mode_set");
        assert_eq!(event.subject_value(), None, "no subject in the payload");
        assert_eq!(
            EventKind::ModeSet.subject(),
            TagKey::Worktree,
            "selected by the envelope's worktree tag"
        );
    }

    #[test]
    fn a_mode_set_without_plan_or_tree_reads_as_none() {
        let event: LedgerEvent =
            serde_json::from_value(json!({"type": "mode_set", "mode": "diagnose", "clean": false}))
                .expect("plan and tree are optional");

        assert_eq!(
            event,
            LedgerEvent::ModeSet(ModeSet {
                mode: Mode::Diagnose,
                clean: false,
                plan: None,
                tree: None,
            }),
            "the optional fields default to none"
        );
    }

    #[test]
    fn subagent_status_carries_the_status_missing_sections_timing_and_tokens() {
        let value = json!({
            "type": "subagent_status",
            "agent": "temper:rust-implementer",
            "agent_id": "a1",
            "status": "done_with_concerns",
            "violation": "missing sections: Review",
            "missing": ["Review"],
            "duration_ms": 61_000,
            "tokens": {"input": 10, "output": 20, "cache_read": 30, "cache_creation": 40},
        });

        let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");

        assert_eq!(
            event,
            LedgerEvent::SubagentStatus(SubagentStatus {
                agent: "temper:rust-implementer".to_owned(),
                agent_id: Some("a1".to_owned()),
                status: Some(AgentStatus::DoneWithConcerns),
                violation: Some("missing sections: Review".to_owned()),
                missing: vec!["Review".to_owned()],
                duration_ms: Some(61_000),
                tokens: Some(TokenUsage {
                    input: 10,
                    output: 20,
                    cache_read: 30,
                    cache_creation: 40,
                }),
            }),
            "every field is read"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            value,
            "and written back under the same keys"
        );
        assert_eq!(
            (EventKind::SubagentStatus.subject(), event.subject_value()),
            (TagKey::Agent, Some("temper:rust-implementer".into())),
            "its subject is the agent type"
        );
    }

    #[test]
    fn a_subagent_status_with_only_the_agent_reads_as_a_bare_violation_record() {
        let event: LedgerEvent =
            serde_json::from_value(json!({"type": "subagent_status", "agent": "Explore"}))
                .expect("only agent is required");

        assert_eq!(
            event,
            LedgerEvent::SubagentStatus(SubagentStatus {
                agent: "Explore".to_owned(),
                agent_id: None,
                status: None,
                violation: None,
                missing: Vec::new(),
                duration_ms: None,
                tokens: None,
            }),
            "the optional fields default to none"
        );
    }

    #[test]
    fn every_agent_status_has_its_stored_name() {
        let names: Vec<String> = [
            AgentStatus::Done,
            AgentStatus::DoneWithConcerns,
            AgentStatus::NeedsContext,
            AgentStatus::Blocked,
        ]
        .iter()
        .map(|status| {
            serde_json::to_value(status)
                .expect("serialises")
                .to_string()
        })
        .collect();

        assert_eq!(
            names,
            [
                "\"done\"",
                "\"done_with_concerns\"",
                "\"needs_context\"",
                "\"blocked\""
            ],
            "snake case, as the ledger's other enums"
        );
    }

    #[test]
    fn verdict_set_carries_the_level_source_commit_patch_id_and_base() {
        let value = json!({
            "type": "verdict_set",
            "pr": 33,
            "verdict": "tests_verified",
            "source": "review_plugin",
            "sha": "a21c5e7b17ac7075034918873814cd7950807243",
            "patch_id": "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c",
            "base": "origin/main",
        });

        let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");

        assert_eq!(
            event,
            LedgerEvent::VerdictSet(VerdictSet {
                pr: 33,
                verdict: Verdict::TestsVerified,
                source: Some(VerdictSource::ReviewPlugin),
                sha: Some("a21c5e7b17ac7075034918873814cd7950807243".to_owned()),
                patch_id: Some("0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c".to_owned()),
                base: Some("origin/main".to_owned()),
            }),
            "every field is read"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            value,
            "and written back under the same keys"
        );
    }

    #[test]
    fn every_verdict_and_source_has_its_stored_name() {
        let verdicts = [
            Verdict::TestsVerified,
            Verdict::Reviewed,
            Verdict::Approved,
            Verdict::ChangesRequested,
            Verdict::NoiseOnly,
        ]
        .map(|verdict| serde_json::to_value(verdict).expect("serialises"));
        let sources = [
            VerdictSource::Local,
            VerdictSource::ReviewPlugin,
            VerdictSource::Human,
        ]
        .map(|source| serde_json::to_value(source).expect("serialises"));

        assert_eq!(
            (verdicts.to_vec(), sources.to_vec()),
            (
                vec![
                    json!("tests_verified"),
                    json!("reviewed"),
                    json!("approved"),
                    json!("changes_requested"),
                    json!("noise_only"),
                ],
                vec![json!("local"), json!("review_plugin"), json!("human")],
            ),
            "snake case, as the ledger's other enums"
        );
    }

    #[test]
    fn the_registry_is_version_8() {
        assert_eq!(
            REGISTRY_VERSION, 8,
            "the migrate.wave.started, migrate.wave.done and migrate.contract types were added \
             in version 7, round_started's ticket (with pr optional) in version 8"
        );
    }

    #[test]
    fn round_started_carries_a_ticket_without_a_pull_request() {
        let value = json!({"type": "round_started", "ticket": "36", "round": 2});

        let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");

        assert_eq!(
            event,
            LedgerEvent::RoundStarted(RoundStarted::for_ticket("36", 2)),
            "ticket and round are read"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            value,
            "and written back under the same keys, with no pr key"
        );
        assert_eq!(event.subject_value(), None, "no pull request, no pr: tag");
    }

    #[test]
    fn a_round_started_written_before_ticket_still_reads_with_its_pull_request() {
        let v6 = json!({"type": "round_started", "pr": 7, "round": 1});

        let event: LedgerEvent = serde_json::from_value(v6.clone()).expect("registry v6 shape");

        assert_eq!(
            event.subject_value().as_deref(),
            Some("7"),
            "the pr: tag is kept"
        );
        assert_eq!(
            event,
            LedgerEvent::RoundStarted(RoundStarted::for_pr(7, 1)),
            "the ticket defaults to none"
        );
        assert_eq!(
            serde_json::to_value(&event).expect("serialises"),
            v6,
            "a pull request round is still written in v6's shape, with no ticket key"
        );
    }

    #[test]
    fn a_round_started_naming_neither_a_pull_request_nor_a_ticket_is_refused() {
        for value in [
            json!({"type": "round_started", "round": 1}),
            json!({"type": "round_started", "pr": null, "ticket": null, "round": 1}),
            json!({"type": "round_started", "ticket": "", "round": 1}),
        ] {
            assert!(
                serde_json::from_value::<LedgerEvent>(value.clone()).is_err(),
                "a round of nothing: {value}"
            );
        }
    }

    #[test]
    fn migrate_events_are_stored_under_their_dotted_names_with_the_migration_as_subject() {
        let cases = [
            (
                json!({"type": "migrate.wave.started", "migration": "users-email", "wave": 1}),
                LedgerEvent::MigrateWaveStarted(MigrateWave {
                    migration: "users-email".to_owned(),
                    wave: 1,
                }),
            ),
            (
                json!({"type": "migrate.wave.done", "migration": "users-email", "wave": 2}),
                LedgerEvent::MigrateWaveDone(MigrateWave {
                    migration: "users-email".to_owned(),
                    wave: 2,
                }),
            ),
            (
                json!({"type": "migrate.contract", "migration": "users-email"}),
                LedgerEvent::MigrateContract(MigrateContract {
                    migration: "users-email".to_owned(),
                }),
            ),
        ];

        for (value, expected) in cases {
            let event: LedgerEvent = serde_json::from_value(value.clone()).expect("registered");
            assert_eq!(event, expected, "{value} reads");
            assert_eq!(
                serde_json::to_value(&event).expect("serialises"),
                value,
                "and is written back under the same keys"
            );
            assert_eq!(
                Some(event.kind().as_str()),
                value["type"].as_str(),
                "its kind is the stored name"
            );
            assert_eq!(
                (event.kind().subject(), event.subject_value()),
                (TagKey::Migration, Some("users-email".into())),
                "its subject is the migration"
            );
        }
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
