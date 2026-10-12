//! A pull request's verdicts read from the ledger, with the head they are judged against:
//! shared by `verdict valid` and the merge gate.

use std::path::Path;

use anneal_ledger::event::VerdictSet;
use anneal_ledger::{
    EventKind, Filter, LedgerEvent, LedgerTag, Recorded, Store, StoreError, TagError, TagKey,
};

use super::assess::{self, Assessment, Head, Record};
use crate::pr_diff::{self, Budget, PrError};

/// The most verdicts of one pull request read, the latest (H-13).
pub(crate) const MAX_VERDICTS: usize = 64;

/// What to read: the pull request, the revision of its head, and the base its patch id is
/// taken from (`None`: the base the latest verdict recorded).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Query<'a> {
    /// The worktree's root.
    pub(crate) root: &'a Path,
    /// The pull request number.
    pub(crate) pr: u64,
    /// The head's revision as given, and the flag that gave it.
    pub(crate) head: (&'static str, &'a str),
    /// `--base`, if given.
    pub(crate) base: Option<&'a str>,
    /// How long each git call may take.
    pub(crate) budget: Budget,
}

/// The pull request's verdicts, at most [`MAX_VERDICTS`] in ledger order, and its head.
#[derive(Debug)]
pub(crate) struct PullRequest {
    /// The pull request number.
    pub(crate) pr: u64,
    /// The head's full commit id.
    pub(crate) head: String,
    /// The base the patch id is taken from: `--base`, else the latest verdict's; `None` when
    /// neither names one.
    pub(crate) base: Option<String>,
    /// The head's patch id from `base`, computed only when a verdict could be fresh by it.
    pub(crate) patch_id: Option<String>,
    verdicts: Vec<(u64, VerdictSet)>,
}

/// Why the verdicts could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    /// git could not resolve the head or base, or diff them.
    #[error(transparent)]
    Pr(#[from] PrError),
    /// The ledger could not be read.
    #[error("reading the ledger")]
    Ledger(#[source] StoreError),
    /// The pull request number is not a tag the ledger accepts.
    #[error("the pull request number as a ledger tag")]
    Tag(#[source] TagError),
}

impl PullRequest {
    /// The verdicts as the assessment reads them.
    pub(crate) fn records(&self) -> Vec<Record<'_>> {
        self.verdicts
            .iter()
            .map(|(position, set)| Record {
                position: *position,
                set,
            })
            .collect()
    }

    /// The assessment of the verdicts against the head.
    pub(crate) fn assess(&self, require_plugin: bool) -> Assessment<'_> {
        let head = Head {
            sha: &self.head,
            base: self.base.as_deref(),
            patch_id: self.patch_id.as_deref(),
        };
        assess::assess(&self.records(), &head, require_plugin)
    }
}

/// Reads the pull request `query` names from `store` (no ledger: no verdicts).
///
/// # Errors
/// The [`ReadError`] when the head or base names no commit, git fails, or the ledger cannot be
/// read.
pub(crate) fn read(store: Option<&Store>, query: &Query<'_>) -> Result<PullRequest, ReadError> {
    let (flag, rev) = query.head;
    let head =
        pr_diff::resolve_commit(query.root, rev, query.budget.next().map_err(PrError::from)?)
            .map_err(PrError::from)?
            .ok_or_else(|| PrError::UnknownCommit {
                flag,
                rev: rev.to_owned(),
            })?;
    let verdicts = match store {
        Some(store) => verdicts(store, query.pr)?,
        None => Vec::new(),
    };
    // The pull request owns its base; the verdicts keep theirs.
    let base = query
        .base
        .map(str::to_owned)
        .or_else(|| verdicts.iter().rev().find_map(|(_, set)| set.base.clone()));
    let mut pull_request = PullRequest {
        pr: query.pr,
        head,
        base,
        patch_id: None,
        verdicts,
    };
    if let Some(base) = pull_request.base.as_deref()
        && assess::needs_patch_id(&pull_request.records(), &pull_request.head, base)
    {
        let resolved = resolve_base(query.root, base, query.budget)?;
        let timeout = query.budget.next().map_err(PrError::from)?;
        pull_request.patch_id =
            pr_diff::patch_id(query.root, &resolved, &pull_request.head, timeout)
                .map_err(PrError::from)?;
    }
    Ok(pull_request)
}

/// The full commit id `base` names.
///
/// # Errors
/// [`PrError::UnknownBase`] when it names none, or the git failure.
pub(crate) fn resolve_base(root: &Path, base: &str, budget: Budget) -> Result<String, PrError> {
    pr_diff::resolve_commit(root, base, budget.next()?)?.ok_or_else(|| PrError::UnknownBase {
        base: base.to_owned(),
    })
}

/// The latest [`MAX_VERDICTS`] `verdict_set` events of pull request `pr`, in ledger order.
fn verdicts(store: &Store, pr: u64) -> Result<Vec<(u64, VerdictSet)>, ReadError> {
    let tag = LedgerTag::new(TagKey::Pr, &pr.to_string()).map_err(ReadError::Tag)?;
    let filter = Filter {
        types: vec![EventKind::VerdictSet],
        tags: vec![tag],
    };
    let recorded = store
        .tail(&filter, MAX_VERDICTS)
        .map_err(ReadError::Ledger)?;
    Ok(recorded.into_iter().filter_map(verdict_of).collect())
}

/// The position and payload of a `verdict_set`; `None` for any other event.
fn verdict_of(recorded: Recorded) -> Option<(u64, VerdictSet)> {
    match recorded.event.event {
        LedgerEvent::VerdictSet(set) => Some((recorded.position, set)),
        _ => None,
    }
}
