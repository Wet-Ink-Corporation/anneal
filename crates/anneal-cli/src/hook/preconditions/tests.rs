//! `names_settings`: the names Windows reads as `.anneal/config.json`, and near misses;
//! `ToolInput::paths`: the files a gated call names; the hook's time budget and the worktree
//! cache.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::json;

use super::input::{Input, MAX_PATHS, Tool, ToolInput};
use super::worktrees::Worktrees;
use super::{Check, GIT_TIMEOUT, RecordError, judge, lexically_normalised, names_settings};
use crate::pr_diff::Budget;

#[test]
fn a_lexical_reading_takes_dotdot_over_the_names_before_it() {
    let rows = [
        ("/r/src/link/../x.py", "/r/src/x.py"),
        ("/r/./a/b/../../c", "/r/c"),
        ("/../r", "/r"),
    ];

    let read: Vec<(&str, PathBuf)> = rows
        .iter()
        .map(|(path, _)| (*path, lexically_normalised(Path::new(path))))
        .collect();

    let expected: Vec<(&str, PathBuf)> = rows
        .iter()
        .map(|(path, normal)| (*path, PathBuf::from(normal)))
        .collect();
    assert_eq!(
        read, expected,
        "`..` pops the name before it, never the root"
    );
}

#[test]
fn with_no_time_left_an_edit_is_denied_as_unjudged_and_not_recorded()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let input: Input = serde_json::from_value(json!({
        "cwd": dir.path(),
        "tool_name": "MultiEdit",
        "tool_input": {"file_path": dir.path().join("a.rs"), "edits": []},
    }))?;
    let spent = Budget::within(Instant::now(), Duration::ZERO, GIT_TIMEOUT);

    let verdict = judge(input, || Ok(dir.path().to_path_buf()), |_| None, spent);

    let denial = verdict.denial.ok_or("a spent budget denies")?;
    assert_eq!(denial.check, Check::Unjudged, "{denial:?}");
    assert!(
        denial
            .reason
            .starts_with("preconditions: no time is left to judge the edit"),
        "{denial:?}"
    );
    assert!(
        matches!(verdict.ledger, Err(RecordError::OutOfTime)),
        "no ledger is opened with no time left: {:?}",
        verdict.ledger.as_ref().err()
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git")]
fn files_in_two_directories_of_one_worktree_find_it_once() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()?;
    assert!(status.success(), "git init");
    std::fs::create_dir_all(dir.path().join("src"))?;
    std::fs::create_dir_all(dir.path().join("tests"))?;
    let lookup = |_: &str| None;
    let mut worktrees = Worktrees::new(&lookup, Budget::per_call(GIT_TIMEOUT));

    for file in ["src/a.rs", "tests/b.rs", "src/c.rs"] {
        let target = dir.path().join(file);
        let anchor = target.parent().ok_or("a parent")?;
        let verdict = worktrees.judge(anchor, &target, Ok(&target));
        assert!(verdict.is_none(), "no ledger: {file} is allowed");
    }

    assert_eq!(
        worktrees.anchors_asked(),
        2,
        "git is asked once per directory"
    );
    assert_eq!(
        worktrees.worktrees_opened(),
        1,
        "the ledger is looked for once"
    );
    Ok(())
}

#[test]
fn a_multi_edit_names_its_file_first_then_each_other_file_once() -> Result<(), serde_json::Error> {
    let input: ToolInput = serde_json::from_value(json!({
        "file_path": "/r/a.rs",
        "edits": [
            {"old_string": "x", "new_string": "y"},
            {"file_path": "/r/b.rs"},
            {"file_path": "/r/a.rs"},
            {"file_path": "/r/b.rs"},
        ],
    }))?;

    let paths = input.paths(Tool::MultiEdit, "MultiEdit");

    assert_eq!(
        paths,
        Ok(vec![PathBuf::from("/r/a.rs"), PathBuf::from("/r/b.rs")])
    );
    Ok(())
}

#[test]
fn a_notebook_edit_names_its_notebook_first_and_needs_it() -> Result<(), serde_json::Error> {
    let both: ToolInput = serde_json::from_value(json!({
        "file_path": "/r/a.rs",
        "notebook_path": "/r/n.ipynb",
    }))?;
    let without: ToolInput = serde_json::from_value(json!({"file_path": "/r/a.rs"}))?;

    let paths = both.paths(Tool::NotebookEdit, "NotebookEdit");
    let missing = without.paths(Tool::NotebookEdit, "NotebookEdit");

    assert_eq!(
        paths,
        Ok(vec![PathBuf::from("/r/n.ipynb"), PathBuf::from("/r/a.rs")])
    );
    assert!(
        matches!(&missing, Err(denial) if denial.check == Check::BadInput
            && denial.reason.ends_with("the NotebookEdit input has no tool_input.notebook_path")),
        "{missing:?}"
    );
    Ok(())
}

#[test]
fn a_call_naming_more_files_than_the_cap_is_bad_input() -> Result<(), serde_json::Error> {
    let edits: Vec<_> = (0..MAX_PATHS)
        .map(|index| json!({"file_path": format!("/r/{index}.rs")}))
        .collect();
    let at_cap: ToolInput =
        serde_json::from_value(json!({"file_path": "/r/0.rs", "edits": edits}))?;
    let over: ToolInput =
        serde_json::from_value(json!({"file_path": "/r/main.rs", "edits": edits}))?;

    let at_cap = at_cap.paths(Tool::MultiEdit, "MultiEdit");
    let over = over.paths(Tool::MultiEdit, "MultiEdit");

    assert_eq!(at_cap.map(|paths| paths.len()), Ok(MAX_PATHS));
    assert!(
        matches!(&over, Err(denial) if denial.check == Check::BadInput),
        "{over:?}"
    );
    Ok(())
}

#[test]
fn names_windows_reads_as_the_settings_file_are_the_settings_file() {
    let rows = [
        "w/.anneal/config.json",
        "w/.anneal/config.json.",
        "w/.anneal/config.json ",
        "w/.anneal/config.json. .",
        "w/.anneal/config.json::$DATA",
        "w/.anneal/config.json:stream",
        "w/.anneal./config.json",
        "w/.ANNEAL/Config.JSON",
    ];

    let missed: Vec<&str> = rows
        .into_iter()
        .filter(|path| !names_settings(Path::new(path)))
        .collect();

    assert_eq!(missed, Vec::<&str>::new(), "each names the settings file");
}

#[test]
fn near_misses_of_the_settings_file_are_not_it() {
    let rows = [
        "w/.anneal/config.json.bak",
        "w/.anneal/config.jsonx",
        "w/.anneal/xconfig.json",
        "w/anneal/config.json",
        "w/.anneal/briefs/config.json",
        "config.json",
    ];

    let matched: Vec<&str> = rows
        .into_iter()
        .filter(|path| names_settings(Path::new(path)))
        .collect();

    assert_eq!(matched, Vec::<&str>::new(), "none names the settings file");
}
