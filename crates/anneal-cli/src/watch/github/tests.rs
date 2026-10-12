//! The pull request and commit readers against recorded REST and GraphQL answers.

use super::{CommitWatch, MAX_CHECKS, PullRequestWatch, RepoRef};
use crate::watch::WatchError;
use crate::watch::fake::{FakeGate, FakeGh, pull_request};
use crate::watch::poll::Source;
use crate::watch::snapshot::{Blocker, Pending, Snapshot};

const HEAD: &str = "4f2c1d0a9b8e7f6a5d4c3b2a1f0e9d8c7b6a5f4e";
const PR_URL: &str = "https://github.com/octo/app/pull/7";

fn poll_pr(gh: &FakeGh, gate: &FakeGate) -> Result<Snapshot, WatchError> {
    PullRequestWatch { gh, gate, pr: 7 }.poll()
}

fn green(pull: &'static str) -> FakeGh {
    pull_request(
        pull,
        "check-runs-pass.json",
        "status-pass.json",
        "threads-resolved.json",
    )
}

fn check(name: &str, conclusion: &str, url: Option<&str>) -> Blocker {
    Blocker::Check {
        name: name.to_owned(),
        conclusion: conclusion.to_owned(),
        url: url.map(str::to_owned),
    }
}

fn job(id: u32) -> String {
    format!("https://github.com/octo/app/actions/runs/500/job/{id}")
}

#[test]
fn a_clear_pull_request_is_judged_by_the_gate_at_its_head() {
    let gate = FakeGate::default();

    let snapshot = poll_pr(&green("pull-open.json"), &gate).expect("the fixtures read");

    assert_eq!(
        snapshot,
        Snapshot {
            head: Some(HEAD.to_owned()),
            blockers: Vec::new(),
            pending: Vec::new(),
        },
        "passed and skipped checks, a passed status and a resolved thread block nothing"
    );
    assert_eq!(
        *gate.judged.borrow(),
        vec![(7, HEAD.to_owned())],
        "the gate saw GitHub's head"
    );
}

#[test]
fn a_pull_request_is_read_from_gh_s_current_repository_and_its_head_s_checks() {
    let gh = green("pull-open.json");

    poll_pr(&gh, &FakeGate::default()).expect("the fixtures read");

    assert_eq!(
        *gh.calls.borrow(),
        vec![
            "repos/{owner}/{repo}/pulls/7".to_owned(),
            "graphql -F owner={owner} -F name={repo} -F number=7".to_owned(),
            format!("repos/{{owner}}/{{repo}}/commits/{HEAD}/check-runs?per_page=100"),
            format!("repos/{{owner}}/{{repo}}/commits/{HEAD}/status?per_page=100"),
        ],
        "gh fills {{owner}} and {{repo}} from the worktree's repository"
    );
}

#[test]
fn a_conflict_blocks_naming_the_pull_request() {
    let snapshot =
        poll_pr(&green("pull-conflict.json"), &FakeGate::default()).expect("the fixtures read");

    assert_eq!(
        snapshot.blockers,
        vec![Blocker::Conflict {
            pr: 7,
            url: PR_URL.to_owned()
        }]
    );
}

#[test]
fn each_unresolved_thread_blocks_by_its_url_and_resolved_ones_do_not() {
    let gh = pull_request(
        "pull-open.json",
        "check-runs-pass.json",
        "status-pass.json",
        "threads-open.json",
    );

    let snapshot = poll_pr(&gh, &FakeGate::default()).expect("the fixtures read");

    assert_eq!(
        snapshot.blockers,
        vec![
            Blocker::Thread {
                url: Some(format!("{PR_URL}#discussion_r2001")),
                path: Some("src/watch.rs".to_owned()),
            },
            Blocker::Thread {
                url: Some(format!("{PR_URL}#discussion_r2003")),
                path: Some("docs/watch.md".to_owned()),
            },
        ]
    );
}

#[test]
fn failing_checks_and_statuses_block_by_name_and_url_and_the_gate_is_not_asked() {
    let gh = pull_request(
        "pull-open.json",
        "check-runs-failing.json",
        "status-error.json",
        "threads-none.json",
    );
    let gate = FakeGate::default();

    let snapshot = poll_pr(&gh, &gate).expect("the fixtures read");

    assert_eq!(
        snapshot.blockers,
        vec![
            check("test (ubuntu-latest)", "failure", Some(&job(9101))),
            check("lint", "timed_out", Some(&job(9103))),
            check(
                "ci/circleci: build",
                "error",
                Some("https://circleci.com/gh/octo/app/78")
            ),
        ]
    );
    assert_eq!(
        snapshot.pending,
        vec![Pending::Check {
            name: "test (windows-latest)".to_owned(),
            status: "in_progress".to_owned()
        }],
        "a running check is still listed"
    );
    assert!(
        gate.judged.borrow().is_empty(),
        "the gate waits for the rest"
    );
}

