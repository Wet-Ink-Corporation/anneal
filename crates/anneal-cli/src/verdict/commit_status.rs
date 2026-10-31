//! The `anneal/verdict` commit status `verdict set` posts on its commit (#116), so GitHub can
//! require what the merge gate decides: branch protection names the context [`CONTEXT`].
//!
//! The state is the merge gate's own answer for the commit (`hook::merge_gate::judge_head`, as
//! `gh pr merge <n> --match-head-commit <sha>` would be judged): [`State::Success`] when it
//! would allow the merge, else [`state_of`] the check that denied it. The token is `GH_TOKEN`,
//! then `GITHUB_TOKEN` ([`GithubEnv::read`], called at the binary edge); with neither nothing
//! is sent. The repository is the `origin` remote's, which must be on `github.com`.
//!
//! Tests point the API at a fake through [`API_VAR`] (`ANNEAL_GITHUB_API`): only a debug build
//! reads it, and only as `http://` on `127.0.0.1` or `[::1]` (`decide::config`'s rule for
//! `ANNEAL_CLEF_URL`), so a shipped binary never sends the token anywhere but GitHub.
//!
//! - `http`: the ureq adapter that sends one status.

pub(super) mod http;

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::{Level, Source};
use crate::decide::config::loopback_base;
use crate::decide::transport::ApiToken;
use crate::envelope::Finding;
use crate::hook::merge_gate::{self, Check, Pinned};
use crate::paths::{self, GIT_TIMEOUT, PathError};
use crate::pr_diff::Budget;
use crate::process::whole_millis;
use http::{Client, Request};

/// The status context branch protection requires.
const CONTEXT: &str = "anneal/verdict";

/// The variables the token is read from, in order.
const TOKEN_VARS: [&str; 2] = ["GH_TOKEN", "GITHUB_TOKEN"];

/// The test-only variable that replaces [`GITHUB_API`] (debug builds, loopback only).
const API_VAR: &str = "ANNEAL_GITHUB_API";

/// GitHub's REST API.
const GITHUB_API: &str = "https://api.github.com";

/// Whether [`API_VAR`] is read at all: only in debug builds (the test binaries).
const HONOUR_API: bool = cfg!(debug_assertions);

/// The host whose repositories get a status.
const GITHUB_HOST: &str = "github.com";

/// Most bytes kept from `git remote get-url origin`.
const REMOTE_OUTPUT_BYTES: u64 = 64 * 1024;

/// A status token as the environment gave it.
#[derive(Debug)]
enum Token {
    /// Fit for an `Authorization` header.
    Valid(ApiToken),
    /// Set, but not printable ASCII without blanks (or not UTF-8); `var` names where it came
    /// from, never its value.
    Unfit {
        /// The variable that held it.
        var: &'static str,
    },
}

/// What the environment says about posting: the token and the API override, read once at the
/// binary edge and passed in. Its `Debug` never shows the token.
#[derive(Debug, Default)]
pub(crate) struct GithubEnv {
    token: Option<Token>,
    api: Option<OsString>,
}

impl GithubEnv {
    /// The first non-empty of [`TOKEN_VARS`], and [`API_VAR`] when set and non-empty.
    pub(crate) fn read(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        let token = TOKEN_VARS.into_iter().find_map(|var| {
            let value = lookup(var).filter(|value| !value.is_empty())?;
            let token = value.into_string().ok().and_then(ApiToken::new);
            Some(token.map_or(Token::Unfit { var }, Token::Valid))
        });
        Self {
            token,
            api: lookup(API_VAR).filter(|value| !value.is_empty()),
        }
    }

    /// The API base: [`GITHUB_API`], or the loopback override when `honoured`.
    ///
    /// # Errors
    /// [`PostError::Endpoint`] when the honoured override is not a loopback `http://` base.
    fn api_base(&self, honoured: bool) -> Result<String, PostError> {
        match self.api.as_ref().filter(|_| honoured) {
            None => Ok(GITHUB_API.to_owned()),
            Some(value) => value
                .to_str()
                .and_then(loopback_base)
                .ok_or(PostError::Endpoint),
        }
    }
}

/// A `github.com` repository, as the API path names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubRepo {
    owner: String,
    name: String,
}

