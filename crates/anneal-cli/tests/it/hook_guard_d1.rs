//! Clef D1 in `anneal hook guard` through the built binary, whose only transport reports every
//! call unreachable: shadow mode logs and allows, enforce mode falls back to an ask.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Output, Stdio};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// A scratch directory holding the anneal home and the ledger.
struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            dir: tempfile::tempdir()?,
        })
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Counts 50 answered D1 decisions, so `enforce` is past its shadow period.
    fn past_the_shadow_period(&self) -> TestResult {
        let decisions = self.home().join("decisions");
        fs::create_dir_all(&decisions)?;
        fs::write(
            decisions.join("counts.json"),
            json!({"kind": "anneal-decision-counts", "version": 1, "answered": {"D1": 50}})
                .to_string(),
        )?;
        Ok(())
    }

    /// Runs `anneal hook guard --fail-closed` on a Bash call of `command`, Clef configured in
    /// `mode` with the shipped thresholds.
    fn guard(&self, mode: &str, command: &str) -> Result<Output, Box<dyn Error>> {
        let mut child = anneal_in(self.dir.path())
            .env("ANNEAL_STORE", self.dir.path().join("ledger.sqlite3"))
            .env("ANNEAL_HOME", self.home())
            .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
            .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", "s3cr3t-T0KEN-value")
            .env("CLAUDE_PLUGIN_OPTION_CLEF_MODE", mode)
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .args(["hook", "guard", "--fail-closed"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let input = json!({
            "session_id": "s-d1",
            "cwd": self.dir.path(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command},
        });
        let mut stdin = child.stdin.take().ok_or("stdin is piped")?;
        stdin.write_all(input.to_string().as_bytes())?;
        drop(stdin);
        Ok(child.wait_with_output()?)
    }

    /// The decision rows in the log.
    fn rows(&self) -> Result<Vec<Value>, Box<dyn Error>> {
        let log = fs::read_to_string(self.home().join("decisions").join("decisions.jsonl"))?;
        Ok(log
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?)
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn in_shadow_mode_an_unreachable_clef_is_logged_and_the_command_allowed() -> TestResult {
    let scratch = Scratch::new()?;

    let output = scratch.guard("shadow", " TOKEN=hunter2 rm -r build ")?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "silent, as without Clef: {output:?}"
    );
    let rows = scratch.rows()?;
    let [row] = rows.as_slice() else {
        return Err(format!("one row, got {rows:?}").into());
    };
    assert_eq!(row["point"], "D1");
    assert_eq!(row["subject"], "destructive_or_irreversible");
    assert_eq!(row["mode"], "shadow");
    assert_eq!(row["unreachable"], true);
    assert_eq!(row["input"], "TOKEN=<redacted> rm -r build");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn in_enforce_mode_an_unreachable_clef_falls_back_to_ask() -> TestResult {
    let scratch = Scratch::new()?;
    scratch.past_the_shadow_period()?;

    let output = scratch.guard("enforce", "rm -r build")?;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "nothing on stderr: {output:?}");
    let decision: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        decision.pointer("/hookSpecificOutput/permissionDecision"),
        Some(&Value::from("ask")),
        "{decision}"
    );
    let reason = decision
        .pointer("/hookSpecificOutput/permissionDecisionReason")
        .and_then(Value::as_str)
        .ok_or("a reason")?;
    assert!(reason.contains("Clef unreachable"), "{reason}");
    assert_eq!(scratch.rows()?.len(), 1, "the call is logged");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn in_enforce_mode_a_denylisted_command_is_still_denied_without_asking_clef() -> TestResult {
    let scratch = Scratch::new()?;
    scratch.past_the_shadow_period()?;

    let output = scratch.guard("enforce", "git push -f origin main")?;

    let decision: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        decision.pointer("/hookSpecificOutput/permissionDecision"),
        Some(&Value::from("deny")),
        "{decision}"
    );
    assert!(
        !scratch
            .home()
            .join("decisions")
            .join("decisions.jsonl")
            .exists(),
        "no row: Clef was not asked"
    );
    Ok(())
}
