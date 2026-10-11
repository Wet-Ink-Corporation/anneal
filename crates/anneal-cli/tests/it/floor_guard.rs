//! `anneal floor-guard` through the built binary, on scratch git repositories: one fixture per
//! category of lowered floor, a clean diff, and the tool errors that must never read as 0.

use std::error::Error;
use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, git};

type TestResult = Result<(), Box<dyn Error>>;

/// A scratch repository on `main` whose first commit holds `files` (`(path, content)`).
fn repo(files: &[(&str, &str)]) -> Result<TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    git(dir.path(), &["init", "-q"]);
    // Byte-for-byte content on every platform, and no signing prompt on CI runners.
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    write(dir.path(), files)?;
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "base"]);
    Ok(dir)
}

fn write(root: &Path, files: &[(&str, &str)]) -> Result<(), Box<dyn Error>> {
    for (path, content) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn floor_guard(cwd: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(anneal_in(cwd).arg("floor-guard").args(args).output()?)
}

/// The envelope after checking the exit code and an empty stderr.
fn envelope(output: &Output, code: i32) -> Result<Value, Box<dyn Error>> {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stderr.is_empty(), "nothing on stderr: {output:?}");
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// `(category, file, line)` of each finding in `data.findings`.
fn findings(envelope: &Value) -> Vec<(Value, Value, Value)> {
    let field = |finding: &Value, key: &str| finding.get(key).cloned().unwrap_or(Value::Null);
    envelope
        .pointer("/data/findings")
        .and_then(Value::as_array)
        .map(|findings| {
            findings
                .iter()
                .map(|f| (field(f, "category"), field(f, "file"), field(f, "line")))
                .collect()
        })
        .unwrap_or_default()
}

/// Runs floor-guard in `dir` and expects exit 1 with exactly `expected`.
fn assert_lowered(dir: &Path, expected: (&str, &str, Value)) -> TestResult {
    let output = floor_guard(dir, &[])?;
    let envelope = envelope(&output, 1)?;
    assert_eq!(envelope.get("ok"), Some(&json!(false)), "{envelope}");
    let (category, file, line) = expected;
    assert_eq!(
        findings(&envelope),
        vec![(json!(category), json!(file), line)],
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_clean_diff_exits_0_with_no_findings() -> TestResult {
    let dir = repo(&[
        ("src/lib.rs", "fn a() {}\n"),
        ("pyproject.toml", "fail_under = 80\n"),
    ])?;
    write(
        dir.path(),
        &[
            ("src/lib.rs", "fn a() {}\nfn b() {}\n"),
            ("pyproject.toml", "fail_under = 85\n"),
        ],
    )?;

    let output = floor_guard(dir.path(), &[])?;

    let envelope = envelope(&output, 0)?;
    assert_eq!(
        (
            &envelope["ok"],
            &envelope["command"],
            &envelope["data"]["base"]
        ),
        (&json!(true), &json!("floor-guard"), &json!("main")),
        "{envelope}"
    );
    assert_eq!(findings(&envelope), Vec::new(), "{envelope}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn an_added_suppression_is_lowered() -> TestResult {
    let dir = repo(&[("src/lib.rs", "fn a() {}\n")])?;
    write(
        dir.path(),
        &[("src/lib.rs", "fn a() {}\n#[allow(dead_code)]\nfn b() {}\n")],
    )?;

    assert_lowered(dir.path(), ("suppression", "src/lib.rs", json!(2)))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_suppression_in_an_untracked_file_is_lowered() -> TestResult {
    let dir = repo(&[("README", "x\n")])?;
    write(dir.path(), &[("app/models.py", "import os  # noqa\n")])?;

    assert_lowered(dir.path(), ("suppression", "app/models.py", json!(1)))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_deleted_test_file_is_lowered() -> TestResult {
    let dir = repo(&[
        ("tests/it.rs", "#[test]\nfn works() {}\n"),
        ("src/lib.rs", "\n"),
    ])?;
    std::fs::remove_file(dir.path().join("tests/it.rs"))?;

    assert_lowered(dir.path(), ("deleted_test", "tests/it.rs", Value::Null))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn an_emptied_test_file_is_lowered() -> TestResult {
    let dir = repo(&[("web/a.spec.js", "const x = require('x')\nx()\n")])?;
    write(dir.path(), &[("web/a.spec.js", "")])?;

    assert_lowered(dir.path(), ("emptied_test", "web/a.spec.js", Value::Null))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_removed_test_function_is_lowered() -> TestResult {
    let dir = repo(&[(
        "src/lib.rs",
        "fn a() {}\n#[test]\nfn one() {}\n#[test]\nfn two() {}\n",
    )])?;
    write(
        dir.path(),
        &[("src/lib.rs", "fn a() {}\n#[test]\nfn one() {}\n")],
    )?;

    assert_lowered(dir.path(), ("removed_test", "src/lib.rs", json!(4)))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_test_removed_from_a_path_with_a_space_is_lowered() -> TestResult {
    // git writes `--- a/<path>\t` for a path with a space; the TAB must not stick to it.
    let dir = repo(&[(
        "my tests/test_y z.py",
        "def test_one():\n    pass\ndef test_two():\n    pass\n",
    )])?;
    write(
        dir.path(),
        &[("my tests/test_y z.py", "def test_one():\n    pass\n")],
    )?;

    assert_lowered(
        dir.path(),
        ("removed_test", "my tests/test_y z.py", json!(3)),
    )
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_test_file_emptied_at_a_path_with_a_space_is_lowered() -> TestResult {
    let dir = repo(&[("tests/my test.rs", "fn helper() {}\n")])?;
    write(dir.path(), &[("tests/my test.rs", "")])?;

    assert_lowered(
        dir.path(),
        ("emptied_test", "tests/my test.rs", Value::Null),
    )
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_test_file_renamed_out_of_a_test_path_is_a_deleted_test() -> TestResult {
    let dir = repo(&[("tests/test_x.py", "def test_x():\n    pass\n")])?;
    std::fs::create_dir_all(dir.path().join("old"))?;
    git(dir.path(), &["mv", "tests/test_x.py", "old/x_py.bak"]);

    assert_lowered(dir.path(), ("deleted_test", "tests/test_x.py", Value::Null))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_lowered_coverage_floor_is_lowered() -> TestResult {
    let dir = repo(&[(
        "pyproject.toml",
        "[tool.coverage.report]\nfail_under = 90\n",
    )])?;
    write(
        dir.path(),
        &[(
            "pyproject.toml",
            "[tool.coverage.report]\nfail_under = 60\n",
        )],
    )?;

    assert_lowered(dir.path(), ("coverage_lowered", "pyproject.toml", json!(2)))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_raised_timeout_is_lowered() -> TestResult {
    let dir = repo(&[("ci/ci.yml", "jobs:\n  test:\n    timeout-minutes: 10\n")])?;
    write(
        dir.path(),
        &[("ci/ci.yml", "jobs:\n  test:\n    timeout-minutes: 90\n")],
    )?;

    assert_lowered(dir.path(), ("timeout_raised", "ci/ci.yml", json!(3)))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_loosened_ratchet_value_is_lowered() -> TestResult {
    let dir = repo(&[("quality/ratchet.json", "{\n  \"max_warnings\": 3\n}\n")])?;
    write(
        dir.path(),
        &[("quality/ratchet.json", "{\n  \"max_warnings\": 7\n}\n")],
    )?;

    assert_lowered(
        dir.path(),
        ("ratchet_loosened", "quality/ratchet.json", json!(2)),
    )
}

/// `.anneal/ratchets.json` with a `warnings` ratchet (down) and a `coverage` ratchet (up).
fn ratchets(warnings: f64, coverage: f64) -> String {
    format!(
        "{{\n  \"version\": 1,\n  \"ratchets\": {{\n    \"warnings\": {{ \"value\": {warnings}, \
         \"direction\": \"down\" }},\n    \"coverage\": {{ \"value\": {coverage}, \"direction\": \
         \"up\" }}\n  }}\n}}\n"
    )
}

const RATCHETS: &str = ".anneal/ratchets.json";

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_raised_down_ratchet_is_lowered_at_its_line() -> TestResult {
    let dir = repo(&[(RATCHETS, &ratchets(10.0, 80.0))])?;
    write(dir.path(), &[(RATCHETS, &ratchets(12.0, 80.0))])?;

    assert_lowered(dir.path(), ("ratchet_loosened", RATCHETS, json!(4)))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_lowered_up_ratchet_is_lowered_with_a_message_naming_it() -> TestResult {
    let dir = repo(&[(RATCHETS, &ratchets(10.0, 80.0))])?;
    write(dir.path(), &[(RATCHETS, &ratchets(10.0, 79.5))])?;

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 1)?;

    assert_eq!(
        envelope.pointer("/data/findings/0/message"),
        Some(&json!(
            "lowers ratchet coverage from 80 to 79.5 (up: higher is better)"
        )),
        "{envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn tightened_ratchets_exit_0() -> TestResult {
    let dir = repo(&[(RATCHETS, &ratchets(10.0, 80.0))])?;
    write(dir.path(), &[(RATCHETS, &ratchets(8.0, 82.0))])?;

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 0)?;

    assert_eq!(findings(&envelope), Vec::new(), "{envelope}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_ratchet_file_that_no_longer_parses_is_lowered() -> TestResult {
    let dir = repo(&[(RATCHETS, &ratchets(10.0, 80.0))])?;
    write(dir.path(), &[(RATCHETS, "{\"version\": 1}\n")])?;

    assert_lowered(dir.path(), ("ratchet_loosened", RATCHETS, Value::Null))
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_deleted_ratchet_file_removes_every_ratchet() -> TestResult {
    let dir = repo(&[(RATCHETS, &ratchets(10.0, 80.0))])?;
    std::fs::remove_file(dir.path().join(RATCHETS))?;

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 1)?;

    assert_eq!(
        findings(&envelope),
        vec![
            (json!("ratchet_loosened"), json!(RATCHETS), Value::Null),
            (json!("ratchet_loosened"), json!(RATCHETS), Value::Null),
        ],
        "one finding per removed ratchet: {envelope}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_ratchet_file_renamed_away_removes_every_ratchet() -> TestResult {
    let dir = repo(&[(RATCHETS, &ratchets(10.0, 80.0))])?;
    git(dir.path(), &["mv", RATCHETS, ".anneal/old.json"]);

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 1)?;

    assert_eq!(
        findings(&envelope),
        vec![
            (json!("ratchet_loosened"), json!(RATCHETS), Value::Null),
            (json!("ratchet_loosened"), json!(RATCHETS), Value::Null),
        ],
        "a rename is read from its old path: one finding per base ratchet: {envelope}"
    );
    Ok(())
}

/// Asserts `envelope` (exit 0) has no finding and lists the ratchet file in `data.skipped`
/// with `reason`.
fn assert_ratchets_skipped(envelope: &Value, reason: &str) {
    assert_eq!(findings(envelope), Vec::new(), "{envelope}");
    assert_eq!(
        envelope.pointer("/data/skipped"),
        Some(&json!([{"file": RATCHETS, "reason": reason}])),
        "the base copy is listed, not dropped: {envelope}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn an_invalid_base_ratchet_file_is_skipped_not_judged() -> TestResult {
    let dir = repo(&[(RATCHETS, "{\"version\": 1, \"ratchets\": [\n")])?;
    write(dir.path(), &[(RATCHETS, &ratchets(10.0, 80.0))])?;

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 0)?;

    assert_ratchets_skipped(
        &envelope,
        "not a valid ratchet file at the merge base: no ratchet to compare",
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_base_ratchet_file_over_1_mib_is_skipped_not_judged() -> TestResult {
    let padded = format!("{}{}", ratchets(10.0, 80.0), " ".repeat(1024 * 1024));
    let dir = repo(&[(RATCHETS, &padded)])?;
    write(dir.path(), &[(RATCHETS, &ratchets(10.0, 80.0))])?;

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 0)?;

    assert_ratchets_skipped(
        &envelope,
        "over 1 MiB at the merge base: no ratchet to compare",
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_new_ratchet_file_is_not_judged() -> TestResult {
    let dir = repo(&[("src/lib.rs", "fn a() {}\n")])?;
    write(dir.path(), &[(RATCHETS, &ratchets(10.0, 80.0))])?;

    let envelope = envelope(&floor_guard(dir.path(), &[])?, 0)?;

    assert_eq!(findings(&envelope), Vec::new(), "{envelope}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn committed_changes_on_a_branch_are_diffed_against_the_merge_base() -> TestResult {
    let dir = repo(&[("src/lib.rs", "fn a() {}\n")])?;
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    write(
        dir.path(),
        &[("src/lib.rs", "#![allow(unused)]\nfn a() {}\n")],
    )?;
    git(dir.path(), &["commit", "-q", "-am", "suppress"]);

    let output = floor_guard(dir.path(), &["--base", "main"])?;

    let envelope = envelope(&output, 1)?;
    assert_eq!(
        findings(&envelope),
        vec![(json!("suppression"), json!("src/lib.rs"), json!(1))],
        "{envelope}"
    );
    Ok(())
}

/// Checks a tool error: exit 2, nothing on stdout, one `ERROR` line containing `needle`.
fn assert_tool_error(output: &Output, needle: &str) -> TestResult {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "no envelope claiming ok: {output:?}"
    );
    let stderr = String::from_utf8(output.stderr.clone())?;
    assert_eq!(stderr.lines().count(), 1, "one line: {stderr}");
    assert!(stderr.starts_with("ERROR floor-guard: "), "{stderr}");
    assert!(stderr.contains(needle), "{needle:?} in {stderr}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn outside_a_git_repository_exits_2() -> TestResult {
    let dir = tempfile::tempdir()?;
    let ceiling = dir.path().parent().ok_or("a parent")?;

    let output = anneal_in(dir.path())
        .arg("floor-guard")
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .output()?;

    assert_tool_error(&output, "not in a git repository")
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_base_that_does_not_resolve_exits_2() -> TestResult {
    let dir = repo(&[("a", "x\n")])?;

    let output = floor_guard(dir.path(), &["--base", "no-such-branch"])?;

    assert_tool_error(&output, "no-such-branch")
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn with_no_main_or_origin_main_and_no_base_it_exits_2() -> TestResult {
    let dir = repo(&[("a", "x\n")])?;
    git(dir.path(), &["checkout", "-q", "-b", "trunk"]);
    git(dir.path(), &["branch", "-q", "-D", "main"]);

    let output = floor_guard(dir.path(), &[])?;

    assert_tool_error(&output, "--base")
}
