//! `anneal plan approve|check`: an approval of a plan that a script can verify.
//!
//! `approve` refuses (exit 3, nothing appended, no ledger opened) while `lint plan` has a
//! finding or the linked spec has any `[NEEDS CLARIFICATION]` marker, and (exit 3, nothing
//! appended, no ledger created) while the design tree the spec's `Tree:` line names has no
//! `grill_confirmed` with its current hash (`plan::tree`, #77); otherwise it appends
//! `plan_approved {plan, hash, by?}`, the plan's id, the `sha256:` hash of its bytes and, when
//! `--actor` or `--via` is given, who approved it and through what (#62), unless the latest
//! approval already has that hash and that approver. Losing a race to an approval of the same
//! hash by another approver appends after it; losing one to another hash is a conflict (exit
//! 3). `check` compares the plan's hash with the latest approval: 0 current, 3 never approved,
//! 4 stale. It never creates a ledger. See `docs/plan.md`.

mod check;
mod tree;

pub(crate) use check::Status;
use tree::{Confirmation, Read as TreeRead};

use std::ffi::OsString;
use std::io::Write;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use anneal_ledger::event::{By, PlanApproved};
use anneal_ledger::{
    Appended, Condition, EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, Recorded, Store,
    TagKey, Timestamp,
};
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::content_hash::sha256;
use crate::decide::label::{Actor, Source, Via};
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::plan::{self as plan_lint, PlanFinding};
use crate::lint::spec::TreeLink;
use crate::lint::{read_whole, read_whole_bytes, spec};
use crate::path_text::slashed;
use crate::paths::{self, StoreSource};

/// `anneal plan`.
#[derive(Debug, Args)]
pub struct PlanArgs {
    #[command(subcommand)]
    action: PlanAction,
}

/// The `anneal plan` subcommands.
#[derive(Debug, Subcommand)]
pub enum PlanAction {
    /// Record the human's approval of a plan: refused while `lint plan` fails, its spec has a
    /// [NEEDS CLARIFICATION] marker, or the design tree its spec's `Tree:` line names is not
    /// confirmed at its current hash
    Approve(ApproveArgs),
    /// Check that the plan's bytes are unchanged since its latest approval (its spec is not
    /// compared)
    Check(CheckArgs),
}

/// `anneal plan approve`.
#[derive(Debug, Args)]
pub struct ApproveArgs {
    /// The plan file
    #[arg(value_name = "FILE")]
    file: PathBuf,
    /// The session id to record
    #[arg(long, value_name = "ID")]
    session: Option<String>,
    /// Who approved the plan [default: none recorded, so no approver is claimed]
    #[arg(long, value_enum)]
    actor: Option<Actor>,
    /// Through what it was approved (mod: a Claude Code mod's buttons) [default: none recorded]
    #[arg(long, value_enum)]
    via: Option<Via>,
    /// Use this ledger file instead of the resolved one
    #[arg(long, value_name = "FILE")]
    store: Option<PathBuf>,
}

/// `anneal plan check`.
#[derive(Debug, Args)]
pub struct CheckArgs {
    /// The plan file
    #[arg(value_name = "FILE")]
    file: PathBuf,
    /// Use this ledger file instead of the resolved one
    #[arg(long, value_name = "FILE")]
    store: Option<PathBuf>,
}

/// Runs `anneal plan <action>` in `cwd`, reading environment variables through `lookup`.
pub(crate) fn run<O: Write>(
    args: &PlanArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.action {
        PlanAction::Approve(approve_args) => approve(approve_args, cwd, lookup, out),
        PlanAction::Check(check_args) => check::check(check_args, cwd, lookup, out),
    }
}

/// A plan read for approval or a check: its id, bytes and hash, and the filter that selects
/// its approvals in the ledger.
#[derive(Debug)]
struct PlanBytes {
    id: String,
    bytes: Vec<u8>,
    hash: String,
    approvals: Filter,
}

