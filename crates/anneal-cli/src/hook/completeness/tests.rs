//! The Stop hook with Clef D4: shadow never changes its output, and an enforcing D4 block is a
//! hard gate under the same cap of two blocks in a row.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::run;
use crate::decide::point::Point;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::hook::HookEnv;
use crate::hook::clef_fake::{Fixed, TestResult, World, parsed, reply};
use crate::test_repo::ScratchRepo;

/// A final report with every required section and no success word outside it.
const REPORT: &str = "Here is where things stand.\n\nStatus: BLOCKED\nGate: fmt ok\n\
                      Review: not yet requested\nChanged: none\nDecisions needed: none\n\
                      Not verified: everything";

/// Answers every D4 question with P(yes) `yes`.
struct Answering {
    yes: f64,
}

impl Transport for Answering {
    fn send(&self, _request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        reply(Ok(
            json!({"probabilities": {"yes": self.yes, "no": 1.0 - self.yes}}),
        ))
    }
}

/// Runs the Stop hook on `reply` in the scratch repository, returning its stdout parsed.
fn stop(world: &World, repo: &ScratchRepo, reply: &str, clef: &Answering) -> TestResult<Value> {
    let stdin = json!({
        "session_id": "s1",
        "cwd": repo.path(),
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": reply,
    })
    .to_string()
    .into_bytes();
    let env = HookEnv {
        stdin: stdin.as_slice(),
        current_dir: || Ok(PathBuf::new()),
        lookup: |name: &str| world.lookup(name),
    };
    let mut out = Vec::new();
    run(env, clef, &Fixed, &mut out)?;
    parsed(&out)
}

#[test]
fn shadow_never_changes_the_stop_output_whatever_clef_answers() -> TestResult {
    let repo = ScratchRepo::new()?;
    // A changed source file and a claim: the deterministic claim gate blocks either way.
    repo.write("src/lib.rs", "pub fn f() {}\n")?;
    let claim = format!("All tests pass.\n\n{REPORT}");
    for reply in [REPORT, claim.as_str()] {
        for yes in [0.0, 1.0] {
            let off = stop(&World::new(&[])?, &repo, reply, &Answering { yes })?;
            let shadow = World::configured()?;

            let output = stop(&shadow, &repo, reply, &Answering { yes })?;

            assert_eq!(output, off, "shadow at P(yes) {yes} on {reply:?}");
            assert_eq!(shadow.rows()?.len(), 5, "shadow still asks every section");
        }
    }
    Ok(())
}

#[test]
fn configured_enforce_inside_the_shadow_period_never_changes_the_stop_output() -> TestResult {
    let repo = ScratchRepo::new()?;
    let off = stop(&World::new(&[])?, &repo, REPORT, &Answering { yes: 0.0 })?;
    let world = World::enforce_configured_in_shadow_period()?;

    let output = stop(&world, &repo, REPORT, &Answering { yes: 0.0 })?;

    assert_eq!(output, off, "enforce counts only after 50 answers");
    assert_eq!(world.rows()?.len(), 5, "every section was still asked");
    Ok(())
}

#[test]
fn an_enforcing_d4_block_names_the_section_and_counts_toward_the_cap_of_two() -> TestResult {
    let repo = ScratchRepo::new()?;
    let world = World::enforcing(Point::D4)?;
    let clef = Answering { yes: 0.1 };

    let first = stop(&world, &repo, REPORT, &clef)?;
    let second = stop(&world, &repo, REPORT, &clef)?;
    let third = stop(&world, &repo, REPORT, &clef)?;

    assert_eq!(first["decision"], json!("block"), "{first}");
    let reason = first["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("Gate (0.10)"), "{reason}");
    assert!(reason.contains("Not verified (0.10)"), "{reason}");
    assert_eq!(
        second["decision"],
        json!("block"),
        "the second block in a row"
    );
    assert_eq!(third.get("decision"), None, "never a third: {third}");
    let context = third
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(context.contains("already blocked twice"), "{context}");
    Ok(())
}
