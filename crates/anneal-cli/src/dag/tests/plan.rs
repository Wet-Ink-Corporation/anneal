//! `dag check --plan` through `check_with`, over markdown tickets in one temporary directory
//! and the plan in another (a plan beside the tickets would be read as a ticket).

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde_json::json;
use tempfile::TempDir;

use super::{emitted, keys, tickets};
use crate::dag::check_with;
use crate::dag::plan_tickets::PlanTasks;
use crate::dag::tracker::markdown::Markdown;
use crate::lint::skills::LintError;

/// A plan with tasks `(id, blockers)`, each a well-formed task, and no spec on disk.
fn plan_text(tasks: &[(&str, &str)]) -> String {
    let mut text = String::from("# Plan\n\n- Spec: `spec.md`\n\n## Goal\n\nA goal.\n\n## Tasks\n");
    for (id, blocked_by) in tasks {
        write!(
            text,
            "\n### {id}: Task {id}\n\n- Files: `src/{id}.rs`\n- Test: `cargo test {id}`\n\
             - Blocked by: {blocked_by}\n"
        )
        .expect("writing to a String cannot fail");
    }
    text.push_str("\n## Review focus\n\n- Everything.\n");
    text
}

/// Writes `text` as `plan.md` in a fresh directory and reads it as `plan.md` from there.
fn read_plan(text: &str) -> (TempDir, PlanTasks) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("plan.md"), text).expect("write the plan");
    let plan = PlanTasks::read(Path::new("plan.md"), dir.path()).expect("the plan is read");
    (dir, plan)
}

#[test]
fn tickets_that_agree_with_the_plan_exit_0_and_report_the_plan() {
    let dir = tickets(&[
        ("T1.md", "---\nblocked_by: []\n---\n"),
        ("T2.md", "---\nblocked_by: [T1]\n---\n"),
    ]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 0, "agree: {envelope}");
    assert_eq!(
        envelope["data"]["plan"],
        json!({"file": "plan.md", "tasks": 2, "no_ticket": [], "no_task": [], "blockers": [],
               "unread": 0}),
        "the plan's part of data, the file as given"
    );
    assert_eq!(envelope["findings"], json!([]), "no finding");
}

#[test]
fn check_with_a_plan_exits_3_with_one_finding_per_mismatch() {
    let dir = tickets(&[
        ("T1.md", "---\nblocked_by: []\n---\n"),
        ("T3.md", "---\nblocked_by: []\n---\n"),
        ("T9.md", "---\nblocked_by: [T1]\n---\n"),
    ]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1"), ("T3", "T1")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 3, "the tickets disagree with the plan");
    assert_eq!(envelope["ok"], json!(false), "not ok");
    assert_eq!(
        keys(&envelope),
        [
            "dag/plan/no-ticket:T2",
            "dag/plan/no-task:T9",
            "dag/plan/blockers:T3",
        ],
        "one finding per mismatch"
    );
    assert_eq!(
        envelope["data"]["plan"]["blockers"],
        json!([{"id": "T3", "plan": ["T1"], "ticket": []}]),
        "both lists"
    );
}

#[test]
fn check_with_a_plan_keeps_the_graph_code_when_the_graph_fails() {
    let dir = tickets(&[("T1.md", "---\nblocked_by: [ghost]\n---\n")]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 2, "dangling wins");
    assert_eq!(
        keys(&envelope),
        ["dag/dangling:T1->ghost", "dag/plan/no-ticket:T2"],
        "the graph's findings, then the plan's; ghost is no task, so not compared"
    );
}

#[test]
fn a_plan_whose_tasks_cannot_all_be_read_is_a_finding() {
    let dir = tickets(&[("T1.md", "---\nblocked_by: []\n---\n")]);
    let text = plan_text(&[("T1", "none")])
        .replace("## Review focus", "### not a task\n\n## Review focus");
    let (_plan_dir, plan) = read_plan(&text);

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 3, "a plan read in part cannot be said to agree");
    assert_eq!(
        envelope["data"]["plan"]["unread"],
        json!(1),
        "one task finding"
    );
    assert_eq!(
        keys(&envelope),
        ["dag/plan/unread:plan.md"],
        "names the plan"
    );
}

