//! The branches of `anneal status` that leave a section absent: an unborn `HEAD` for
//! `verdict`, and a build plan that cannot be read for `plan` and `tasks`.

use std::fs;

use anneal_ledger::LedgerEvent;
use anneal_ledger::event::{Verdict, VerdictSet, VerdictSource};
use serde_json::{Value, json};

use super::{Fixture, PLAN, append, build_mode};
use crate::test_repo::Setup;

/// The keys of the envelope's findings, in order.
fn finding_keys(envelope: &Value) -> Vec<&Value> {
    envelope["findings"]
        .as_array()
        .map(|findings| findings.iter().map(|finding| &finding["key"]).collect())
        .unwrap_or_default()
}

#[test]
fn an_unborn_head_has_no_verdict_section_and_no_finding() -> Setup<()> {
    let fixture = Fixture::new()?;
    let head = fixture.repo.head()?;
    let verdict = LedgerEvent::VerdictSet(VerdictSet {
        pr: 7,
        verdict: Verdict::TestsVerified,
        source: Some(VerdictSource::Local),
        sha: Some(head),
        patch_id: None,
        base: Some("main".to_owned()),
        by: None,
    });
    append(&fixture.store()?, verdict, None, "2026-10-19T10:00:01Z")?;
    fixture
        .repo
        .git(&["checkout", "-q", "--orphan", "unborn"])?;

    let (code, envelope) = fixture.json(&["--fields", "verdict"])?;

    assert_eq!(code, 0, "{envelope}");
    assert_eq!(
        envelope["data"],
        json!({"schema": 1}),
        "a verdict exists, but HEAD names no commit"
    );
    assert_eq!(
        envelope["findings"],
        json!([]),
        "an unborn HEAD is not a problem"
    );
    Ok(())
}

#[test]
fn an_unreadable_build_plan_is_exactly_the_plan_and_tasks_findings() -> Setup<()> {
    let fixture = Fixture::new()?;
    build_mode(&fixture.store()?, &fixture.key()?)?;
    fs::remove_file(fixture.repo.path().join(PLAN))?;

    let (code, envelope) = fixture.json(&["--fields", "phase,plan,tasks"])?;

    assert_eq!(code, 0, "an unreadable section does not fail the command");
    assert_eq!(
        envelope["data"]["phase"]["plan"],
        json!(PLAN),
        "the phase is read: {envelope}"
    );
    assert_eq!(
        (&envelope["data"]["plan"], &envelope["data"]["tasks"]),
        (&Value::Null, &Value::Null),
        "plan and tasks are absent"
    );
    assert_eq!(
        finding_keys(&envelope),
        [
            &json!("status/unreadable:plan"),
            &json!("status/unreadable:tasks"),
        ],
        "one finding for each section the plan feeds: {envelope}"
    );
    let message = envelope["findings"][0]["message"]
        .as_str()
        .ok_or("the finding has a message")?;
    assert!(
        message.starts_with("plan could not be read: the plan docs/plan.md: "),
        "the reason names the plan as build mode does, then the read's error: {message}"
    );
    assert_eq!(
        envelope["findings"][1]["message"].as_str(),
        message
            .strip_prefix("plan")
            .map(|rest| format!("tasks{rest}"))
            .as_deref(),
        "both findings give the same reason"
    );
    Ok(())
}
