//! `anneal hook completeness`, the `Stop` hook, through the built binary on scratch git
//! repositories: a success claim needs a green `test_run` after the last source change, a
//! `[DEBUG-` leftover blocks, at most two blocks come in a row until a tool call, the active
//! playbook's missing events are context, and every failure is open.
//!
//! Source modification times are set to fixed instants (2001 and 2100), so whether a test run
//! is "after" an edit never depends on the clock's second.

use std::error::Error;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, UNIX_EPOCH};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

const SOURCE: &str = "src/lib.rs";
const SESSION: &str = "session-1";

/// 2001-09-09: before any test run these tests record.
const PAST: Duration = Duration::from_secs(1_000_000_000);
/// 2100-01-01: after any test run these tests record.
const FUTURE: Duration = Duration::from_hours(1_139_568);

/// A committed repository, an anneal home and a transcript, each in its own directory.
struct Session {
    repo: TempDir,
    home: TempDir,
    transcript: PathBuf,
}

impl Session {
    fn new() -> Result<Self, Box<dyn Error>> {
        let repo = tempfile::tempdir()?;
        git(repo.path(), &["init", "-q"]);
        git(repo.path(), &["config", "core.autocrlf", "false"]);
        std::fs::create_dir_all(repo.path().join("src"))?;
        std::fs::write(repo.path().join(SOURCE), "pub fn f() {}\n")?;
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "base"]);
        let home = tempfile::tempdir()?;
        let transcript = home.path().join("transcript.jsonl");
        std::fs::write(&transcript, b"{\"type\":\"user\"}\n")?;
        Ok(Self {
            repo,
            home,
            transcript,
        })
    }

    fn root(&self) -> &Path {
        self.repo.path()
    }

    /// Changes the source file and sets its modification time to `UNIX_EPOCH + at`.
    fn edit(&self, at: Duration) -> TestResult {
        let path = self.root().join(SOURCE);
        std::fs::write(&path, "pub fn f() -> u8 { 1 }\n")?;
        File::options()
            .write(true)
            .open(&path)?
            .set_modified(UNIX_EPOCH + at)?;
        Ok(())
    }

    /// `anneal task done T1 -- git <args>` in the repository: `--version` is green, an
    /// unknown subcommand red.
    fn task_done(&self, git_args: &[&str]) -> TestResult {
        let output = anneal_in(self.root())
            .args(["task", "done", "T1", "--", "git"])
            .args(git_args)
            .output()?;
        assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
        Ok(())
    }

    /// Appends a green `test_run` stamped `ts` that ran for `duration_ms`, tagged with this
    /// worktree, as `anneal ledger append` lets anyone do.
    fn green_run_at(&self, ts: &str, duration_ms: u64) -> TestResult {
        let off = anneal_in(self.root()).args(["mode", "off"]).output()?;
        let entered: Value = serde_json::from_slice(&off.stdout)?;
        let worktree = entered
            .pointer("/data/worktree")
            .cloned()
            .ok_or("mode reports its worktree")?;
        let run = json!({
            "type": "test_run",
            "command": "cargo test",
            "outcome": "green",
            "exit": 0,
            "duration_ms": duration_ms,
            "ts": ts,
            "worktree": worktree,
        });
        let appended = anneal_in(self.root())
            .args(["ledger", "append", &run.to_string()])
            .output()?;
        assert_eq!(appended.status.code(), Some(0), "{appended:?}");
        Ok(())
    }

    /// Appends a line holding a tool call to the transcript.
    fn tool_call(&self) -> TestResult {
        let mut file = File::options().append(true).open(&self.transcript)?;
        writeln!(
            file,
            r#"{{"type":"assistant","message":{{"id":"m9","content":[{{"type":"tool_use","id":"t","name":"Bash","input":{{}}}}]}}}}"#
        )?;
        Ok(())
    }

    fn input(&self, message: &str, active: bool) -> Value {
        json!({
            "session_id": SESSION,
            "transcript_path": self.transcript,
            "cwd": self.root(),
            "hook_event_name": "Stop",
            "stop_hook_active": active,
            "last_assistant_message": message,
        })
    }

    /// Runs the hook on `stdin` with this session's anneal home.
    fn run(&self, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
        hook(self.root(), self.home.path(), stdin)
    }

    /// Runs the hook as Claude Code does at the end of a reply, and returns its decision.
    fn stop(&self, message: &str) -> Result<Decision, Box<dyn Error>> {
        let output = self.run(&serde_json::to_vec(&self.input(message, false))?)?;
        Decision::of(&output)
    }
}

