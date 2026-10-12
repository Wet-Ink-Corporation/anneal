//! `anneal hook brief --fail-closed`, the `PreToolUse` gate on the Agent tool, through the
//! built binary: no brief, no spawn; and no spawn from inside a subagent unless it is an
//! orchestrate lead.

use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A brief with all nine fields filled.
const BRIEF: &str = "# Brief: 36\n\n\
    ## Goal\nShip it.\n\n## Repository\nhere\n\n## Owned scope\nsrc/\n\n\
    ## Do not touch\ndocs/\n\n## Invariants\nnone broken\n\n## Success criteria\ntests pass\n\n\
    ## Verification\ncargo test\n\n## Edits allowed\nyes\n\n## Return contract\na report\n";

/// A repository with `.anneal/briefs/36.md` (valid) and `bad.md` (only a goal).
fn repo() -> TestResult<TempDir> {
    let dir = tempfile::tempdir()?;
    git(dir.path(), &["init", "-q"]);
    let briefs = dir.path().join(".anneal").join("briefs");
    fs::create_dir_all(&briefs)?;
    fs::write(briefs.join("36.md"), BRIEF)?;
    fs::write(briefs.join("bad.md"), "## Goal\nShip it.\n")?;
    Ok(dir)
}

/// The `PreToolUse` input for an Agent call with `prompt`, from the main agent or, with
/// `inside`, from a subagent of that type.
fn agent_call(dir: &Path, prompt: &str, inside: Option<&str>) -> Value {
    let mut input = json!({
        "session_id": "s1",
        "transcript_path": dir.join("t.jsonl"),
        "cwd": dir,
        "hook_event_name": "PreToolUse",
        "tool_name": "Agent",
        "tool_input": {"prompt": prompt, "description": "fix", "subagent_type": "general-purpose"},
        "tool_use_id": "toolu_1",
    });
    if let (Some(agent_type), Some(object)) = (inside, input.as_object_mut()) {
        object.insert("agent_id".to_owned(), json!("agent-1"));
        object.insert("agent_type".to_owned(), json!(agent_type));
    }
    input
}