/// Reads the plan at `file`, as `lint plan` does (relative to the process's working
/// directory): a missing file, a directory or one over the read cap is a usage error, and so
/// is a path that is not UTF-8 or that the ledger refuses as a tag value. Checked before any
/// ledger is opened.
fn read(file: &Path) -> Result<PlanBytes, CliError> {
    let id = plan_id(file).ok_or_else(|| CliError::Usage {
        reason: "the plan path must be UTF-8: it is recorded as the plan id".to_owned(),
    })?;
    let tag = LedgerTag::new(TagKey::Plan, &id).map_err(|error| CliError::Usage {
        reason: format!("the plan path is recorded as a ledger tag: {error}"),
    })?;
    let approvals = Filter {
        types: vec![EventKind::PlanApproved],
        tags: vec![tag],
    };
    let bytes = read_whole_bytes(file).map_err(CliError::Plan)?;
    let hash = sha256(&bytes);
    Ok(PlanBytes {
        id,
        bytes,
        hash,
        approvals,
    })
}

/// The plan's id: its path as given, separated by `/` on every platform, without `.`
/// components or repeated separators, so `./p.md`, `p.md` and (on Windows) `.\p.md` are one
/// plan. Leading separators are kept as written: `/abs/p.md` stays absolute and a UNC path
/// (`\\server\share\p.md`) keeps its `//`. `None` when the path is not UTF-8.
fn plan_id(file: &Path) -> Option<String> {
    Some(normalised(file.to_str()?, MAIN_SEPARATOR))
}

/// The plan id for the path `text` on a platform whose separator is `separator`.
fn normalised(text: &str, separator: char) -> String {
    let slashed = slashed(text, separator);
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for segment in slashed.split('/') {
        // Empty segments before the first name are the root (`/`) or a UNC prefix (`//`).
        if segment == "." || (inside && segment.is_empty()) {
            continue;
        }
        inside |= !segment.is_empty();
        kept.push(segment);
    }
    kept.join("/")
}

/// Whether the plan recorded under `id` is approved at `bytes`, its current content: what
/// `plan check` reports, for a caller that already holds the plan and the ledger (the
/// preconditions hook, which reads the plan by its worktree-relative id).
///
/// # Errors
/// [`CliError::Usage`] when `id` cannot be a ledger tag, [`CliError::Ledger`] when the ledger
/// cannot be read.
pub(crate) fn approval_of(store: &Store, id: &str, bytes: &[u8]) -> Result<Status, CliError> {
    Ok(approval_record(store, id, bytes)?.status)
}

/// A plan's approval as `plan check` judges it: the status, the hash of the plan's current
/// bytes, and the latest approval's hash, ledger position and time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Approval {
    /// Whether the plan is approved at its current bytes.
    pub(crate) status: Status,
    /// The `sha256:` hash of the plan's current bytes.
    pub(crate) hash: String,
    /// The latest approval, if any: its hash, position and `ts`.
    pub(crate) latest: Option<ApprovalRecord>,
}

/// One `plan_approved` event: what it approved and where the ledger holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalRecord {
    /// The hash it approved.
    pub(crate) hash: String,
    /// Its ledger position.
    pub(crate) position: u64,
    /// When it was recorded.
    pub(crate) ts: String,
}

/// [`approval_of`] with the hashes and the latest approval, for a caller that shows them
/// (`anneal status`).
///
/// # Errors
/// As [`approval_of`].
pub(crate) fn approval_record(store: &Store, id: &str, bytes: &[u8]) -> Result<Approval, CliError> {
    let tag = LedgerTag::new(TagKey::Plan, id).map_err(|error| CliError::Usage {
        reason: format!("the plan path is recorded as a ledger tag: {error}"),
    })?;
    let approvals = Filter {
        types: vec![EventKind::PlanApproved],
        tags: vec![tag],
    };
    let previous = latest(store, &approvals)?;
    let hash = sha256(bytes);
    let status = check::status_of(&hash, previous.as_ref());
    let latest = previous.and_then(|recorded| match recorded.event.event {
        LedgerEvent::PlanApproved(PlanApproved { hash, .. }) => Some(ApprovalRecord {
            hash,
            position: recorded.position,
            ts: recorded.event.ts.as_str().to_owned(),
        }),
        _ => None,
    });
    Ok(Approval {
        status,
        hash,
        latest,
    })
}

