mod plan;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{check, frontier};
use crate::dag::tracker::Tracker;
use crate::dag::tracker::github::{GhApi, GhError, GitHub};
use crate::dag::tracker::markdown::Markdown;

/// Runs `report` and returns (exit code, envelope).
fn emitted(
    report: impl FnOnce(&mut Vec<u8>) -> Result<crate::exit_codes::Code, crate::error::CliError>,
) -> (u8, Value) {
    let mut out = Vec::new();
    let code = report(&mut out).expect("the report is written");
    let envelope = serde_json::from_slice(&out).expect("one JSON envelope");
    (code.value(), envelope)
}

fn tickets(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, text) in files {
        fs::write(dir.path().join(name), text).expect("write ticket");
    }
    dir
}

fn keys(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["key"].as_str().expect("key"))
        .collect()
}

#[test]
fn check_reports_counts_and_exits_0_on_a_valid_graph() {
    let dir = tickets(&[
        ("1.md", "---\nstatus: done\n---\n"),
        ("2.md", "---\nblocked_by: [1]\n---\n"),
    ]);

    let (code, envelope) = emitted(|out| check(&Markdown::new(dir.path()), out));

    assert_eq!(code, 0, "valid");
    assert_eq!(envelope["command"], json!("dag check"), "command");
    assert_eq!(
        envelope["data"],
        json!({"tracker": "markdown", "tickets": 2, "done": 1, "edges": 1, "dangling": [], "cycle": []}),
        "data"
    );
    assert_eq!(envelope["findings"], json!([]), "no findings");
}

#[test]
fn check_names_the_cycle_and_each_dangling_reference_as_findings() {
    let dir = tickets(&[
        ("a.md", "---\nblocked_by: [b]\n---\n"),
        ("b.md", "---\nblocked_by: [a, ghost]\n---\n"),
    ]);

    let (code, envelope) = emitted(|out| check(&Markdown::new(dir.path()), out));

    assert_eq!(code, 2, "dangling takes precedence");
    assert_eq!(envelope["ok"], json!(false), "not ok");
    assert_eq!(
        envelope["data"]["cycle"],
        json!(["a", "b"]),
        "the cycle in order"
    );
    assert_eq!(
        envelope["data"]["dangling"],
        json!([{"ticket": "b", "missing": "ghost"}]),
        "the dangling ref"
    );
    assert_eq!(
        keys(&envelope),
        ["dag/dangling:b->ghost", "dag/cycle:a->b"],
        "both findings"
    );
}

#[test]
fn frontier_lists_ready_tickets_in_order_with_estimates_and_budget_flags() {
    let dir = tickets(&[
        ("1.md", "---\nstatus: done\n---\n"),
        ("10.md", "---\ntitle: Big\nfiles: [missing.rs]\n---\nabcd\n"),
        ("2.md", "---\nblocked_by: [1]\n---\nab\n"),
        ("3.md", "---\nblocked_by: [10]\n---\n"),
    ]);

    let (code, envelope) =
        emitted(|out| frontier(&Markdown::new(dir.path()), dir.path(), 1_000, out));

    assert_eq!(code, 0, "valid graph");
    assert_eq!(envelope["command"], json!("dag frontier"), "command");
    let data = &envelope["data"];
    assert_eq!(data["budget"], json!(1_000), "budget echoed");
    assert_eq!(data["ready"], json!(2), "2 and 10");
    assert_eq!(data["over_budget"], json!(1), "10 is over");
    assert_eq!(
        data["tickets"],
        json!([
            {"id": "2", "title": "2", "blocked_by": ["1"], "over_budget": false,
             "estimate": {"tokens": 1, "body_tokens": 1, "files": []}},
            {"id": "10", "title": "Big", "blocked_by": [], "over_budget": true,
             "estimate": {"tokens": 2_002, "body_tokens": 2, "files": [
                 {"path": "missing.rs", "tokens": 2_000, "basis": "default"}]}},
        ]),
        "numeric order; 3 waits for 10; 1 is done"
    );
    assert_eq!(
        keys(&envelope),
        ["dag/over-budget:10"],
        "the flag is a finding too"
    );
}

#[test]
fn the_frontier_of_an_invalid_graph_is_empty_and_exits_with_the_check_code() {
    let dir = tickets(&[("1.md", "---\nblocked_by: [1]\n---\n"), ("2.md", "free\n")]);

    let (code, envelope) =
        emitted(|out| frontier(&Markdown::new(dir.path()), dir.path(), 1_000, out));

    assert_eq!(code, 1, "cycle");
    assert_eq!(envelope["data"]["tickets"], json!([]), "nothing listed");
    assert_eq!(keys(&envelope), ["dag/cycle:1"], "the reason");
}