/// Runs `anneal hook completeness` in `cwd` with `ANNEAL_HOME` at `home`.
fn hook(cwd: &Path, home: &Path, stdin: &[u8]) -> Result<Output, Box<dyn Error>> {
    let mut child = anneal_in(cwd)
        .args(["hook", "completeness"])
        .env("ANNEAL_HOME", home)
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

/// What the hook answered.
#[derive(Debug)]
struct Decision {
    /// The `reason` of a `decision: block`.
    block: Option<String>,
    /// The Stop `additionalContext`.
    context: Option<String>,
    /// The `systemMessage` (an `ERROR` line).
    system: Option<String>,
}

impl Decision {
    fn of(output: &Output) -> Result<Self, Box<dyn Error>> {
        assert_eq!(output.status.code(), Some(0), "fails open: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        if output.stdout.is_empty() {
            return Ok(Self {
                block: None,
                context: None,
                system: None,
            });
        }
        let value: Value = serde_json::from_slice(&output.stdout)?;
        let text = |pointer: &str| {
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let block = match value.get("decision").and_then(Value::as_str) {
            Some("block") => Some(text("/reason").ok_or("a block has a reason")?),
            Some(other) => return Err(format!("unexpected decision {other}").into()),
            None => None,
        };
        if let Some(event) = text("/hookSpecificOutput/hookEventName") {
            assert_eq!(event, "Stop", "Stop output");
        }
        Ok(Self {
            block,
            context: text("/hookSpecificOutput/additionalContext"),
            system: text("/systemMessage"),
        })
    }

    fn is_silent(&self) -> bool {
        self.block.is_none() && self.context.is_none() && self.system.is_none()
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn all_tests_pass_without_a_fresh_green_run_blocks_and_a_green_run_after_the_edit_passes()
-> TestResult {
    let session = Session::new()?;
    session.edit(PAST)?;

    let before = session.stop("All tests pass.")?;

    let reason = before.block.ok_or("no test run at all: blocked")?;
    assert!(reason.contains("\"all tests pass\""), "{reason}");
    assert!(reason.contains(SOURCE), "names the changed file: {reason}");
    assert!(
        reason.contains("`anneal task done"),
        "says what to run: {reason}"
    );

    session.task_done(&["--version"])?;
    let after = session.stop("All tests pass.")?;

    assert!(after.is_silent(), "a green run after the edit: {after:?}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_green_run_before_the_last_edit_or_a_red_latest_run_still_blocks() -> TestResult {
    let session = Session::new()?;
    session.task_done(&["--version"])?;
    session.edit(FUTURE)?;

    assert!(
        session.stop("Done, it works now.")?.block.is_some(),
        "the run predates the edit"
    );

    session.edit(PAST)?;
    session.task_done(&["definitely-not-a-command"])?;
    assert!(
        session.stop("Done.")?.block.is_some(),
        "the latest run is red"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_claim_with_no_source_change_or_a_reply_without_a_claim_is_silent() -> TestResult {
    let session = Session::new()?;

    assert!(
        session.stop("All tests pass.")?.is_silent(),
        "nothing changed"
    );
    session.edit(PAST)?;
    assert!(
        session
            .stop("I changed src/lib.rs; the tests are not passing yet.")?
            .is_silent(),
        "no claim"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_third_consecutive_block_is_impossible_until_a_tool_call() -> TestResult {
    let session = Session::new()?;
    session.edit(PAST)?;

    let decisions: Vec<Decision> = (0..3)
        .map(|_| session.stop("All tests pass."))
        .collect::<Result<_, _>>()?;

    let blocked: Vec<bool> = decisions.iter().map(|d| d.block.is_some()).collect();
    assert_eq!(blocked, [true, true, false], "two blocks, then no third");
    let third = decisions.last().ok_or("three stops")?;
    let context = third.context.as_deref().ok_or("the gate becomes context")?;
    assert!(context.contains("\"all tests pass\""), "{context}");
    assert!(
        context.contains("twice in a row"),
        "says why it did not block: {context}"
    );

    session.tool_call()?;
    assert!(
        session.stop("All tests pass.")?.block.is_some(),
        "a tool call resets the count"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_inflected_claim_with_a_stale_edit_blocks() -> TestResult {
    let session = Session::new()?;
    session.edit(PAST)?;

    let reason = session
        .stop("All tests passed.")?
        .block
        .ok_or("no test run at all: blocked")?;

    assert!(reason.contains("\"all tests passed\""), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn an_edit_made_while_the_tests_ran_is_untested_and_one_before_they_started_is_tested() -> TestResult
{
    let session = Session::new()?;
    // Ran from 2020-01-01T00:00:00Z (1_577_836_800 s) to 00:10:00Z.
    session.green_run_at("2020-01-01T00:10:00Z", 600_000)?;

    // 00:05:01, while it ran.
    session.edit(Duration::from_secs(1_577_837_101))?;
    let during = session.stop("All tests pass.")?;
    // 2019-12-31T23:55:01, before it started.
    session.edit(Duration::from_secs(1_577_836_501))?;
    let before = session.stop("All tests pass.")?;

    assert!(during.block.is_some(), "edited mid-run: {during:?}");
    assert!(before.is_silent(), "edited before the run: {before:?}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_run_stamped_in_the_future_does_not_satisfy_a_claim() -> TestResult {
    let session = Session::new()?;
    session.edit(PAST)?;
    session.green_run_at("2099-01-01T00:00:00Z", 1_000)?;

    let decision = session.stop("All tests pass.")?;

    assert!(
        decision.block.is_some(),
        "a future stamp is not believed: {decision:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_file_the_leftover_scan_skipped_is_a_nudge() -> TestResult {
    let session = Session::new()?;
    let big = vec![b'a'; 1024 * 1024 + 1];
    std::fs::write(session.root().join("big.txt"), big)?;

    let decision = session.stop("Still looking.")?;

    assert_eq!(decision.block, None, "a skipped file is not a leftover");
    let context = decision.context.ok_or("a nudge")?;
    assert!(context.contains("skipped 1 file"), "{context}");
    assert!(context.contains("`anneal repro leftovers`"), "{context}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_green_run_in_a_sibling_worktree_does_not_satisfy_a_claim() -> TestResult {
    let session = Session::new()?;
    let parent = tempfile::tempdir()?;
    let sibling = parent.path().join("sibling");
    let sibling_arg = sibling.to_str().ok_or("a UTF-8 temporary path")?;
    git(session.root(), &["worktree", "add", "-q", sibling_arg]);
    session.edit(PAST)?;

    let run = anneal_in(&sibling)
        .args(["task", "done", "T1", "--", "git", "--version"])
        .output()?;
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    let decision = session.stop("All tests pass.")?;

    assert!(
        decision.block.is_some(),
        "only this worktree's runs count: {decision:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_block_count_that_cannot_be_read_turns_the_gate_into_context() -> TestResult {
    let session = Session::new()?;
    std::fs::create_dir(session.home.path().join("stop-blocks.json"))?;
    session.edit(PAST)?;

    let decision = session.stop("All tests pass.")?;

    assert_eq!(decision.block, None, "an uncounted block is never made");
    let context = decision.context.ok_or("the gate is still shown")?;
    assert!(context.contains("\"all tests pass\""), "{context}");
    assert!(context.contains("did not block"), "{context}");
    let system = decision.system.ok_or("the failure is an ERROR line")?;
    assert!(system.contains("not a regular file"), "{system}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_stop_already_continued_by_a_stop_hook_is_left_alone() -> TestResult {
    let session = Session::new()?;
    session.edit(PAST)?;

    let output = session.run(&serde_json::to_vec(
        &session.input("All tests pass.", true),
    )?)?;

    assert!(
        Decision::of(&output)?.is_silent(),
        "stop_hook_active: no output"
    );
    assert!(
        !session.home.path().join("stop-blocks.json").exists(),
        "nothing counted"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_debug_leftover_blocks_even_without_a_claim() -> TestResult {
    let session = Session::new()?;
    std::fs::write(
        session.root().join("probe.py"),
        format!("print('[{}-a1] x')\n", "DEBUG"),
    )?;

    let reason = session
        .stop("Still looking.")?
        .block
        .ok_or("a leftover blocks")?;

    assert!(reason.contains("probe.py:1"), "names the line: {reason}");
    assert!(reason.contains("`anneal repro leftovers`"), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_block_is_recorded_as_stop_blocked_in_an_existing_ledger() -> TestResult {
    let session = Session::new()?;
    let mode = anneal_in(session.root()).args(["mode", "off"]).output()?;
    assert_eq!(mode.status.code(), Some(0), "{mode:?}");
    session.edit(PAST)?;

    assert!(session.stop("Fixed.")?.block.is_some(), "blocked");

    let query = anneal_in(session.root())
        .args(["ledger", "query", "--type", "stop_blocked"])
        .output()?;
    let listing: Value = serde_json::from_slice(&query.stdout)?;
    let points: Vec<&str> = listing
        .pointer("/data/events")
        .and_then(Value::as_array)
        .ok_or("events")?
        .iter()
        .filter_map(|recorded| recorded.pointer("/event/point").and_then(Value::as_str))
        .collect();
    assert_eq!(points, ["completeness.claim"], "one event per hard gate");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn diagnose_mode_without_a_red_reproduction_is_a_nudge_not_a_block() -> TestResult {
    let session = Session::new()?;
    let mode = anneal_in(session.root())
        .args(["mode", "diagnose"])
        .output()?;
    assert_eq!(mode.status.code(), Some(0), "{mode:?}");

    let decision = session.stop("Here is what I found so far.")?;

    assert_eq!(decision.block, None, "a nudge never blocks");
    let context = decision.context.ok_or("a nudge")?;
    assert!(context.contains("`anneal repro record"), "{context}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn unreadable_input_fails_open_with_one_error_line() -> TestResult {
    let session = Session::new()?;

    let decision = Decision::of(&session.run(b"not json")?)?;

    assert_eq!((decision.block, decision.context), (None, None), "allows");
    let system = decision.system.ok_or("an ERROR line")?;
    assert!(system.starts_with("ERROR "), "{system}");
    assert_eq!(system.lines().count(), 1, "one line: {system}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn outside_a_git_repository_the_hook_is_silent() -> TestResult {
    let dir = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let input = json!({
        "session_id": SESSION,
        "transcript_path": dir.path().join("t.jsonl"),
        "cwd": dir.path(),
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": "All tests pass.",
    });

    let output = hook(dir.path(), home.path(), &serde_json::to_vec(&input)?)?;

    assert!(Decision::of(&output)?.is_silent(), "nothing to check");
    Ok(())
}
