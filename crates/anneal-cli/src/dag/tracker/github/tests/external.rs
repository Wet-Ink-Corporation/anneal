//! Blockers outside the issues read: fetched one by one, closed ones satisfied, open ones
//! blocking, missing ones left dangling.

use super::{FakeGh, fixture, repo, summary};
use crate::dag::ticket::Status;
use crate::dag::tracker::github::{GhApi, GhError, GitHub, GitHubError, MAX_EXTERNAL, failure};
use crate::dag::tracker::{Tracker, TrackerError};

/// The sub-issues of #4 (#40 by closed #11, #41 by open #12, #43 by merged pull request #30),
/// those three outside issues, and "Not Found" for anything else.
fn recorded() -> impl GhApi {
    FakeGh(|endpoint: &str| match endpoint {
        "repos/octo/tickets/issues/4/sub_issues?per_page=100&page=1" => {
            Ok(fixture("sub-issues-outside.json"))
        }
        "repos/octo/tickets/issues/11" => Ok(fixture("issue-11-closed.json")),
        "repos/octo/tickets/issues/12" => Ok(fixture("issue-12-open.json")),
        "repos/octo/tickets/issues/30" => Ok(fixture("pull-30-merged.json")),
        _ => Err(GhError::NotFound {
            endpoint: endpoint.to_owned(),
        }),
    })
}

/// One page holding issue #50 with `body`; every other call fails with `other`.
fn one_issue(body: &str, other: fn(&str) -> GhError) -> impl GhApi {
    let issue = serde_json::json!([{"number": 50, "title": "t", "state": "open", "body": body}]);
    FakeGh(move |endpoint: &str| {
        if endpoint.starts_with("repos/octo/tickets/issues/5/sub_issues?") {
            Ok(serde_json::to_vec(&issue).expect("serialises"))
        } else {
            Err(other(endpoint))
        }
    })
}

fn not_found(endpoint: &str) -> GhError {
    GhError::NotFound {
        endpoint: endpoint.to_owned(),
    }
}

fn forbidden(endpoint: &str) -> GhError {
    GhError::Failed {
        endpoint: endpoint.to_owned(),
        stderr: "gh: API rate limit exceeded (HTTP 403)".to_owned(),
    }
}

#[test]
fn outside_blockers_are_fetched_as_external_leaves_with_their_state() -> Result<(), TrackerError> {
    let tickets = GitHub::new(recorded(), repo(), Some(4)).read_with_external()?;

    assert_eq!(
        summary(&tickets.in_scope),
        [
            ("40", Status::Open, vec!["11"]),
            ("41", Status::Open, vec!["12"]),
            ("43", Status::Open, vec!["30"]),
        ],
        "the sub-issues"
    );
    assert_eq!(
        summary(&tickets.external),
        [
            ("11", Status::Done, vec![]),
            ("12", Status::Open, vec![]),
            ("30", Status::Done, vec![]),
        ],
        "closed issue and merged pull request are done; #11's own `Blocked by #99` is not followed"
    );
    Ok(())
}

#[test]
fn read_alone_stays_within_the_set() -> Result<(), TrackerError> {
    let tickets = GitHub::new(recorded(), repo(), Some(4)).read()?;

    assert_eq!(tickets.len(), 3, "only the sub-issues");
    Ok(())
}

#[test]
fn an_outside_issue_that_is_not_found_is_left_dangling() -> Result<(), TrackerError> {
    let tickets = GitHub::new(one_issue("Blocked by #404", not_found), repo(), Some(5))
        .read_with_external()?;

    assert_eq!(
        summary(&tickets.in_scope),
        [("50", Status::Open, vec!["404"])],
        "the reference is kept"
    );
    assert!(tickets.external.is_empty(), "nothing to stand in for #404");
    Ok(())
}

#[test]
fn any_other_failure_fetching_an_outside_issue_is_an_error() {
    let result =
        GitHub::new(one_issue("Blocked by #7", forbidden), repo(), Some(5)).read_with_external();

    assert!(
        matches!(
            &result,
            Err(TrackerError::GitHub(GitHubError::Gh(GhError::Failed { endpoint, .. })))
                if endpoint == "repos/octo/tickets/issues/7"
        ),
        "got {result:?}"
    );
}

#[test]
fn more_outside_references_than_the_cap_fail_before_any_is_fetched() {
    let refs: Vec<String> = (1000..=1000 + MAX_EXTERNAL)
        .map(|n| format!("#{n}"))
        .collect();
    let body = format!("Blocked by {}", refs.join(", "));

    let result = GitHub::new(one_issue(&body, forbidden), repo(), Some(5)).read_with_external();

    assert!(
        matches!(
            &result,
            Err(TrackerError::GitHub(GitHubError::TooManyExternal { count }))
                if *count == MAX_EXTERNAL + 1
        ),
        "got {result:?}"
    );
}

#[test]
fn gh_saying_http_404_is_not_found_and_anything_else_a_failure() {
    let missing = failure("repos/o/r/issues/9", b"gh: Not Found (HTTP 404)\n");
    let limited = failure(
        "repos/o/r/issues/9",
        b"gh: API rate limit exceeded (HTTP 403)\n",
    );

    assert!(
        matches!(&missing, GhError::NotFound { endpoint } if endpoint == "repos/o/r/issues/9"),
        "got {missing:?}"
    );
    assert!(
        matches!(&limited, GhError::Failed { stderr, .. }
            if stderr == "gh: API rate limit exceeded (HTTP 403)"),
        "got {limited:?}"
    );
}
