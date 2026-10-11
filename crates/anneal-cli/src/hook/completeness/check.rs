//! The checks the completeness hook runs on a worktree: the hard gates and the nudges.

use std::ffi::OsString;
use std::path::Path;

use anneal_ledger::event::TestRun;
use anneal_ledger::{
    EventKind, Filter, LedgerEvent, LedgerTag, Store, StoreError, TagKey, Timestamp,
};

use crate::error_chain::reason;
use crate::existing_ledger;
use crate::hook::completeness::claim::success_claim;
use crate::hook::completeness::edits::{self, Edit};
use crate::hook::completeness::{GIT_TIMEOUT, playbook};
use crate::hook::guard::BUSY_TIMEOUT;
use crate::mode::{self, scope};
use crate::paths::{self, PathError};
use crate::repro::leftovers::{self, LeftoverScan};

/// How many leftover lines a block names; the rest are counted.
const LISTED_LEFTOVERS: usize = 5;

/// How many of the worktree's latest `test_run`s are read to find a believable one.
const RUNS_READ: usize = 16;

/// How far past now a `test_run` may be stamped and still be believed (clock skew).
const FUTURE_SLACK_MS: i64 = 60_000;

/// A hard gate: blocks the stop, until the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    /// A success claim without a green `test_run` after the last source change.
    Claim,
    /// A `[DEBUG-<id>]` tag is left in the worktree.
    Leftovers,
}

impl Gate {
    /// The gate's id, as in the `stop_blocked` point.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Leftovers => "leftovers",
        }
    }
}

/// What the checks found in a worktree.
#[derive(Debug, Default)]
pub(crate) struct Findings {
    /// The hard gates that failed, each with the line the model is told.
    pub(crate) gates: Vec<(Gate, String)>,
    /// Context for the model: what the playbook still needs.
    pub(crate) nudges: Vec<String>,
    /// What could not be checked, for the one `ERROR` line.
    pub(crate) notes: Vec<String>,
    /// The ledger, when one exists: blocks are recorded in it.
    pub(crate) store: Option<Store>,
    /// The worktree key, when the working directory is in a worktree.
    pub(crate) worktree: Option<String>,
}

impl Findings {
    /// Notes that `what` failed with `error`.
    pub(crate) fn note(&mut self, what: &str, error: &dyn std::error::Error) {
        self.notes.push(format!("{what}: {}", reason(error)));
    }
}

