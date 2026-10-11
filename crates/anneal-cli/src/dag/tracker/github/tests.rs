use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{GhApi, GhCli, GhError, GitHub, GitHubError, MAX_PAGES, PER_PAGE, Repo, blocked_by};
use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::dag::tracker::{Tracker, TrackerError};

mod cli;
mod external;

/// A `gh` that answers from a function of the endpoint: no process, no network.
struct FakeGh<F>(F);

impl<F: Fn(&str) -> Result<Vec<u8>, GhError>> GhApi for FakeGh<F> {
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        (self.0)(endpoint)
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/fixtures/dag/github",
        name,
    ]
    .iter()
    .collect();
    fs::read(&path).expect("fixture exists")
}

fn not_found(endpoint: &str) -> GhError {
    GhError::Failed {
        endpoint: endpoint.to_owned(),
        stderr: "gh: Not Found (HTTP 404)".to_owned(),
    }
}

fn repo() -> Repo {
    "octo/tickets".parse().expect("valid repo")
}

fn summary(tickets: &[Ticket]) -> Vec<(&str, Status, Vec<&str>)> {
    tickets
        .iter()
        .map(|t| {
            let blockers = t.blocked_by.iter().map(TicketId::as_str).collect();
            (t.id.as_str(), t.status, blockers)
        })
        .collect()
}

#[test]
fn blocked_by_needs_a_word_boundary_and_no_negation() {
    let cases: [(&str, &[&str]); 6] = [
        ("Unblocked by #5", &[]),
        ("Unblocked by #5. Blocked by #6", &["6"]),
        ("no longer blocked by #12", &[]),
        ("Not blocked by #13", &[]),
        ("re_blocked by #14", &[]),
        ("(blocked by #15)", &["15"]),
    ];
    for (body, expected) in cases {
        let found: Vec<String> = blocked_by(body).iter().map(ToString::to_string).collect();
        assert_eq!(found, expected, "{body:?}");
    }
}

#[test]
fn blocked_by_reads_plain_bold_and_listed_references() {
    let cases: [(&str, &[&str]); 7] = [
        ("Blocked by #10", &["10"]),
        ("**Blocked by:** #10 · **Blueprint:** x", &["10"]),
        ("blocked BY: #1, #2 and #3.", &["1", "2", "3"]),
        ("line one\nBlocked by #4\nBlocked by #5, #4", &["4", "5"]),
        ("Blocked by octo/other#4", &[]),
        ("Not blocked: #9", &[]),
        ("Blocked by # 7", &[]),
    ];
    for (body, expected) in cases {
        let found: Vec<String> = blocked_by(body).iter().map(ToString::to_string).collect();
        assert_eq!(found, expected, "{body:?}");
    }
}

#[test]
fn the_sub_issues_of_a_parent_become_tickets_from_the_recorded_fixture() -> Result<(), TrackerError>
{
    let gh = FakeGh(|endpoint: &str| {
        if endpoint == "repos/octo/tickets/issues/1/sub_issues?per_page=100&page=1" {
            Ok(fixture("sub-issues.json"))
        } else {
            Err(not_found(endpoint))
        }
    });

    let tickets = GitHub::new(gh, repo(), Some(1)).read()?;

    assert_eq!(
        summary(&tickets),
        [
            ("10", Status::Done, vec![]),
            ("19", Status::Open, vec!["10"]),
            ("20", Status::Open, vec![]),
            ("21", Status::Open, vec!["10", "19", "20"]),
        ],
        "closed is done; refs from bodies, sorted and once each; null body is empty"
    );
    let first = tickets.first().expect("four tickets");
    assert_eq!(first.title, "CLI core", "title kept");
    assert!(first.body.contains("cli.rs"), "body kept for the estimate");
    Ok(())
}

#[test]
fn without_a_parent_every_issue_but_pull_requests_is_a_ticket() -> Result<(), TrackerError> {
    let gh = FakeGh(|endpoint: &str| {
        if endpoint == "repos/octo/tickets/issues?state=all&per_page=100&page=1" {
            Ok(fixture("repo-issues.json"))
        } else {
            Err(not_found(endpoint))
        }
    });

    let tickets = GitHub::new(gh, repo(), None).read()?;

    assert_eq!(
        summary(&tickets),
        [("1", Status::Open, vec![])],
        "PR #2 skipped"
    );
    Ok(())
}

