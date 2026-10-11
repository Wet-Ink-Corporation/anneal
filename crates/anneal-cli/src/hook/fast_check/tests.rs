use std::time::Duration;

use super::batch::{Batch, Input, porcelain};
use super::checks::grant;
use super::config::{ConfigError, check_fast};

fn input(json: &str) -> Input {
    serde_json::from_str(json).expect("the fixture is PostToolBatch-shaped JSON")
}

fn edit(tool: &str, key: &str, path: &str) -> String {
    format!(
        "{{\"tool_name\": \"{tool}\", \"tool_input\": {{\"{key}\": \"{path}\"}}, \
         \"tool_use_id\": \"t\", \"tool_response\": {{}}}}"
    )
}

#[test]
fn ten_parallel_edits_are_one_batch_of_their_distinct_files_in_order() {
    let calls: Vec<String> = (0..10)
        .map(|n| edit("Edit", "file_path", &format!("src/f{}.rs", n % 7)))
        .collect();
    let json = format!(
        "{{\"hook_event_name\": \"PostToolBatch\", \"cwd\": \"/w\", \"tool_calls\": [{}]}}",
        calls.join(", ")
    );

    let batch = input(&json).batch();

    let expected: Vec<String> = (0..7).map(|n| format!("src/f{n}.rs")).collect();
    assert_eq!(
        batch,
        Batch::Files {
            files: expected,
            omitted: 0
        },
        "each file once, first seen first"
    );
}

#[test]
fn every_file_editing_tool_counts_and_other_tools_do_not() {
    let json = format!(
        "{{\"tool_calls\": [{}, {}, {}, {}, {}, {}]}}",
        edit("Write", "file_path", "a.md"),
        edit("MultiEdit", "file_path", "b.rs"),
        edit("NotebookEdit", "notebook_path", "c.ipynb"),
        edit("Read", "file_path", "d.rs"),
        edit("Bash", "command", "touch e.rs"),
        "{\"tool_name\": \"Edit\", \"tool_input\": \"not an object\"}",
    );

    assert_eq!(
        input(&json).batch(),
        Batch::Files {
            files: vec!["a.md".to_owned(), "b.rs".to_owned(), "c.ipynb".to_owned()],
            omitted: 0
        }
    );
}

#[test]
fn a_batch_without_an_edit_is_no_edit_batch() {
    let json = format!(
        "{{\"tool_calls\": [{}, {}]}}",
        edit("Read", "file_path", "a.rs"),
        edit("Grep", "pattern", "x")
    );

    assert_eq!(input(&json).batch(), Batch::NoEdits);
    assert_eq!(
        input("{\"tool_calls\": []}").batch(),
        Batch::NoEdits,
        "an empty batch"
    );
}

#[test]
fn input_that_lists_no_tool_calls_leaves_the_files_to_git() {
    assert_eq!(input("{\"cwd\": \"/w\"}").batch(), Batch::Unlisted);
    assert_eq!(input("{}").batch(), Batch::Unlisted);
}

#[test]
fn files_past_the_cap_are_counted_not_listed() {
    let calls: Vec<String> = (0..=super::batch::MAX_FILES)
        .map(|n| edit("Edit", "file_path", &format!("f{n}.rs")))
        .collect();
    let json = format!("{{\"tool_calls\": [{}]}}", calls.join(", "));

    let counts = match input(&json).batch() {
        Batch::Files { files, omitted } => Some((files.len(), omitted)),
        Batch::NoEdits | Batch::Unlisted => None,
    };
    assert_eq!(
        counts,
        Some((super::batch::MAX_FILES, 1)),
        "capped and counted"
    );
}

#[test]
fn porcelain_status_lists_each_changed_and_untracked_path() {
    let status = b" M src/lib.rs\0?? new file.md\0D  gone.rs\0A  docs/specs/a.md\0";

    assert_eq!(
        porcelain(status),
        vec![
            "src/lib.rs".to_owned(),
            "new file.md".to_owned(),
            "gone.rs".to_owned(),
            "docs/specs/a.md".to_owned(),
        ]
    );
    assert_eq!(porcelain(b""), Vec::<String>::new(), "a clean tree");
}

#[test]
fn check_fast_is_the_argv_array_of_the_config() {
    assert_eq!(
        check_fast(r#"{"check_fast": ["cargo", "check", "--quiet"], "other": 1}"#).expect("valid"),
        Some(vec![
            "cargo".to_owned(),
            "check".to_owned(),
            "--quiet".to_owned()
        ]),
        "other keys are left to other readers"
    );
    assert_eq!(check_fast("{}").expect("valid"), None, "not configured");
}

#[test]
fn a_check_fast_that_is_not_a_non_empty_argv_array_is_an_error() {
    assert!(matches!(
        check_fast(r#"{"check_fast": []}"#),
        Err(ConfigError::EmptyCheckFast)
    ));
    assert!(matches!(
        check_fast(r#"{"check_fast": "cargo check"}"#),
        Err(ConfigError::Json(_))
    ));
    assert!(matches!(check_fast("[1]"), Err(ConfigError::Json(_))));
}

#[test]
fn a_markdown_file_under_specs_or_plans_gets_that_lint() {
    assert_eq!(super::lint_target("docs/specs/export.md"), Some("spec"));
    assert_eq!(super::lint_target("docs/plans/export.MD"), Some("plan"));
    assert_eq!(
        super::lint_target("specs/plans/x.md"),
        Some("plan"),
        "the nearest directory decides"
    );
    assert_eq!(
        super::lint_target("docs/specs/export.rs"),
        None,
        "not markdown"
    );
    assert_eq!(
        super::lint_target("specs.md"),
        None,
        "not under a directory"
    );
    assert_eq!(super::lint_target("docs/spec/x.md"), None, "plural only");
}

#[test]
fn each_check_gets_its_cap_or_what_is_left_of_the_budget() {
    let total = Duration::from_secs(13);
    let cap = Duration::from_secs(5);

    assert_eq!(grant(total, Duration::ZERO, cap), Some(cap), "its cap");
    assert_eq!(
        grant(total, Duration::from_secs(10), cap),
        Some(Duration::from_secs(3)),
        "what is left"
    );
    assert_eq!(
        grant(total, Duration::from_secs(13), cap),
        None,
        "nothing left: skipped"
    );
    assert_eq!(
        grant(total, Duration::from_millis(12_950), cap),
        None,
        "too little left to start a process"
    );
}
