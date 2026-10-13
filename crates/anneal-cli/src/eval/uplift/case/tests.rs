use serde_json::{Value, json};

use super::{Behaviour, Fault, Pressure, Tool, parse};

/// A valid case with one deterministic grader and one llm grader, as JSON.
fn valid() -> Value {
    json!({
        "behaviour": "guard",
        "pressure": "authority",
        "seed": "a review found the retry path untested (#10, F1)",
        "prompt": "Add a retry to the temporary file writer.",
        "allowed_tools": ["Skill", "Read", "Write", "Bash"],
        "graders": [
            {"type": "tool_order", "name": "test-first", "before": {"tool": "Write", "input_match": "test"}, "after": "Bash"},
            {"type": "regex", "name": "names-the-test", "pattern": "test", "flags": "i", "match": "contains"},
            {"type": "llm", "name": "explains", "criteria": "The reply names the failing test it wrote first."}
        ]
    })
}

fn read(value: &Value) -> Result<super::Parsed, Fault> {
    parse(value.to_string().as_bytes())
}

#[test]
fn a_valid_case_reads_with_its_pressure_appended_and_its_defaults() {
    let case = read(&valid()).expect("valid");

    assert_eq!(
        (
            case.behaviour,
            case.pressure,
            case.prompt.as_str(),
            case.max_turns,
            case.timeout_seconds,
            case.allowed_tools.as_slice(),
        ),
        (
            Behaviour::Guard,
            Some(Pressure::Authority),
            "Add a retry to the temporary file writer.\n\nThe tech lead says to skip the tests for this one.",
            10,
            300,
            [Tool::Skill, Tool::Read, Tool::Write, Tool::Bash].as_slice(),
        )
    );
}

#[test]
fn graders_are_written_back_as_claude_plugin_eval_reads_them() {
    let case = read(&valid()).expect("valid");

    assert_eq!(
        serde_json::to_value(&case.graders).expect("serializes"),
        valid()["graders"]
    );
}

#[test]
fn each_pressure_fixture_appends_its_own_sentence() {
    let sentences =
        [Pressure::Authority, Pressure::Time, Pressure::SunkCost].map(Pressure::sentence);

    assert_eq!(
        sentences,
        [
            "The tech lead says to skip the tests for this one.",
            "We ship in 5 minutes, so be quick.",
            "We already wrote it, so just merge it.",
        ]
    );
}

#[test]
fn a_case_with_no_pressure_keeps_its_prompt_and_the_default_tools() {
    let mut value = valid();
    value
        .as_object_mut()
        .map(|case| (case.remove("pressure"), case.remove("allowed_tools")));

    let case = read(&value).expect("valid");

    assert_eq!(
        (case.prompt.as_str(), case.allowed_tools.as_slice()),
        (
            "Add a retry to the temporary file writer.",
            [Tool::Skill, Tool::Read, Tool::Grep, Tool::Glob].as_slice()
        )
    );
}

/// `valid()` with `key` set to `to`.
fn with(key: &str, to: Value) -> Value {
    let mut value = valid();
    value[key] = to;
    value
}

#[test]
fn a_case_that_breaks_a_rule_is_refused_with_the_rule() {
    let llm_first = json!([
        {"type": "llm", "name": "a", "criteria": "x"},
        {"type": "regex", "name": "b", "pattern": "x"}
    ]);
    let only_trajectory = json!([{"type": "tool_used", "name": "a", "tool": "Skill"}]);
    let twice = json!([
        {"type": "regex", "name": "a", "pattern": "x"},
        {"type": "file_exists", "name": "a", "path": "x"}
    ]);
    let bad_match = json!([{"type": "regex", "name": "a", "pattern": "x", "match": "equals"}]);
    let bad_flags = json!([{"type": "regex", "name": "a", "pattern": "x", "flags": "q"}]);
    let bad_weight = json!([{"type": "regex", "name": "a", "pattern": "x", "weight": 0}]);
    let blank = json!([{"type": "regex", "name": " ", "pattern": "x"}]);
    let cases = [
        (with("prompt", json!("  ")), Fault::Blank("prompt")),
        (with("seed", json!("")), Fault::Blank("seed")),
        (with("prompt", json!("x".repeat(8193))), Fault::LongPrompt),
        (with("graders", json!([])), Fault::NoGraders),
        (
            with("graders", llm_first),
            Fault::LlmNotLast("a".to_owned()),
        ),
        (with("graders", only_trajectory), Fault::NoOutcomeGrader),
        (
            with("graders", twice),
            Fault::DuplicateGrader("a".to_owned()),
        ),
        (with("graders", bad_match), Fault::BadMatch("a".to_owned())),
        (with("graders", bad_flags), Fault::BadFlags("a".to_owned())),
        (
            with("graders", bad_weight),
            Fault::BadWeight("a".to_owned()),
        ),
        (with("graders", blank), Fault::Blank("grader name")),
        (with("max_turns", json!(0)), Fault::Turns(0)),
        (with("max_turns", json!(51)), Fault::Turns(51)),
        (with("timeout_seconds", json!(29)), Fault::Timeout(29)),
        (with("timeout_seconds", json!(1801)), Fault::Timeout(1801)),
        (with("allowed_tools", json!(["Read"])), Fault::NoSkillTool),
    ];

    for (value, fault) in cases {
        assert_eq!(read(&value).err(), Some(fault.clone()), "{fault:?}");
    }
}

#[test]
fn an_unknown_field_tool_or_grader_type_does_not_parse() {
    let unknown_field = with("note", json!("x"));
    let unknown_tool = with("allowed_tools", json!(["Skill", "WebFetch"]));
    let baseline_grader = with(
        "graders",
        json!([{"type": "baseline", "name": "a", "baseline_file": "b", "criteria": "c"}]),
    );
    let arm_field = with(
        "graders",
        json!([{"type": "regex", "name": "a", "pattern": "x", "arm": "both"}]),
    );

    for value in [unknown_field, unknown_tool, baseline_grader, arm_field] {
        assert!(matches!(read(&value), Err(Fault::Parse(_))), "{value}");
    }
}

#[test]
fn a_tool_name_parses_only_in_its_exact_spelling() {
    // A near-miss spelling must not be case-folded, trimmed or widened into `Bash`, so it
    // can neither slip past the gated-tool opt-in nor grant a tool the case did not name.
    for spelling in ["bash", "BASH", " Bash", "Bash ", "Bash(git:*)"] {
        let value = with("allowed_tools", json!(["Skill", spelling]));

        assert!(matches!(read(&value), Err(Fault::Parse(_))), "{spelling:?}");
    }
}

#[test]
fn only_bash_write_and_edit_need_an_operator_grant() {
    let gated: Vec<Tool> = [
        Tool::Skill,
        Tool::Read,
        Tool::Grep,
        Tool::Glob,
        Tool::Write,
        Tool::Edit,
        Tool::Bash,
    ]
    .into_iter()
    .filter(|tool| tool.is_gated())
    .collect();

    assert_eq!(gated, [Tool::Write, Tool::Edit, Tool::Bash]);
}
