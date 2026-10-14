//! `MultiEdit` and `NotebookEdit` are judged as Edit and Write are (#80): every path a call
//! names, from the field each tool writes through.

use std::path::Path;

use serde_json::{Value, json};

use super::{Repo, SOURCE, TestResult, assert_allowed, denial, hook_in};

/// The `PreToolUse` input for `tool` with `tool_input`, in `cwd`.
fn call(cwd: &Path, tool: &str, tool_input: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "session_id": "s-it",
        "transcript_path": "/dev/null",
        "cwd": cwd,
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": tool_input,
        "tool_use_id": "toolu_1",
    }))
}

/// A `MultiEdit` of `file` (absolute) with one edit.
fn multi_edit(file: &Path) -> Value {
    json!({
        "file_path": file,
        "edits": [{"old_string": "fn main", "new_string": "fn start"}],
    })
}

/// A `NotebookEdit` of `notebook` (absolute).
fn notebook_edit(notebook: &Path) -> Value {
    json!({
        "notebook_path": notebook,
        "new_source": "print(1)",
        "edit_mode": "replace",
    })
}

impl Repo {
    /// Runs the hook on a `tool` call with `tool_input`.
    fn call(
        &self,
        tool: &str,
        tool_input: &Value,
    ) -> Result<std::process::Output, Box<dyn std::error::Error>> {
        hook_in(self.path(), &call(self.path(), tool, tool_input)?)
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_multi_edit_of_a_source_file_is_denied_and_of_a_test_file_allowed() -> TestResult {
    let repo = Repo::diagnosing()?;

    let source = repo.call("MultiEdit", &multi_edit(&repo.path().join(SOURCE)))?;
    let test = repo.call("MultiEdit", &multi_edit(&repo.path().join("tests/cli.rs")))?;

    let reason = denial(&source)?;
    assert!(reason.starts_with("ERROR diagnose mode: "), "{reason}");
    assert_allowed(&test, "a MultiEdit of a test file");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_multi_edit_naming_a_source_file_in_any_edit_is_denied() -> TestResult {
    let repo = Repo::diagnosing()?;
    let input = json!({
        "file_path": repo.path().join("tests/cli.rs"),
        "edits": [
            {"old_string": "a", "new_string": "b"},
            {"file_path": repo.path().join(SOURCE), "old_string": "a", "new_string": "b"},
        ],
    });

    let reason = denial(&repo.call("MultiEdit", &input)?)?;

    assert!(reason.starts_with("ERROR diagnose mode: "), "{reason}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_notebook_edit_of_a_source_notebook_is_denied_and_of_a_test_notebook_allowed() -> TestResult {
    let repo = Repo::diagnosing()?;

    let source = repo.call(
        "NotebookEdit",
        &notebook_edit(&repo.path().join("src/analysis.ipynb")),
    )?;
    let test = repo.call(
        "NotebookEdit",
        &notebook_edit(&repo.path().join("tests/analysis.ipynb")),
    )?;

    let reason = denial(&source)?;
    assert!(reason.starts_with("ERROR diagnose mode: "), "{reason}");
    assert_allowed(&test, "a NotebookEdit of a test notebook");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_notebook_edit_without_a_notebook_path_is_denied_as_bad_input() -> TestResult {
    let repo = Repo::new()?;
    let input = json!({"file_path": repo.path().join("tests/analysis.ipynb")});

    let reason = denial(&repo.call("NotebookEdit", &input)?)?;

    assert!(
        reason.contains("tool_input.notebook_path"),
        "names the missing field: {reason}"
    );
    assert!(!repo.ledger().exists(), "no ledger created for the record");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn a_multi_edit_or_notebook_edit_of_the_anneal_config_is_denied_with_no_mode() -> TestResult {
    let repo = Repo::new()?;
    let config = repo.path().join(".anneal").join("config.json");

    let multi = repo.call("MultiEdit", &multi_edit(&config))?;
    let notebook = repo.call("NotebookEdit", &notebook_edit(&config))?;

    assert!(denial(&multi)?.contains(".anneal/config.json"), "MultiEdit");
    assert!(
        denial(&notebook)?.contains(".anneal/config.json"),
        "NotebookEdit"
    );
    Ok(())
}
