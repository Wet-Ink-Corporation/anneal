//! `anneal verify risk` through the built binary: D5's deterministic triggers fire with Clef
//! unreachable, and a change that cannot be read is exit 2 with one `ERROR` line.

use std::error::Error;
use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

/// A scratch repository on `main` with one commit, then `files` written uncommitted.
fn repo(files: &[(&str, &str)]) -> Result<TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.path().join("README.md"), "scratch\n")?;
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    for (path, content) in files {
        let path = dir.path().join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(dir)
}

/// Runs `anneal verify risk --base main` in `cwd` with Clef configured to enforce D5 at an
/// endpoint that refuses every connection (`anneal_in`'s `ANNEAL_CLEF_URL`), the decision log
/// in `home`.
fn risk_unreachable(cwd: &Path, home: &Path) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd)
        .args(["verify", "risk", "--base", "main"])
        .env("ANNEAL_HOME", home)
        .env("CLAUDE_PLUGIN_OPTION_CF_ACCOUNT_ID", "acct0123")
        .env("CLAUDE_PLUGIN_OPTION_CF_API_TOKEN", "s3cr3t-T0KEN")
        .env("CLAUDE_PLUGIN_OPTION_CLEF_MODE", "shadow,D5=enforce")
        .output()?)
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert!(output.stderr.is_empty(), "nothing on stderr: {output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn the_deterministic_triggers_fire_with_clef_unreachable() -> TestResult {
    let lines = "x\n".repeat(401);
    let cases = [
        (
            ("src/payments/charge.rs", "fn charge() {}\n"),
            json!([{"trigger": "path", "path": "src/payments/charge.rs",
                    "glob": "**/*payment*/**", "category": "payments"}]),
        ),
        (
            ("notes.txt", lines.as_str()),
            json!([{"trigger": "lines", "lines": 401, "limit": 400}]),
        ),
    ];
    for (file, expected) in cases {
        let dir = repo(&[file])?;
        let home = tempfile::tempdir()?;

        let output = risk_unreachable(dir.path(), home.path())?;

        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let envelope = envelope(&output)?;
        assert_eq!(envelope["data"]["doubt_review"], json!(true), "{envelope}");
        assert_eq!(envelope["data"]["triggers"], expected, "{envelope}");
        assert_eq!(
            envelope["data"]["clef"]["clef"],
            json!("no_answer"),
            "{envelope}"
        );
        assert!(
            !output.stdout.windows(12).any(|w| w == b"s3cr3t-T0KEN"),
            "the token never reaches stdout"
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn outside_a_repository_it_exits_2_with_one_error_line_and_nothing_on_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;

    let output = risk_unreachable(dir.path(), home.path())?;

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("ERROR verify risk: reading the change"),
        "{stderr}"
    );
    Ok(())
}
