//! Review threads by GraphQL, and through the Claude Code proxy's route when GraphQL is
//! refused.

use super::{MAX_THREADS, THREADS_QUERY, unresolved};
use crate::watch::WatchError;
use crate::watch::fake::FakeGh;
use crate::watch::snapshot::Blocker;

const PR_URL: &str = "https://github.com/octo/app/pull/7";
const REFUSED: &str = "gh: GitHub GraphQL is not available from Claude Code sessions; use the \
                       REST API (HTTP 403)";

#[test]
fn the_query_asks_for_exactly_the_threads_it_can_judge() {
    let asks = THREADS_QUERY.contains(&format!("reviewThreads(first: {MAX_THREADS})"));

    assert!(asks, "{THREADS_QUERY}");
}

#[test]
fn more_review_threads_than_are_read_fail_rather_than_judge_part_of_them() {
    let gh = FakeGh::new().route("graphql", &["threads-too-many.json"]);

    let result = unresolved(&gh, 7, PR_URL);

    assert!(
        matches!(
            result,
            Err(WatchError::TooMany {
                what: "review threads",
                limit: MAX_THREADS
            })
        ),
        "{result:?}"
    );
}

#[test]
fn when_graphql_is_refused_the_proxy_s_thread_route_is_read_instead() {
    let gh = FakeGh::new()
        .failing("graphql", REFUSED)
        .route("/ccr/review_threads", &["ccr-threads.json"]);

    let threads = unresolved(&gh, 7, PR_URL).expect("the route answers");

    assert_eq!(
        threads,
        vec![Blocker::Thread {
            url: Some(format!("{PR_URL}#discussion_r2001")),
            path: Some("src/watch.rs".to_owned()),
        }],
        "the unresolved thread, by its first comment"
    );
    assert_eq!(
        gh.calls.borrow().last().map(String::as_str),
        Some("repos/{owner}/{repo}/pulls/7/ccr/review_threads")
    );
}

#[test]
fn a_proxy_list_as_long_as_the_cap_fails_since_it_may_have_been_cut_short() {
    let gh = FakeGh::new()
        .failing("graphql", REFUSED)
        .route("/ccr/review_threads", &["ccr-threads-too-many.json"]);

    let result = unresolved(&gh, 7, PR_URL);

    assert!(
        matches!(
            result,
            Err(WatchError::TooMany {
                what: "review threads",
                limit: MAX_THREADS
            })
        ),
        "the route gives no total, so 100 threads may be the first 100 of more: {result:?}"
    );
}

#[test]
fn when_both_fail_the_graphql_failure_is_reported() {
    let gh = FakeGh::new()
        .failing("graphql", REFUSED)
        .missing("/ccr/review_threads");

    let result = unresolved(&gh, 7, PR_URL);

    assert!(
        matches!(
            &result,
            Err(WatchError::Gh(crate::dag::tracker::github::GhError::Failed { endpoint, .. }))
                if endpoint == "graphql"
        ),
        "{result:?}"
    );
}
