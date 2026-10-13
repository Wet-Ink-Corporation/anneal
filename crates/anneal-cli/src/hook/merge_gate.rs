//! `anneal hook merge-gate`: the `PreToolUse` gate on `gh pr merge`. It fails closed.
//!
//! It reads Claude Code's `PreToolUse` input on stdin and finds every `gh pr merge` the Bash
//! command runs, read as the guard reads commands (`guard::find_pr_merge`: wrappers, `bash -c`,
//! `eval`, scripts on stdin, variables). A call that runs none, and any other tool, is allowed
//! silently. Each merge must name its pull request by number (`7`, `#7`, or an
//! `http(s)://<host>/<owner>/<repo>/pull/7` URL of the `origin` repository), pin its head with a
//! full `--match-head-commit`, and name the repository the gate judges, the `origin` remote's,
//! with `-R`/`--repo <host>/<owner>/<repo>`: gh takes the repository from `--repo` before
//! `GH_REPO` or the working directory, and the host in it before `GH_HOST`, so what gh merges
//! is what was judged. Each denial names the exact command, with the worktree's `HEAD`. As
//! defence in depth it also denies a command that sets `GH_REPO`/`GH_HOST` or changes directory
//! (`cd`, `pushd`, `popd`, `env --chdir`, `sudo --chdir`/`--login`, `su --login`,
//! `find -execdir`) anywhere. Then it needs, for that head (`pull` holds the checks above):
//!
//! - a fresh `tests_verified`-or-better verdict after the latest fresh `changes_requested`
//!   (fresh: recorded for the head's commit, or for the same patch id from the same base, so a
//!   rebase keeps it);
//! - a review verdict (`verdict::assess`: the review plugin's, or a human `approved` while
//!   `review.require_plugin` is false), unless the skip-rule skips the change (`ship::skip`).
//!
//! A denial is Claude Code's deny decision whose reason is one `ERROR` line naming what is
//! missing and the command that records it, plus a `gate_blocked` event in an existing ledger;
//! the gate never creates a ledger (none means no verdict). Input or a command it cannot read,
//! and a ledger, git or setting it cannot read, deny. git and the ledger get [`GIT_BUDGET`]
//! from the hook's start, each git call at most [`GIT_TIMEOUT`]. See `docs/verdict.md`.

mod pull;

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{EventEnvelope, LedgerEvent, Store, StoreError, Timestamp, TimestampError};
use serde::Deserialize;

use crate::error_chain::reason;
use crate::existing_ledger::{self, LedgerError};
use crate::hook::guard::{
    BUSY_TIMEOUT, Context, GIT_TIMEOUT, MAX_INPUT_BYTES, MergeScan, PrMerge, find_pr_merge,
};
use crate::hook::{HookEnv, StdinError, read_stdin, write_deny};
use crate::paths::{self, PathError};
use crate::pr_diff::Budget;
use crate::repo_config;
use crate::ship::skip_rule::{self, Sensitive};
use crate::ship::{self, Change};
use crate::verdict::assess::{Invalid, Outcome};
use crate::verdict::pull_request::{self, PullRequest, Query};
use pull::Pull;

/// What git and the ledger reads may start within, from the hook's start: the 5 s hook
/// `timeout` less one ledger read's lock wait begun at the end of it, the record's lock wait
/// ([`BUSY_TIMEOUT`] each) and process start-up.
const GIT_BUDGET: Duration = Duration::from_millis(3_500);

/// The `point` of the gate's `gate_blocked` events, before the check's id.
const POINT_PREFIX: &str = "merge-gate.";

/// What a denial is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// The hook input could not be read.
    BadInput,
    /// The command could not be read.
    Unreadable,
    /// The merge names no pull request by number.
    Target,
    /// The merge does not name `origin`'s repository with `-R`/`--repo <host>/<owner>/<repo>`,
    /// or names it by a URL not of `origin`.
    Repo,
    /// The command sets `GH_REPO` or `GH_HOST`.
    Env,
    /// The command changes the working directory before gh runs (`cd`, `env --chdir`, ...).
    Directory,
    /// The merge does not pin its head with a full `--match-head-commit`.
    Head,
    /// The pull request has no verdict.
    NoVerdict,
    /// Every verdict is for other code.
    Stale,
    /// The latest fresh verdict requests changes.
    ChangesRequested,
    /// No fresh `tests_verified`-or-better verdict.
    Tests,
    /// No review verdict, and the skip-rule wants one.
    Review,
    /// The ledger, git or the settings could not be read.
    Unjudged,
}

