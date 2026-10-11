use std::error::Error;

use anneal_ledger::event::{ReproRed, TestOutcome, TestRun};
use anneal_ledger::{EventEnvelope, LedgerEvent, LedgerTag, Store, TagKey, Timestamp};

use super::{RULES, nudges};
use crate::mode::{Active, BuildMode, Entered};

type TestResult = Result<(), Box<dyn Error>>;

const WORKTREE: &str = "/repo";

fn store() -> Result<(tempfile::TempDir, Store), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(&dir.path().join("ledger.sqlite3"))?;
    Ok((dir, store))
}

fn append(store: &Store, event: LedgerEvent, worktree: &str) -> Result<u64, Box<dyn Error>> {
    let mut envelope = EventEnvelope::new(event, "cli", Timestamp::now()?);
    envelope.worktree = Some(worktree.to_owned());
    Ok(store.append(&envelope)?)
}

fn test_run(outcome: TestOutcome, exit: i32) -> LedgerEvent {
    LedgerEvent::TestRun(TestRun {
        task: Some("T1".to_owned()),
        command: "cargo test".to_owned(),
        outcome,
        exit: Some(exit),
        duration_ms: Some(1),
        timeout_secs: None,
    })
}

fn repro() -> LedgerEvent {
    LedgerEvent::ReproRed(ReproRed {
        task: "bug".to_owned(),
        test: "cargo test".to_owned(),
    })
}

fn build_at(position: u64) -> Entered {
    Entered {
        position,
        mode: Active::Build(BuildMode {
            plan: "plan.md".to_owned(),
            tree: "tree.json".to_owned(),
            clean: true,
        }),
    }
}

fn diagnose_at(position: u64) -> Entered {
    Entered {
        position,
        mode: Active::Diagnose,
    }
}

fn worktree() -> Result<LedgerTag, Box<dyn Error>> {
    Ok(LedgerTag::new(TagKey::Worktree, WORKTREE)?)
}

#[test]
fn build_mode_with_edits_needs_a_green_test_run_after_entry() -> TestResult {
    let (_dir, store) = store()?;
    let before = append(&store, test_run(TestOutcome::Green, 0), WORKTREE)?;
    append(&store, test_run(TestOutcome::Red, 1), WORKTREE)?;
    let entered = build_at(before);

    let missing = nudges(&store, &worktree()?, &entered, true)?;

    assert_eq!(
        missing,
        [RULES[0].nudge],
        "a green run before entry and a red one after do not count"
    );
    append(&store, test_run(TestOutcome::Green, 0), WORKTREE)?;
    assert!(
        nudges(&store, &worktree()?, &entered, true)?.is_empty(),
        "a green run after entry does"
    );
    Ok(())
}

#[test]
fn build_mode_without_source_edits_needs_nothing() -> TestResult {
    let (_dir, store) = store()?;

    let missing = nudges(&store, &worktree()?, &build_at(0), false)?;

    assert!(missing.is_empty(), "nothing was built yet: {missing:?}");
    Ok(())
}

#[test]
fn diagnose_mode_needs_a_red_reproduction_in_this_worktree_after_entry() -> TestResult {
    let (_dir, store) = store()?;
    append(&store, repro(), "/sibling")?;

    let missing = nudges(&store, &worktree()?, &diagnose_at(0), false)?;

    assert_eq!(
        missing,
        [RULES[1].nudge],
        "a sibling's repro does not count"
    );
    append(&store, repro(), WORKTREE)?;
    assert!(
        nudges(&store, &worktree()?, &diagnose_at(0), false)?.is_empty(),
        "this worktree's does"
    );
    Ok(())
}

#[test]
fn each_rule_names_the_command_that_satisfies_it() {
    for rule in RULES {
        assert!(
            rule.nudge.contains("`anneal "),
            "{:?}: a nudge says what to run",
            rule.event
        );
    }
}
