//! `anneal hook fast-check` through the built binary, on scratch git repositories: one run per
//! `PostToolBatch` however many edits it holds, a lowered ratchet blocking, silence when every
//! check passes, and failing open (a note, never a block) for everything but a lowered floor.

use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

const RATCHETS: &str = ".anneal/ratchets.json";
const CONFIG: &str = ".anneal/config.json";

/// `.anneal/ratchets.json` with one `warnings` ratchet (lower is better).
fn ratchets(warnings: u32) -> String {
    format!(
        "{{\n  \"version\": 1,\n  \"ratchets\": {{\n    \"warnings\": {{ \"value\": {warnings}, \
         \"direction\": \"down\" }}\n  }}\n}}\n"
    )
}

/// A scratch repository on `main` with a source file, a glossary and a ratchet file committed.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Result<Self, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        git(dir.path(), &["init", "-q"]);
        // Byte-for-byte content on every platform, and no signing prompt on CI runners.
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        let repo = Self { dir };
        repo.write("src/lib.rs", "pub fn a() {}\n")?;
        repo.write(
            "CONTEXT.md",
            "# Glossary\n\n**Note**: a note.\n_Avoid_: memo.\n",
        )?;
        repo.write(RATCHETS, &ratchets(10))?;
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "base"]);
        Ok(repo)
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, content: &str) -> std::io::Result<()> {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    }

    /// Enters an anneal mode, as the build and diagnose skills do.
    fn enter_mode(&self) -> TestResult {
        let output = anneal_in(self.path()).args(["mode", "diagnose"]).output()?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        Ok(())
    }

    /// Sets the `check:fast` command.
    fn check_fast(&self, argv: &[&str]) -> TestResult {
        self.write(
            CONFIG,
            &serde_json::to_string(&json!({ "check_fast": argv }))?,
        )?;
        Ok(())
    }

    /// Runs the hook on a batch of `calls` (`(tool, file)`, file relative to the root).
    fn batch(&self, calls: &[(&str, &str)]) -> Result<Output, Box<dyn Error>> {
        let calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(n, (tool, file))| {
                json!({
                    "tool_name": tool,
                    "tool_input": {"file_path": self.path().join(file), "old_string": "a"},
                    "tool_use_id": format!("toolu_{n}"),
                    "tool_response": {"success": true},
                })
            })
            .collect();
        self.hook(&json!({
            "session_id": "s-it",
            "transcript_path": "/dev/null",
            "cwd": self.path(),
            "permission_mode": "default",
            "hook_event_name": "PostToolBatch",
            "tool_calls": calls,
        }))
    }

    /// Runs `anneal hook fast-check` in the repository with `input` on stdin.
    fn hook(&self, input: &Value) -> Result<Output, Box<dyn Error>> {
        let mut child = anneal_in(self.path())
            .args(["hook", "fast-check"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or("stdin is piped")?;
        stdin.write_all(&serde_json::to_vec(input)?)?;
        drop(stdin);
        Ok(child.wait_with_output()?)
    }
}

/// Asserts the hook exited 0 and wrote nothing at all.
fn assert_silent(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    assert!(output.stdout.is_empty(), "silent: {output:?}");
    assert!(output.stderr.is_empty(), "silent: {output:?}");
}

/// The hook's JSON output after checking it exited 0.
fn decision(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// The notes the hook added for the model.
fn context(decision: &Value) -> &str {
    decision
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The ledger the counting `check:fast` appends to, outside the repository.
fn counter(dir: &Path) -> PathBuf {
    dir.join("runs.sqlite3")
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn ten_parallel_edits_in_one_batch_run_check_fast_exactly_once() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    let runs = tempfile::tempdir()?;
    let store = counter(runs.path());
    let store = store.to_str().ok_or("a UTF-8 temp path")?;
    repo.check_fast(&[
        env!("CARGO_BIN_EXE_anneal"),
        "ledger",
        "append",
        "--store",
        store,
        r#"{"type":"task_done","task":"fast-check-run"}"#,
    ])?;
    let files: Vec<String> = (0..10).map(|n| format!("src/f{n}.rs")).collect();
    for file in &files {
        repo.write(file, "pub fn f() {}\n")?;
    }
    let calls: Vec<(&str, &str)> = files.iter().map(|file| ("Edit", file.as_str())).collect();

    let output = repo.batch(&calls)?;

    assert_silent(&output);
    let query = anneal_in(repo.path())
        .args(["ledger", "query", "--store", store])
        .output()?;
    let events: Value = serde_json::from_slice(&query.stdout)?;
    assert_eq!(
        events
            .pointer("/data/events")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "PostToolBatch fires once for the ten edits, and the hook runs check:fast once: {events}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_lowered_ratchet_blocks_with_one_error_line() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.write(RATCHETS, &ratchets(12))?;

    let decision = decision(&repo.batch(&[("Edit", RATCHETS)])?)?;

    assert_eq!(
        decision.get("decision"),
        Some(&json!("block")),
        "{decision}"
    );
    let reason = decision
        .get("reason")
        .and_then(Value::as_str)
        .ok_or("a reason")?;
    assert!(
        reason.starts_with("ERROR fast-check: ")
            && !reason.contains('\n')
            && reason.contains("raises ratchet warnings from 10 to 12"),
        "one ERROR line naming the ratchet: {reason}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_hook_is_silent_when_every_check_passes() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.check_fast(&["git", "--version"])?;
    repo.write(RATCHETS, &ratchets(9))?;
    repo.write("src/lib.rs", "pub fn a() {}\npub fn b() {}\n")?;

    assert_silent(&repo.batch(&[("Edit", RATCHETS), ("Write", "src/lib.rs")])?);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn with_no_mode_the_hook_is_silent_even_over_a_lowered_floor() -> TestResult {
    let repo = Repo::new()?;
    repo.write(RATCHETS, &ratchets(12))?;

    assert_silent(&repo.batch(&[("Edit", RATCHETS)])?);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_batch_without_an_edit_is_silent() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.write(RATCHETS, &ratchets(12))?;

    assert_silent(&repo.batch(&[("Read", RATCHETS), ("Grep", "src")])?);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_failing_check_fast_is_a_note_with_its_exit_and_output() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.check_fast(&["git", "rev-parse", "--verify", "no-such-ref-anywhere"])?;

    let decision = decision(&repo.batch(&[("Edit", "src/lib.rs")])?)?;

    assert_eq!(
        decision.get("decision"),
        None,
        "a note, not a block: {decision}"
    );
    assert_eq!(
        decision.pointer("/hookSpecificOutput/hookEventName"),
        Some(&json!("PostToolBatch")),
        "{decision}"
    );
    let notes = context(&decision);
    assert!(
        notes.contains("check_fast") && notes.contains("exit 128") && notes.contains("fatal"),
        "names the command, its exit and its output: {notes}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_check_fast_that_cannot_start_is_a_note() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.check_fast(&["anneal-no-such-program-xyz"])?;

    let decision = decision(&repo.batch(&[("Edit", "src/lib.rs")])?)?;

    assert_eq!(decision.get("decision"), None, "{decision}");
    assert!(
        context(&decision).contains("anneal-no-such-program-xyz"),
        "{decision}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_floor_guard_that_cannot_run_is_a_note_not_a_block() -> TestResult {
    let repo = Repo::new()?;
    // No `main` and no `origin/main`: floor-guard has no base and exits 2.
    git(repo.path(), &["branch", "-m", "trunk"]);
    repo.enter_mode()?;
    repo.write(RATCHETS, &ratchets(12))?;

    let decision = decision(&repo.batch(&[("Edit", RATCHETS)])?)?;

    assert_eq!(
        decision.get("decision"),
        None,
        "a tool error never blocks: {decision}"
    );
    assert!(
        context(&decision).contains("floor-guard"),
        "the note names floor-guard: {decision}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn an_avoided_term_in_a_changed_file_is_a_note() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.write("docs/notes.md", "Write a memo for each change.\n")?;

    let decision = decision(&repo.batch(&[("Write", "docs/notes.md")])?)?;

    assert_eq!(decision.get("decision"), None, "{decision}");
    let notes = context(&decision);
    assert!(
        notes.contains("docs/notes.md:1") && notes.contains("memo"),
        "names the file, line and term: {notes}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_changed_spec_or_plan_that_fails_its_lint_is_a_note() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.write("docs/specs/export.md", "# Export\n")?;
    repo.write("docs/plans/export.md", "# Export plan\n")?;

    let decision = decision(&repo.batch(&[
        ("Write", "docs/specs/export.md"),
        ("Write", "docs/plans/export.md"),
    ])?)?;

    assert_eq!(decision.get("decision"), None, "{decision}");
    let notes = context(&decision);
    assert!(
        notes.contains("anneal lint spec docs/specs/export.md")
            && notes.contains("anneal lint plan docs/plans/export.md"),
        "each lint names its file: {notes}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn input_that_lists_no_calls_takes_the_changed_files_from_git() -> TestResult {
    let repo = Repo::new()?;
    repo.enter_mode()?;
    repo.write("docs/notes.md", "Write a memo for each change.\n")?;

    let decision = decision(&repo.hook(&json!({
        "cwd": repo.path(),
        "hook_event_name": "PostToolBatch",
    }))?)?;

    assert!(
        context(&decision).contains("docs/notes.md:1"),
        "the untracked file was scanned: {decision}"
    );
    Ok(())
}
