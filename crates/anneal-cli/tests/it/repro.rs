//! `anneal repro record` and `anneal repro leftovers` through the built binary, in scratch git
//! repositories: only a command that exits non-zero records `repro_red`, tagged with the
//! worktree; one that exits 0, cannot start, or cannot be tagged records nothing; leftovers are
//! reported as `file:line`.
//!
//! The commands are `git` subcommands, which exist on every CI platform: `git --version` exits
//! 0 and `git definitely-not-a-command` exits 1.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

const RED: [&str; 2] = ["git", "definitely-not-a-command"];
const GREEN: [&str; 2] = ["git", "--version"];

/// A scratch repository with one commit.
fn repo() -> Result<TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "base"]);
    Ok(dir)
}

/// The ledger `repro record` writes to: the one in the git common dir.
fn ledger(root: &Path) -> PathBuf {
    root.join(".git").join("anneal").join("ledger.sqlite3")
}

/// Runs `anneal repro <args>` in `cwd`.
fn repro(cwd: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd).arg("repro").args(args).output()?)
}

/// `repro record [flags] -- <command>` in `cwd`.
fn record(cwd: &Path, flags: &[&str], command: &[&str]) -> Result<Output, Box<dyn Error>> {
    let mut args = vec!["record"];
    args.extend_from_slice(flags);
    args.push("--");
    args.extend_from_slice(command);
    repro(cwd, &args)
}

/// The `repro_red` events in the repository's ledger, through `anneal ledger query`.
fn repros(root: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let output = anneal_in(root)
        .args(["ledger", "query", "--type", "repro_red"])
        .output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let listing: Value = serde_json::from_slice(&output.stdout)?;
    let events = listing
        .pointer("/data/events")
        .and_then(Value::as_array)
        .ok_or("events")?;
    // The caller owns the events it inspects.
    Ok(events
        .iter()
        .filter_map(|recorded| recorded.get("event").cloned())
        .collect())
}

/// The worktree key `anneal mode` writes for the repository, read back from a real entry.
fn worktree_key(root: &Path) -> Result<Value, Box<dyn Error>> {
    let output = anneal_in(root).args(["mode", "diagnose"]).output()?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    Ok(envelope
        .pointer("/data/worktree")
        .cloned()
        .ok_or("mode reports its worktree")?)
}

