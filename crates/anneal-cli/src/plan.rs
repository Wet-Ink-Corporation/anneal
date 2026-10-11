//! `anneal plan approve|check`: an approval of a plan that a script can verify.
//!
//! `approve` refuses (exit 3, nothing appended, no ledger opened) while `lint plan` has a
//! finding or the linked spec has any `[NEEDS CLARIFICATION]` marker; otherwise it appends
//! `plan_approved {plan, hash}`, the plan's id and the `sha256:` hash of its bytes, unless the
//! latest approval already has that hash. `check` compares the plan's hash with the latest
//! approval: 0 current, 3 never approved, 4 stale. It never creates a ledger. See
//! `docs/plan.md`.

mod check;

use std::borrow::Cow;
use std::ffi::OsString;
use std::io::Write;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use anneal_ledger::event::PlanApproved;
use anneal_ledger::{
    Appended, Condition, EventEnvelope, EventKind, Filter, LedgerEvent, LedgerTag, Recorded, Store,
    TagKey, Timestamp,
};
use clap::{Args, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::plan::{self as plan_lint, PlanFinding};
use crate::lint::{read_whole, read_whole_bytes, spec};
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
    /// Record the human's approval of a plan: refused while `lint plan` fails or its spec has
    /// a [NEEDS CLARIFICATION] marker
    Approve(ApproveArgs),
    /// Check that the plan is approved and unchanged since
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

/// A plan read for approval or a check: its id, bytes and hash.
#[derive(Debug)]
struct Read {
    id: String,
    bytes: Vec<u8>,
    hash: String,
}

/// Reads the plan at `file`, as `lint plan` does (relative to the process's working
/// directory): a missing file, a directory or one over the read cap is a usage error, and so
/// is a path that is not UTF-8 or that the ledger refuses as a tag value. Checked before any
/// ledger is opened.
fn read(file: &Path) -> Result<(Read, Filter), CliError> {
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
    Ok((Read { id, bytes, hash }, approvals))
}

/// The plan's id: its path as given, separated by `/` on every platform, without `.` or empty
/// components, so `./p.md`, `p.md` and (on Windows) `.\p.md` are one plan. `None` when the
/// path is not UTF-8.
fn plan_id(file: &Path) -> Option<String> {
    let text = file.to_str()?;
    let slashed = if MAIN_SEPARATOR == '/' {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.replace(MAIN_SEPARATOR, "/"))
    };
    let kept: Vec<&str> = slashed
        .split('/')
        .enumerate()
        // A leading empty segment is the root of an absolute path; keep it.
        .filter(|(at, segment)| *segment != "." && (*at == 0 || !segment.is_empty()))
        .map(|(_, segment)| segment)
        .collect();
    Some(kept.join("/"))
}

/// `sha256:` and the 64 hex digits of the SHA-256 of `bytes`.
fn sha256(bytes: &[u8]) -> String {
    let hex = Sha256::digest(bytes)
        .into_iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16));
    "sha256:".chars().chain(hex).collect()
}

/// The latest event in `store` among `approvals` (the approvals of one plan).
fn latest(store: &Store, approvals: &Filter) -> Result<Option<Recorded>, CliError> {
    Ok(store.tail(approvals, 1)?.pop())
}

