//! A pull request and a commit's checks, read from GitHub's REST API through `gh api` (`GET`
//! only), and a pull request's review threads through [`super::threads`].
//!
//! The checks of a commit are its check runs (`commits/{sha}/check-runs`, the latest run per
//! name) and its commit statuses (`commits/{sha}/status`, the latest per context); a `ci` ref
//! is resolved to its commit first (`commits/{ref}`), once per poll. A check run not
//! `completed` and a status `pending` are pending, as are no checks at all and a pull request
//! whose mergeability GitHub has not computed. A check run that ended other than `success`,
//! `neutral` or `skipped`, and a status other than `success`, block (an unknown value blocks
//! too: no evidence is not green). A pull request passes GitHub only on `mergeable_state`
//! `clean` or `has_hooks` and when it is not a draft (see `Merge`); only then is the merge
//! gate asked. At most [`MAX_CHECKS`] check runs and as many statuses are read; more fail the
//! poll rather than judge part of them (H-13).

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::WatchError;
use super::gate::HeadGate;
use super::poll::Source;
use super::snapshot::{Blocker, Pending, Snapshot};
use super::threads;
use crate::dag::tracker::github::{GhApi, GhError, GhGraphql, Repo};

/// The most check runs, and the most commit statuses, of one commit read.
pub(crate) const MAX_CHECKS: usize = 100;

/// The repository a watch reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepoRef<'a> {
    /// gh's repository for the working directory (its remotes, or `GH_REPO`).
    Current,
    /// `--repo owner/name`.
    Named(&'a Repo),
}

impl RepoRef<'_> {
    /// The `owner/name` of a REST endpoint, gh's placeholders for the current repository.
    pub(crate) fn rest_path(self) -> String {
        match self {
            Self::Current => "{owner}/{repo}".to_owned(),
            Self::Named(repo) => repo.to_string(),
        }
    }

    /// `owner/name` when named, for the envelope.
    pub(crate) fn named(self) -> Option<String> {
        match self {
            Self::Current => None,
            Self::Named(repo) => Some(repo.to_string()),
        }
    }
}

/// A pull request of the current repository, judged by GitHub and then by the merge gate.
#[derive(Debug)]
pub(crate) struct PullRequestWatch<'a, G, H> {
    /// The REST and GraphQL port.
    pub(crate) gh: &'a G,
    /// The merge gate.
    pub(crate) gate: &'a H,
    /// The pull request number.
    pub(crate) pr: u64,
}

impl<G: GhApi + GhGraphql, H: HeadGate> Source for PullRequestWatch<'_, G, H> {
    fn poll(&self) -> Result<Snapshot, WatchError> {
        let pr = self.pr;
        let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{pr}");
        let Pull {
            html_url: url,
            state,
            merged,
            draft,
            mergeable,
            mergeable_state,
            head,
        } = get(self.gh, &endpoint, || format!("pull request {pr}"))?;
        let mut snapshot = Snapshot::default();
        if merged {
            snapshot.head = Some(head.sha);
            return Ok(snapshot);
        }
        if state != "open" {
            return Err(WatchError::Closed { pr });
        }
        let unresolved = threads::unresolved(self.gh, pr, &url)?;
        let merge = Merge::of(mergeable, mergeable_state);
        // Each blocker owns its copy of the address: one pull request can be several blockers.
        match merge {
            Merge::Conflict => snapshot.blockers.push(Blocker::Conflict {
                pr,
                url: url.clone(),
            }),
            Merge::Unknown => snapshot.pending.push(Pending::Mergeable),
            _ => {}
        }
        if draft || matches!(merge, Merge::Draft) {
            let url = url.clone();
            snapshot.blockers.push(Blocker::Draft { pr, url });
        }
        snapshot.blockers.extend(unresolved);
        read_checks(self.gh, RepoRef::Current, &head.sha, &mut snapshot)?;
        let still_blocked = || {
            let again: Pull = get(self.gh, &endpoint, || format!("pull request {pr}"))?;
            Ok(again.still_blocked(&head.sha))
        };
        merge
            .after_checks(&snapshot, still_blocked)?
            .record(pr, url, &mut snapshot);
        if snapshot.blockers.is_empty()
            && snapshot.pending.is_empty()
            && let Some(block) = self.gate.judge(pr, &head.sha)
        {
            snapshot.blockers.push(Blocker::Gate {
                check: block.check,
                reason: block.reason,
            });
        }
        snapshot.head = Some(head.sha);
        Ok(snapshot)
    }
}

/// The checks of the commit a ref names.
#[derive(Debug)]
pub(crate) struct CommitWatch<'a, G> {
    /// The REST port.
    pub(crate) gh: &'a G,
    /// The repository.
    pub(crate) repo: RepoRef<'a>,
    /// A branch, tag or commit, as given.
    pub(crate) git_ref: &'a str,
}

