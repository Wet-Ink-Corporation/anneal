//! The GitHub Issues tracker, read through `gh api` (REST, `GET` only).
//!
//! With `--issue <n>` the tickets are the sub-issues of issue `n`
//! (`repos/{owner}/{repo}/issues/{n}/sub_issues`); without it, every issue in the repository
//! (`repos/{owner}/{repo}/issues?state=all`), pull requests skipped. A ticket's id is its issue
//! number, it is done when the issue is closed, and its blockers are the `#n` references after
//! `Blocked by` in its body (any case, an optional `:` and markdown emphasis, then `#a`,
//! `#a, #b` or `#a and #b`). A reference to an issue outside the tickets read is dangling.
//!
//! Every call goes through the [`GhApi`] port, so tests answer from recorded JSON with no
//! process or network; [`GhCli`] runs the real `gh` with a timeout and capped output (H-13).

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::dag::tracker::{Tracker, TrackerError};
use crate::process::{self, WaitError};

/// Issues asked for per page (GitHub's maximum).
pub const PER_PAGE: usize = 100;
/// The most pages read (H-13): 2,000 issues. A read that fills them all fails instead of
/// returning part of the graph.
pub const MAX_PAGES: usize = 20;
/// How long one `gh api` call may take before it is killed (H-13).
pub const GH_TIMEOUT: Duration = Duration::from_secs(30);
/// The most bytes kept from one `gh api` call's output (H-13).
pub const MAX_GH_OUTPUT: u64 = 16 * 1024 * 1024;

/// Read-only access to GitHub's REST API: the port the reader depends on.
pub trait GhApi {
    /// The response body of `GET <endpoint>` (relative to the API root, query included).
    ///
    /// # Errors
    /// A [`GhError`] when the call fails or its answer is unusable.
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError>;
}

/// Why a `gh api` call failed.
#[derive(Debug, thiserror::Error)]
pub enum GhError {
    /// `gh` could not be started (not installed, not on PATH).
    #[error("running gh")]
    Spawn(#[source] io::Error),
    /// `gh` could not be waited on or its output read.
    #[error("waiting for gh api {endpoint}")]
    Wait {
        /// The endpoint asked for.
        endpoint: String,
        /// The I/O failure.
        #[source]
        source: io::Error,
    },
    /// `gh` did not answer in time and was killed.
    #[error("gh api {endpoint} did not answer within {}s", timeout.as_secs())]
    Timeout {
        /// The endpoint asked for.
        endpoint: String,
        /// The limit.
        timeout: Duration,
    },
    /// `gh` exited non-zero (not logged in, not found, rate limited, offline).
    #[error("gh api {endpoint} failed: {stderr}")]
    Failed {
        /// The endpoint asked for.
        endpoint: String,
        /// What `gh` said, trimmed.
        stderr: String,
    },
    /// The answer reached [`MAX_GH_OUTPUT`], so it may be cut short.
    #[error("gh api {endpoint} printed {MAX_GH_OUTPUT} bytes or more")]
    TooLarge {
        /// The endpoint asked for.
        endpoint: String,
    },
}

/// The real `gh`, run as `gh api --method GET <endpoint>` with prompts and colour off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCli {
    program: OsString,
    timeout: Duration,
}

impl GhCli {
    /// `gh` from PATH, killed after [`GH_TIMEOUT`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            program: OsString::from("gh"),
            timeout: GH_TIMEOUT,
        }
    }
}