/// Runs every check on the worktree holding `cwd`, with `reply` the model's last message.
/// Outside a git repository there is nothing to check.
pub(crate) fn run(
    cwd: &Path,
    reply: Option<&str>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Findings {
    let mut found = Findings::default();
    let root = match paths::worktree_root_within(cwd, GIT_TIMEOUT) {
        Ok(root) => root,
        Err(PathError::NotARepository { .. }) => return found,
        Err(error) => {
            found.note("the worktree could not be found", &error);
            return found;
        }
    };
    let root_path = Path::new(&root);
    check_leftovers(root_path, &mut found);
    let edit = match edits::latest_source_edit(root_path, GIT_TIMEOUT) {
        Ok(edit) => Some(edit),
        Err(error) => {
            found.note("the changed files could not be read", &error);
            None
        }
    };
    match existing_ledger::open_existing(root_path, lookup, GIT_TIMEOUT, BUSY_TIMEOUT) {
        Ok(store) => found.store = store,
        Err(error) => found.note("the ledger could not be read", &error),
    }
    let key = scope::worktree_key(&root);
    let tag = worktree_tag(&key);
    found.worktree = Some(key);
    if let (Some(claim), Some(Some(edit))) = (reply.and_then(success_claim), &edit) {
        check_claim(&claim, edit, tag.as_ref(), &mut found);
    }
    let edited = matches!(edit, Some(Some(_)));
    check_playbook(&root, tag.as_ref(), edited, &mut found);
    found
}

/// The `worktree:` tag of `key`; `None` when the ledger cannot tag it. `anneal task done` and
/// `anneal mode` refuse such a worktree, so it has no runs and no mode to read.
fn worktree_tag(key: &str) -> Option<LedgerTag> {
    LedgerTag::new(TagKey::Worktree, key).ok()
}

/// Adds the leftovers gate when a `[DEBUG-<id>]` tag is left under `root`.
fn check_leftovers(root: &Path, found: &mut Findings) {
    let scan = match leftovers::scan_within(root, leftovers::LIMITS, GIT_TIMEOUT) {
        Ok(scan) => scan,
        Err(error) => return found.note("the [DEBUG- scan failed", &error),
    };
    let skipped = scan.skipped().len();
    if skipped > 0 {
        let why = if scan.over_budget() {
            "ran out of its read budget"
        } else {
            "could not read every file"
        };
        let files = if skipped == 1 { "file" } else { "files" };
        found.nudges.push(format!(
            "the [DEBUG- leftover scan {why} and skipped {skipped} {files}; run `anneal repro \
             leftovers` to see them"
        ));
    }
    if !scan.is_clean() {
        found.gates.push((Gate::Leftovers, leftovers_line(&scan)));
    }
}

/// The block line naming the first leftovers.
fn leftovers_line(scan: &LeftoverScan) -> String {
    let named: Vec<String> = scan
        .leftovers()
        .iter()
        .take(LISTED_LEFTOVERS)
        .map(|leftover| format!("{}:{}", leftover.file(), leftover.line()))
        .collect();
    let total = u64::try_from(scan.leftovers().len())
        .unwrap_or(u64::MAX)
        .saturating_add(scan.omitted());
    format!(
        "[DEBUG-<id>] instrumentation is left at {} ({total} in all); remove it, then check \
         with `anneal repro leftovers`",
        named.join(", ")
    )
}

/// Adds the claim gate unless the worktree's latest believable `test_run` is green and started
/// after `edit`.
fn check_claim(claim: &str, edit: &Edit, tag: Option<&LedgerTag>, found: &mut Findings) {
    let fresh = match (&found.store, tag) {
        (Some(store), Some(tag)) => {
            let now_ms = match Timestamp::now() {
                Ok(now) => now.unix_millis(),
                Err(error) => return found.note("the clock could not be read", &error),
            };
            match latest_green_start(store, tag, now_ms) {
                // Strictly before: an edit in the start's own millisecond may be untested.
                Ok(start) => start.is_some_and(|start| edit.at_ms < start),
                Err(error) => return found.note("the test runs could not be read", &error),
            }
        }
        _ => false,
    };
    if !fresh {
        found.gates.push((
            Gate::Claim,
            format!(
                "the reply says \"{claim}\", but {} changed after the last green test run \
                 recorded in this worktree (or none is recorded); run the tests with `anneal \
                 task done <id> -- <test command>`, then report what it shows",
                edit.file
            ),
        ));
    }
}

/// When the worktree's latest believable `test_run` started, in Unix ms, if it is green and
/// says how long it ran; `None` otherwise.
///
/// A run is believable when it is stamped no later than `now_ms` plus [`FUTURE_SLACK_MS`]: a
/// later stamp is a clock from elsewhere or a forged event, and is passed over (of the last
/// [`RUNS_READ`] runs). The start is the stamp minus the run's `duration_ms`. `task done` stamps
/// after the run ends, to the second, so the start is never later than the real one: an edit
/// made while the tests ran always counts as untested.
fn latest_green_start(
    store: &Store,
    tag: &LedgerTag,
    now_ms: Option<i64>,
) -> Result<Option<i64>, StoreError> {
    let runs = Filter {
        types: vec![EventKind::TestRun],
        // The filter owns its tags.
        tags: vec![tag.clone()],
    };
    let latest = store.tail(&runs, RUNS_READ)?;
    let horizon = now_ms.map(|now| now.saturating_add(FUTURE_SLACK_MS));
    let believable = latest.iter().rev().find_map(|run| {
        let stamped = run.event.ts.unix_millis()?;
        horizon
            .is_none_or(|horizon| stamped <= horizon)
            .then_some((run, stamped))
    });
    Ok(
        believable.and_then(|(run, stamped)| match &run.event.event {
            event @ LedgerEvent::TestRun(TestRun {
                duration_ms: Some(duration),
                ..
            }) if playbook::is_green_run(event) => {
                // A duration past i64 saturates the start to the far past: no edit is before it.
                let duration = i64::try_from(*duration).unwrap_or(i64::MAX);
                Some(stamped.saturating_sub(duration))
            }
            _ => None,
        }),
    )
}

/// Adds the nudges of the worktree's playbook, from its anneal mode.
fn check_playbook(root: &str, tag: Option<&LedgerTag>, edited: bool, found: &mut Findings) {
    let (Some(store), Some(tag)) = (&found.store, tag) else {
        return;
    };
    let nudges = mode::active(store, root).and_then(|entered| match entered {
        Some(entered) => playbook::nudges(store, tag, &entered, edited),
        None => Ok(Vec::new()),
    });
    match nudges {
        Ok(nudges) => found.nudges.extend(nudges.into_iter().map(str::to_owned)),
        Err(error) => found.note("the anneal mode could not be read", &error),
    }
}