impl<G: GhApi> Source for CommitWatch<'_, G> {
    /// Resolves the ref to its commit once (`commits/<ref>`, one changed file asked for), then
    /// reads that commit's checks, so a ref that moves between the reads cannot mix two
    /// commits.
    fn poll(&self) -> Result<Snapshot, WatchError> {
        let git_ref = self.git_ref;
        let endpoint = format!(
            "repos/{}/commits/{}?per_page=1",
            self.repo.rest_path(),
            path_segment(git_ref)?
        );
        let Commit { sha } = get(self.gh, &endpoint, || format!("commit {git_ref}"))?;
        let mut snapshot = Snapshot::default();
        read_checks(self.gh, self.repo, &sha, &mut snapshot)?;
        snapshot.head = Some(sha);
        Ok(snapshot)
    }
}

/// Adds the failing check runs and statuses of commit `sha` to `snapshot`'s blockers, and the
/// unfinished ones (or the lack of any) to its pending work.
fn read_checks<G: GhApi>(
    gh: &G,
    repo: RepoRef<'_>,
    sha: &str,
    snapshot: &mut Snapshot,
) -> Result<(), WatchError> {
    let base = format!("repos/{}/commits/{}", repo.rest_path(), path_segment(sha)?);
    let missing = || format!("commit {sha}");
    let runs: CheckRuns = get(gh, &format!("{base}/check-runs?per_page=100"), missing)?;
    let status: CombinedStatus = get(gh, &format!("{base}/status?per_page=100"), missing)?;
    within(runs.total_count, runs.check_runs.len(), "check runs")?;
    within(status.total_count, status.statuses.len(), "commit statuses")?;
    if runs.check_runs.is_empty() && status.statuses.is_empty() {
        snapshot.pending.push(Pending::NoChecks);
    }
    let judged = runs
        .check_runs
        .into_iter()
        .map(CheckRun::judge)
        .chain(status.statuses.into_iter().map(Status::judge));
    for judged in judged {
        match judged {
            Judged::Passed => {}
            Judged::Pending(pending) => snapshot.pending.push(pending),
            Judged::Blocks(blocker) => snapshot.blockers.push(blocker),
        }
    }
    Ok(())
}

/// Fails when a list of `total` items arrived with only `read` of them, or holds more than
/// [`MAX_CHECKS`].
fn within(total: usize, read: usize, what: &'static str) -> Result<(), WatchError> {
    if total > read || total > MAX_CHECKS {
        return Err(WatchError::TooMany {
            what,
            limit: MAX_CHECKS,
        });
    }
    Ok(())
}

/// `GET endpoint`, decoded; a 404 is [`WatchError::Missing`] naming `what()`.
fn get<G: GhApi, T: DeserializeOwned>(
    gh: &G,
    endpoint: &str,
    what: impl Fn() -> String,
) -> Result<T, WatchError> {
    let body = match gh.get(endpoint) {
        Ok(body) => body,
        Err(GhError::NotFound { .. }) => return Err(WatchError::Missing { what: what() }),
        Err(error) => return Err(error.into()),
    };
    serde_json::from_slice(&body).map_err(|source| WatchError::Json {
        what: "the answer",
        source,
    })
}

/// `text` as one path segment: every byte but ASCII letters, digits, `-`, `.`, `_` and `~`
/// percent-encoded, so a ref can neither add a path segment or query nor fill one of gh's
/// `{owner}`, `{repo}` or `{branch}` placeholders.
///
/// # Errors
/// [`WatchError::BadRef`] for an empty text, `.` or `..`, which as a segment would name the
/// commit list or a parent path rather than a commit.
fn path_segment(text: &str) -> Result<String, WatchError> {
    if matches!(text, "" | "." | "..") {
        return Err(WatchError::BadRef {
            git_ref: text.to_owned(),
        });
    }
    let mut segment = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            segment.push(char::from(byte));
        } else {
            segment.push('%');
            for nibble in [byte >> 4, byte & 0x0f] {
                if let Some(digit) = char::from_digit(u32::from(nibble), 16) {
                    segment.push(digit.to_ascii_uppercase());
                }
            }
        }
    }
    Ok(segment)
}

/// What GitHub's `mergeable` and `mergeable_state` say of an open pull request. Only `clean`
/// and `has_hooks` let it through; a state this version does not know refuses it.
#[derive(Debug, PartialEq, Eq)]
enum Merge {
    /// `clean` or `has_hooks`.
    Clean,
    /// `mergeable: false`, or `dirty`.
    Conflict,
    /// `draft`.
    Draft,
    /// Not computed yet: `mergeable: null`, or `unknown`.
    Unknown,
    /// `blocked`: branch protection, waiting on the checks or on something they do not show.
    Blocked,
    /// `unstable`: a check that is not required has not passed.
    Unstable,
    /// `behind`, or any other state.
    Refused(String),
}