impl GithubRepo {
    /// The repository a remote URL names (`https://`, `ssh://` or scp-like `git@host:o/r`),
    /// read as the merge gate reads `origin`.
    ///
    /// # Errors
    /// [`PostError::OriginUnreadable`] when the URL does not read as `<host>/<owner>/<repo>`
    /// or its owner or repository is only dots (`.`, `..`, which the API path would resolve),
    /// [`PostError::NotGithub`] when its host is not `github.com`.
    fn from_remote_url(url: &str) -> Result<Self, PostError> {
        let full = merge_gate::origin_repo(url).ok_or(PostError::OriginUnreadable)?;
        let mut parts = full.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(GITHUB_HOST), Some(owner), Some(name)) if only_dots(owner) || only_dots(name) => {
                Err(PostError::OriginUnreadable)
            }
            (Some(GITHUB_HOST), Some(owner), Some(name)) => Ok(Self {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            // `origin_repo` always gives three parts; any other host is not GitHub's.
            _ => Err(PostError::NotGithub { origin: full }),
        }
    }
}

/// Whether `segment` holds nothing but dots (an empty one counts, and is refused too).
fn only_dots(segment: &str) -> bool {
    segment.chars().all(|c| c == '.')
}

impl fmt::Display for GithubRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// A commit status's state, as GitHub names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    /// The merge gate would allow the merge.
    Success,
    /// The gate wants a verdict this commit may still get.
    Pending,
    /// The gate denies, and no further verdict is awaited: changes were requested, or the gate
    /// could not judge (it fails closed).
    Failure,
}

/// The state for the merge gate's answer: `None` (it allows) or the check that denied.
const fn state_of(check: Option<Check>) -> State {
    match check {
        None => State::Success,
        Some(Check::NoVerdict | Check::Stale | Check::Tests | Check::Review) => State::Pending,
        Some(
            Check::ChangesRequested
            | Check::Unjudged
            | Check::BadInput
            | Check::Unreadable
            | Check::Shape
            | Check::Target
            | Check::Repo
            | Check::Env
            | Check::Directory
            | Check::Head,
        ) => State::Failure,
    }
}

/// The status description: the level, the source and the gate's answer, within GitHub's 140
/// characters (every part is a short fixed word).
fn description(level: Level, source: Source, check: Option<Check>) -> String {
    let (level, source) = (level.as_str(), source.as_str());
    match check {
        None => format!("{level} by {source}: the merge gate passes"),
        Some(check) if state_of(Some(check)) == State::Pending => {
            format!("{level} by {source}: the merge gate wants {}", check.id())
        }
        Some(check) => format!(
            "{level} by {source}: the merge gate denies ({})",
            check.id()
        ),
    }
}

/// Why no status was posted. No variant holds the token, a URL or a ureq message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PostError {
    /// The token's variable holds something no header can carry.
    #[error("{var} is not a token (printable ASCII without blanks)")]
    Token {
        /// The variable.
        var: &'static str,
    },
    /// The API override is not a loopback `http://` base.
    #[error("{API_VAR} must be http:// on 127.0.0.1 or [::1], with an optional port and path")]
    Endpoint,
    /// git could not tell the `origin` remote.
    #[error("the origin remote cannot be read")]
    Git(#[source] PathError),
    /// No `origin`, or its URL does not read as a repository.
    #[error("the origin remote does not name a github.com repository")]
    OriginUnreadable,
    /// `origin` is on another host.
    #[error("the origin remote names {origin}, not a github.com repository")]
    NotGithub {
        /// `<host>/<owner>/<repo>`, without user, password or port.
        origin: String,
    },
    /// GitHub could not be reached.
    #[error("GitHub could not be reached: {reason}")]
    Unreachable {
        /// What failed.
        reason: &'static str,
    },
    /// No answer within the request's timeout.
    #[error("GitHub did not answer within {:?}", whole_millis(*after))]
    TimedOut {
        /// The timeout.
        after: Duration,
    },
    /// GitHub refused the status.
    #[error("GitHub answered HTTP {code}{detail}")]
    Status {
        /// The HTTP status code.
        code: u16,
        /// `: ` and GitHub's `message`, printable and cut short, when its body gave one; else
        /// empty.
        detail: String,
    },
    /// The answer was not HTTP that could be read.
    #[error("GitHub's answer could not be read")]
    BadResponse,
    /// ureq could not build the request from the URL and headers, so nothing was sent.
    #[error("the status request could not be built")]
    Request,
    /// The request body could not be written.
    #[error("writing the status as JSON")]
    Body(#[source] serde_json::Error),
}

/// Why a status was not asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Skip {
    /// `--no-status`.
    NoStatus,
    /// Neither `GH_TOKEN` nor `GITHUB_TOKEN` is set.
    NoToken,
}

