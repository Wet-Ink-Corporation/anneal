//! `anneal verdict set` posting the `anneal/verdict` commit status (#116), against a fake
//! GitHub on a loopback port named by `ANNEAL_GITHUB_API`; never the network.

use std::process::Output;

use serde_json::{Value, json};

use crate::pr_repo::{PrRepo, TestResult};
use crate::support::{Served, anneal_in, with_fake_http};

/// GitHub's answer to a created status (its body is not read).
const CREATED: &str = "201 Created";

/// The arguments of a tests-verified verdict from anneal's own checks.
const TESTS_VERIFIED: [&str; 4] = ["--level", "tests-verified", "--source", "local"];

/// `anneal verdict set --pr 7 --sha HEAD <rest>` with the API at `api` and, when given,
/// `GH_TOKEN`.
fn set(repo: &PrRepo, api: &str, token: Option<&str>, rest: &[&str]) -> TestResult<Output> {
    let mut command = anneal_in(repo.path());
    command.env("ANNEAL_GITHUB_API", api);
    if let Some(token) = token {
        command.env("GH_TOKEN", token);
    }
    command.args(["verdict", "set", "--pr", "7", "--sha", "HEAD"]);
    Ok(command.args(rest).output()?)
}

/// `verdict set <rest>` with a token, against a fake answering `201 Created`; returns the
/// output and the request the fake read.
fn set_posting(repo: &PrRepo, rest: &[&str]) -> TestResult<(Output, Served)> {
    let (output, served) = with_fake_http(CREATED, "{}", |api| set(repo, api, Some("t0k"), rest));
    Ok((output?, served))
}

/// The JSON body of the status request the fake read.
fn status_body(served: &Served) -> TestResult<Value> {
    Ok(serde_json::from_slice(&served.body)?)
}