impl Merge {
    fn of(mergeable: Option<bool>, state: String) -> Self {
        if mergeable == Some(false) {
            return Self::Conflict;
        }
        match state.as_str() {
            "dirty" => Self::Conflict,
            "draft" => Self::Draft,
            _ if mergeable.is_none() => Self::Unknown,
            "unknown" => Self::Unknown,
            "clean" | "has_hooks" => Self::Clean,
            "blocked" => Self::Blocked,
            "unstable" => Self::Unstable,
            _ => Self::Refused(state),
        }
    }

    /// What the state means once the checks are read into `snapshot`: `blocked` with nothing
    /// else blocking or pending is branch protection the checks do not show (a required
    /// review, or a required check that never reported), so it blocks, but only when
    /// `still_blocked`, a second read of the pull request, confirms it: the first read came
    /// before the checks, so a last check that finished in between left it stale. Unconfirmed,
    /// or while anything else is pending, it waits. `unstable` waits.
    ///
    /// # Errors
    /// The failure of `still_blocked`, which is called only in that one case.
    fn after_checks(
        self,
        snapshot: &Snapshot,
        still_blocked: impl FnOnce() -> Result<bool, WatchError>,
    ) -> Result<AfterChecks, WatchError> {
        Ok(match self {
            Self::Clean | Self::Conflict | Self::Draft | Self::Unknown => AfterChecks::Judged,
            Self::Unstable => AfterChecks::Waits("unstable"),
            Self::Blocked
                if !snapshot.blockers.is_empty()
                    || !snapshot.pending.is_empty()
                    || !still_blocked()? =>
            {
                AfterChecks::Waits("blocked")
            }
            Self::Blocked => AfterChecks::Blocks("blocked".to_owned()),
            Self::Refused(state) => AfterChecks::Blocks(state),
        })
    }
}

/// What a merge state adds once the checks are read.
#[derive(Debug, PartialEq, Eq)]
enum AfterChecks {
    /// Nothing: the state was judged before the checks.
    Judged,
    /// Pending, as `mergeable_state` this.
    Waits(&'static str),
    /// A `merge_state` blocker, as `mergeable_state` this.
    Blocks(String),
}

impl AfterChecks {
    /// Adds this to `snapshot`, naming pull request `pr` at `url`.
    fn record(self, pr: u64, url: String, snapshot: &mut Snapshot) {
        match self {
            Self::Judged => {}
            Self::Waits(state) => snapshot.pending.push(Pending::MergeState {
                state: state.to_owned(),
            }),
            Self::Blocks(state) => snapshot
                .blockers
                .push(Blocker::MergeState { pr, state, url }),
        }
    }
}

/// What one check says.
enum Judged {
    Passed,
    Pending(Pending),
    Blocks(Blocker),
}

/// The fields of a REST pull request read; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Pull {
    html_url: String,
    state: String,
    #[serde(default)]
    merged: bool,
    draft: bool,
    mergeable: Option<bool>,
    mergeable_state: String,
    head: Head,
}

impl Pull {
    /// Whether this read still shows the pull request open, unmerged and `blocked` at `head`.
    fn still_blocked(self, head: &str) -> bool {
        !self.merged
            && self.state == "open"
            && self.head.sha == head
            && Merge::of(self.mergeable, self.mergeable_state) == Merge::Blocked
    }
}

#[derive(Debug, Deserialize)]
struct Head {
    sha: String,
}

/// The field of a REST commit read; its files and the rest are ignored.
#[derive(Debug, Deserialize)]
struct Commit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct CheckRuns {
    total_count: usize,
    check_runs: Vec<CheckRun>,
}

#[derive(Debug, Deserialize)]
struct CheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
    details_url: Option<String>,
    html_url: Option<String>,
}

impl CheckRun {
    fn judge(self) -> Judged {
        if self.status != "completed" {
            return Judged::Pending(Pending::Check {
                name: self.name,
                status: self.status,
            });
        }
        match self.conclusion.as_deref() {
            Some("success" | "neutral" | "skipped") => Judged::Passed,
            other => Judged::Blocks(Blocker::Check {
                conclusion: other.unwrap_or("none").to_owned(),
                name: self.name,
                url: non_empty(self.details_url).or_else(|| non_empty(self.html_url)),
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CombinedStatus {
    total_count: usize,
    statuses: Vec<Status>,
}

#[derive(Debug, Deserialize)]
struct Status {
    context: String,
    state: String,
    target_url: Option<String>,
}

impl Status {
    fn judge(self) -> Judged {
        match self.state.as_str() {
            "success" => Judged::Passed,
            "pending" => Judged::Pending(Pending::Check {
                name: self.context,
                status: self.state,
            }),
            _ => Judged::Blocks(Blocker::Check {
                name: self.context,
                conclusion: self.state,
                url: non_empty(self.target_url),
            }),
        }
    }
}

/// `url`, unless it is empty.
pub(crate) fn non_empty(url: Option<String>) -> Option<String> {
    url.filter(|url| !url.is_empty())
}

#[cfg(test)]
mod tests;