fn run(dir: &Path, stdin: &[u8]) -> TestResult<Output> {
    let mut child = anneal_in(dir)
        .args(["hook", "brief", "--fail-closed"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin is piped")?;
    input.write_all(stdin)?;
    drop(input);
    Ok(child.wait_with_output()?)
}

/// Runs the hook on `input`; `None` when it allowed silently, else the deny reason.
fn judge(dir: &Path, input: &Value) -> TestResult<Option<String>> {
    let output = run(dir, &serde_json::to_vec(input)?)?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    let field = |name| {
        decision
            .pointer(&format!("/hookSpecificOutput/{name}"))
            .and_then(Value::as_str)
    };
    assert_eq!(field("permissionDecision"), Some("deny"), "{decision}");
    let reason = field("permissionDecisionReason").ok_or("a reason")?;
    Ok(Some(reason.to_owned()))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_spawn_whose_prompt_names_a_valid_brief_is_allowed_silently() -> TestResult {
    let dir = repo()?;
    let prompt = "Fix the round counter.\nBrief: .anneal/briefs/36.md\n";

    assert_eq!(
        judge(dir.path(), &agent_call(dir.path(), prompt, None))?,
        None
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_spawn_without_a_brief_line_is_denied() -> TestResult {
    let dir = repo()?;

    let reason = judge(
        dir.path(),
        &agent_call(dir.path(), "Just fix it, please.", None),
    )?;

    let reason = reason.ok_or("denied")?;
    assert!(
        reason.starts_with("ERROR brief: ") && reason.contains("anneal brief build"),
        "names the command that writes one: {reason}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_spawn_whose_brief_fails_lint_brief_is_denied_naming_the_fields() -> TestResult {
    let dir = repo()?;

    let reason = judge(
        dir.path(),
        &agent_call(dir.path(), "Brief: .anneal/briefs/bad.md", None),
    )?;

    let reason = reason.ok_or("denied")?;
    assert!(
        reason.contains("Repository") && reason.contains("Return contract"),
        "{reason}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_brief_path_outside_the_briefs_directory_or_missing_is_denied() -> TestResult {
    let dir = repo()?;
    let absolute = dir.path().join(".anneal").join("briefs").join("36.md");
    let not_under = |path: &str| {
        format!(
            "ERROR brief: the brief {path:?} is not a relative path under .anneal/briefs/ (no \
             `..`, `.`, `\\` or absolute paths)"
        )
    };
    let cases = [
        ("../36.md".to_owned(), not_under("../36.md")),
        (
            ".anneal/briefs/../../README.md".to_owned(),
            not_under(".anneal/briefs/../../README.md"),
        ),
        (".anneal/notes.md".to_owned(), not_under(".anneal/notes.md")),
        (
            ".anneal/briefs/37.md".to_owned(),
            "ERROR brief: the brief .anneal/briefs/37.md does not exist; write it with \
             `anneal brief build`"
                .to_owned(),
        ),
        (
            absolute.display().to_string(),
            not_under(&absolute.display().to_string()),
        ),
    ];

    for (path, expected) in cases {
        let prompt = format!("Brief: {path}");
        let reason = judge(dir.path(), &agent_call(dir.path(), &prompt, None))?;
        assert_eq!(reason, Some(expected), "{path}");
    }
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_brief_that_is_a_symlink_out_of_the_worktree_is_denied() -> TestResult {
    let dir = repo()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("36.md"), BRIEF)?;
    std::os::unix::fs::symlink(
        outside.path().join("36.md"),
        dir.path().join(".anneal").join("briefs").join("link.md"),
    )?;

    let reason = judge(
        dir.path(),
        &agent_call(dir.path(), "Brief: .anneal/briefs/link.md", None),
    )?;

    assert_eq!(
        reason.as_deref(),
        Some(
            "ERROR brief: the brief .anneal/briefs/link.md resolves outside .anneal/briefs/ of \
             this worktree"
        ),
        "a valid brief, but outside the worktree"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_spawn_from_inside_a_subagent_is_denied_even_with_a_valid_brief() -> TestResult {
    let dir = repo()?;

    let reason = judge(
        dir.path(),
        &agent_call(dir.path(), "Brief: .anneal/briefs/36.md", Some("Explore")),
    )?;

    let reason = reason.ok_or("denied")?;
    assert!(
        reason.contains("Explore") && reason.contains("orchestrate.leads"),
        "names the agent and the allowlist: {reason}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_orchestrate_lead_may_spawn_with_a_valid_brief_and_the_allowlist_is_configurable()
-> TestResult {
    let dir = repo()?;
    let call = |agent| agent_call(dir.path(), "Brief: .anneal/briefs/36.md", Some(agent));
    let default_lead = judge(dir.path(), &call("orchestrate-lead"))?;
    fs::write(
        dir.path().join(".anneal").join("config.json"),
        r#"{"orchestrate": {"leads": ["anneal:lead"]}}"#,
    )?;

    let configured = judge(dir.path(), &call("anneal:lead"))?;
    let replaced = judge(dir.path(), &call("orchestrate-lead"))?;

    assert_eq!(
        (default_lead, configured, replaced.is_some()),
        (None, None, true),
        "the configured list replaces the default"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn other_tools_are_not_judged() -> TestResult {
    let dir = repo()?;
    let mut input = agent_call(dir.path(), "no brief", Some("Explore"));
    input["tool_name"] = json!("Bash");

    assert_eq!(judge(dir.path(), &input)?, None, "only the Agent tool");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn unreadable_input_is_denied() -> TestResult {
    let dir = repo()?;

    let output = run(dir.path(), b"not json")?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        decision["hookSpecificOutput"]["permissionDecision"],
        json!("deny"),
        "fails closed: {decision}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_denial_is_recorded_as_gate_blocked_in_an_existing_ledger_and_creates_none() -> TestResult {
    let dir = repo()?;
    let ledger = dir
        .path()
        .join(".git")
        .join("anneal")
        .join("ledger.sqlite3");
    judge(dir.path(), &agent_call(dir.path(), "no brief", None))?;
    let created = ledger.exists();
    let appended = anneal_in(dir.path())
        .args(["ledger", "append", r#"{"type":"task_done","task":"T1"}"#])
        .output()?;
    assert_eq!(appended.status.code(), Some(0), "{appended:?}");

    judge(dir.path(), &agent_call(dir.path(), "no brief", None))?;

    let blocked = anneal_in(dir.path())
        .args(["ledger", "query", "--type", "gate_blocked"])
        .output()?;
    let blocked: Value = serde_json::from_slice(&blocked.stdout)?;
    assert!(!created, "the first denial created no ledger");
    assert_eq!(
        blocked["data"]["events"][0]["event"]["point"],
        json!("brief.no-brief"),
        "{blocked}"
    );
    Ok(())
}
