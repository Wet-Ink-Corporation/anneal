//! The bridge through a fake [`Runner`]: what `weighin` is asked, and how its answers read.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::batch::ParkRequest;
use super::{BridgeError, Call, PARK_TIMEOUT, Ran, RunError, Runner, decided_items, park};
use crate::process::WaitError;

/// What a fake `weighin` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    program: PathBuf,
    args: Vec<String>,
    input: String,
    cwd: PathBuf,
    timeout: Duration,
}

/// Answers every call with one fixed result and keeps what it was asked.
struct Fake {
    answer: Result<Ran, fn() -> RunError>,
    asked: RefCell<Vec<Asked>>,
}

impl Fake {
    fn answering(code: Option<i32>, stdout: &str, stderr: &str) -> Self {
        Self {
            answer: Ok(Ran {
                code,
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            }),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn failing(error: fn() -> RunError) -> Self {
        Self {
            answer: Err(error),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<Asked> {
        // The test reads its own copy of the calls.
        self.asked.borrow().clone()
    }
}

impl Runner for Fake {
    fn run(&self, call: &Call<'_>) -> Result<Ran, RunError> {
        self.asked.borrow_mut().push(Asked {
            program: call.program.to_path_buf(),
            args: call.args.iter().map(|arg| (*arg).to_owned()).collect(),
            input: String::from_utf8_lossy(call.input).into_owned(),
            cwd: call.cwd.to_path_buf(),
            timeout: call.timeout,
        });
        match &self.answer {
            // Each call hands out its own copy of the fixed answer.
            Ok(ran) => Ok(ran.clone()),
            Err(error) => Err(error()),
        }
    }
}

const RECEIPT: &str = "✓ raised wi-e41e02 (raised)\n\
    ✓ deferred wi-e41e02 until next sitting (default per-tenant by 2026-10-19) · tracking atom \
    .kb/_intake/open_questions/wi-e41e02-should-the-cache-be-shared.md\n";

fn request() -> ParkRequest<'static> {
    ParkRequest {
        question: "Should the cache be shared across tenants?",
        did: "blocked",
        flip: "a tenant needs isolation",
        at: "src/cache.rs:12",
        blocks: "7",
        resolve_by: "2026-10-19",
        default: "per-tenant",
    }
}

#[test]
fn a_park_pipes_one_batch_in_the_working_directory_and_reads_the_item() {
    let fake = Fake::answering(Some(0), RECEIPT, "");

    let item = park(
        &fake,
        Path::new("/opt/weighin"),
        Path::new("/repo"),
        &request(),
    )
    .expect("parked");

    assert_eq!(item.as_str(), "wi-e41e02");
    let asked = fake.asked();
    let [call] = asked.as_slice() else {
        panic!("one call: {asked:?}");
    };
    assert_eq!(call.program, Path::new("/opt/weighin"));
    assert_eq!(call.args, ["batch"]);
    assert_eq!(call.cwd, Path::new("/repo"));
    assert_eq!(call.timeout, PARK_TIMEOUT);
    assert!(
        call.input
            .starts_with(r#"[{"op":"raise","q":"Should the cache"#),
        "the batch on stdin: {}",
        call.input
    );
}

#[test]
fn a_non_zero_exit_is_a_failure_naming_its_code_and_first_stderr_line() {
    let fake = Fake::answering(
        Some(1),
        "✓ raised wi-bc9073 (raised)\n",
        "weighin: 1 of 2 operations failed; the rest were applied\nmore\n",
    );

    let error = park(&fake, Path::new("w"), Path::new("."), &request()).expect_err("exit 1");

    assert_eq!(
        error.to_string(),
        "weighin exited with 1: weighin: 1 of 2 operations failed; the rest were applied"
    );
}

#[test]
fn a_signal_a_timeout_and_an_unreadable_receipt_are_failures() {
    let killed = Fake::answering(None, RECEIPT, "");
    let slow = Fake::failing(|| RunError::Wait(WaitError::Timeout));
    let garbled = Fake::answering(Some(0), "✓ ok\n", "");

    let killed = park(&killed, Path::new("w"), Path::new("."), &request());
    let slow = park(&slow, Path::new("w"), Path::new("."), &request());
    let garbled = park(&garbled, Path::new("w"), Path::new("."), &request());

    assert!(
        matches!(killed, Err(BridgeError::Exit { code: None, .. })),
        "{killed:?}"
    );
    assert!(
        matches!(
            slow,
            Err(BridgeError::Run(RunError::Wait(WaitError::Timeout)))
        ),
        "{slow:?}"
    );
    assert!(
        matches!(garbled, Err(BridgeError::Receipt(_))),
        "{garbled:?}"
    );
}

#[test]
fn output_over_the_cap_is_refused_rather_than_read_in_part() {
    let long = "x".repeat(super::MAX_OUTPUT_BYTES + 1);
    let fake = Fake::answering(Some(0), &long, "");

    let error = park(&fake, Path::new("w"), Path::new("."), &request()).expect_err("too long");

    assert!(matches!(error, BridgeError::TooLarge { .. }), "{error:?}");
}

#[test]
fn decided_items_are_read_from_open_all_json_within_the_given_time() {
    let listed = r#"[{"id": "wi-e41e02", "status": "decided", "question": "Shared?",
                      "decision": {"option": "shared"}}]"#;
    let fake = Fake::answering(Some(0), listed, "");

    let found = decided_items(
        &fake,
        Path::new("w"),
        Path::new("/repo"),
        Duration::from_millis(700),
    )
    .expect("read");

    let asked = fake.asked();
    let [call] = asked.as_slice() else {
        panic!("one call: {asked:?}");
    };
    assert_eq!(call.args, ["open", "--all", "--json"]);
    assert_eq!(call.input, "", "nothing on stdin");
    assert_eq!(call.timeout, Duration::from_millis(700));
    let ids: Vec<&str> = found.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["wi-e41e02"]);
}

#[test]
fn a_listing_that_does_not_parse_is_a_failure() {
    let fake = Fake::answering(Some(0), "1 open in repo\n", "");

    let error =
        decided_items(&fake, Path::new("w"), Path::new("."), PARK_TIMEOUT).expect_err("not JSON");

    assert!(matches!(error, BridgeError::Listing(_)), "{error:?}");
}
