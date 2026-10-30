//! Assessing a pull request's verdicts against its head: pure, no I/O.
//!
//! A verdict is **fresh** when it was set for the head's commit, or when its patch id equals the
//! head's, taken from the same base (a rebase that changed nothing else). Among the fresh
//! verdicts, in ledger order:
//!
//! - the pull request's verdict is the latest one ([`Assessment::outcome`]): a level is valid,
//!   `noise_only` is noise-only, `changes_requested` (or none at all) is invalid;
//! - a `changes_requested` voids the fresh verdicts before it;
//! - after it, the latest level (`tests_verified` or better) meets the tests requirement and
//!   the latest review verdict the review requirement: the review plugin's `reviewed`,
//!   `approved` or `noise_only`, or, while the plugin is not required, a human `approved` whose
//!   `by.actor` is `human` (one with no actor, or a model's, does not count; `via` is for audit
//!   only).

use anneal_ledger::event::{Actor, By, Verdict, VerdictSet, VerdictSource};
use serde::Serialize;

/// One `verdict_set` of the pull request and where the ledger holds it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Record<'a> {
    /// Its ledger position.
    pub(crate) position: u64,
    /// The event.
    pub(crate) set: &'a VerdictSet,
}

/// The head the verdicts are judged against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Head<'a> {
    /// Its full commit id.
    pub(crate) sha: &'a str,
    /// The base the patch id is taken from, as given; `None` when there is none, and then no
    /// verdict is fresh by patch id.
    pub(crate) base: Option<&'a str>,
    /// `git patch-id --verbatim` of `git diff <base>...<sha>`; `None` when the diff is empty or
    /// the patch id was not needed ([`needs_patch_id`]).
    pub(crate) patch_id: Option<&'a str>,
}

/// Why a verdict is fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Fresh {
    /// It was set for the head's commit.
    BySha,
    /// Its patch id is the head's, from the same base.
    ByPatchId,
}

/// A fresh verdict.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Judged<'a> {
    /// The verdict.
    pub(crate) record: Record<'a>,
    /// Why it is fresh.
    pub(crate) fresh: Fresh,
}

/// Why the pull request has no valid verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Invalid {
    /// No verdict was ever set.
    NoVerdict,
    /// Every verdict is for other code.
    Stale,
    /// The latest fresh verdict requests changes.
    ChangesRequested,
}

/// The pull request's verdict: `verdict valid`'s exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The latest fresh verdict is a level.
    Valid,
    /// The latest fresh verdict says every finding was noise.
    NoiseOnly,
    /// Why there is no valid verdict.
    Invalid(Invalid),
}

/// What the verdicts say about the head.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Assessment<'a> {
    /// The latest fresh verdict.
    pub(crate) latest: Option<Judged<'a>>,
    /// The latest fresh level after the latest fresh `changes_requested`.
    pub(crate) tests: Option<Judged<'a>>,
    /// The latest fresh review verdict after the latest fresh `changes_requested`.
    pub(crate) review: Option<Judged<'a>>,
    /// How many verdicts are for other code.
    pub(crate) stale: usize,
}

impl Assessment<'_> {
    /// The pull request's verdict.
    pub(crate) const fn outcome(&self) -> Outcome {
        match self.latest {
            None if self.stale == 0 => Outcome::Invalid(Invalid::NoVerdict),
            None => Outcome::Invalid(Invalid::Stale),
            Some(judged) => match judged.record.set.verdict {
                Verdict::TestsVerified | Verdict::Reviewed | Verdict::Approved => Outcome::Valid,
                Verdict::NoiseOnly => Outcome::NoiseOnly,
                Verdict::ChangesRequested => Outcome::Invalid(Invalid::ChangesRequested),
            },
        }
    }
}

/// Assesses `records` (one pull request's verdicts, in ledger order) against `head`;
/// `require_plugin` is `review.require_plugin`.
pub(crate) fn assess<'a>(
    records: &[Record<'a>],
    head: &Head<'_>,
    require_plugin: bool,
) -> Assessment<'a> {
    let mut assessment = Assessment {
        latest: None,
        tests: None,
        review: None,
        stale: 0,
    };
    for &record in records {
        let Some(fresh) = freshness(record.set, head) else {
            assessment.stale = assessment.stale.saturating_add(1);
            continue;
        };
        let judged = Judged { record, fresh };
        assessment.latest = Some(judged);
        let verdict = record.set.verdict;
        if verdict == Verdict::ChangesRequested {
            assessment.tests = None;
            assessment.review = None;
        }
        if is_level(verdict) {
            assessment.tests = Some(judged);
        }
        if counts_as_review(record.set, require_plugin) {
            assessment.review = Some(judged);
        }
    }
    assessment
}

/// Whether any verdict in `records` could be fresh only by patch id from `base`, so the head's
/// patch id is worth computing.
pub(crate) fn needs_patch_id(records: &[Record<'_>], sha: &str, base: &str) -> bool {
    records.iter().any(|record| {
        record.set.sha.as_deref() != Some(sha)
            && record.set.patch_id.is_some()
            && record.set.base.as_deref() == Some(base)
    })
}

/// Why `set` is fresh for `head`, if it is.
fn freshness(set: &VerdictSet, head: &Head<'_>) -> Option<Fresh> {
    let sha = set.sha.as_deref()?;
    if sha == head.sha {
        return Some(Fresh::BySha);
    }
    let same_base = head.base.is_some() && set.base.as_deref() == head.base;
    let same_patch = set.patch_id.is_some() && set.patch_id.as_deref() == head.patch_id;
    (same_base && same_patch).then_some(Fresh::ByPatchId)
}

/// Whether `verdict` is a level: `tests_verified` or better.
const fn is_level(verdict: Verdict) -> bool {
    match verdict {
        Verdict::TestsVerified | Verdict::Reviewed | Verdict::Approved => true,
        Verdict::ChangesRequested | Verdict::NoiseOnly => false,
    }
}

/// Whether `set` meets `ship`'s review requirement.
fn counts_as_review(set: &VerdictSet, require_plugin: bool) -> bool {
    match (set.source, set.verdict) {
        (
            Some(VerdictSource::ReviewPlugin),
            Verdict::Reviewed | Verdict::Approved | Verdict::NoiseOnly,
        ) => true,
        // The interim rule (Ryan, 2026-10-11): until the review plugin exists. Only an
        // approval whose `by` names a human actor counts (Ryan, 2026-10-19, wi-10f284); `via`
        // is for audit only.
        (Some(VerdictSource::Human), Verdict::Approved) => {
            !require_plugin && set.by.and_then(By::actor) == Some(Actor::Human)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
