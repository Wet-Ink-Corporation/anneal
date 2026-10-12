//! `anneal watch` end to end over recorded fixtures: one test per exit code, plus the long
//! poll, the default timeout and the deployment id. No process, network or sleep.

use std::time::Duration;

use clap::Parser;
use serde_json::{Value, json};

use super::{Ports, WatchArgs, execute};
use crate::cli::{Cli, Command};
use crate::error::CliError;
use crate::watch::fake::{FakeClock, FakeGate, FakeGh, pull_request};

const HEAD: &str = "4f2c1d0a9b8e7f6a5d4c3b2a1f0e9d8c7b6a5f4e";

fn parse(args: &[&str]) -> WatchArgs {
    let words = ["anneal", "watch"].iter().chain(args).copied();
    match Cli::try_parse_from(words)
        .expect("the arguments parse")
        .command
    {
        Some(Command::Watch(args)) => args,
        other => panic!("not a watch: {other:?}"),
    }
}

/// What a watch wrote and how it slept.
struct Ran {
    code: u8,
    envelope: Value,
    sleeps: Vec<Duration>,
}

/// Runs `anneal watch <args>` against `gh` and `gate` on a fake clock.
fn run(args: &[&str], gh: &FakeGh, gate: &FakeGate) -> Ran {
    let clock = FakeClock::default();
    let mut out = Vec::new();
    let ports = Ports {
        gh,
        gate,
        clock: &clock,
    };
    let code = execute(&parse(args).action, &ports, &mut out).expect("the watch ran");
    Ran {
        code: code.value(),
        envelope: serde_json::from_slice(&out).expect("one JSON envelope"),
        sleeps: clock.sleeps.take(),
    }
}

/// The error of `anneal watch <args>` against `gh`, and what it wrote on stdout.
fn fail(args: &[&str], gh: &FakeGh) -> (CliError, Vec<u8>) {
    let clock = FakeClock::default();
    let mut out = Vec::new();
    let ports = Ports {
        gh,
        gate: &FakeGate::default(),
        clock: &clock,
    };
    let error = execute(&parse(args).action, &ports, &mut out).expect_err("the watch fails");
    (error, out)
}

/// A pull request with passing checks and no open thread: clear on GitHub's side.
fn green(pull: &'static str) -> FakeGh {
    pull_request(
        pull,
        "check-runs-pass.json",
        "status-pass.json",
        "threads-resolved.json",
    )
}

fn keys(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .expect("findings is a list")
        .iter()
        .map(|finding| finding["key"].as_str().expect("a key"))
        .collect()
}

#[test]
fn exit_0_when_github_finds_the_pull_request_clear_and_the_gate_allows_its_head() {
    let ran = run(&["pr", "7"], &green("pull-open.json"), &FakeGate::default());

    assert_eq!(ran.code, 0);
    assert_eq!(
        (
            &ran.envelope["ok"],
            &ran.envelope["command"],
            &ran.envelope["data"]
        ),
        (
            &json!(true),
            &json!("watch pr"),
            &json!({
                "target": {"kind": "pr", "number": 7},
                "state": "clear",
                "head": HEAD,
                "blockers": [],
                "pending": [],
                "polls": 1,
                "waited_secs": 0,
                "timeout_secs": 1800,
            })
        ),
        "the default timeout is 30 minutes"
    );
}

#[test]
fn exit_2_for_a_merge_conflict() {
    let ran = run(
        &["pr", "7"],
        &green("pull-conflict.json"),
        &FakeGate::default(),
    );

    assert_eq!(ran.code, 2);
    assert_eq!(
        ran.envelope["data"]["blockers"],
        json!([{"kind": "conflict", "pr": 7, "url": "https://github.com/octo/app/pull/7"}])
    );
    assert_eq!(keys(&ran.envelope), ["watch/conflict:7"]);
}

