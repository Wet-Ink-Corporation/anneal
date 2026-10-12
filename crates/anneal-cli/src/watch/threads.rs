//! A pull request's unresolved review threads: the one thing `watch` reads through GraphQL,
//! since GitHub's REST API has no resolved state for a thread.
//!
//! Claude Code's cloud sessions refuse `gh api graphql` and answer the same question on a REST
//! route of their GitHub proxy (`repos/{owner}/{repo}/pulls/{n}/ccr/review_threads`). When the
//! GraphQL query fails, that route is read instead; when it fails too (as it does anywhere but
//! behind that proxy), the GraphQL failure is the error. At most [`MAX_THREADS`] threads are
//! read; more fail the poll rather than judge part of them (H-13).

use serde::Deserialize;

use super::WatchError;
use super::snapshot::Blocker;
use crate::dag::tracker::github::{Field, GhApi, GhError, GhGraphql};

/// The most review threads of one pull request read.
pub(crate) const MAX_THREADS: usize = 100;

/// The review threads of a pull request of gh's current repository.
pub(crate) const THREADS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { \
     repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
     reviewThreads(first: 100) { totalCount nodes { isResolved path comments(first: 1) { \
     nodes { url } } } } } } }";

/// The unresolved review threads of pull request `pr` (web address `pr_url`) of gh's current
/// repository, each as a [`Blocker::Thread`] in GitHub's order.
///
/// # Errors
/// The GraphQL failure when neither GraphQL nor the proxy's route answers, a [`WatchError`]
/// when an answer cannot be read, and [`WatchError::TooMany`] past [`MAX_THREADS`].
pub(crate) fn unresolved<G: GhApi + GhGraphql>(
    gh: &G,
    pr: u64,
    pr_url: &str,
) -> Result<Vec<Blocker>, WatchError> {
    let number = pr.to_string();
    let fields = [
        Field::Typed("owner", "{owner}"),
        Field::Typed("name", "{repo}"),
        Field::Typed("number", &number),
    ];
    match gh.graphql(THREADS_QUERY, &fields) {
        Ok(body) => from_graphql(&body, pr),
        Err(refused @ GhError::Failed { .. }) => {
            let route = format!("repos/{{owner}}/{{repo}}/pulls/{pr}/ccr/review_threads");
            match gh.get(&route) {
                Ok(body) => from_proxy(&body, pr_url),
                Err(_unavailable) => Err(refused.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// The unresolved threads in a GraphQL answer.
fn from_graphql(body: &[u8], pr: u64) -> Result<Vec<Blocker>, WatchError> {
    let answer: Answer = serde_json::from_slice(body).map_err(|source| WatchError::Json {
        what: "the review threads",
        source,
    })?;
    let threads = answer
        .data
        .repository
        .and_then(|repository| repository.pull_request)
        .ok_or_else(|| WatchError::Missing {
            what: format!("pull request {pr}"),
        })?
        .review_threads;
    if threads.total_count > threads.nodes.len() || threads.total_count > MAX_THREADS {
        return Err(too_many());
    }
    Ok(threads
        .nodes
        .into_iter()
        .flatten()
        .filter(|thread| !thread.is_resolved)
        .map(|thread| Blocker::Thread {
            url: thread
                .comments
                .nodes
                .into_iter()
                .flatten()
                .next()
                .map(|comment| comment.url),
            path: thread.path,
        })
        .collect())
}

/// The unresolved threads in the proxy's answer, each named by its first comment's
/// `<pr_url>#discussion_r<id>`, as GitHub links a review comment.
fn from_proxy(body: &[u8], pr_url: &str) -> Result<Vec<Blocker>, WatchError> {
    let threads: Vec<ProxyThread> =
        serde_json::from_slice(body).map_err(|source| WatchError::Json {
            what: "the review threads",
            source,
        })?;
    if threads.len() > MAX_THREADS {
        return Err(too_many());
    }
    Ok(threads
        .into_iter()
        .filter(|thread| !thread.resolved)
        .map(|thread| Blocker::Thread {
            url: thread
                .comment_ids
                .first()
                .map(|id| format!("{pr_url}#discussion_r{id}")),
            path: thread.path,
        })
        .collect())
}

const fn too_many() -> WatchError {
    WatchError::TooMany {
        what: "review threads",
        limit: MAX_THREADS,
    }
}

#[derive(Debug, Deserialize)]
struct Answer {
    data: Data,
}

#[derive(Debug, Deserialize)]
struct Data {
    repository: Option<Repository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    pull_request: Option<PullRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    review_threads: Connection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    total_count: usize,
    nodes: Vec<Option<Thread>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thread {
    is_resolved: bool,
    path: Option<String>,
    comments: Comments,
}

#[derive(Debug, Deserialize)]
struct Comments {
    nodes: Vec<Option<Comment>>,
}

#[derive(Debug, Deserialize)]
struct Comment {
    url: String,
}

/// A thread as the proxy's route gives it; other fields are ignored.
#[derive(Debug, Deserialize)]
struct ProxyThread {
    resolved: bool,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    comment_ids: Vec<u64>,
}

#[cfg(test)]
mod tests;
