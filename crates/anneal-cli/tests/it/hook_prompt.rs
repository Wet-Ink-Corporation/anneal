//! `anneal hook route` and `anneal hook ruled-before`, the `UserPromptSubmit` hooks, through
//! the built binary. Its transport has no HTTP client, so Clef is always unreachable: the hooks
//! log the miss and stay silent, and with Clef off they touch nothing.

use std::error::Error;
use std::io::Write as _;
use std::path::Path;
use std::process::{Output, Stdio};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// The plugin options a hook reads. `anneal_in` removes them all, so the caller's environment
/// cannot leak in; a test sets the ones it needs.
const OPTIONS: [&str; 3] = [
    "CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID",
    "CLAUDE_PLUGIN_OPTION_CF_API_TOKEN",
    "CLAUDE_PLUGIN_OPTION_CLEF_MODE",
];

/// Runs `anneal hook <hook>` in `project` with the anneal home `home` (set explicitly, over
/// `anneal_in`'s own) and the account `account` (none: Clef off), on a prompt.
fn run_hook(
    hook: &str,
    project: &Path,
    home: &Path,
    account: Option<&str>,
) -> Result<Output, Box<dyn Error>> {
    let mut command = anneal_in(project);
    if let Some(account) = account {
        command
            .env(OPTIONS[0], account)
            .env(OPTIONS[1], "s3cr3t-T0KEN")
            .env(OPTIONS[2], "enforce");
    }
    let mut child = command
        .args(["hook", hook])
        .env("ANNEAL_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let input = json!({
        "session_id": "s1",
        "transcript_path": project.join("session.jsonl"),
        "cwd": project,
        "hook_event_name": "UserPromptSubmit",
        "prompt": "let us switch the licence to GPL",
    });
    child
        .stdin
        .take()
        .ok_or("stdin is piped")?
        .write_all(input.to_string().as_bytes())?;
    Ok(child.wait_with_output()?)
}

/// A project with one ruling, so ruled-before has something to ask about.
fn project() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join(".out-of-scope");
    std::fs::create_dir_all(&store)?;
    std::fs::write(
        store.join("licence.md"),
        "---\nid: licence\nruled_at: 2026-10-11\nsummary: MIT OR Apache-2.0.\n---\n",
    )?;
    Ok(dir)
}

fn rows(home: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let text = std::fs::read_to_string(home.join("decisions").join("decisions.jsonl"))?;
    Ok(text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Value>, _>>()?)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_clef_unreachable_both_hooks_exit_0_silently_and_log_the_miss() -> TestResult {
    for (hook, questions) in [("route", 2), ("ruled-before", 1)] {
        let project = project()?;
        let home = tempfile::tempdir()?;

        let output = run_hook(hook, project.path(), home.path(), Some("acct0123"))?;

        assert_eq!(output.status.code(), Some(0), "{hook}: {output:?}");
        assert!(
            output.stdout.is_empty(),
            "{hook}: no context, no note: {output:?}"
        );
        let rows = rows(home.path())?;
        assert_eq!(rows.len(), questions, "{hook}: {rows:?}");
        for row in rows {
            assert_eq!(row["unreachable"], json!(true), "{hook}");
            assert_eq!(row["action"], Value::Null, "{hook}");
            let latency = row["latency_ms"].as_u64().ok_or("a latency")?;
            assert!(
                latency < 2000,
                "{hook}: Clef cost {latency} ms, within its 2 s"
            );
        }
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_clef_off_both_hooks_exit_0_and_touch_nothing() -> TestResult {
    for hook in ["route", "ruled-before"] {
        let project = project()?;
        let home = tempfile::tempdir()?;

        let output = run_hook(hook, project.path(), home.path(), None)?;

        assert_eq!(output.status.code(), Some(0), "{hook}: {output:?}");
        assert!(output.stdout.is_empty(), "{hook}: {output:?}");
        assert!(output.stderr.is_empty(), "{hook}: {output:?}");
        assert_eq!(
            std::fs::read_dir(home.path())?.count(),
            0,
            "{hook}: nothing written to the anneal home"
        );
    }
    Ok(())
}