#[test]
fn exit_3_for_unresolved_review_threads_naming_each_url() {
    let gh = pull_request(
        "pull-open.json",
        "check-runs-pass.json",
        "status-pass.json",
        "threads-open.json",
    );

    let ran = run(&["pr", "7"], &gh, &FakeGate::default());

    assert_eq!(ran.code, 3);
    assert_eq!(
        keys(&ran.envelope),
        [
            "watch/thread:https://github.com/octo/app/pull/7#discussion_r2001",
            "watch/thread:https://github.com/octo/app/pull/7#discussion_r2003",
        ]
    );
}

#[test]
fn exit_4_for_failing_checks_naming_each_check() {
    let gh = pull_request(
        "pull-open.json",
        "check-runs-failing.json",
        "status-error.json",
        "threads-none.json",
    );

    let ran = run(&["pr", "7"], &gh, &FakeGate::default());

    assert_eq!(ran.code, 4);
    assert_eq!(
        keys(&ran.envelope),
        [
            "watch/check:test (ubuntu-latest)",
            "watch/check:lint",
            "watch/check:ci/circleci: build",
        ]
    );
    assert_eq!(ran.envelope["data"]["state"], json!("blocked"));
}

#[test]
fn exit_5_when_checks_are_still_running_at_the_timeout_naming_them() {
    let gh = pull_request(
        "pull-unknown.json",
        "check-runs-running.json",
        "status-pending.json",
        "threads-none.json",
    );

    let ran = run(&["pr", "7", "--timeout", "30"], &gh, &FakeGate::default());

    assert_eq!(ran.code, 5);
    assert_eq!(
        ran.sleeps,
        [Duration::from_secs(10), Duration::from_secs(20)],
        "backs off 10 s then 20 s, reaching the 30 s timeout"
    );
    assert_eq!(
        (
            &ran.envelope["data"]["state"],
            &ran.envelope["data"]["polls"],
            &ran.envelope["data"]["waited_secs"],
        ),
        (&json!("timeout"), &json!(3), &json!(30))
    );
    assert_eq!(
        keys(&ran.envelope),
        [
            "watch/pending:mergeable",
            "watch/pending:check:test (ubuntu-latest)",
            "watch/pending:check:ci/circleci: build",
        ]
    );
}

#[test]
fn exit_6_when_the_merge_gate_would_deny_the_head() {
    let gate = FakeGate::denying("merge-gate: #7 has no verdict; run its tests");

    let ran = run(&["pr", "7"], &green("pull-open.json"), &gate);

    assert_eq!(ran.code, 6);
    assert_eq!(
        ran.envelope["data"]["blockers"],
        json!([{
            "kind": "gate",
            "check": "no-verdict",
            "reason": "merge-gate: #7 has no verdict; run its tests",
        }])
    );
}

#[test]
fn exit_6_for_a_draft_even_when_the_merge_gate_would_allow_it() {
    let ran = run(
        &["pr", "7"],
        &green("pull-draft.json"),
        &FakeGate::default(),
    );

    assert_eq!(ran.code, 6);
    assert_eq!(
        ran.envelope["data"]["blockers"],
        json!([{"kind": "draft", "pr": 7, "url": "https://github.com/octo/app/pull/7"}])
    );
    assert_eq!(keys(&ran.envelope), ["watch/draft:7"]);
}

#[test]
fn exit_6_naming_the_merge_state_when_github_still_blocks_once_everything_passed() {
    let ran = run(
        &["pr", "7"],
        &green("pull-blocked.json"),
        &FakeGate::default(),
    );

    assert_eq!(ran.code, 6);
    assert_eq!(
        ran.envelope["data"]["blockers"],
        json!([{
            "kind": "merge_state",
            "pr": 7,
            "state": "blocked",
            "url": "https://github.com/octo/app/pull/7",
        }])
    );
    assert_eq!(keys(&ran.envelope), ["watch/merge-state:blocked"]);
}

#[test]
fn exit_5_while_github_reports_the_pull_request_unstable() {
    let ran = run(
        &["pr", "7", "--timeout", "30"],
        &green("pull-unstable.json"),
        &FakeGate::default(),
    );

    assert_eq!(ran.code, 5);
    assert_eq!(keys(&ran.envelope), ["watch/pending:merge-state:unstable"]);
}