/// What became of the status, as `data.commit_status` shows it.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(crate) enum CommitStatus {
    /// GitHub accepted it.
    Posted {
        /// The state posted.
        state: State,
        /// The description posted.
        description: String,
        /// `<owner>/<repo>`.
        repository: String,
    },
    /// None was asked for.
    Skipped {
        /// Why.
        reason: Skip,
    },
    /// It was asked for and not posted.
    Failed {
        /// The state it would have had, when the gate was asked.
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<State>,
        /// Why it failed.
        reason: String,
    },
}

impl CommitStatus {
    /// Why the status was asked for and not posted; `None` when it was posted or skipped.
    pub(crate) fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed { reason, .. } => Some(reason),
            Self::Posted { .. } | Self::Skipped { .. } => None,
        }
    }
}

/// The finding of a `verdict set` for pull request `pr` whose status was not posted, for
/// `reason`.
pub(crate) fn finding(pr: u64, reason: &str) -> Finding {
    Finding::new(
        format!("verdict/status:{pr}"),
        format!(
            "the verdict is recorded, but its {CONTEXT} commit status was not posted: {reason}. \
             fix: correct that and run `anneal verdict set` again, or pass --no-status."
        ),
    )
}

/// Writes the one `WARNING` line for a status of pull request `pr` at `sha` that was not
/// posted, for `reason`, and flushes `err`.
///
/// # Errors
/// The I/O error from writing or flushing `err`.
pub(crate) fn warn<E: Write>(mut err: E, pr: u64, sha: &str, reason: &str) -> io::Result<()> {
    writeln!(
        err,
        "WARNING verdict set: the verdict for #{pr} is recorded, but its {CONTEXT} commit \
         status on {sha} was not posted: {reason}"
    )?;
    err.flush()
}

/// The verdict a status is posted for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Subject<'a> {
    /// The pull request.
    pub(crate) pr: u64,
    /// The full commit id.
    pub(crate) sha: &'a str,
    /// The level recorded.
    pub(crate) level: Level,
    /// Who recorded it.
    pub(crate) source: Source,
}

/// Where `verdict set` runs: the working directory and environment (the ledger and the gate
/// are found from them) and its worktree's root (git runs there).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Place<'a, L> {
    /// The working directory.
    pub(crate) cwd: &'a Path,
    /// The worktree's root.
    pub(crate) root: &'a Path,
    /// The process's environment, for the ledger's location.
    pub(crate) lookup: L,
}

/// Posts the status of `subject`, the verdict just recorded, through `client`: the merge
/// gate's answer for its commit, on `origin`'s repository. Every failure is an outcome, never
/// an error: the verdict is recorded whatever becomes of the status.
pub(crate) fn post<L: Fn(&str) -> Option<OsString>>(
    env: &GithubEnv,
    subject: Subject<'_>,
    place: Place<'_, L>,
    client: &Client,
) -> CommitStatus {
    let token = match &env.token {
        None => {
            return CommitStatus::Skipped {
                reason: Skip::NoToken,
            };
        }
        Some(Token::Unfit { var }) => return failed(None, &PostError::Token { var }),
        Some(Token::Valid(token)) => token,
    };
    let pinned = Pinned {
        pr: subject.pr,
        head: subject.sha,
    };
    let budget = Budget::per_call(GIT_TIMEOUT);
    let check = merge_gate::judge_head(place.cwd, place.lookup, pinned, budget)
        .map(|denial| denial.check());
    let state = state_of(check);
    let description = description(subject.level, subject.source, check);
    let target = env
        .api_base(HONOUR_API)
        .and_then(|base| origin(place.root).map(|repo| (base, repo)));
    let (base, repo) = match target {
        Ok(target) => target,
        Err(error) => return failed(Some(state), &error),
    };
    let request = Request {
        url: format!(
            "{base}/repos/{}/{}/statuses/{}",
            repo.owner, repo.name, subject.sha
        ),
        token,
        state,
        description: &description,
    };
    match client.post(&request) {
        Ok(()) => CommitStatus::Posted {
            state,
            description,
            repository: repo.to_string(),
        },
        Err(error) => failed(Some(state), &error),
    }
}

/// The failed outcome for `error`.
fn failed(state: Option<State>, error: &PostError) -> CommitStatus {
    CommitStatus::Failed {
        state,
        reason: crate::error_chain::reason(error),
    }
}

/// The `github.com` repository of the `origin` remote under `root`.
fn origin(root: &Path) -> Result<GithubRepo, PostError> {
    let args = ["remote", "get-url", "origin"];
    let output = paths::run_git_capped(root, &args, GIT_TIMEOUT, REMOTE_OUTPUT_BYTES)
        .map_err(PostError::Git)?;
    if !output.status.success() {
        // git ran and found no `origin`.
        return Err(PostError::OriginUnreadable);
    }
    GithubRepo::from_remote_url(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests;
