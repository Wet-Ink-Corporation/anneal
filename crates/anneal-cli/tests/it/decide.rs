//! `anneal decide` through the built binary. The binary has no HTTP client yet (a pending
//! dependency decision), so every configured call is unreachable: it must exit 5 well within
//! the 2 s Clef budget, log the row as unreachable, and never show the token.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

const SECRET: &str = "s3cr3t-T0KEN-value";
const ACCOUNT_VAR: &str = "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID";
const TOKEN_VAR: &str = "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN";
const MODE_VAR: &str = "CLAUDE_PLUGIN_OPTION_CLEF_MODE";

const D1: [&str; 7] = [
    "decide",
    "--point",
    "D1",
    "--question",
    "noul",
    "--state",
    r#"{"tool":"Bash","input":{"command":"rm -r build"},"phase":"build"}"#,
];

/// `anneal` in `cwd` with no plugin configuration or anneal home from the caller.
fn anneal(cwd: &Path) -> Command {
    let mut command = anneal_in(cwd);
    for var in [
        ACCOUNT_VAR,
        TOKEN_VAR,
        MODE_VAR,
        "ANNEAL_HOME",
        "CLAUDE_PLUGIN_ROOT",
        "CLAUDE_PLUGIN_DATA",
    ] {
        command.env_remove(var);
    }
    command
}

/// The plugin directory of this repository.
fn plugin_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin")
}

fn rows(log: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let text = std::fs::read_to_string(log)?;
    Ok(text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn unreachable_clef_exits_5_within_2_2_s_and_logs_the_row_as_unreachable() -> TestResult {
    let home = tempfile::tempdir()?;
    let started = Instant::now();

    let output = anneal(home.path())
        .args(D1)
        .env("ANNEAL_HOME", home.path())
        .env(ACCOUNT_VAR, "acct0123")
        .env(TOKEN_VAR, SECRET)
        .output()?;

    assert!(
        started.elapsed() < Duration::from_millis(2200),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    assert_eq!(envelope(&output)?["data"]["fallback"], json!("ask"));
    let rows = rows(&home.path().join("decisions").join("decisions.jsonl"))?;
    assert_eq!(rows.len(), 1, "one row");
    assert_eq!(rows[0]["unreachable"], json!(true));
    assert_eq!(rows[0]["error"], json!("unreachable"));
    assert_eq!(rows[0]["outcome"], Value::Null);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_token_is_never_on_stdout_stderr_or_in_the_log() -> TestResult {
    let home = tempfile::tempdir()?;

    let output = anneal(home.path())
        .args(D1)
        .env("ANNEAL_HOME", home.path())
        .env(ACCOUNT_VAR, "acct0123")
        .env(TOKEN_VAR, SECRET)
        .output()?;
    let refused = anneal(home.path())
        .args(D1)
        .env("ANNEAL_HOME", home.path())
        .env(ACCOUNT_VAR, "acct0123")
        .env(TOKEN_VAR, SECRET)
        .env(MODE_VAR, "loud")
        .output()?;

    for output in [&output, &refused] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(SECRET));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
    }
    assert_eq!(refused.status.code(), Some(70), "a bad mode: {refused:?}");
    let log = std::fs::read_to_string(home.path().join("decisions").join("decisions.jsonl"))?;
    assert!(!log.contains(SECRET), "{log}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn without_an_account_clef_is_off_exit_3_and_nothing_is_logged() -> TestResult {
    let home = tempfile::tempdir()?;

    let output = anneal(home.path())
        .args(D1)
        .env("ANNEAL_HOME", home.path())
        .output()?;

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert_eq!(envelope(&output)?["data"]["reason"], json!("no_account"));
    assert!(!home.path().join("decisions").exists(), "nothing logged");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_log_lives_in_the_anneal_home_not_in_the_plugins_directories() -> TestResult {
    let home = tempfile::tempdir()?;
    let data = tempfile::tempdir()?;

    let output = anneal(home.path())
        .args(D1)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CLAUDE_PLUGIN_ROOT", plugin_root())
        .env("CLAUDE_PLUGIN_DATA", data.path())
        .env(ACCOUNT_VAR, "acct0123")
        .env(TOKEN_VAR, SECRET)
        .output()?;

    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let log = home
        .path()
        .join(".anneal")
        .join("decisions")
        .join("decisions.jsonl");
    assert_eq!(rows(&log)?.len(), 1, "the row is under ~/.anneal");
    assert_eq!(
        std::fs::read_dir(data.path())?.count(),
        0,
        "plugin data untouched"
    );
    assert!(
        !plugin_root().join("decisions").exists(),
        "plugin root untouched"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn label_appends_an_outcome_row() -> TestResult {
    let home = tempfile::tempdir()?;

    let output = anneal(home.path())
        .args(["decide", "label", "d1760000000000-7-0", "destructive"])
        .env("ANNEAL_HOME", home.path())
        .output()?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let rows = rows(&home.path().join("decisions").join("decisions.jsonl"))?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["kind"], json!("label"));
    assert_eq!(rows[0]["id"], json!("d1760000000000-7-0"));
    assert_eq!(rows[0]["outcome"], json!("destructive"));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_state_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let home = tempfile::tempdir()?;

    let output = anneal(home.path())
        .args(["decide", "--point", "D1", "--question", "noul"])
        .env("ANNEAL_HOME", home.path())
        .output()?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr.lines().count(), 1, "one ERROR line: {stderr}");
    assert!(stderr.starts_with("ERROR "), "{stderr}");
    Ok(())
}