struct Recorded;

impl GhApi for Recorded {
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        let path: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
            "tests/fixtures/dag/github/sub-issues.json",
        ]
        .iter()
        .collect();
        if endpoint.starts_with("repos/octo/tickets/issues/1/sub_issues?") {
            fs::read(path).map_err(|source| GhError::Wait {
                endpoint: endpoint.to_owned(),
                source,
            })
        } else {
            Err(GhError::Failed {
                endpoint: endpoint.to_owned(),
                stderr: "HTTP 404".to_owned(),
            })
        }
    }
}

#[test]
fn the_github_frontier_from_the_recorded_fixture_has_the_unblocked_issues() {
    let tracker = GitHub::new(Recorded, "octo/tickets".parse().expect("repo"), Some(1));
    let root = Path::new("/nonexistent-anneal-root");

    let (code, envelope) = emitted(|out| frontier(&tracker, root, 100_000, out));

    assert_eq!(tracker.name(), "github", "name");
    assert_eq!(code, 0, "valid graph");
    let ids: Vec<&str> = envelope["data"]["tickets"]
        .as_array()
        .expect("tickets")
        .iter()
        .map(|t| t["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["19", "20"], "10 is closed; 21 waits for 19 and 20");
}

/// Recorded answers for the sub-issues of #4 and the outside issues they name (closed #11,
/// open #12, merged pull request #30), the one-issue page of #5 (`Blocked by #404`), and
/// "Not Found" for anything else.
struct Outside;

impl GhApi for Outside {
    fn get(&self, endpoint: &str) -> Result<Vec<u8>, GhError> {
        let name = match endpoint {
            "repos/octo/tickets/issues/4/sub_issues?per_page=100&page=1" => {
                "sub-issues-outside.json"
            }
            "repos/octo/tickets/issues/11" => "issue-11-closed.json",
            "repos/octo/tickets/issues/12" => "issue-12-open.json",
            "repos/octo/tickets/issues/30" => "pull-30-merged.json",
            "repos/octo/tickets/issues/5/sub_issues?per_page=100&page=1" => {
                return Ok(br#"[{"number": 50, "title": "t", "state": "open", "body": "Blocked by #404"}]"#.to_vec());
            }
            _ => {
                return Err(GhError::NotFound {
                    endpoint: endpoint.to_owned(),
                });
            }
        };
        let path: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
            "tests/fixtures/dag/github",
            name,
        ]
        .iter()
        .collect();
        fs::read(path).map_err(|source| GhError::Wait {
            endpoint: endpoint.to_owned(),
            source,
        })
    }
}

#[test]
fn a_closed_outside_blocker_is_satisfied_and_an_open_one_is_listed_as_external() {
    let tracker = GitHub::new(Outside, "octo/tickets".parse().expect("repo"), Some(4));
    let root = Path::new("/nonexistent-anneal-root");

    let (code, envelope) = emitted(|out| frontier(&tracker, root, 100_000, out));

    assert_eq!(code, 0, "outside issues that exist resolve every reference");
    let ids: Vec<&str> = envelope["data"]["tickets"]
        .as_array()
        .expect("tickets")
        .iter()
        .map(|t| t["id"].as_str().expect("id"))
        .collect();
    assert_eq!(
        ids,
        ["40", "43"],
        "#11 closed, #30 merged; 41 waits for open #12"
    );
    assert_eq!(
        envelope["data"]["external_blockers"],
        json!([{"id": "12", "title": "Lint skills", "blocks": ["41"]}]),
        "the open outside blocker and what it holds up"
    );
}

#[test]
fn check_counts_only_the_tickets_in_scope() {
    let tracker = GitHub::new(Outside, "octo/tickets".parse().expect("repo"), Some(4));

    let (code, envelope) = emitted(|out| check(&tracker, out));

    assert_eq!(code, 0, "valid");
    assert_eq!(
        envelope["data"]["tickets"],
        json!(3),
        "#40, #41, #43; not the outside issues"
    );
    assert_eq!(
        envelope["data"]["done"],
        json!(0),
        "none of the three is closed"
    );
}

#[test]
fn an_outside_blocker_github_does_not_know_is_dangling() {
    let tracker = GitHub::new(Outside, "octo/tickets".parse().expect("repo"), Some(5));

    let (code, envelope) = emitted(|out| check(&tracker, out));

    assert_eq!(code, 2, "dangling");
    assert_eq!(
        envelope["findings"][0]["message"],
        json!("50 is blocked by 404, which is not a ticket"),
        "named"
    );
}
