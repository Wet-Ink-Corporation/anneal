use super::{CONFIG_PATH, FileError, GateConfigError, MAX_SENSITIVE_GLOBS, gate, gate_from_text};
use crate::glob::Glob;

#[test]
fn with_no_config_the_review_plugin_is_not_required_and_no_glob_is_added() {
    let root = tempfile::tempdir().expect("a temporary directory");

    let config = gate(root.path()).expect("an absent file is the defaults");

    assert_eq!(
        (config.require_plugin, config.sensitive_globs.len()),
        (false, 0),
        "the interim rule: a human approval counts until the switch is flipped"
    );
}

#[test]
fn review_require_plugin_and_ship_sensitive_globs_are_read_beside_other_keys() {
    let text = r#"{"check_fast": ["cargo", "check"],
        "review": {"require_plugin": true},
        "ship": {"sensitive_globs": ["**/billing/**", "ops/*.yml"]}}"#;

    let config = gate_from_text(text).expect("valid");

    let globs: Vec<&str> = config.sensitive_globs.iter().map(Glob::as_str).collect();
    assert_eq!(
        (config.require_plugin, globs),
        (true, vec!["**/billing/**", "ops/*.yml"]),
        "both settings read; check_fast belongs to fast-check"
    );
}

#[test]
fn an_unknown_key_inside_review_or_ship_is_refused_rather_than_ignored() {
    let typo = gate_from_text(r#"{"review": {"require_plugn": true}}"#);
    let ship = gate_from_text(r#"{"ship": {"globs": []}}"#);

    assert!(
        matches!(
            (&typo, &ship),
            (Err(GateConfigError::Json(_)), Err(GateConfigError::Json(_)))
        ),
        "a misspelt switch must not silently read as false: {typo:?} {ship:?}"
    );
}

#[test]
fn a_bad_glob_or_too_many_globs_is_refused() {
    let bad = gate_from_text(r#"{"ship": {"sensitive_globs": ["/abs/**"]}}"#);
    let many: Vec<String> = (0..=MAX_SENSITIVE_GLOBS)
        .map(|n| format!("d{n}/**"))
        .collect();
    let many = gate_from_text(&serde_json::json!({"ship": {"sensitive_globs": many}}).to_string());

    assert!(
        matches!(&bad, Err(GateConfigError::Glob { glob, .. }) if glob == "/abs/**"),
        "{bad:?}"
    );
    assert!(
        matches!(&many, Err(GateConfigError::TooManyGlobs)),
        "{many:?}"
    );
}

#[test]
fn a_config_path_that_is_not_a_regular_file_is_refused_unopened() {
    let root = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(root.path().join(CONFIG_PATH)).expect("a directory in its place");

    let result = gate(root.path());

    assert!(
        matches!(result, Err(GateConfigError::File(FileError::NotAFile))),
        "{result:?}"
    );
}
