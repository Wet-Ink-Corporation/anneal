//! The GitHub Issues tracker, read through `gh api` (REST, `GET` only).
//!
//! With `--issue <n>` the tickets are the sub-issues of issue `n`
//! (`repos/{owner}/{repo}/issues/{n}/sub_issues`); without it, every issue in the repository
//! (`repos/{owner}/{repo}/issues?state=all`), pull requests skipped. A ticket's id is its issue
//! number, it is done when the issue is closed, and its blockers are the `#n` references after
//! `Blocked by` in its body (any case, an optional `:` and markdown emphasis, then `#a`,
//! `#a, #b` or `#a and #b`).
//!
//! A blocker outside the tickets read (a story of an earlier release, a pull request) is
//! fetched on its own (`repos/{owner}/{repo}/issues/{n}`, at most [`MAX_EXTERNAL`] of them) and
//! becomes an external ticket: closed satisfies the blocker, open blocks. Its own blockers are
//! not followed. One GitHub does not know (HTTP 404) or has deleted (HTTP 410) stays dangling.
//! `Blocked by` counts only as a whole word (not in `Unblocked by`) and not after `no longer `
//! or `not `.
//!
//! Every call goes through the [`GhApi`] port, so tests answer from recorded JSON with no
//! process or network; [`GhCli`] runs the real `gh` with a timeout and capped output (H-13).

mod gh;

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};

use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::dag::tracker::{Tickets, Tracker, TrackerError};
pub use gh::{GH_TIMEOUT, GhApi, GhCli, GhError, MAX_GH_OUTPUT};

/// Issues asked for per page (GitHub's maximum).
pub const PER_PAGE: usize = 100;
/// The most pages read (H-13): 2,000 issues. A read that fills them all fails instead of
/// returning part of the graph.
pub const MAX_PAGES: usize = 20;
/// The most distinct outside blockers fetched (H-13): one `gh` call each. Past it the read
/// fails instead of leaving some unresolved.
pub const MAX_EXTERNAL: usize = 100;

/// Why the GitHub tickets could not be read.
#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    /// A `gh api` call failed.
    #[error(transparent)]
    Gh(#[from] GhError),
    /// An answer was not the issue or list of issues expected.
    #[error("gh api {endpoint} did not return the issues expected")]
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
    /// The tickets name more outside blockers than [`MAX_EXTERNAL`].
    #[error("{count} blockers outside the issues read, over the limit of {MAX_EXTERNAL}")]
    TooManyExternal {
        /// How many distinct outside blockers there are.
        count: usize,
    },
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

    fn read_with_external(&self) -> Result<Tickets, TrackerError> {
        let in_scope = self.read_issues()?;
        let external = self.read_external(&in_scope)?;
        Ok(Tickets { in_scope, external })
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
            let issues: Vec<Issue> = decode(&body, endpoint)?;
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

    /// Fetches each issue (or pull request) that a ticket in `in_scope` is blocked by but that
    /// is not among them, in id order, as a ticket with no blockers of its own. One GitHub does
    /// not find is skipped, so the reference stays dangling.
    fn read_external(&self, in_scope: &[Ticket]) -> Result<Vec<Ticket>, GitHubError> {
        let known: BTreeSet<&TicketId> = in_scope.iter().map(|ticket| &ticket.id).collect();
        let outside: BTreeSet<&TicketId> = in_scope
            .iter()
            .flat_map(|ticket| &ticket.blocked_by)
            .filter(|id| !known.contains(id))
            .collect();
        if outside.len() > MAX_EXTERNAL {
            return Err(GitHubError::TooManyExternal {
                count: outside.len(),
            });
        }
        let mut external = Vec::with_capacity(outside.len());
        for id in outside {
            let endpoint = format!("repos/{}/issues/{id}", self.repo);
            let body = match self.gh.get(&endpoint) {
                Ok(body) => body,
                // Not an error to report here: the graph reports the reference as dangling.
                Err(GhError::NotFound { .. }) => continue,
                Err(other) => return Err(other.into()),
            };
            let issue: Issue = decode(&body, endpoint)?;
            external.push(Ticket {
                blocked_by: Vec::new(),
                ..Ticket::from(issue)
            });
        }
        Ok(external)
    }
}

/// Decodes the JSON answer `body` of `endpoint`.
fn decode<T: DeserializeOwned>(body: &[u8], endpoint: String) -> Result<T, GitHubError> {
    serde_json::from_slice(body).map_err(|source| GitHubError::Json { endpoint, source })
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

/// Words that, right before `blocked by`, say the issue is not blocked.
const NEGATIONS: [&str; 2] = ["no longer ", "not "];

/// The issue numbers named after each `Blocked by` in `body`, sorted, each once. The phrase
/// counts only at the start of the body or after a character that is not a letter, digit or
/// `_` (so not in `Unblocked by`), and not right after `no longer ` or `not `.
fn blocked_by(body: &str) -> Vec<TicketId> {
    const PHRASE: &str = "blocked by";
    // ASCII lowercasing keeps every byte offset, so offsets in `lower` index `body` too.
    let lower = body.to_ascii_lowercase();
    let mut numbers = Vec::new();
    let mut from = 0;
    while let Some(found) = lower.get(from..).and_then(|rest| rest.find(PHRASE)) {
        let start = from.saturating_add(found);
        from = start.saturating_add(PHRASE.len());
        let Some(before) = lower.get(..start) else {
            continue;
        };
        let at_word_start = before
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let negated = NEGATIONS.iter().any(|negation| before.ends_with(negation));
        if !at_word_start || negated {
            continue;
        }
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
