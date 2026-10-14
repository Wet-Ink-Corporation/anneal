//! `names_settings`: the names Windows reads as `.anneal/config.json`, and near misses;
//! `ToolInput::paths`: the files a gated call names.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::input::{MAX_PATHS, Tool, ToolInput};
use super::{Check, names_settings};

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
