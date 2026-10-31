//! `verdict`: the review verdict for `HEAD`, judged as `anneal verdict valid` judges it
//! (`verdict::pull_request::read` and `assess`), so a rebase that kept the patch id keeps it.
//!
//! The ledger records verdicts per pull request, and nothing maps `HEAD` to one, so the pull
//! request is the one of the newest `verdict_set` whose commit is `HEAD`, else of the newest
//! `verdict_set` in the ledger. With no verdict at all, or an unborn `HEAD`, the section is
//! absent.

use std::path::Path;
use std::time::Duration;

use anneal_ledger::event::{Verdict, VerdictSet, VerdictSource};
use anneal_ledger::{EventKind, Filter, LedgerEvent, Store};
use serde::Serialize;

use super::Unreadable;
use crate::pr_diff::{self, Budget};
use crate::repo_config;
use crate::verdict::assess::{Fresh, Invalid, Judged, Outcome};
use crate::verdict::pull_request::{self, MAX_VERDICTS, Query};

/// How long each git call may take: a UI refreshes often (H-13).
const GIT_CALL: Duration = Duration::from_secs(2);

/// `verdict`: the verdict for `HEAD`.
#[derive(Debug, Serialize)]
pub(crate) struct HeadVerdict {
    /// The pull request judged.
    pr: u64,
    /// `HEAD`'s full commit id.
    head: String,
    /// The base the patch id is taken from: the latest verdict's.
    base: Option<String>,
    /// `valid`, `noise_only`, `no_verdict`, `stale` or `changes_requested`, as `verdict valid`.
    outcome: Shown,
    /// True exactly when `outcome` is `valid`.
    valid: bool,
    /// The latest verdict still fresh for `HEAD`: by commit, or by patch id after a rebase.
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<FreshVerdict>,
    /// How many of the pull request's verdicts are for other code.
    stale: usize,
}

impl HeadVerdict {
    /// The pull request, the outcome and the fresh verdict's level, for the text form.
    pub(super) fn parts(&self) -> (u64, Shown, Option<Verdict>) {
        (
            self.pr,
            self.outcome,
            self.verdict.as_ref().map(|fresh| fresh.verdict),
        )
    }
}

/// The verdict's outcome, as `verdict valid` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Shown {
    /// The latest fresh verdict is a level.
    Valid,
    /// The latest fresh verdict says every finding was noise.
    NoiseOnly,
    /// No verdict was ever set for the pull request.
    NoVerdict,
    /// Every verdict is for other code.
    Stale,
    /// The latest fresh verdict requests changes.
    ChangesRequested,
}

impl Shown {
    /// The name, for the text form.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::NoiseOnly => "noise_only",
            Self::NoVerdict => "no_verdict",
            Self::Stale => "stale",
            Self::ChangesRequested => "changes_requested",
        }
    }
}

impl From<Outcome> for Shown {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Valid => Self::Valid,
            Outcome::NoiseOnly => Self::NoiseOnly,
            Outcome::Invalid(Invalid::NoVerdict) => Self::NoVerdict,
            Outcome::Invalid(Invalid::Stale) => Self::Stale,
            Outcome::Invalid(Invalid::ChangesRequested) => Self::ChangesRequested,
        }
    }
}

/// A fresh verdict: what it says, who set it, for which commit, and why it is fresh.
#[derive(Debug, Serialize)]
struct FreshVerdict {
    event: u64,
    verdict: Verdict,
    source: Option<VerdictSource>,
    sha: Option<String>,
    fresh: Fresh,
}

impl From<Judged<'_>> for FreshVerdict {
    fn from(judged: Judged<'_>) -> Self {
        let set = judged.record.set;
        Self {
            event: judged.record.position,
            verdict: set.verdict,
            source: set.source,
            // The pull request read is dropped after this; the section keeps its own copy.
            sha: set.sha.clone(),
            fresh: judged.fresh,
        }
    }
}

/// The verdict for `HEAD` of the worktree at `root`; `None` with no verdict in `store` or an
/// unborn `HEAD`.
pub(super) fn read(store: &Store, root: &Path) -> Result<Option<HeadVerdict>, Unreadable> {
    let budget = Budget::per_call(GIT_CALL);
    let filter = Filter {
        types: vec![EventKind::VerdictSet],
        tags: Vec::new(),
    };
    let sets: Vec<VerdictSet> = store
        .tail(&filter, MAX_VERDICTS)?
        .into_iter()
        .filter_map(|recorded| match recorded.event.event {
            LedgerEvent::VerdictSet(set) => Some(set),
            _ => None,
        })
        .collect();
    let Some(newest) = sets.last() else {
        return Ok(None);
    };
    let timeout = budget.next()?;
    let Some(head) = pr_diff::resolve_commit(root, "HEAD", timeout)? else {
        return Ok(None);
    };
    let pr = sets
        .iter()
        .rev()
        .find(|set| set.sha.as_deref() == Some(head.as_str()))
        .unwrap_or(newest)
        .pr;
    let config = repo_config::gate(root)?;
    let query = Query {
        root,
        pr,
        head: ("HEAD", &head),
        base: None,
        budget,
    };
    let pull = pull_request::read(Some(store), &query)?;
    let assessment = pull.assess(config.require_plugin);
    let outcome = Shown::from(assessment.outcome());
    Ok(Some(HeadVerdict {
        pr,
        verdict: assessment.latest.map(FreshVerdict::from),
        stale: assessment.stale,
        valid: outcome == Shown::Valid,
        outcome,
        base: pull.base,
        head: pull.head,
    }))
}