/// The hash an approval records; `None` for any other event.
const fn approved_hash(recorded: &Recorded) -> Option<&String> {
    match &recorded.event.event {
        LedgerEvent::PlanApproved(PlanApproved { hash, .. }) => Some(hash),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
struct Refused<'a> {
    plan: &'a str,
    hash: &'a str,
    spec: Option<&'a Path>,
    needs_clarification: Option<usize>,
}

#[derive(Debug, Serialize)]
struct Approved<'a> {
    plan: &'a str,
    hash: &'a str,
    spec: Option<&'a Path>,
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
    let (plan, approvals) = read(&args.file)?;
    // Built first, so a session id the ledger refuses is a usage error before any ledger opens.
    let event = approval(&plan, args.session.as_deref())?;
    let outcome = plan_lint::examine(&args.file, &plan.bytes);
    let spec_path = outcome.spec.as_deref();
    if let Some(refusal) = refusal(&outcome)? {
        return refuse(&plan, spec_path, &refusal, out);
    }
    let location = paths::ledger_store(args.store.as_deref(), lookup, cwd)?;
    let store = Store::open(&location.path)?;
    let (appended, position) = match record(&store, &event, approvals, &plan.hash)? {
        Recording::Already(position) => (false, position),
        Recording::Appended(position) => (true, position),
        Recording::Conflict => {
            let refusal = Refusal {
                needs_clarification: None,
                findings: vec![Finding::new(
                    format!("plan/approve/conflict:{}", plan.id),
                    "another approval of this plan was recorded at the same time. fix: run \
                     `anneal plan check`, then approve again if it is not current.",
                )],
            };
            return refuse(&plan, spec_path, &refusal, out);
        }
    };
    let data = Approved {
        plan: &plan.id,
        hash: &plan.hash,
        spec: spec_path,
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

/// Why an approval is refused: its findings, and the spec's marker count when that is why.
#[derive(Debug)]
struct Refusal {
    needs_clarification: Option<usize>,
    findings: Vec<Finding>,
}

/// The refusal for a plan with lint findings, or whose spec has markers; `None` when it may
/// be approved.
fn refusal(outcome: &plan_lint::Outcome) -> Result<Option<Refusal>, CliError> {
    let findings = &outcome.report.findings;
    let Some(spec_path) = outcome.spec.as_deref().filter(|_| findings.is_empty()) else {
        // A plan without a spec link has a lint finding, so this lists at least one.
        return Ok(Some(Refusal {
            needs_clarification: None,
            findings: findings.iter().map(PlanFinding::to_envelope).collect(),
        }));
    };
    let spec_text = read_whole(spec_path).map_err(CliError::Plan)?;
    let open = spec::needs_clarification(&spec_text);
    if open == 0 {
        return Ok(None);
    }
    let finding = Finding::new(
        format!("plan/approve/needs-clarification:{}", spec_path.display()),
        format!(
            "{}: the spec has {open} [NEEDS CLARIFICATION] marker(s); plan approval needs none. \
             fix: resolve each with the human, then approve again.",
            spec_path.display()
        ),
    );
    Ok(Some(Refusal {
        needs_clarification: Some(open),
        findings: vec![finding],
    }))
}

/// Writes the refusal envelope: exit 3, not approved.
fn refuse<O: Write>(
    plan: &Read,
    spec: Option<&Path>,
    refusal: &Refusal,
    out: &mut O,
) -> Result<Code, CliError> {
    let data = Refused {
        plan: &plan.id,
        hash: &plan.hash,
        spec,
        needs_clarification: refusal.needs_clarification,
    };
    let envelope = Envelope::new(
        CommandName::Plan,
        Some(APPROVE),
        exit_codes::PLAN_NOT_APPROVED,
        data,
    );
    emit(&envelope.with_findings(&refusal.findings), out)
}

/// The `plan_approved` event for `plan`, with the session it names.
fn approval(plan: &Read, session: Option<&str>) -> Result<EventEnvelope, CliError> {
    let event = LedgerEvent::PlanApproved(PlanApproved {
        // The event owns what it records; `plan` is still reported afterwards.
        plan: plan.id.clone(),
        hash: plan.hash.clone(),
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
    /// Another approval of the plan landed after the one read; nothing was appended.
    Conflict,
}

/// Appends `event` unless the latest of `approvals` already has `hash`, and only if no other
/// approval lands after the one read.
fn record(
    store: &Store,
    event: &EventEnvelope,
    approvals: Filter,
    hash: &str,
) -> Result<Recording, CliError> {
    let previous = latest(store, &approvals)?;
    if let Some(recorded) = &previous
        && approved_hash(recorded).is_some_and(|approved| approved == hash)
    {
        return Ok(Recording::Already(recorded.position));
    }
    let after = previous.map_or(0, |recorded| recorded.position);
    let condition = Condition::none_matching(approvals).after(after);
    Ok(match store.append_if(event, &condition)? {
        Appended::At(position) => Recording::Appended(position),
        Appended::Conflict => Recording::Conflict,
    })
}

fn emit<O: Write, D: Serialize>(envelope: &Envelope<'_, D>, out: &mut O) -> Result<Code, CliError> {
    envelope.emit(out).map_err(CliError::Output)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{plan_id, sha256};

    #[test]
    fn the_hash_is_sha256_in_lowercase_hex() {
        assert_eq!(
            sha256(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "the FIPS 180-2 test vector"
        );
    }

    #[test]
    fn the_plan_id_drops_only_current_directory_components() {
        assert_eq!(
            plan_id(Path::new("./a/./b.md")).as_deref(),
            Some("a/b.md"),
            "dots"
        );
        assert_eq!(
            plan_id(Path::new("a/../b.md")).as_deref(),
            Some("a/../b.md"),
            "parents kept"
        );
    }

    #[test]
    fn the_plan_id_keeps_the_root_and_drops_empty_components() {
        assert_eq!(
            plan_id(Path::new("/abs/./p.md")).as_deref(),
            Some("/abs/p.md"),
            "absolute"
        );
        assert_eq!(
            plan_id(Path::new("a//b.md")).as_deref(),
            Some("a/b.md"),
            "doubled separator"
        );
    }
}