#[test]
fn running_checks_and_unknown_mergeability_are_pending() {
    let gh = pull_request(
        "pull-unknown.json",
        "check-runs-running.json",
        "status-pending.json",
        "threads-none.json",
    );

    let snapshot = poll_pr(&gh, &FakeGate::default()).expect("the fixtures read");

    assert_eq!(
        (snapshot.blockers, snapshot.pending),
        (
            Vec::new(),
            vec![
                Pending::Mergeable,
                Pending::Check {
                    name: "test (ubuntu-latest)".to_owned(),
                    status: "in_progress".to_owned()
                },
                Pending::Check {
                    name: "ci/circleci: build".to_owned(),
                    status: "pending".to_owned()
                },
            ]
        )
    );
}

#[test]
fn a_commit_no_check_has_reported_on_is_pending_not_clear() {
    let gh = pull_request(
        "pull-open.json",
        "check-runs-none.json",
        "status-none.json",
        "threads-none.json",
    );

    let snapshot = poll_pr(&gh, &FakeGate::default()).expect("the fixtures read");

    assert_eq!(snapshot.pending, vec![Pending::NoChecks]);
}

#[test]
fn a_denying_gate_blocks_a_pull_request_github_finds_clear() {
    let gate = FakeGate::denying("merge-gate: #7 has no verdict");

    let snapshot = poll_pr(&green("pull-open.json"), &gate).expect("the fixtures read");

    assert_eq!(
        snapshot.blockers,
        vec![Blocker::Gate {
            check: "no-verdict",
            reason: "merge-gate: #7 has no verdict".to_owned()
        }]
    );
}

#[test]
fn a_merged_pull_request_is_clear_without_reading_more_or_asking_the_gate() {
    let gh = green("pull-merged.json");
    let gate = FakeGate::denying("never asked");

    let snapshot = poll_pr(&gh, &gate).expect("the fixtures read");

    assert_eq!(
        (snapshot.blockers.len(), snapshot.pending.len()),
        (0, 0),
        "nothing blocks a merged pull request"
    );
    assert_eq!(gh.calls.borrow().len(), 1, "only the pull request is read");
    assert!(gate.judged.borrow().is_empty(), "no merge left to gate");
}

#[test]
fn a_pull_request_closed_unmerged_is_a_query_error() {
    let result = poll_pr(&green("pull-closed.json"), &FakeGate::default());

    assert!(
        matches!(result, Err(WatchError::Closed { pr: 7 })),
        "{result:?}"
    );
}

#[test]
fn a_pull_request_github_does_not_know_is_a_query_error() {
    let gh = FakeGh::new().missing("/pulls/7");

    let result = poll_pr(&gh, &FakeGate::default());

    assert!(
        matches!(&result, Err(WatchError::Missing { what }) if what == "pull request 7"),
        "{result:?}"
    );
}

#[test]
fn more_checks_than_are_read_fail_rather_than_judge_part_of_them() {
    let gh = pull_request(
        "pull-open.json",
        "check-runs-too-many.json",
        "status-pass.json",
        "threads-none.json",
    );

    let result = poll_pr(&gh, &FakeGate::default());

    assert!(
        matches!(
            result,
            Err(WatchError::TooMany {
                what: "check runs",
                limit: MAX_CHECKS
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_commit_s_failed_check_run_and_status_block_and_its_ref_is_escaped_in_the_path() {
    let gh = FakeGh::new()
        .route("/check-runs", &["check-runs-cancelled.json"])
        .route("/status", &["status-failure.json"]);
    let repo = "octo/app".parse().expect("valid");

    let snapshot = CommitWatch {
        gh: &gh,
        repo: RepoRef::Named(&repo),
        git_ref: "feature/x {owner}",
    }
    .poll()
    .expect("the fixtures read");

    assert_eq!(
        snapshot.blockers,
        vec![
            check("build", "cancelled", Some(&job(9401))),
            check("deploy/preview", "failure", None),
        ]
    );
    assert_eq!(
        snapshot.head.as_deref(),
        Some(HEAD),
        "the commit the ref names"
    );
    assert_eq!(
        gh.calls.borrow().first().map(String::as_str),
        Some("repos/octo/app/commits/feature%2Fx%20%7Bowner%7D/check-runs?per_page=100"),
        "a ref can neither leave its path segment nor fill a gh placeholder"
    );
}

#[test]
fn a_ref_that_names_no_commit_is_a_query_error() {
    let gh = FakeGh::new().missing("/check-runs").missing("/status");

    let result = CommitWatch {
        gh: &gh,
        repo: RepoRef::Current,
        git_ref: "no-such-branch",
    }
    .poll();

    assert!(
        matches!(&result, Err(WatchError::Missing { what }) if what == "commit no-such-branch"),
        "{result:?}"
    );
}