impl Default for GhCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GhApi for GhCli {
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        let mut child = Command::new(&self.program)
            .args(["api", "--method", "GET", endpoint])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(GhError::Spawn)?;
        let output = process::wait_with_deadline(&mut child, self.timeout, MAX_GH_OUTPUT, "gh")
            .map_err(|error| match error {
                WaitError::Io(source) => GhError::Wait {
                    endpoint: endpoint.to_owned(),
                    source,
                },
                WaitError::Timeout => GhError::Timeout {
                    endpoint: endpoint.to_owned(),
                    timeout: self.timeout,
                },
            })?;
        if !output.status.success() {
            return Err(GhError::Failed {
                endpoint: endpoint.to_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        if u64::try_from(output.stdout.len()).map_or(true, |len| len >= MAX_GH_OUTPUT) {
            return Err(GhError::TooLarge {
                endpoint: endpoint.to_owned(),
            });
        }
        Ok(output.stdout)
    }
}

/// Why the GitHub tickets could not be read.
#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    /// A `gh api` call failed.
    #[error(transparent)]
    Gh(#[from] GhError),
    /// An answer was not the list of issues expected.
    #[error("gh api {endpoint} did not return a list of issues")]
    Json {
        /// The endpoint asked for.
        endpoint: String,
        /// The decoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// Every one of the [`MAX_PAGES`] pages was full.
    #[error("more than {} issues; narrow the graph with --issue", MAX_PAGES * PER_PAGE)]
    TooMany,
}

/// A repository as `owner/name`, each part of ASCII letters, digits, `-`, `_` and `.` (and
/// neither `.` nor `..`), so it is safe to put in an endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    owner: String,
    name: String,
}

/// A `--repo` value that is not `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected owner/name of letters, digits, `-`, `_` and `.`, got {0:?}")]
pub struct RepoError(String);

impl FromStr for Repo {
    type Err = RepoError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let valid = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        match text.split_once('/') {
            Some((owner, name)) if valid(owner) && valid(name) => Ok(Self {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            _ => Err(RepoError(text.to_owned())),
        }
    }
}

impl fmt::Display for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// The issues of a repository, or the sub-issues of one issue, as tickets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHub<G> {
    gh: G,
    repo: Repo,
    parent: Option<u64>,
}

impl<G> GitHub<G> {
    /// Reads `repo` through `gh`: the sub-issues of `parent`, or every issue when `None`.
    #[must_use]
    pub const fn new(gh: G, repo: Repo, parent: Option<u64>) -> Self {
        Self { gh, repo, parent }
    }
}

impl<G: GhApi> Tracker for GitHub<G> {
    fn name(&self) -> &'static str {
        "github"
    }

    fn read(&self) -> Result<Vec<Ticket>, TrackerError> {
        Ok(self.read_issues()?)
    }
}

impl<G: GhApi> GitHub<G> {
    fn endpoint(&self, page: usize) -> String {
        let repo = &self.repo;
        match self.parent {
            Some(parent) => {
                format!("repos/{repo}/issues/{parent}/sub_issues?per_page={PER_PAGE}&page={page}")
            }
            None => format!("repos/{repo}/issues?state=all&per_page={PER_PAGE}&page={page}"),
        }
    }

    /// Reads page after page until one is not full, at most [`MAX_PAGES`].
    fn read_issues(&self) -> Result<Vec<Ticket>, GitHubError> {
        let mut tickets = Vec::new();
        for page in 1..=MAX_PAGES {
            let endpoint = self.endpoint(page);
            let body = self.gh.get(&endpoint)?;
            let issues: Vec<Issue> = match serde_json::from_slice(&body) {
                Ok(issues) => issues,
                Err(source) => return Err(GitHubError::Json { endpoint, source }),
            };
            let full = issues.len() >= PER_PAGE;
            tickets.extend(
                issues
                    .into_iter()
                    .filter(|issue| issue.pull_request.is_none())
                    .map(Ticket::from),
            );
            if !full {
                return Ok(tickets);
            }
        }
        Err(GitHubError::TooMany)
    }
}

/// The fields of a REST issue the reader uses; the rest are ignored.
#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    /// Present only on pull requests, which the issues endpoint also lists.
    #[serde(default)]
    pull_request: Option<IgnoredAny>,
}

impl From<Issue> for Ticket {
    fn from(issue: Issue) -> Self {
        // GitHub sends `null` for an issue with no description: an empty body, not a failure.
        let body = issue.body.unwrap_or_default();
        Self {
            id: TicketId::new(issue.number.to_string()),
            title: issue.title,
            blocked_by: blocked_by(&body),
            body,
            files: Vec::new(),
            status: if issue.state == "closed" {
                Status::Done
            } else {
                Status::Open
            },
        }
    }
}

/// The issue numbers named after each `Blocked by` in `body`, sorted, each once.
fn blocked_by(body: &str) -> Vec<TicketId> {
    const PHRASE: &str = "blocked by";
    // ASCII lowercasing keeps every byte offset, so offsets in `lower` index `body` too.
    let lower = body.to_ascii_lowercase();
    let mut numbers = Vec::new();
    let mut from = 0;
    while let Some(found) = lower.get(from..).and_then(|rest| rest.find(PHRASE)) {
        from = from.saturating_add(found).saturating_add(PHRASE.len());
        if let Some(rest) = body.get(from..) {
            numbers.extend(references(rest));
        }
    }
    numbers.sort_unstable();
    numbers.dedup();
    numbers
        .into_iter()
        .map(|number| TicketId::new(number.to_string()))
        .collect()
}

/// The `#n` list at the start of `text`, after an optional `:` and emphasis: `#a`, `#a, #b`,
/// `#a and #b`. Stops at anything else.
fn references(text: &str) -> Vec<u64> {
    let mut rest = text.trim_start_matches([':', '*', '_', ' ', '\t']);
    let mut numbers = Vec::new();
    while let Some(after_hash) = rest.strip_prefix('#') {
        let digits = after_hash.bytes().take_while(u8::is_ascii_digit).count();
        let Some((number, tail)) = after_hash.split_at_checked(digits) else {
            break;
        };
        let Ok(number) = number.parse::<u64>() else {
            break;
        };
        numbers.push(number);
        rest = tail.trim_start_matches([',', ' ', '\t']);
        if let Some(after_and) = rest.strip_prefix("and ") {
            rest = after_and.trim_start_matches([' ', '\t']);
        }
    }
    numbers
}

#[cfg(test)]
mod tests;
