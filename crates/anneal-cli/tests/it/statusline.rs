//! `anneal statusline` through the built binary: one line, a saved reading, failing open.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::{Output, Stdio};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// Runs `anneal statusline` in `dir` with `stdin`, the anneal home under `dir`.
fn status_line(dir: &Path, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
    let mut child = anneal_in(dir)
        .arg("statusline")
        .env("ANNEAL_HOME", dir.join("home"))
        .env_remove("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("stdin is piped")?
        .write_all(stdin)?;
    Ok(child.wait_with_output()?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_status_line_prints_usage_and_saves_it_for_the_monitor() -> TestResult {
    let dir = tempfile::tempdir()?;
    let input = br#"{"session_id":"s1","context_window":{"used_percentage":30,"context_window_size":200000}}"#;

    let output = status_line(dir.path(), input)?;

    assert_eq!(output.status.code(), Some(0), "exit 0: {output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "anneal ctx 60k/200k 30%\n",
        "one line"
    );
    assert!(output.stderr.is_empty(), "nothing on stderr");
    let saved = std::fs::read_to_string(dir.path().join("home").join("context-usage.json"))?;
    assert!(
        saved.contains("\"used_tokens\": 60000.0"),
        "the reading is in the anneal home: {saved}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_unreadable_input_still_exits_0_with_a_line_and_one_error_line() -> TestResult {
    let dir = tempfile::tempdir()?;

    let output = status_line(dir.path(), b"not json")?;

    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "anneal ctx - (input unreadable)\n",
        "still a status line"
    );
    let err = String::from_utf8(output.stderr)?;
    assert!(
        err.starts_with("ERROR ") && err.lines().count() == 1,
        "one ERROR line: {err}"
    );
    Ok(())
}
