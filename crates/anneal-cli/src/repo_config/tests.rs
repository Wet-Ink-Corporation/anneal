use super::{
    CONFIG_PATH, DEFAULT_LEADS, FileError, GateConfigError, LeadsError, MAX_LEAD_BYTES, MAX_LEADS,
    MAX_SENSITIVE_GLOBS, gate, gate_from_text, leads_from_text, orchestrate_leads,
};
use crate::glob::Glob;

#[test]
fn with_no_config_only_the_orchestrate_lead_may_spawn_from_a_subagent() {
    let root = tempfile::tempdir().expect("a temporary directory");

    let leads = orchestrate_leads(root.path()).expect("an absent file is the default");

    assert_eq!(leads.names(), ["orchestrate-lead"], "the default allowlist");
    assert_eq!(DEFAULT_LEADS, ["orchestrate-lead"], "documented default");
}

#[test]
fn orchestrate_leads_are_read_beside_other_keys_and_default_when_absent() {
    let configured = leads_from_text(
        r#"{"review": {"require_plugin": true},
            "orchestrate": {"leads": ["lead", "anneal:orchestrate-lead"]}}"#,
    );
    let absent = leads_from_text(r#"{"orchestrate": {}}"#);

    assert_eq!(
        (
            configured.expect("valid").names().to_vec(),
            absent.expect("valid").names().len()
        ),
        (
            vec!["lead".to_owned(), "anneal:orchestrate-lead".to_owned()],
            1
        ),
        "configured leads replace the default; no leads key keeps it"
    );
}

#[test]
fn leads_that_are_not_a_list_of_names_or_too_many_are_refused() {
    let not_a_list = leads_from_text(r#"{"orchestrate": {"leads": "lead"}}"#);
    let misspelt = leads_from_text(r#"{"orchestrate": {"lead": ["x"]}}"#);
    let names: Vec<String> = (0..=MAX_LEADS).map(|n| format!("\"l{n}\"")).collect();
    let too_many = leads_from_text(&format!(
        r#"{{"orchestrate": {{"leads": [{}]}}}}"#,
        names.join(",")
    ));

    assert!(
        matches!(not_a_list, Err(LeadsError::Json(_))),
        "{not_a_list:?}"
    );
    assert!(matches!(misspelt, Err(LeadsError::Json(_))), "{misspelt:?}");
    assert!(matches!(too_many, Err(LeadsError::TooMany)), "{too_many:?}");
}

#[test]
fn a_lead_that_is_empty_too_long_or_holds_a_control_character_is_refused_by_index() {
    let long = "l".repeat(MAX_LEAD_BYTES + 1);
    let leads = |name: &str| {
        leads_from_text(&serde_json::json!({"orchestrate": {"leads": ["lead", name]}}).to_string())
    };

    let refused = [
        leads(""),
        leads(&long),
        leads("lead\n"),
        leads("le\u{7f}ad"),
    ]
    .map(|result| result.err().map(|error| error.to_string()));

    let problem = |what: &str| {
        Some(format!(
            "{CONFIG_PATH}: orchestrate.leads[1] {what}; a lead is 1 to {MAX_LEAD_BYTES} bytes \
             of printable text"
        ))
    };
    assert_eq!(
        refused,
        [
            problem("is empty"),
            problem(&format!("is longer than {MAX_LEAD_BYTES} bytes")),
            problem("holds a control character"),
            problem("holds a control character"),
        ],
        "each refused, naming the entry"
    );
}

#[test]
fn a_lead_at_the_length_cap_and_an_empty_list_are_valid() {
    let longest = "l".repeat(MAX_LEAD_BYTES);
    let at_cap = leads_from_text(
        &serde_json::json!({"orchestrate": {"leads": [longest.as_str()]}}).to_string(),
    )
    .expect("a lead of exactly the cap");
    let none = leads_from_text(r#"{"orchestrate": {"leads": []}}"#).expect("an empty list");

    assert!(at_cap.contains(&longest), "the long lead is listed");
    assert!(
        !none.contains("orchestrate-lead"),
        "an empty list lets no subagent spawn, not even the default lead"
    );
    assert_eq!(none.to_string(), "", "and lists nobody");
}

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
