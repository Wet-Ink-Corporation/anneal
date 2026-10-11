use std::time::Duration;

use anneal_ledger::event::{TestOutcome, TestRun};
use anneal_ledger::{Filter, LedgerEvent, Store};

use super::{DEFAULT_TIMEOUT_SECS, DoneArgs, Ending, Run, TaskError, record, run_command};

#[test]
#[cfg_attr(miri, ignore = "opens a SQLite ledger")]
fn recording_a_timed_out_run_appends_one_red_test_run_and_no_task_done() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = dir.path().join("ledger.sqlite3");
    let args = DoneArgs {
        id: "T1".to_owned(),
        timeout: DEFAULT_TIMEOUT_SECS,
        session: None,
        store: None,
        command: vec!["cargo".into(), "test".into()],
    };
    let run = Run {
        ending: Ending::TimedOut {
            after: Duration::from_secs(5),
        },
        duration: Duration::from_millis(5_004),
    };

    let returned = record(&args, &run, "cargo test", &store).expect("records");

    let stored: Vec<LedgerEvent> = Store::open(&store)
        .expect("the ledger opens")
        .query(&Filter::default())
        .expect("the ledger reads")
        .into_iter()
        .map(|recorded| recorded.event.event)
        .collect();
    assert_eq!(
        stored,
        vec![LedgerEvent::TestRun(TestRun {
            task: Some("T1".to_owned()),
            command: "cargo test".to_owned(),
            outcome: TestOutcome::Red,
            exit: None,
            duration_ms: Some(5_004),
            timeout_secs: Some(5),
        })],
        "exactly one red test_run with no exit code and the timeout, and no task_done"
    );
    assert_eq!(
        returned.len(),
        1,
        "record reports the one event it appended"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn a_command_past_its_deadline_is_killed_and_recorded_red_with_the_timeout() {
    let cwd = std::env::temp_dir();

    // A zero deadline has always passed by the first check, so the kill is deterministic.
    let run = run_command("git", &["--version"], &cwd, Duration::ZERO).expect("git starts");

    assert_eq!(
        run.ending,
        Ending::TimedOut {
            after: Duration::ZERO
        },
        "killed at the deadline"
    );
    assert!(!run.is_green(), "a timeout is never green");
    let event = run.event("T1", "git --version");
    assert_eq!(
        (event.outcome, event.exit, event.timeout_secs),
        (TestOutcome::Red, None, Some(0)),
        "red, no exit code, the deadline recorded"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn a_command_that_exits_0_is_green_with_its_exit_code() {
    let cwd = std::env::temp_dir();

    let run = run_command("git", &["--version"], &cwd, Duration::from_secs(60)).expect("git runs");

    assert!(run.is_green(), "exit 0 is green: {run:?}");
    let event = run.event("T1", "git --version");
    assert_eq!(
        (
            event.task.as_deref(),
            event.outcome,
            event.exit,
            event.timeout_secs
        ),
        (Some("T1"), TestOutcome::Green, Some(0), None),
        "green, exit 0, no timeout"
    );
}

#[test]
#[cfg_attr(miri, ignore = "starts a subprocess")]
fn a_missing_program_is_a_spawn_error_naming_it() {
    let cwd = std::env::temp_dir();

    let error = run_command(
        "anneal-no-such-program-22",
        &[],
        &cwd,
        Duration::from_secs(1),
    )
    .expect_err("cannot start");

    assert!(
        matches!(&error, TaskError::Spawn { program, .. } if program == "anneal-no-such-program-22"),
        "{error:?}"
    );
    assert_eq!(error.code().value(), 2, "not started is exit 2");
}