/// The latest event in `store` among `approvals` (the approvals of one plan).
fn latest(store: &Store, approvals: &Filter) -> Result<Option<Recorded>, CliError> {
    Ok(store.tail(approvals, 1)?.pop())
}

/// The approval `event` is; `None` for any other event.
const fn approved(event: &EventEnvelope) -> Option<&PlanApproved> {
    match &event.event {
        LedgerEvent::PlanApproved(approval) => Some(approval),
        _ => None,
    }
}

/// The hash an approval records; `None` for any other event.
fn approved_hash(recorded: &Recorded) -> Option<&str> {
    approved(&recorded.event).map(|approval| approval.hash.as_str())
}

#[derive(Debug, Serialize)]
struct Refused<'a> {
    plan: &'a str,
    hash: &'a str,
    spec: Option<&'a Path>,
    needs_clarification: Option<usize>,
    /// Lint findings dropped at `lint plan`'s cap; present only when some were.
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted: Option<usize>,
    /// The spec's design tree, when the refusal is about it; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    tree: Option<&'a Path>,
}

#[derive(Debug, Serialize)]
struct Approved<'a> {
    plan: &'a str,
    hash: &'a str,
    spec: Option<&'a Path>,
    /// The spec's confirmed design tree; absent for a spec with no `Tree:` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    tree: Option<&'a Path>,
    /// Who approved it and through what; absent when the approval names neither.
    #[serde(skip_serializing_if = "Option::is_none")]
    by: Option<By>,
    appended: bool,
    store: &'a Path,
    source: StoreSource,
    position: u64,
}

const APPROVE: &str = "approve";

fn approve<O: Write>(
    args: &ApproveArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let plan = read(&args.file)?;
    // Built first, so a session id the ledger refuses is a usage error before any ledger opens.
    let by = Source {
        actor: args.actor,
        via: args.via,
    }
    .by();
    let event = approval_by(&plan, args.session.as_deref(), by)?;
    let outcome = plan_lint::examine(&args.file, &plan.bytes);
    let spec_path = outcome.spec.as_deref();
    let (spec, link) = match check_lint_and_spec(&outcome)? {
        Checked::Refused(refusal) => return refuse(&plan, spec_path, &refusal, out),
        Checked::Passed { spec, tree } => (spec, tree),
    };
    // What the files alone decide is decided before the ledger is located.
    let linked = match link {
        None => None,
        Some(link) => match tree::read(spec, &link)? {
            TreeRead::Tree(linked) => Some(linked),
            TreeRead::Refused(refusal) => return refuse(&plan, spec_path, &refusal, out),
        },
    };
    let location = paths::ledger_store(args.store.as_deref(), lookup, cwd)?;
    let (store, tree) = match linked {
        None => (Store::open(&location.path)?, None),
        Some(linked) => match tree::confirmed(linked, &location.path)? {
            Confirmation::Confirmed { ledger, path } => (ledger, Some(path)),
            Confirmation::Refused(refusal) => return refuse(&plan, spec_path, &refusal, out),
        },
    };
    let previous = latest(&store, &plan.approvals)?;
    let (appended, position) = match record(&store, &event, &plan, previous.as_ref())? {
        Recording::Already(position) => (false, position),
        Recording::Appended(position) => (true, position),
        Recording::Conflict => return refuse(&plan, spec_path, &conflict(&plan), out),
    };
    let data = Approved {
        plan: &plan.id,
        hash: &plan.hash,
        spec: spec_path,
        tree: tree.as_deref(),
        by,
        appended,
        store: &location.path,
        source: location.source,
        position,
    };
    emit(
        &Envelope::new(
            CommandName::Plan,
            Some(APPROVE),
            exit_codes::PLAN_APPROVED,
            data,
        ),
        out,
    )
}