/// Asserts a usage-style failure: `code`, nothing on stdout, one `ERROR` line on stderr, which
/// it returns.
fn one_error_line(output: &Output, code: i32) -> Result<String, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on stdout: {output:?}");
    let stderr = std::str::from_utf8(&output.stderr)?;
    let errors: Vec<&str> = stderr.lines().filter(|l| l.starts_with("ERROR ")).collect();
    assert_eq!(errors.len(), 1, "one ERROR line: {stderr}");
    Ok(errors.concat())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_failing_command_records_repro_red_tagged_with_the_worktree_and_exits_0() -> TestResult {
    let dir = repo()?;
    let key = worktree_key(dir.path())?;

    let output = record(dir.path(), &["--id", "bug-7", "--session", "s1"], &RED)?;

    assert_eq!(output.status.code(), Some(0), "red recorded: {output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        (
            &envelope["ok"],
            &envelope["command"],
            &envelope["data"]["recorded"],
            &envelope["data"]["outcome"],
            &envelope["data"]["exit"],
        ),
        (
            &json!(true),
            &json!("repro record"),
            &json!(true),
            &json!("red"),
            &json!(1)
        ),
        "{envelope}"
    );
    let events = repros(dir.path())?;
    let [event] = events.as_slice() else {
        return Err(format!("one repro_red: {events:?}").into());
    };
    assert_eq!(
        (
            &event["task"],
            &event["test"],
            &event["worktree"],
            &event["session_id"]
        ),
        (
            &json!("bug-7"),
            &json!("git definitely-not-a-command"),
            &key,
            &json!("s1")
        ),
        "{event}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_id_defaults_to_repro() -> TestResult {
    let dir = repo()?;

    let output = record(dir.path(), &[], &RED)?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let events = repros(dir.path())?;
    assert_eq!(
        events.iter().map(|e| &e["task"]).collect::<Vec<_>>(),
        vec![&json!("repro")],
        "{events:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_command_that_exits_0_records_nothing_exits_3_and_opens_no_ledger() -> TestResult {
    let dir = repo()?;

    let output = record(dir.path(), &[], &GREEN)?;

    assert_eq!(output.status.code(), Some(3), "not red: {output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        (
            &envelope["ok"],
            &envelope["data"]["recorded"],
            &envelope["data"]["outcome"],
            &envelope["findings"][0]["key"],
        ),
        (
            &json!(false),
            &json!(false),
            &json!("green"),
            &json!("repro/not-red:repro")
        ),
        "{envelope}"
    );
    assert!(!ledger(dir.path()).exists(), "no ledger was opened");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("git version"),
        "the command's output is on stderr: {stderr}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_command_that_cannot_start_exits_2_and_records_nothing() -> TestResult {
    let dir = repo()?;

    let output = record(dir.path(), &[], &["anneal-no-such-program-25"])?;

    let line = one_error_line(&output, 2)?;
    assert!(
        line.starts_with("ERROR repro: could not start anneal-no-such-program-25"),
        "{line}"
    );
    assert!(!ledger(dir.path()).exists(), "no ledger was opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn outside_a_git_worktree_record_is_a_usage_error_and_runs_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    // A `.git` file pointing nowhere stops git here, even if a repository encloses the temp dir.
    std::fs::write(dir.path().join(".git"), "gitdir: does-not-exist\n")?;

    let output = record(dir.path(), &[], &GREEN)?;

    let line = one_error_line(&output, 2)?;
    assert!(line.contains("git worktree"), "{line}");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(!stderr.contains("git version"), "nothing ran: {stderr}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn an_id_the_ledger_cannot_tag_is_a_usage_error_and_runs_nothing() -> TestResult {
    let dir = repo()?;
    let long_id = "b".repeat(300);

    let output = record(dir.path(), &["--id", &long_id], &GREEN)?;

    one_error_line(&output, 2)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(!stderr.contains("git version"), "nothing ran: {stderr}");
    assert!(!ledger(dir.path()).exists(), "no ledger was opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn record_takes_no_store_flag() -> TestResult {
    let dir = repo()?;

    let output = repro(
        dir.path(),
        &["record", "--store", "x.sqlite3", "--", "git", "--version"],
    )?;

    one_error_line(&output, 2)?;
    assert!(!dir.path().join("x.sqlite3").exists(), "no ledger there");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn leftovers_lists_each_tagged_line_as_file_and_line_and_exits_1() -> TestResult {
    let dir = repo()?;
    let tag = format!("[{}-x1]", "DEBUG");
    std::fs::create_dir_all(dir.path().join("src"))?;
    std::fs::write(
        dir.path().join("src/lib.rs"),
        format!("fn a() {{}}\n// {tag}\n"),
    )?;
    let sub = dir.path().join("src");

    let output = repro(&sub, &["leftovers"])?;

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        (&envelope["command"], &envelope["data"]["leftovers"]),
        (
            &json!("repro leftovers"),
            &json!([{"file": "src/lib.rs", "line": 2}])
        ),
        "the whole worktree is scanned from a subdirectory: {envelope}"
    );
    let message = envelope["findings"][0]["message"]
        .as_str()
        .ok_or("a finding")?;
    assert!(message.starts_with("src/lib.rs:2: "), "{message}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn leftovers_in_a_clean_worktree_exits_0() -> TestResult {
    let dir = repo()?;
    std::fs::write(dir.path().join("notes.md"), "tag lines with [DEBUG-<id>]\n")?;

    let output = repro(dir.path(), &["leftovers"])?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["data"]["leftovers"], json!([]), "{envelope}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn leftovers_past_1000_lines_counts_the_rest_and_still_exits_1() -> TestResult {
    let dir = repo()?;
    let lines: Vec<String> = (0..1005).map(|n| format!("[{}-n{n}]", "DEBUG")).collect();
    std::fs::write(dir.path().join("many.txt"), lines.join("\n"))?;

    let output = repro(dir.path(), &["leftovers"])?;

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    let findings = envelope["findings"].as_array().ok_or("findings")?;
    assert_eq!(
        (
            findings.len(),
            &envelope["data"]["omitted"],
            findings.last().map(|f| &f["key"])
        ),
        (1001, &json!(5), Some(&json!("repro/leftover-omitted"))),
        "1,000 listed and one finding for the rest"
    );
    Ok(())
}
