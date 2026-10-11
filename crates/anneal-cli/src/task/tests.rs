use std::time::Duration;

use anneal_ledger::event::TestOutcome;

use super::{Ending, TaskError, run_command};

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
