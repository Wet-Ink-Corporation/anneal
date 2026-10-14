//! `anneal dag check|frontier` through the built binary, over the markdown graph fixtures in
//! `tests/fixtures/dag/` (acyclic, cycle, dangling, empty).

use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::support::anneal_with_home;

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests/fixtures/dag"]
        .iter()
        .collect()
}

/// Runs `anneal dag <args>` in the fixtures directory.
fn dag(args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    Ok(anneal_with_home(&fixtures(), home.path())
        .arg("dag")
        .args(args)
        .output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn finding_messages(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|f| f["message"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_exits_0_on_the_acyclic_fixture() -> TestResult {
    let output = dag(&["check", "--dir", "acyclic"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(envelope["ok"], json!(true), "ok");
    assert_eq!(
        envelope["data"],
        json!({"tracker": "markdown", "tickets": 4, "done": 1, "edges": 4, "dangling": [], "cycle": []}),
        "README.md is not a ticket"
    );
    assert!(output.stderr.is_empty(), "nothing on stderr");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_exits_1_naming_the_cycle_in_order() -> TestResult {
    let output = dag(&["check", "--dir", "cycle"])?;

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        envelope["data"]["cycle"],
        json!(["a", "c", "b"]),
        "a by c, c by b, b by a"
    );
    assert_eq!(
        finding_messages(&envelope),
        ["cycle: a -> c -> b -> a (each is blocked by the next)"],
        "one finding naming the ids"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_exits_2_naming_each_dangling_reference() -> TestResult {
    let output = dag(&["check", "--dir", "dangling"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        finding_messages(&envelope),
        [
            "1 is blocked by 9, which is not a ticket",
            "2 is blocked by 7, which is not a ticket",
        ],
        "each (ticket, missing) pair"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_exits_0_on_the_empty_fixture() -> TestResult {
    let output = dag(&["check", "--dir", "empty"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        envelope(&output)?["data"]["tickets"],
        json!(0),
        "no tickets"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn frontier_lists_the_ready_tickets_in_a_stable_order() -> TestResult {
    let first = dag(&["frontier", "--dir", "acyclic"])?;
    let second = dag(&["frontier", "--dir", "acyclic"])?;

    assert_eq!(first.status.code(), Some(0), "{first:?}");
    assert_eq!(first.stdout, second.stdout, "byte-identical across runs");
    let envelope = envelope(&first)?;
    let ids: Vec<&str> = envelope["data"]["tickets"]
        .as_array()
        .map(|t| t.iter().filter_map(|t| t["id"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        ids,
        ["002-cli", "003-guard"],
        "001 is done; 004 waits for both"
    );
    assert_eq!(
        envelope["data"]["budget"],
        json!(100_000),
        "the default budget"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn frontier_flags_tickets_over_the_budget() -> TestResult {
    let output = dag(&["frontier", "--dir", "acyclic", "--budget", "100"])?;

    assert_eq!(
        output.status.code(),
        Some(0),
        "a flag does not fail the run"
    );
    let envelope = envelope(&output)?;
    assert_eq!(
        envelope["data"]["over_budget"],
        json!(1),
        "002-cli names a file"
    );
    assert_eq!(
        finding_messages(&envelope),
        ["002-cli is estimated at 2010 tokens, over the 100-token budget"],
        "body 10 + the unsized file 2000"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn flags_of_the_other_tracker_are_usage_errors() -> TestResult {
    let cases = [
        (
            vec!["check", "--repo", "octo/tickets"],
            "ERROR --repo and --issue need --tracker github\n",
        ),
        (
            vec!["check", "--tracker", "github"],
            "ERROR --tracker github needs --repo owner/name\n",
        ),
        (
            vec![
                "frontier",
                "--tracker",
                "github",
                "--repo",
                "o/r",
                "--dir",
                "x",
            ],
            "ERROR --dir needs --tracker markdown\n",
        ),
    ];
    for (args, stderr) in cases {
        let output = dag(&args)?;
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: nothing on stdout");
        assert_eq!(String::from_utf8(output.stderr)?, stderr, "{args:?}");
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_tickets_directory_exits_70_with_one_error_line() -> TestResult {
    let output = dag(&["check", "--dir", "absent"])?;

    assert_eq!(output.status.code(), Some(70), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR dag: reading the markdown tickets: listing tickets in ")
            && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    Ok(())
}

/// The recorded define chain's plan and tickets.
fn dry_run() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "../../evals/fixtures/dry-run"]
        .iter()
        .collect()
}

/// A working directory holding `plan.md` with tasks `(id, blockers)` and `tickets/<id>.md`
/// for each ticket `(id, blocked_by)`.
fn plan_and_tickets(
    tasks: &[(&str, &str)],
    tickets: &[(&str, &str)],
) -> Result<tempfile::TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let mut plan = String::from("# Plan\n\n- Spec: `spec.md`\n\n## Goal\n\nA goal.\n\n## Tasks\n");
    for (id, blocked_by) in tasks {
        write!(
            plan,
            "\n### {id}: Task {id}\n\n- Files: `src/{id}.rs`\n- Test: `cargo test {id}`\n\
             - Blocked by: {blocked_by}\n"
        )?;
    }
    plan.push_str("\n## Review focus\n\n- Everything.\n");
    std::fs::write(dir.path().join("plan.md"), plan)?;
    std::fs::create_dir(dir.path().join("tickets"))?;
    for (id, blocked_by) in tickets {
        std::fs::write(
            dir.path().join("tickets").join(format!("{id}.md")),
            format!("---\nblocked_by: [{blocked_by}]\n---\n"),
        )?;
    }
    Ok(dir)
}

/// Runs `anneal dag check --plan plan.md` in `dir`, whose tickets are in `tickets/`.
fn check_plan_in(dir: &std::path::Path) -> Result<Output, Box<dyn Error>> {
    let home = tempfile::tempdir()?;
    Ok(anneal_with_home(dir, home.path())
        .args(["dag", "check", "--plan", "plan.md"])
        .output()?)
}

fn finding_keys(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .map(|findings| findings.iter().filter_map(|f| f["key"].as_str()).collect())
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_with_the_dry_run_plan_is_clean() -> TestResult {
    let fixture = dry_run();
    let tickets = fixture.join("tickets");
    let plan = fixture.join("plan.md");
    let tickets = tickets.to_str().ok_or("fixture path is UTF-8")?;
    let plan = plan.to_str().ok_or("fixture path is UTF-8")?;

    let output = dag(&["check", "--dir", tickets, "--plan", plan])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        envelope["data"]["plan"],
        json!({"file": plan, "tasks": 4, "no_ticket": [], "no_task": [], "blockers": [],
               "unread": 0}),
        "every task has its ticket, with the same blockers"
    );
    assert_eq!(envelope["findings"], json!([]), "no finding");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_with_a_plan_reports_a_task_with_no_ticket() -> TestResult {
    let dir = plan_and_tickets(&[("T1", "none"), ("T2", "T1")], &[("T1", "")])?;

    let output = check_plan_in(dir.path())?;

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        finding_keys(&envelope),
        ["dag/plan/no-ticket:T2"],
        "T2 has no ticket"
    );
    assert_eq!(envelope["ok"], json!(false), "not ok");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_with_a_plan_reports_a_ticket_with_no_task() -> TestResult {
    let dir = plan_and_tickets(&[("T1", "none")], &[("T1", ""), ("T5", "T1")])?;

    let output = check_plan_in(dir.path())?;

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert_eq!(
        finding_keys(&envelope(&output)?),
        ["dag/plan/no-task:T5"],
        "T5 is no task of the plan"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_with_a_plan_reports_differing_blockers() -> TestResult {
    let dir = plan_and_tickets(&[("T1", "none"), ("T2", "T1")], &[("T1", ""), ("T2", "")])?;

    let output = check_plan_in(dir.path())?;

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let envelope = envelope(&output)?;
    assert_eq!(
        finding_messages(&envelope),
        ["ticket T2 is blocked by [], but the plan's task is blocked by [T1]"],
        "the ticket dropped its blocker"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_with_a_missing_plan_is_a_usage_error() -> TestResult {
    let output = dag(&["check", "--dir", "acyclic", "--plan", "absent.md"])?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("ERROR plan: no such path: ") && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_without_a_plan_has_no_plan_in_its_data() -> TestResult {
    let output = dag(&["check", "--dir", "acyclic"])?;

    let envelope = envelope(&output)?;
    assert_eq!(
        envelope["data"].get("plan"),
        None,
        "the envelope is as before"
    );
    Ok(())
}
