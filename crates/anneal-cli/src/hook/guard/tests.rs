use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use anneal_ledger::event::GateBlocked;
use anneal_ledger::{EventKind, Filter, LedgerEvent, Store};
use serde_json::{Value, json};

use super::{Decision, Fallback, MAX_INPUT_BYTES, run};
use crate::hook::HookEnv;

/// Runs the guard on `stdin` with the ledger at `store`, and returns its stdout.
fn guard(stdin: &[u8], store: &Path) -> String {
    let lookup = |var: &str| (var == "ANNEAL_STORE").then(|| OsString::from(store));
    let env = HookEnv {
        stdin,
        current_dir: || Ok(PathBuf::from("/")),
        lookup,
    };
    let mut out = Vec::new();
    run(env, &mut out).expect("a Vec accepts every write");
    String::from_utf8(out).expect("UTF-8")
}

fn bash(command: &str) -> Vec<u8> {
    json!({
        "session_id": "s-guard",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command, "description": "test"},
    })
    .to_string()
    .into_bytes()
}

/// The `permissionDecisionReason` of a deny decision, after checking the decision's shape.
fn denied(stdout: &str) -> String {
    let output: Value = serde_json::from_str(stdout).expect("one JSON object");
    let reason = output
        .pointer("/hookSpecificOutput/permissionDecisionReason")
        .and_then(Value::as_str)
        .expect("a reason")
        .to_owned();
    assert_eq!(
        output,
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }}),
        "a PreToolUse deny decision and nothing else"
    );
    assert!(
        stdout.ends_with("}\n") && stdout.lines().count() == 1,
        "one line"
    );
    reason
}

#[test]
fn an_allowed_command_writes_nothing() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let stdout = guard(
        &bash("cargo test -p anneal-cli"),
        &dir.path().join("ledger.sqlite3"),
    );

    assert_eq!(stdout, "", "silent on pass");
}

#[test]
fn a_denied_command_gets_a_deny_decision_with_one_error_line() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let stdout = guard(
        &bash("git push -f origin main"),
        &dir.path().join("ledger.sqlite3"),
    );

    assert_eq!(
        denied(&stdout),
        "ERROR guard force-push: force push to protected branch main (main, master and \
         release/* are protected)",
        "the rule and the reason on one ERROR line"
    );
}

#[test]
fn a_denial_is_recorded_as_gate_blocked_with_the_session() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = dir.path().join("ledger.sqlite3");

    guard(&bash("rm -rf /"), &store);

    let events = Store::open(&store)
        .and_then(|ledger| {
            ledger.query(&Filter {
                types: vec![EventKind::GateBlocked],
                ..Filter::default()
            })
        })
        .expect("the ledger reads");
    let [recorded] = events.as_slice() else {
        panic!("one gate_blocked event, got {events:?}");
    };
    let LedgerEvent::GateBlocked(GateBlocked { point, reason }) = &recorded.event.event else {
        panic!("a gate_blocked event: {recorded:?}");
    };
    assert_eq!(point, "guard.rm-rf", "the point names the rule");
    assert!(reason.starts_with("rm -rf /: "), "the reason: {reason}");
    assert_eq!(recorded.event.actor, "hook", "written by the hook");
    assert_eq!(
        recorded.event.session_id.as_deref(),
        Some("s-guard"),
        "tagged with the session"
    );
}

#[test]
fn a_ledger_failure_still_denies_and_says_so() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    // A directory is not a ledger file, so the append fails.
    let stdout = guard(&bash("git add -f target"), dir.path());

    let reason = denied(&stdout);
    assert!(
        reason.starts_with("ERROR guard git-add-force: git add -f stages"),
        "still denied: {reason}"
    );
    assert!(
        reason.contains("(the block was not recorded in the ledger: "),
        "the ledger failure is noted: {reason}"
    );
}

#[test]
fn input_that_is_not_json_is_denied() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let stdout = guard(b"this is not json", &dir.path().join("ledger.sqlite3"));

    assert!(
        denied(&stdout).starts_with("ERROR guard bad-input: the hook input cannot be read: "),
        "fail closed: {stdout}"
    );
}

#[test]
fn input_over_the_cap_is_denied() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let huge = vec![b' '; usize::try_from(MAX_INPUT_BYTES).expect("fits") + 1];

    let stdout = guard(&huge, &dir.path().join("ledger.sqlite3"));

    assert!(
        denied(&stdout).contains("larger than 1048576 bytes"),
        "fail closed: {stdout}"
    );
}

#[test]
fn a_bash_input_without_a_command_is_denied() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let input = json!({"tool_name": "Bash", "tool_input": {}}).to_string();

    let stdout = guard(input.as_bytes(), &dir.path().join("ledger.sqlite3"));

    assert_eq!(
        denied(&stdout),
        "ERROR guard bad-input: the hook input cannot be read: the Bash input has no command",
        "fail closed"
    );
}

#[test]
fn an_unreadable_command_is_denied() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let stdout = guard(&bash("echo 'unclosed"), &dir.path().join("ledger.sqlite3"));

    assert_eq!(
        denied(&stdout),
        "ERROR guard unparseable: the command cannot be read: an unclosed '",
        "fail closed"
    );
}

#[test]
fn another_tool_is_not_judged() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let input = json!({"tool_name": "Write", "tool_input": {"file_path": "clippy.toml"}});

    let stdout = guard(
        input.to_string().as_bytes(),
        &dir.path().join("ledger.sqlite3"),
    );

    assert_eq!(stdout, "", "the guard judges Bash only");
}

#[test]
fn the_fallback_allows_what_the_denylist_leaves_undecided() {
    assert_eq!(
        Fallback::default().decide(),
        Decision::Allow,
        "the denylist's undecided commands are allowed unless enforced Clef D1 decides otherwise"
    );
}

#[test]
fn a_failed_read_of_stdin_is_denied() {
    struct Broken;
    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("pipe broke"))
        }
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = dir.path().join("ledger.sqlite3");
    let lookup = |var: &str| (var == "ANNEAL_STORE").then(|| OsString::from(&store));
    let env = HookEnv {
        stdin: Broken,
        current_dir: || Ok(PathBuf::from("/")),
        lookup,
    };
    let mut out = Vec::new();

    run(env, &mut out).expect("a Vec accepts every write");

    let stdout = String::from_utf8(out).expect("UTF-8");
    assert!(
        denied(&stdout).ends_with("reading stdin: pipe broke"),
        "an internal error denies: {stdout}"
    );
}