#[test]
fn exit_7_with_nothing_on_stdout_when_gh_cannot_be_queried() {
    let gh = FakeGh::new().failing("", "gh: HTTP 401: Bad credentials");

    let (error, out) = fail(&["pr", "7"], &gh);

    assert_eq!(error.code().value(), 7, "{error:?}");
    assert!(out.is_empty(), "an error writes no envelope");
}

#[test]
fn a_closed_pull_request_exits_7_too() {
    let (error, out) = fail(&["pr", "7"], &green("pull-closed.json"));

    assert!(
        matches!(&error, CliError::Watch(_)) && error.code().value() == 7,
        "{error:?}"
    );
    assert!(out.is_empty(), "an error writes no envelope");
}

#[test]
fn a_pull_request_still_running_is_polled_again_until_clear() {
    let gh = FakeGh::new()
        .route("/pulls/7", &["pull-unknown.json", "pull-open.json"])
        .route(
            "/check-runs",
            &["check-runs-running.json", "check-runs-pass.json"],
        )
        .route("/status", &["status-pass.json"])
        .route("graphql", &["threads-none.json"]);

    let ran = run(&["pr", "7"], &gh, &FakeGate::default());

    assert_eq!(
        (ran.code, ran.sleeps, &ran.envelope["data"]["polls"]),
        (0, vec![Duration::from_secs(10)], &json!(2))
    );
}

#[test]
fn a_commit_s_failing_checks_exit_4_naming_the_ref_and_repository() {
    let gh = FakeGh::new()
        .route("/commits/main?", &["commit.json"])
        .route("/check-runs", &["check-runs-cancelled.json"])
        .route("/status", &["status-failure.json"]);

    let ran = run(
        &["ci", "main", "--repo", "octo/app"],
        &gh,
        &FakeGate::default(),
    );

    assert_eq!(ran.code, 4);
    assert_eq!(
        (&ran.envelope["command"], &ran.envelope["data"]["target"]),
        (
            &json!("watch ci"),
            &json!({"kind": "ci", "ref": "main", "repo": "octo/app"})
        )
    );
}

#[test]
fn a_commit_with_passing_checks_exits_0_and_never_asks_the_gate() {
    let gh = FakeGh::new()
        .route("/commits/main?", &["commit.json"])
        .route("/check-runs", &["check-runs-neutral.json"])
        .route("/status", &["status-none.json"]);
    let gate = FakeGate::denying("never asked");

    let ran = run(&["ci", "main"], &gh, &gate);

    assert_eq!(ran.code, 0);
    assert!(
        gate.judged.borrow().is_empty(),
        "the gate is for pull requests"
    );
}

#[test]
fn a_failed_deployment_exits_4_and_a_successful_one_0() {
    let failed = run(
        &["deploy", "42"],
        &FakeGh::new().route("/deployments/42", &["deploy-failure.json"]),
        &FakeGate::default(),
    );
    let deployed = run(
        &["deploy", "42", "--provider", "github"],
        &FakeGh::new().route("/deployments/42", &["deploy-success.json"]),
        &FakeGate::default(),
    );

    assert_eq!((failed.code, deployed.code), (4, 0));
    assert_eq!(
        deployed.envelope["data"]["target"],
        json!({"kind": "deploy", "id": "42", "provider": "github", "repo": null})
    );
}

#[test]
fn a_github_deployment_id_that_is_not_a_number_is_a_usage_error() {
    let gh = FakeGh::new().route("/deployments/", &["deploy-success.json"]);

    let (error, out) = fail(&["deploy", "prod-42"], &gh);

    assert!(
        matches!(&error, CliError::Usage { reason } if reason.contains("prod-42")),
        "{error:?}"
    );
    assert!(
        gh.calls.borrow().is_empty() && out.is_empty(),
        "nothing is asked or written"
    );
}
