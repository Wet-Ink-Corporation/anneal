//! `anneal snapshot write --json` through the built binary: the mod's snapshot is saved where
//! the context monitor reads it, bad input is a usage error, and the bootstrap's status-line
//! hint stays away while the mod saves usage.

use std::error::Error;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Output, Stdio};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// What the bootstrap's setup hint says.
const HINT: &str = "the context monitor is off";

/// Runs `anneal <args>` in `dir` with `stdin`, the anneal home under `dir`.
fn anneal(dir: &Path, args: &[&str], stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
    let mut child = anneal_in(dir)
        .args(args)
        .env("ANNEAL_HOME", dir.join("home"))
        .env("ANNEAL_STORE", dir.join("ledger.sqlite3"))
        .env_remove("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin is piped")?;
    match input.write_all(stdin) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {}
        written => written?,
    }
    drop(input);
    Ok(child.wait_with_output()?)
}

/// The context the bootstrap injects at a session start in `dir`.
fn session_start(dir: &Path) -> Result<String, Box<dyn Error>> {
    let input = json!({"session_id": "s2", "cwd": dir, "hook_event_name": "SessionStart"});
    let output = anneal(dir, &["hook", "bootstrap"], &serde_json::to_vec(&input)?)?;
    let json: Value = serde_json::from_slice(&output.stdout)?;
    Ok(json
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
        .ok_or("additionalContext")?
        .to_owned())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_written_snapshot_is_the_mod_s_and_the_monitor_warns_on_it() -> TestResult {
    let dir = tempfile::tempdir()?;
    let input = br#"{"session_id":"s1","used_tokens":130000,"window":200000}"#;

    let written = anneal(dir.path(), &["snapshot", "write", "--json"], input)?;
    let monitor = anneal(
        dir.path(),
        &["hook", "context-monitor"],
        br#"{"session_id":"s1","hook_event_name":"PostToolBatch"}"#,
    )?;

    assert_eq!(written.status.code(), Some(0), "written: {written:?}");
    let envelope: Value = serde_json::from_slice(&written.stdout)?;
    assert_eq!(
        envelope.pointer("/data/source"),
        Some(&json!("mod")),
        "the envelope says whose: {envelope}"
    );
    let saved = std::fs::read_to_string(dir.path().join("home").join("context-usage-mod.json"))?;
    assert!(
        saved.contains("\"source\": \"mod\""),
        "the mod's file: {saved}"
    );
    let warned: Value = serde_json::from_slice(&monitor.stdout)?;
    assert!(
        warned
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("65%")),
        "65% of the window warns: {warned}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn invalid_input_exits_2_with_one_error_line_and_nothing_on_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = anneal(
        dir.path(),
        &["snapshot", "write", "--json"],
        br#"{"session_id":"s1","window":0}"#,
    )?;

    assert_eq!(output.status.code(), Some(2), "a usage error: {output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    let err = String::from_utf8(output.stderr)?;
    assert_eq!(
        err, "ERROR snapshot write: window must be more than 0 tokens\n",
        "one ERROR line naming the field"
    );
    assert!(
        !dir.path()
            .join("home")
            .join("context-usage-mod.json")
            .exists(),
        "nothing saved"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_fresh_mod_snapshot_keeps_the_status_line_hint_away() -> TestResult {
    let (with_mod, without) = (tempfile::tempdir()?, tempfile::tempdir()?);
    let input = br#"{"session_id":"s1","used_percentage":10,"window":200000}"#;
    anneal(with_mod.path(), &["snapshot", "write", "--json"], input)?;

    let suppressed = session_start(with_mod.path())?;
    let hinted = session_start(without.path())?;

    assert!(
        !suppressed.contains(HINT),
        "the mod saves usage: {suppressed}"
    );
    assert!(hinted.contains(HINT), "without it the hint shows: {hinted}");
    Ok(())
}
