//! `repro record`'s ending of a run, below the binary: a timeout cannot be forced through the
//! command line (`--timeout` is at least 1 s and git answers faster), nor portably a kill by a
//! signal, so both are driven here.

use std::time::Duration;

use anneal_ledger::event::ReproRed;
use anneal_ledger::{EventEnvelope, LedgerEvent, Timestamp};
use serde_json::Value;

use super::{Attempt, Outcome, finish, verdict};
use crate::paths::{PathError, StoreLocation};
use crate::task::{Ending, Run};

fn envelope() -> EventEnvelope {
    let event = LedgerEvent::ReproRed(ReproRed {
        task: "bug-1".to_owned(),
        test: "cargo test parse".to_owned(),
    });
    EventEnvelope::new(event, "cli", Timestamp::now().expect("the clock reads"))
}

#[cfg(unix)]
#[test]
fn a_run_killed_by_a_signal_is_red_and_recorded() {
    use std::os::unix::process::ExitStatusExt;

    // The wait status of a process killed by SIGKILL: no exit code.
    let killed = std::process::ExitStatus::from_raw(9);
    assert_eq!(killed.code(), None, "a signal leaves no exit code");

    let (outcome, code, findings) = verdict("bug-1", Ending::Exited(killed));

    assert_eq!(
        (outcome, code.value(), findings.len()),
        (Outcome::Red, 0, 0),
        "a crash reproduces a failure"
    );
}

#[test]
fn a_timed_out_run_records_nothing_and_exits_4_without_resolving_a_ledger() {
    let attempt = Attempt {
        id: "bug-1",
        command: "cargo test parse",
        envelope: envelope(),
    };
    let run = Run {
        ending: Ending::TimedOut {
            after: Duration::from_secs(5),
        },
        duration: Duration::from_millis(5_004),
    };
    // Resolving a ledger fails, so the run only succeeds if it never asks for one.
    let no_ledger = || -> Result<StoreLocation, PathError> { Err(PathError::NoHome) };
    let mut out = Vec::new();

    let code = finish(attempt, &run, no_ledger, &mut out).expect("it reports");

    assert_eq!(code.value(), 4, "timed out");
    let reported: Value = serde_json::from_slice(&out).expect("one JSON envelope");
    assert_eq!(
        (
            &reported["ok"],
            &reported["data"]["recorded"],
            &reported["data"]["outcome"],
            &reported["data"]["exit"],
            &reported["data"]["timeout_secs"],
            &reported["data"]["ledger"],
            &reported["findings"][0]["key"],
        ),
        (
            &Value::Bool(false),
            &Value::Bool(false),
            &Value::from("timed_out"),
            &Value::Null,
            &Value::from(5),
            &Value::Null,
            &Value::from("repro/timed-out:bug-1"),
        ),
        "{reported}"
    );
    let message = reported["findings"][0]["message"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("a hang is not a reproduction"),
        "{message}"
    );
}