/// Why an approval is refused: its findings, the spec's marker count when that is why, how
/// many lint findings were dropped at the cap, if any, and the spec's design tree when that is
/// why.
#[derive(Debug)]
struct Refusal {
    needs_clarification: Option<usize>,
    findings: Vec<Finding>,
    omitted: Option<usize>,
    tree: Option<PathBuf>,
}

impl Refusal {
    /// The refusal with one finding and nothing else.
    fn of(finding: Finding) -> Self {
        Self {
            needs_clarification: None,
            findings: vec![finding],
            omitted: None,
            tree: None,
        }
    }
}

/// What the checks before the ledger found.
#[derive(Debug)]
enum Checked<'o> {
    /// The approval is refused.
    Refused(Refusal),
    /// The plan and its spec pass; the spec at this path links this design tree, if any.
    Passed {
        spec: &'o Path,
        tree: Option<TreeLink>,
    },
}

/// The checks of the plan's lint findings and its spec's markers: the refusal for a plan with
/// lint findings, or whose spec has markers; otherwise [`Checked::Passed`] with the spec and
/// the design tree it links, which the ledger must show confirmed.
fn check_lint_and_spec(outcome: &plan_lint::Outcome) -> Result<Checked<'_>, CliError> {
    let report = &outcome.report;
    let findings = &report.findings;
    let Some(spec_path) = outcome.spec.as_deref().filter(|_| findings.is_empty()) else {
        // A plan without a spec link has a lint finding, so this lists at least one.
        return Ok(Checked::Refused(Refusal {
            needs_clarification: None,
            findings: findings.iter().map(PlanFinding::to_envelope).collect(),
            omitted: (report.omitted > 0).then_some(report.omitted),
            tree: None,
        }));
    };
    let spec_text = read_whole(spec_path).map_err(CliError::Plan)?;
    let open = spec::needs_clarification(&spec_text);
    if open == 0 {
        return Ok(Checked::Passed {
            spec: spec_path,
            tree: spec::tree_link(&spec_text),
        });
    }
    let finding = Finding::new(
        format!("plan/approve/needs-clarification:{}", spec_path.display()),
        format!(
            "{}: the spec has {open} [NEEDS CLARIFICATION] marker(s); plan approval needs none. \
             fix: resolve each with the human, then approve again.",
            spec_path.display()
        ),
    );
    Ok(Checked::Refused(Refusal {
        needs_clarification: Some(open),
        ..Refusal::of(finding)
    }))
}

/// The refusal when another approval of the plan landed between reading and appending.
fn conflict(plan: &PlanBytes) -> Refusal {
    Refusal::of(Finding::new(
        format!("plan/approve/conflict:{}", plan.id),
        "another approval of this plan was recorded at the same time. fix: run `anneal plan \
         check`, then approve again if it is not current.",
    ))
}

/// Writes the refusal envelope: exit 3, not approved.
fn refuse<O: Write>(
    plan: &PlanBytes,
    spec: Option<&Path>,
    refusal: &Refusal,
    out: &mut O,
) -> Result<Code, CliError> {
    let data = Refused {
        plan: &plan.id,
        hash: &plan.hash,
        spec,
        needs_clarification: refusal.needs_clarification,
        omitted: refusal.omitted,
        tree: refusal.tree.as_deref(),
    };
    let envelope = Envelope::new(
        CommandName::Plan,
        Some(APPROVE),
        exit_codes::PLAN_NOT_APPROVED,
        data,
    );
    emit(&envelope.with_findings(&refusal.findings), out)
}

/// The `plan_approved` event for `plan`, with the session it names and no approver.
#[cfg(test)]
fn approval(plan: &PlanBytes, session: Option<&str>) -> Result<EventEnvelope, CliError> {
    approval_by(plan, session, None)
}