#[test]
fn an_outside_blocker_ticket_file_does_not_fail_the_check() {
    let dir = tickets(&[
        ("T1.md", "---\nblocked_by: [REL-5]\n---\n"),
        ("T2.md", "---\nblocked_by: [T1]\n---\n"),
        ("REL-5.md", "---\nblocked_by: []\n---\n"),
    ]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(
        code, 0,
        "REL-5 is a blocker from outside the plan: {envelope}"
    );
    assert_eq!(envelope["findings"], json!([]), "no finding");
}

#[test]
fn a_plan_blocker_outside_the_plan_is_unread() {
    let dir = tickets(&[
        ("T1.md", "---\nblocked_by: []\n---\n"),
        ("T2.md", "---\nblocked_by: [T1]\n---\n"),
    ]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1, REL-5")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 3, "lint plan refuses an outside blocker in the plan");
    assert_eq!(envelope["data"]["plan"]["unread"], json!(1), "one finding");
    assert_eq!(
        keys(&envelope),
        ["dag/plan/unread:plan.md"],
        "the blockers among the plan's tasks agree"
    );
}

#[test]
fn findings_lint_plan_dropped_count_as_unread() {
    let tasks: Vec<String> = (1..=1_001).map(|n| format!("T{n}")).collect();
    let pairs: Vec<(&str, &str)> = tasks.iter().map(|id| (id.as_str(), id.as_str())).collect();
    let dir = tickets(&[]);
    let (_plan_dir, plan) = read_plan(&plan_text(&pairs));

    let (_, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(
        envelope["data"]["plan"]["unread"],
        json!(1),
        "1,001 self-blocked tasks: lint plan keeps 1,000 findings and drops one, which may hide \
         anything"
    );
}

#[test]
fn a_cycle_and_a_plan_mismatch_exit_1_with_the_plan_findings_listed() {
    let dir = tickets(&[
        ("T1.md", "---\nblocked_by: [T2]\n---\n"),
        ("T2.md", "---\nblocked_by: [T1]\n---\n"),
    ]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1"), ("T3", "none")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 1, "the cycle's code wins");
    assert_eq!(
        keys(&envelope),
        [
            "dag/cycle:T1->T2",
            "dag/plan/no-ticket:T3",
            "dag/plan/blockers:T1",
        ],
        "the graph's finding, then the plan's"
    );
}

#[test]
fn a_repeated_task_heading_is_unread() {
    let dir = tickets(&[("T1.md", "---\nblocked_by: []\n---\n")]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T1", "none")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 3, "a repeated task may hide one: {envelope}");
    assert_eq!(envelope["data"]["plan"]["unread"], json!(1), "one finding");
    assert_eq!(
        keys(&envelope),
        ["dag/plan/unread:plan.md"],
        "names the plan"
    );
}

#[test]
fn a_malformed_blocked_by_line_is_unread() {
    let dir = tickets(&[
        ("T1.md", "---\nblocked_by: []\n---\n"),
        ("T2.md", "---\nblocked_by: []\n---\n"),
    ]);
    let (_plan_dir, plan) = read_plan(&plan_text(&[("T1", "none"), ("T2", "T1 and more")]));

    let (code, envelope) = emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

    assert_eq!(code, 3, "a blocker may be missing: {envelope}");
    assert_eq!(envelope["data"]["plan"]["unread"], json!(1), "one finding");
    assert_eq!(
        keys(&envelope),
        ["dag/plan/unread:plan.md"],
        "names the plan"
    );
}

#[test]
fn mismatches_are_capped_at_max_findings_and_the_rest_counted() {
    for (count, omitted) in [(1_000, None), (1_001, Some(1))] {
        let tasks: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
        let pairs: Vec<(&str, &str)> = tasks.iter().map(|id| (id.as_str(), "none")).collect();
        let dir = tickets(&[]);
        let (_plan_dir, plan) = read_plan(&plan_text(&pairs));

        let (code, envelope) =
            emitted(|out| check_with(&Markdown::new(dir.path()), Some(&plan), out));

        assert_eq!(code, 3, "{count}: no task has a ticket");
        let findings = envelope["findings"].as_array().expect("findings");
        assert_eq!(findings.len(), 1_000, "{count}: capped");
        assert_eq!(
            findings.last().map(|f| &f["key"]),
            Some(&json!("dag/plan/no-ticket:1000")),
            "{count}: the first in id order, numbers by value"
        );
        let listed = envelope["data"]["plan"]["no_ticket"]
            .as_array()
            .map(Vec::len);
        assert_eq!(listed, Some(1_000), "{count}: capped in data too");
        assert_eq!(
            envelope["data"]["plan"].get("omitted"),
            omitted.map(|n: usize| json!(n)).as_ref(),
            "{count}: the rest counted, only when some were left out"
        );
    }
}

#[test]
fn a_missing_plan_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");

    let read = PlanTasks::read(Path::new("absent.md"), dir.path());

    assert!(
        matches!(read, Err(LintError::MissingPath { .. })),
        "missing: {read:?}"
    );
}