impl Check {
    /// The check's id, as in the `gate_blocked` point.
    const fn id(self) -> &'static str {
        match self {
            Self::BadInput => "bad-input",
            Self::Unreadable => "unreadable",
            Self::Target => "target",
            Self::Repo => "repo",
            Self::Env => "env",
            Self::Directory => "directory",
            Self::Head => "head",
            Self::NoVerdict => "no-verdict",
            Self::Stale => "stale",
            Self::ChangesRequested => "changes-requested",
            Self::Tests => "tests",
            Self::Review => "review",
            Self::Unjudged => "unjudged",
        }
    }
}

/// Why a merge is denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denial {
    check: Check,
    /// One line, starting `merge-gate: `.
    reason: String,
}

impl Denial {
    fn new(check: Check, reason: &str) -> Self {
        Self {
            check,
            reason: format!("merge-gate: {reason}"),
        }
    }

    /// The id of the check that denied, as in the `gate_blocked` point.
    pub(crate) const fn check_id(&self) -> &'static str {
        self.check.id()
    }

    /// The one-line reason, starting `merge-gate: `.
    pub(crate) fn into_reason(self) -> String {
        self.reason
    }
}

/// The denial for a merge that could not be judged because `what` failed with `error`.
fn unjudged(what: &str, error: &dyn Error) -> Denial {
    Denial::new(
        Check::Unjudged,
        &format!(
            "{what} ({}), so the merge cannot be checked; fix that, then try again",
            reason(error)
        ),
    )
}

/// The fields of Claude Code's `PreToolUse` input the gate uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Input {
    #[serde(default)]
    session_id: Option<String>,
    /// Where the worktree and the ledger are found; the process's working directory when
    /// absent.
    #[serde(default)]
    cwd: Option<PathBuf>,
    tool_name: String,
    #[serde(default, rename = "tool_input")]
    tool: ToolInput,
}

/// The Bash tool's input; other fields are ignored.
#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    #[serde(default)]
    command: Option<String>,
}