/// The `plan_approved` event for `plan`, with the session it names and who approved it.
fn approval_by(
    plan: &PlanBytes,
    session: Option<&str>,
    by: Option<By>,
) -> Result<EventEnvelope, CliError> {
    let event = LedgerEvent::PlanApproved(PlanApproved {
        // The event owns what it records; `plan` is still reported afterwards.
        plan: plan.id.clone(),
        hash: plan.hash.clone(),
        by,
    });
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.session_id = session.map(str::to_owned);
    if let Err(error) = envelope.tags() {
        return Err(CliError::Usage {
            reason: format!("--session is recorded as a ledger tag: {error}"),
        });
    }
    Ok(envelope)
}

/// What recording an approval did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recording {
    /// The latest approval already has the hash; nothing was appended.
    Already(u64),
    /// Appended at this position.
    Appended(u64),
    /// Another approval of the plan, with another hash, landed after `previous` (or approvals
    /// of the same hash by other approvers kept landing first, [`MAX_APPROVER_RACES`] times);
    /// nothing was appended.
    Conflict,
}

/// How many times in a row an approval may lose its race to an approval of the same bytes by
/// another approver before it gives up as a conflict (H-13).
const MAX_APPROVER_RACES: usize = 8;

/// What [`record`] needs of the ledger: a conditioned append and the latest approval of a
/// plan. The [`Store`] in production; a fake in the tests, which reaches the race cap.
trait ApprovalLedger {
    /// Appends `event` only when `condition` holds.
    fn append_if(&self, event: &EventEnvelope, condition: &Condition)
    -> Result<Appended, CliError>;

    /// The latest event among `approvals`.
    fn latest(&self, approvals: &Filter) -> Result<Option<Recorded>, CliError>;
}

impl ApprovalLedger for Store {
    fn append_if(
        &self,
        event: &EventEnvelope,
        condition: &Condition,
    ) -> Result<Appended, CliError> {
        Ok(Self::append_if(self, event, condition)?)
    }

    fn latest(&self, approvals: &Filter) -> Result<Option<Recorded>, CliError> {
        latest(self, approvals)
    }
}

/// Appends `event`, the approval of `plan`, unless `previous` (the latest approval of `plan`
/// the caller read) already has its hash, and only if no approval lands after `previous`.
/// When one did, it is read back: one with the same hash and approver means the plan is
/// already approved, and one with another hash is a conflict. An approval counts as the same
/// only when it also names the same approver (`by`), so a human's approval of content a model
/// approved is recorded too: when the winner has the same hash but another approver, `event`
/// is appended after it, under the same condition.
fn record(
    store: &impl ApprovalLedger,
    event: &EventEnvelope,
    plan: &PlanBytes,
    previous: Option<&Recorded>,
) -> Result<Recording, CliError> {
    let by = approved(event).and_then(|approval| approval.by);
    let already = |recorded: &Recorded| {
        approved(&recorded.event)
            .is_some_and(|approval| approval.hash == plan.hash && approval.by == by)
            .then_some(Recording::Already(recorded.position))
    };
    if let Some(done) = previous.and_then(already) {
        return Ok(done);
    }
    let mut after = previous.map_or(0, |recorded| recorded.position);
    for _ in 0..MAX_APPROVER_RACES {
        // The condition owns its filter; `plan` keeps the filter to read the winner back.
        let condition = Condition::none_matching(plan.approvals.clone()).after(after);
        if let Appended::At(position) = store.append_if(event, &condition)? {
            return Ok(Recording::Appended(position));
        }
        let Some(winner) = store.latest(&plan.approvals)? else {
            return Ok(Recording::Conflict);
        };
        if let Some(done) = already(&winner) {
            return Ok(done);
        }
        if approved_hash(&winner) != Some(plan.hash.as_str()) {
            return Ok(Recording::Conflict);
        }
        // The same bytes, approved by another approver: this approval is still news.
        after = winner.position;
    }
    Ok(Recording::Conflict)
}

fn emit<O: Write, D: Serialize>(envelope: &Envelope<'_, D>, out: &mut O) -> Result<Code, CliError> {
    envelope.emit(out).map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
