//! `anneal plan check`: the plan's hash against its latest approval.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anneal_ledger::Store;
use serde::Serialize;

use super::{CheckArgs, approved_hash, emit, latest, read};
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths::{self, StoreSource};

/// Whether the plan is approved at its current content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// The latest approval has the plan's hash.
    Current,
    /// No approval of the plan.
    NotApproved,
    /// The latest approval has another hash: the plan changed since.
    Stale,
}

impl Status {
    const fn code(self) -> Code {
        match self {
            Self::Current => exit_codes::PLAN_APPROVED,
            Self::NotApproved => exit_codes::PLAN_NOT_APPROVED,
            Self::Stale => exit_codes::PLAN_STALE,
        }
    }
}

#[derive(Debug, Serialize)]
struct Checked<'a> {
    plan: &'a str,
    hash: &'a str,
    status: Status,
    approved: Option<ApprovalData<'a>>,
    store: &'a Path,
    source: StoreSource,
}

#[derive(Debug, Serialize)]
struct ApprovalData<'a> {
    hash: &'a str,
    position: u64,
    ts: &'a str,
}

/// Runs `anneal plan check`: 0 current, 3 not approved, 4 stale. Never creates a ledger.
pub(super) fn check<O: Write>(
    args: &CheckArgs,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    out: &mut O,
) -> Result<Code, CliError> {
    let plan = read(&args.file)?;
    let location = paths::ledger_store(args.store.as_deref(), lookup, cwd)?;
    // A ledger that does not exist holds no approval; opening it would create it. Anything
    // else (a directory, an unreadable file) is opened, so its failure is reported (70)
    // rather than read as "not approved".
    let previous = match fs::metadata(&location.path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Ok(_) | Err(_) => latest(&Store::open(&location.path)?, &plan.approvals)?,
    };
    let approved = previous.as_ref().and_then(|recorded| {
        approved_hash(recorded).map(|hash| ApprovalData {
            hash,
            position: recorded.position,
            ts: recorded.event.ts.as_str(),
        })
    });
    let status = match &approved {
        None => Status::NotApproved,
        Some(approval) if *approval.hash == plan.hash => Status::Current,
        Some(_) => Status::Stale,
    };
    let data = Checked {
        plan: &plan.id,
        hash: &plan.hash,
        status,
        approved,
        store: &location.path,
        source: location.source,
    };
    let findings = match status {
        Status::Current => Vec::new(),
        Status::NotApproved => vec![Finding::new(
            format!("plan/check/not-approved:{}", plan.id),
            format!(
                "{} has no approval. fix: have the human review it, then run `anneal plan \
                 approve {}`.",
                plan.id, plan.id
            ),
        )],
        Status::Stale => vec![Finding::new(
            format!("plan/check/stale:{}", plan.id),
            format!(
                "{} changed since it was approved. fix: have the human review the change, then \
                 run `anneal plan approve {}`.",
                plan.id, plan.id
            ),
        )],
    };
    let envelope = Envelope::new(CommandName::Plan, Some("check"), status.code(), data);
    emit(&envelope.with_findings(&findings), out)
}
