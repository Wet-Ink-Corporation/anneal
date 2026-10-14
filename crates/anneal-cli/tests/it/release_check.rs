//! Release 4's exit check through the built binary: in one anneal home, the guard (D1), route
//! (D3), ruled-before (D6) and `grill classify` (D7) each log a row, and `anneal calibrate`
//! reads all four points. `anneal_in` points Clef at a loopback port that refuses, so every row
//! is unreachable; the answered run, with a fake Clef, is `release_check` in the crate's unit
//! tests.

use std::error::Error;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn Error>>;

/// `anneal` in `project` with `home` as the anneal home, Clef configured in shadow and the
/// thresholds built in.
fn anneal(project: &Path, home: &Path) -> Command {
    let mut command = anneal_in(project);
    command
        .env("ANNEAL_HOME", home)
        .env("ANNEAL_STORE", project.join("ledger.sqlite3"))
        .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
        .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", "s3cr3t-T0KEN")
        .env("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "shadow")
        .env_remove("CLAUDE_PLUGIN_ROOT");
    command
}

/// Runs `anneal hook <hook>` with `input` on stdin.
fn hook(project: &Path, home: &Path, hook: &str, input: &Value) -> Result<Output, Box<dyn Error>> {
    let mut child = anneal(project, home)
        .args(["hook", hook])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("stdin is piped")?
        .write_all(input.to_string().as_bytes())?;
    Ok(child.wait_with_output()?)
}

/// A project with one out-of-scope ruling (for D6) and a design tree with one open question
/// (for D7).
fn project() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join(".out-of-scope");
    std::fs::create_dir_all(&store)?;
    std::fs::write(
        store.join("licence.md"),
        "---\nid: licence\nruled_at: 2026-10-11\nsummary: MIT OR Apache-2.0.\n---\n",
    )?;
    let tree = json!({
        "version": 1,
        "goal": "export orders as CSV",
        "turns": [],
        "nodes": [{
            "id": "library", "question": "Which CSV library does the repository use?",
            "depends_on": [], "status": "open", "answer": "the one in Cargo.toml",
            "is_decision": 0.8, "confidence": 0.5
        }],
    });
    std::fs::write(dir.path().join("tree.json"), tree.to_string())?;
    Ok(dir)
}

/// One session in `dir`: a Bash call through the guard, one prompt through route and
/// ruled-before, and a grill round's classify; returns each run's output in that order.
fn session(dir: &Path, home: &Path) -> Result<[Output; 4], Box<dyn Error>> {
    let bash = json!({
        "session_id": "s1", "cwd": dir, "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": "ls -la"},
    });
    let prompt = json!({
        "session_id": "s1", "transcript_path": dir.join("session.jsonl"), "cwd": dir,
        "hook_event_name": "UserPromptSubmit", "prompt": "let us switch the licence to GPL",
    });
    let classify = [
        "grill",
        "classify",
        "--tree",
        "tree.json",
        "--node",
        "library",
    ];
    Ok([
        hook(dir, home, "guard", &bash)?,
        hook(dir, home, "route", &prompt)?,
        hook(dir, home, "ruled-before", &prompt)?,
        anneal(dir, home).args(classify).output()?,
    ])
}

/// `point question unanswered` for each question of calibrate's `envelope` with an
/// unanswered decision, sorted.
fn unanswered(envelope: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    let field =
        |value: &Value, name: &str| value.get(name).map_or_else(String::new, Value::to_string);
    let points = envelope
        .pointer("/data/points")
        .and_then(Value::as_array)
        .ok_or("calibrate lists the points")?;
    let mut read = Vec::new();
    for point in points {
        let questions = point
            .get("questions")
            .and_then(Value::as_array)
            .ok_or("a point lists its questions")?;
        for question in questions {
            let count = field(question, "unanswered");
            if count != "0" {
                let (name, asked) = (field(point, "point"), field(question, "question"));
                read.push(format!("{name} {asked} {count}"));
            }
        }
    }
    read.sort();
    Ok(read)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn one_session_logs_d1_d3_d6_and_d7_and_calibrate_reads_all_four() -> TestResult {
    let project = project()?;
    let home = tempfile::tempdir()?;

    let ran = session(project.path(), home.path())?;
    let calibrated = anneal(project.path(), home.path())
        .arg("calibrate")
        .output()?;

    for output in &ran {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
    let [.., classify] = &ran;
    let classified: Value = serde_json::from_slice(&classify.stdout)?;
    assert_eq!(
        [&classified["data"]["clef"], &classified["data"]["written"]],
        [&json!("no_answer"), &json!(false)],
        "Clef unreachable: D7 falls back to the model's estimate"
    );
    assert_eq!(calibrated.status.code(), Some(0), "{calibrated:?}");
    assert_eq!(
        unanswered(&serde_json::from_slice(&calibrated.stdout)?)?,
        [
            r#""D1" "noul" 1"#,
            r#""D3" "choice" 1"#,
            r#""D3" "score" 1"#,
            r#""D6" "choice" 1"#,
            r#""D7" "noul" 1"#,
        ],
        "calibrate reads each point's (unreachable) decision"
    );
    Ok(())
}