/// A page of `count` open issues numbered from `first`.
fn page(first: u64, count: u64) -> Vec<u8> {
    let issues: Vec<serde_json::Value> = (first..first + count)
        .map(|n| serde_json::json!({"number": n, "title": format!("t{n}"), "state": "open", "body": null}))
        .collect();
    serde_json::to_vec(&issues).expect("serialises")
}

fn page_number(endpoint: &str) -> u64 {
    endpoint
        .rsplit_once("page=")
        .and_then(|(_, n)| n.parse().ok())
        .expect("a page parameter")
}

#[test]
fn pages_are_read_until_a_short_one() -> Result<(), Box<dyn std::error::Error>> {
    let per_page = u64::try_from(PER_PAGE)?;
    let gh = FakeGh(move |endpoint: &str| match page_number(endpoint) {
        1 => Ok(page(1, per_page)),
        2 => Ok(page(per_page + 1, 1)),
        _ => Err(not_found(endpoint)),
    });

    let tickets = GitHub::new(gh, repo(), None).read()?;

    assert_eq!(tickets.len(), PER_PAGE + 1, "both pages");
    Ok(())
}

#[test]
fn past_the_page_cap_the_read_fails_instead_of_truncating() {
    let per_page = u64::try_from(PER_PAGE).expect("fits");
    let gh = FakeGh(move |endpoint: &str| {
        let number = page_number(endpoint);
        assert!(
            number <= u64::try_from(MAX_PAGES).expect("fits"),
            "never past the cap"
        );
        Ok(page((number - 1) * per_page + 1, per_page))
    });

    let result = GitHub::new(gh, repo(), None).read();

    assert!(
        matches!(&result, Err(TrackerError::GitHub(GitHubError::TooMany))),
        "got {result:?}"
    );
}

#[test]
fn a_gh_failure_and_unexpected_json_are_errors() {
    let failing = GitHub::new(FakeGh(|e: &str| Err(not_found(e))), repo(), Some(9)).read();
    let garbled = GitHub::new(
        FakeGh(|_: &str| Ok(b"{\"message\":1}".to_vec())),
        repo(),
        None,
    )
    .read();

    assert!(
        matches!(&failing, Err(TrackerError::GitHub(GitHubError::Gh(GhError::Failed { endpoint, .. })))
            if endpoint.starts_with("repos/octo/tickets/issues/9/sub_issues")),
        "got {failing:?}"
    );
    assert!(
        matches!(
            &garbled,
            Err(TrackerError::GitHub(GitHubError::Json { .. }))
        ),
        "got {garbled:?}"
    );
}

#[test]
fn a_repo_is_owner_slash_name_of_safe_characters() {
    let valid: Repo = "my-org/repo.name_2".parse().expect("valid");
    assert_eq!(valid.to_string(), "my-org/repo.name_2", "round-trips");
    for invalid in [
        "octo",
        "octo/",
        "/x",
        "a/b/c",
        "a/b?page=9",
        "../x",
        "a/..",
        "a b/c",
    ] {
        assert!(invalid.parse::<Repo>().is_err(), "{invalid:?} rejected");
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns git")]
fn the_cli_adapter_reports_a_failing_program_with_its_stderr() {
    // `git api ...` is an unknown git command: a real program that fails, with no network.
    let gh = GhCli {
        program: "git".into(),
        timeout: Duration::from_secs(30),
    };

    let result = gh.get("repos/octo/tickets/issues");

    assert!(
        matches!(&result, Err(GhError::Failed { endpoint, stderr })
            if endpoint == "repos/octo/tickets/issues" && stderr.contains("api")),
        "got {result:?}"
    );
}

#[test]
fn a_missing_program_is_a_spawn_error() {
    let gh = GhCli {
        program: Path::new("/nonexistent/anneal-test-gh").into(),
        timeout: Duration::from_secs(1),
    };

    let result = gh.get("x");

    assert!(matches!(&result, Err(GhError::Spawn(_))), "got {result:?}");
}