/// The envelope on `output`'s stdout.
fn envelope(output: &Output) -> TestResult<Value> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_verdict_the_merge_gate_accepts_posts_success_on_its_commit() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    let head = repo.head()?;

    let (output, served) = set_posting(&repo, &TESTS_VERIFIED)?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(
        served.head.starts_with(&format!(
            "POST /repos/owner/repo/statuses/{head} HTTP/1.1\r\n"
        )),
        "{}",
        served.head
    );
    assert_eq!(served.header("authorization"), Some("Bearer t0k"));
    assert_eq!(
        status_body(&served)?,
        json!({
            "state": "success",
            "context": "anneal/verdict",
            "description": "tests-verified by local: the merge gate passes",
        })
    );
    assert_eq!(
        envelope(&output)?["data"]["commit_status"],
        json!({
            "outcome": "posted",
            "state": "success",
            "description": "tests-verified by local: the merge gate passes",
            "repository": "owner/repo",
        })
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_change_that_still_needs_a_review_is_pending() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    repo.write("src/auth.rs", "pub fn check() {}\n")?;
    repo.commit("a sensitive path")?;

    let (output, served) = set_posting(&repo, &TESTS_VERIFIED)?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let body = status_body(&served)?;
    assert_eq!(
        (&body["state"], &body["description"]),
        (
            &json!("pending"),
            &json!("tests-verified by local: the merge gate wants review")
        ),
        "{body}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_gate_that_cannot_judge_posts_failure() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    repo.write(".anneal/config.json", "{ not json")?;

    let (output, served) = set_posting(&repo, &["--level", "approved", "--source", "human"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let body = status_body(&served)?;
    assert_eq!(
        (&body["state"], &body["description"]),
        (
            &json!("failure"),
            &json!("approved by human: the merge gate denies (unjudged)")
        ),
        "{body}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn no_status_posts_nothing() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;

    let (output, served) = with_fake_http(CREATED, "{}", |api| {
        let rest = [TESTS_VERIFIED.as_slice(), &["--no-status"]].concat();
        set(&repo, api, Some("t0k"), &rest)
    });

    let output = output?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(served.head, "", "no request was sent");
    assert_eq!(
        envelope(&output)?["data"]["commit_status"],
        json!({"outcome": "skipped", "reason": "no_status"})
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn without_a_token_nothing_is_posted_and_stderr_stays_empty() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;

    let (output, served) =
        with_fake_http(CREATED, "{}", |api| set(&repo, api, None, &TESTS_VERIFIED));

    let output = output?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(served.head, "", "no request was sent");
    assert_eq!(
        envelope(&output)?["data"]["commit_status"],
        json!({"outcome": "skipped", "reason": "no_token"})
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_refused_status_keeps_the_verdict_and_exits_4_with_one_warning() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;

    let (output, served) = with_fake_http(
        "500 Internal Server Error",
        r#"{"message":"Server Error"}"#,
        |api| set(&repo, api, Some("t0k"), &TESTS_VERIFIED),
    );

    let output = output?;
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(served.head.starts_with("POST "), "the status was asked for");
    let stderr = std::str::from_utf8(&output.stderr)?;
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("WARNING verdict set: ")
            && stderr.contains("GitHub answered HTTP 500: Server Error"),
        "{stderr}"
    );
    assert!(!stderr.contains("t0k"), "{stderr}");
    let envelope = envelope(&output)?;
    assert_eq!(
        (
            &envelope["ok"],
            &envelope["data"]["commit_status"]["outcome"],
            &envelope["findings"][0]["key"],
        ),
        (&json!(false), &json!("failed"), &json!("verdict/status:7")),
        "{envelope}"
    );
    let events = repo.envelope(&["ledger", "query", "--type", "verdict_set"], 0)?;
    assert_eq!(
        events["data"]["events"][0]["event"]["verdict"],
        json!("tests_verified"),
        "the verdict is recorded: {events}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_repository_with_no_github_origin_fails_the_post_without_sending() -> TestResult {
    let repo = PrRepo::new()?;
    repo.git(&[
        "remote",
        "add",
        "origin",
        "https://gitlab.com/owner/repo.git",
    ])?;

    let (output, served) = with_fake_http(CREATED, "{}", |api| {
        set(&repo, api, Some("t0k"), &TESTS_VERIFIED)
    });

    let output = output?;
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(served.head, "", "no request was sent");
    assert_eq!(
        envelope(&output)?["data"]["commit_status"],
        json!({
            "outcome": "failed",
            "state": "success",
            "reason": "the origin remote names gitlab.com/owner/repo, not a github.com repository",
        })
    );
    Ok(())
}

/// Whether `token` shows in `output`'s stdout or stderr.
fn shows(output: &Output, token: &str) -> bool {
    [&output.stdout, &output.stderr]
        .into_iter()
        .any(|stream| String::from_utf8_lossy(stream).contains(token))
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_refused_status_shows_the_token_in_neither_stream() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;

    let (output, _) = with_fake_http(
        "401 Unauthorized",
        r#"{"message":"Bad credentials"}"#,
        |api| set(&repo, api, Some("t0k"), &TESTS_VERIFIED),
    );

    let output = output?;
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(
        !output.stdout.is_empty() && !output.stderr.is_empty(),
        "{output:?}"
    );
    assert!(!shows(&output, "t0k"), "{output:?}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_token_with_a_blank_exits_4_without_sending_or_showing_it() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;

    let (output, served) = with_fake_http(CREATED, "{}", |api| {
        set(&repo, api, Some("two words"), &TESTS_VERIFIED)
    });

    let output = output?;
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(served.head, "", "no request was sent");
    assert!(!shows(&output, "two words"), "{output:?}");
    assert_eq!(
        envelope(&output)?["data"]["commit_status"],
        json!({
            "outcome": "failed",
            "reason": "GH_TOKEN is not a token (printable ASCII without blanks)",
        })
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git, the binary and a loopback server")]
fn a_human_approval_counts_as_the_review_only_with_actor_human() -> TestResult {
    let repo = PrRepo::new()?;
    repo.ensure_origin()?;
    repo.write("src/auth.rs", "pub fn check() {}\n")?;
    repo.commit("a sensitive path")?;
    let approved = ["--level", "approved", "--source", "human"];

    let (without_actor, unattributed) = set_posting(&repo, &approved)?;
    let with_actor = [approved.as_slice(), &["--actor", "human"]].concat();
    let (by_a_human, attributed) = set_posting(&repo, &with_actor)?;

    assert_eq!(without_actor.status.code(), Some(0), "{without_actor:?}");
    let body = status_body(&unattributed)?;
    assert_eq!(
        (&body["state"], &body["description"]),
        (
            &json!("pending"),
            &json!("approved by human: the merge gate wants review")
        ),
        "{body}"
    );
    assert_eq!(by_a_human.status.code(), Some(0), "{by_a_human:?}");
    let body = status_body(&attributed)?;
    assert_eq!(
        (&body["state"], &body["description"]),
        (
            &json!("success"),
            &json!("approved by human: the merge gate passes")
        ),
        "{body}"
    );
    Ok(())
}
