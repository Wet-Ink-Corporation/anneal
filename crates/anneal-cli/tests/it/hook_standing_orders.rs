//! `anneal hook standing-orders`, the `SubagentStart` hook, through the built binary: the
//! repository's `.anneal/standing-orders.md` reaches every subagent's context, and the hook
//! never blocks.

use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const ORDERS: &str = "# Standing orders\n\n- Never push to main.\n- Report with a Status block.\n";

/// A repository, with `orders` as its standing orders when given, and a `src/` directory.
fn repo(orders: Option<&str>) -> TestResult<TempDir> {
    let dir = tempfile::tempdir()?;
    git(dir.path(), &["init", "-q"]);
    fs::create_dir_all(dir.path().join("src"))?;
    if let Some(orders) = orders {
        fs::create_dir_all(dir.path().join(".anneal"))?;
        fs::write(
            dir.path().join(".anneal").join("standing-orders.md"),
            orders,
        )?;
    }
    Ok(dir)
}

/// Runs the hook as Claude Code does when a subagent starts in `cwd`.
fn start(cwd: &Path) -> TestResult<Output> {
    let input = json!({
        "session_id": "s1",
        "transcript_path": cwd.join("t.jsonl"),
        "cwd": cwd,
        "hook_event_name": "SubagentStart",
        "agent_id": "agent-abc123",
        "agent_type": "Explore",
    });
    let mut child = anneal_in(cwd)
        .args(["hook", "standing-orders"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin is piped")?;
    stdin.write_all(&serde_json::to_vec(&input)?)?;
    drop(stdin);
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    Ok(output)
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_standing_orders_appear_in_the_subagent_context() -> TestResult {
    let dir = repo(Some(ORDERS))?;

    let output = start(&dir.path().join("src"))?;

    let written: Value = serde_json::from_slice(&output.stdout)?;
    let specific = &written["hookSpecificOutput"];
    assert_eq!(
        specific["hookEventName"],
        json!("SubagentStart"),
        "{written}"
    );
    let context = specific["additionalContext"]
        .as_str()
        .ok_or("additionalContext")?;
    assert!(
        context.contains("- Never push to main.\n- Report with a Status block.\n"),
        "the file, found from the worktree root: {context}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn without_standing_orders_the_hook_is_silent() -> TestResult {
    let dir = repo(None)?;

    let output = start(dir.path())?;

    assert!(output.stdout.is_empty(), "{output:?}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn standing_orders_over_16_kib_are_one_error_line_and_not_injected() -> TestResult {
    let dir = repo(Some(&"x".repeat(16 * 1024 + 1)))?;

    let output = start(dir.path())?;

    let written: Value = serde_json::from_slice(&output.stdout)?;
    let message = written["systemMessage"].as_str().ok_or("a systemMessage")?;
    assert!(
        message.starts_with("ERROR standing-orders: ") && message.contains("16384"),
        "{message}"
    );
    assert_eq!(
        written.get("hookSpecificOutput"),
        None,
        "nothing injected: {written}"
    );
    Ok(())
}