/// Why the hook input could not be used.
#[derive(Debug, thiserror::Error)]
enum InputError {
    /// stdin could not be read, or held more than [`MAX_INPUT_BYTES`].
    #[error(transparent)]
    Stdin(#[from] StdinError),
    /// stdin was not a JSON object of the expected shape.
    #[error("the input is not PreToolUse JSON")]
    Json(#[source] serde_json::Error),
}

/// Why a denial could not be recorded in the ledger.
#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error(transparent)]
    Ledger(LedgerError),
    #[error("reading the clock")]
    Clock(#[source] TimestampError),
    #[error("appending gate_blocked")]
    Append(#[source] StoreError),
}

/// The ledger a denial is recorded in: none when no ledger exists (the gate never creates
/// one), or why it could not be opened.
type Ledger = Result<Option<Store>, LedgerError>;

/// Runs the gate: reads the input from `env.stdin`, judges every `gh pr merge` its Bash command
/// runs against the pull request's verdicts (in the ledger found from the input's `cwd`, or
/// `env.current_dir`, and `env.lookup`), and on a denial records `gate_blocked` and writes the
/// deny decision to `out`. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure denies.
pub(crate) fn run<R, D, L, W>(env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    let budget = Budget::within(Instant::now(), GIT_BUDGET, GIT_TIMEOUT);
    let HookEnv {
        stdin,
        current_dir,
        lookup,
    } = env;
    let (judged, session) = match read_input(stdin) {
        Ok(mut input) => {
            let session = input.session_id.take();
            (judge(input, current_dir, &lookup, budget), session)
        }
        Err(error) => (Some(bad_input(&error, current_dir, &lookup, budget)), None),
    };
    let Some((denial, ledger)) = judged else {
        return Ok(());
    };
    let unrecorded = match ledger {
        Ok(Some(store)) => record(&store, &denial, session).err(),
        Ok(None) => None,
        Err(error) => Some(RecordError::Ledger(error)),
    };
    let line = match unrecorded {
        None => format!("ERROR {}", denial.reason),
        Some(error) => format!(
            "ERROR {} (the block was not recorded in the ledger: {})",
            denial.reason,
            reason(&error)
        ),
    };
    write_deny(out, &line)
}

/// Reads and parses at most [`MAX_INPUT_BYTES`] of `stdin`.
fn read_input<R: Read>(stdin: R) -> Result<Input, InputError> {
    let bytes = read_stdin(stdin, MAX_INPUT_BYTES)?;
    serde_json::from_slice(&bytes).map_err(InputError::Json)
}

/// The denial for unreadable input, with the ledger seen from the process's working directory.
fn bad_input(
    error: &InputError,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
    budget: Budget,
) -> (Denial, Ledger) {
    let denial = Denial::new(
        Check::BadInput,
        &format!("the hook input cannot be read: {}", reason(error)),
    );
    let ledger = current_dir()
        .map_err(LedgerError::CurrentDir)
        .and_then(|cwd| open_existing(&cwd, lookup, budget));
    (denial, ledger)
}

/// Judges `input`: `None` to allow, else the denial and the ledger to record it in.
fn judge(
    input: Input,
    current_dir: impl FnOnce() -> io::Result<PathBuf>,
    lookup: impl Fn(&str) -> Option<OsString>,
    budget: Budget,
) -> Option<(Denial, Ledger)> {
    if input.tool_name != "Bash" {
        return None;
    }
    let cwd = match input.cwd.map_or_else(current_dir, Ok) {
        Ok(cwd) => cwd,
        Err(error) => {
            let denial = unjudged("the working directory cannot be read", &error);
            return Some((denial, Err(LedgerError::CurrentDir(error))));
        }
    };
    let Some(command) = input.tool.command else {
        let denial = Denial::new(
            Check::BadInput,
            "the hook input cannot be read: the Bash input has no command",
        );
        return Some((denial, open_existing(&cwd, lookup, budget)));
    };
    let merges = match find_pr_merge(&command, &Context::new(&lookup)) {
        MergeScan::Merges(merges) if merges.is_empty() => return None,
        MergeScan::Merges(merges) => merges,
        MergeScan::Unreadable(denial) => {
            let denial = Denial::new(
                Check::Unreadable,
                &format!(
                    "the command cannot be read, so a gh pr merge in it cannot be checked: {}",
                    denial.reason
                ),
            );
            return Some((denial, open_existing(&cwd, lookup, budget)));
        }
    };
    let ledger = open_existing(&cwd, lookup, budget);
    let denial = match &ledger {
        Ok(store) => judge_merges(&merges, store.as_ref(), &cwd, budget),
        Err(error) => Some(unjudged("the ledger cannot be read", error)),
    }?;
    Some((denial, ledger))
}

/// The ledger seen from `cwd` when its file exists: git to find it and each later statement's
/// lock wait both within what is left of `budget` (and at most [`GIT_TIMEOUT`] and
/// [`BUSY_TIMEOUT`]).
fn open_existing(cwd: &Path, lookup: impl Fn(&str) -> Option<OsString>, budget: Budget) -> Ledger {
    existing_ledger::open_existing(
        cwd,
        lookup,
        budget.cap(GIT_TIMEOUT),
        budget.cap(BUSY_TIMEOUT),
    )
}

/// The first denial among `merges` run from `cwd`, with the verdicts in `store`.
fn judge_merges(
    merges: &[PrMerge],
    store: Option<&Store>,
    cwd: &Path,
    budget: Budget,
) -> Option<Denial> {
    let pulls = merges
        .iter()
        .map(|merge| pull::pull(merge).map(|pull| (pull, merge.repo.as_deref())))
        .collect::<Result<Vec<_>, _>>();
    let pulls = match pulls {
        Ok(pulls) => pulls,
        Err(denial) => return Some(denial),
    };
    with_gate(store, cwd, budget, |gate| {
        pulls
            .into_iter()
            .find_map(|(pull, repo)| gate.judge(pull, repo))
    })
}

/// A merge of a pull request pinned to a head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pinned<'a> {
    /// The pull request number.
    pub(crate) pr: u64,
    /// The full commit id `--match-head-commit` would name.
    pub(crate) head: &'a str,
}

/// The denial the gate would give `gh pr merge <pr> --match-head-commit <head>` (`pinned`) run
/// from `cwd`; `None` when it would allow the merge. The ledger is the existing one seen from
/// `cwd` (none means no verdict); a ledger, git or setting that cannot be read denies, as in
/// the hook. `anneal watch pr` asks this of a pull request's head on GitHub.
pub(crate) fn judge_head(
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
    pinned: Pinned<'_>,
    budget: Budget,
) -> Option<Denial> {
    let ledger = open_existing(cwd, lookup, budget);
    let store = match &ledger {
        Ok(store) => store.as_ref(),
        Err(error) => return Some(unjudged("the ledger cannot be read", error)),
    };
    with_gate(store, cwd, budget, |gate| {
        gate.judge_pinned(pinned.pr, ("--match-head-commit", pinned.head))
    })
}

/// Runs `judge` with the gate of the worktree seen from `cwd`: its root, settings and
/// sensitive globs, and the verdicts in `store`; or the denial for the one that cannot be read.
fn with_gate(
    store: Option<&Store>,
    cwd: &Path,
    budget: Budget,
    judge: impl FnOnce(&Gate<'_>) -> Option<Denial>,
) -> Option<Denial> {
    let root = match budget.next() {
        Ok(timeout) => paths::worktree_root_within(cwd, timeout),
        Err(error) => return Some(unjudged("no time is left for git", &error)),
    };
    let root = match root {
        Ok(root) => root,
        Err(error @ PathError::NotARepository { .. }) => {
            return Some(unjudged("the session is in no git worktree", &error));
        }
        Err(error) => return Some(unjudged("the worktree cannot be found", &error)),
    };
    let root = Path::new(&root);
    let config = match repo_config::gate(root) {
        Ok(config) => config,
        Err(error) => return Some(unjudged("the gate settings cannot be read", &error)),
    };
    let rules = match skip_rule::sensitive_rules(config.sensitive_globs) {
        Ok(rules) => rules,
        Err(error) => return Some(unjudged("a default sensitive glob", &error)),
    };
    let gate = Gate {
        root,
        store,
        require_plugin: config.require_plugin,
        rules: &rules,
        budget,
    };
    judge(&gate)
}

/// What one merge is judged with.
struct Gate<'a> {
    root: &'a Path,
    store: Option<&'a Store>,
    require_plugin: bool,
    rules: &'a [Sensitive],
    budget: Budget,
}

impl Gate<'_> {
    /// The denial for merging `pull` with `--repo` `repo`, if any.
    fn judge(&self, pull: Pull<'_>, repo: Option<&str>) -> Option<Denial> {
        let origin = match self.budget.next() {
            Ok(timeout) => pull::read_origin(self.root, timeout),
            Err(error) => Err(unjudged("no time is left for git", &error)),
        };
        let origin = match origin {
            Ok(origin) => origin,
            Err(denial) => return Some(denial),
        };
        let origin = origin.as_deref();
        if let Some(denial) = pull
            .url_repo
            .and_then(|url_repo| pull::check_url(url_repo, origin))
        {
            return Some(denial);
        }
        let pinned = match self.budget.next() {
            Ok(timeout) => pull::pinned_head(pull, origin, self.root, timeout),
            Err(error) => Err(unjudged("no time is left for git", &error)),
        };
        let head = match pinned {
            Ok(head) => head,
            Err(denial) => return Some(denial),
        };
        if let Some(denial) = pull::check_repo(pull.pr, head, repo, origin) {
            return Some(denial);
        }
        self.judge_pinned(pull.pr, ("--match-head-commit", head))
    }

    /// The denial for merging pull request `pr` pinned to `head` (the revision, and the flag
    /// that gave it), if any.
    fn judge_pinned(&self, pr: u64, head: (&'static str, &str)) -> Option<Denial> {
        let query = Query {
            root: self.root,
            pr,
            head,
            base: None,
            budget: self.budget,
        };
        let pull_request = match pull_request::read(self.store, &query) {
            Ok(pull_request) => pull_request,
            Err(error) => return Some(unjudged(&format!("#{pr} cannot be read"), &error)),
        };
        let assessment = pull_request.assess(self.require_plugin);
        if assessment.tests.is_none() {
            return Some(missing_tests(&pull_request, assessment.outcome()));
        }
        if assessment.review.is_some() {
            return None;
        }
        self.skip_rule(&pull_request)
    }

    /// `None` when the skip-rule skips the pull request's change, else the review denial.
    fn skip_rule(&self, pull_request: &PullRequest) -> Option<Denial> {
        let change = Change {
            root: self.root,
            base: pull_request.base.as_deref(),
            head: &pull_request.head,
        };
        let skip = match ship::skip(change, self.rules, self.budget) {
            Ok(skip) => skip,
            Err(error) => return Some(unjudged("the skip-rule cannot be applied", &error)),
        };
        if skip.decision.skip() {
            return None;
        }
        let (pr, head) = (pull_request.pr, &pull_request.head);
        let counts = if self.require_plugin {
            "only a review-plugin verdict counts while review.require_plugin is true: run the \
             review plugin"
                .to_owned()
        } else {
            format!(
                "run the review plugin, or record a human approval: `anneal verdict set --pr \
                 {pr} --sha {head} --level approved --source human`"
            )
        };
        Some(Denial::new(
            Check::Review,
            &format!(
                "#{pr} has no review verdict for {head}, and the skip-rule wants a full review \
                 of its change ({} files, {} lines, against {}); {counts}",
                skip.decision.files, skip.decision.lines, skip.base
            ),
        ))
    }
}

/// The denial for pull request `pull_request`, which has no fresh `tests_verified`-or-better
/// verdict, by its `outcome`.
fn missing_tests(pull_request: &PullRequest, outcome: Outcome) -> Denial {
    let (pr, head) = (pull_request.pr, &pull_request.head);
    let set = format!(
        "`anneal verdict set --pr {pr} --sha {head} --level tests-verified --source local`"
    );
    match outcome {
        Outcome::Invalid(Invalid::NoVerdict) => Denial::new(
            Check::NoVerdict,
            &format!("#{pr} has no verdict; run its tests, then record them: {set}"),
        ),
        Outcome::Invalid(Invalid::Stale) => Denial::new(
            Check::Stale,
            &format!(
                "#{pr}'s verdicts are stale: each is for other code than {head}; verify it \
                 again, then {set}"
            ),
        ),
        Outcome::Invalid(Invalid::ChangesRequested) => Denial::new(
            Check::ChangesRequested,
            &format!(
                "#{pr}: its latest verdict requests changes; make them, verify again, then {set}"
            ),
        ),
        Outcome::Valid | Outcome::NoiseOnly => Denial::new(
            Check::Tests,
            &format!("#{pr} has no tests-verified verdict for {head}; run its tests, then {set}"),
        ),
    }
}

/// Appends a `gate_blocked` event for `denial`, tagged with `session`, to `store`.
fn record(store: &Store, denial: &Denial, session: Option<String>) -> Result<(), RecordError> {
    let event = LedgerEvent::GateBlocked(GateBlocked {
        point: format!("{POINT_PREFIX}{}", denial.check.id()),
        // The ledger keeps one copy of the reason, the deny decision shows the other.
        reason: denial.reason.clone(),
    });
    let mut envelope =
        EventEnvelope::new(event, "hook", Timestamp::now().map_err(RecordError::Clock)?);
    envelope.session_id = session;
    store
        .append(&envelope)
        .map(drop)
        .map_err(RecordError::Append)
}

#[cfg(test)]
mod tests;
