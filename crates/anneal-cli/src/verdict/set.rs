//! `anneal verdict set`: records the verdict, then posts its `anneal/verdict` commit status
//! (`commit_status`) unless `--no-status` is given. The verdict stays recorded whatever becomes
//! of the status; a status that was asked for and not posted exits
//! [`exit_codes::VERDICT_STATUS_UNPOSTED`] with a finding and one `WARNING` line on stderr.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anneal_ledger::event::VerdictSet;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, Timestamp};
use serde::Serialize;

use super::commit_status::http::Client;
use super::commit_status::{self, CommitStatus, GithubEnv, Skip, Subject};
use super::{Io, Place, SetArgs, pull_request};
use crate::command::CommandName;
use crate::decide::label;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths::{self, GIT_TIMEOUT};
use crate::pr_diff::{self, Budget, PrError};

/// How long posting the commit status may take, DNS to last byte (H-13).
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `verdict set` in the worktree whose root is `root`, with `io`.
pub(super) fn run<L, O, E>(
    args: &SetArgs,
    root: &Path,
    io: Io<'_, L, O, E>,
) -> Result<Code, CliError>
where
    L: Fn(&str) -> Option<OsString>,
    O: Write,
    E: Write,
{
    let Io {
        cwd,
        lookup,
        github,
        out,
        err,
    } = io;
    let recorded = record(args, Place { cwd, root }, &lookup)?;
    let at = commit_status::Place { cwd, root, lookup };
    let status = status(args, &recorded.sha, at, github);
    report(args, &recorded, &status, (out, err))
}

/// A verdict `verdict set` appended: the envelope, the ledger it went to and its position,
/// and the full commit it is for.
struct Recorded {
    envelope: EventEnvelope,
    store: PathBuf,
    position: u64,
    sha: String,
}

/// `data` of `verdict set`: the event's payload, where it was recorded and what became of its
/// commit status.
#[derive(Debug, Serialize)]
struct SetData<'a> {
    #[serde(flatten)]
    event: &'a LedgerEvent,
    store: &'a Path,
    position: u64,
    commit_status: &'a CommitStatus,
}

/// Resolves `args`' commit and base and appends their `verdict_set` to the ledger seen from
/// `place`.
fn record(
    args: &SetArgs,
    place: Place<'_>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<Recorded, CliError> {
    let Place { cwd, root } = place;
    let budget = Budget::per_call(GIT_TIMEOUT);
    let sha = pr_diff::resolve_commit(root, &args.sha, budget.next().map_err(PrError::from)?)
        .map_err(PrError::from)?
        .ok_or_else(|| PrError::UnknownCommit {
            flag: "--sha",
            rev: args.sha.clone(),
        })?;
    let base = match &args.base {
        Some(base) => base.clone(),
        None => pr_diff::default_base(root, budget.next().map_err(PrError::from)?)
            .map_err(PrError::from)?
            .ok_or(PrError::NoBase)?
            .to_owned(),
    };
    let base_commit = pull_request::resolve_base(root, &base, budget)?;
    let patch_id = pr_diff::patch_id(
        root,
        &base_commit,
        &sha,
        budget.next().map_err(PrError::from)?,
    )
    .map_err(PrError::from)?;
    let event = LedgerEvent::VerdictSet(VerdictSet {
        pr: args.pr,
        verdict: args.level.into(),
        source: Some(args.source.into()),
        // The event stores one copy; the commit status names the commit after the append.
        sha: Some(sha.clone()),
        patch_id,
        base: Some(base),
        by: label::Source {
            actor: args.actor,
            via: args.via,
        }
        .by(),
    });
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.session_id.clone_from(&args.session);
    // The pull request and session become ledger tags; check them before a ledger is opened
    // (or created), so a value the store would refuse is a usage error that leaves no file.
    if let Err(error) = envelope.tags() {
        return Err(CliError::Usage {
            reason: format!("the --session is recorded as a ledger tag: {error}"),
        });
    }
    let location = paths::ledger_store(None, lookup, cwd)?;
    let position = Store::open(&location.path)?.append(&envelope)?;
    Ok(Recorded {
        envelope,
        store: location.path,
        position,
        sha,
    })
}

/// The commit status of the verdict `args` recorded for `sha`: skipped with `--no-status`,
/// else posted from `at`.
fn status<L: Fn(&str) -> Option<OsString>>(
    args: &SetArgs,
    sha: &str,
    at: commit_status::Place<'_, L>,
    github: &GithubEnv,
) -> CommitStatus {
    if args.no_status {
        return CommitStatus::Skipped {
            reason: Skip::NoStatus,
        };
    }
    let subject = Subject {
        pr: args.pr,
        sha,
        level: args.level,
        source: args.source,
    };
    commit_status::post(github, subject, at, &Client::new(STATUS_TIMEOUT))
}

/// Writes the envelope of `recorded` and its `status` to `out` and, when the status was asked
/// for and not posted, the warning to `err`; returns the exit code.
fn report<O: Write, E: Write>(
    args: &SetArgs,
    recorded: &Recorded,
    status: &CommitStatus,
    (mut out, mut err): (O, E),
) -> Result<Code, CliError> {
    let failure = status.failure();
    let findings: Vec<Finding> = failure
        .map(|reason| commit_status::finding(args.pr, reason))
        .into_iter()
        .collect();
    let code = match failure {
        Some(_) => exit_codes::VERDICT_STATUS_UNPOSTED,
        None => exit_codes::VERDICT_VALID,
    };
    let data = SetData {
        event: &recorded.envelope.event,
        store: &recorded.store,
        position: recorded.position,
        commit_status: status,
    };
    Envelope::new(CommandName::Verdict, Some("set"), code, data)
        .with_findings(&findings)
        .emit(&mut out)
        .map_err(CliError::Output)?;
    let Some(reason) = failure else {
        return Ok(code);
    };
    out.flush().map_err(CliError::Output)?;
    match commit_status::warn(&mut err, args.pr, &recorded.sha, reason) {
        // Best effort, as `calibrate`'s summary: the envelope on stdout already carries the
        // failure (its finding and this exit code), and stderr is the stream that refused it.
        Ok(()) | Err(_) => Ok(code),
    }
}
