use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::json;

use super::{ClaudeEvaluator, arguments, case_file, trials};
use crate::eval::routing::real::claude::Options;
use crate::eval::routing::real::result::parse;
use crate::eval::uplift::Trial;
use crate::eval::uplift::case::parse as parse_case;
use crate::eval::uplift::drive::{Arm, Batch};
use crate::eval::uplift::suites::{Case, SuiteKind};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn options() -> Options<'static> {
    Options {
        program: Path::new("claude"),
        model: Some("sonnet"),
        concurrency: 2,
        trust_plugin: true,
        time_limit: Duration::from_secs(60),
    }
}

fn case(tools: &serde_json::Value) -> Case {
    let text = json!({
        "behaviour": "guard",
        "pressure": "time",
        "seed": "s",
        "prompt": "Fix the flaky test.",
        "max_turns": 6,
        "allowed_tools": tools,
        "graders": [{"type": "regex", "name": "repro", "pattern": "red", "target": "trace"}]
    })
    .to_string();
    Case {
        skill: "diagnose".to_owned(),
        suite: SuiteKind::Regression,
        slug: "flaky".to_owned(),
        file: "diagnose/regression/flaky.json".to_owned(),
        body: parse_case(text.as_bytes()).expect("valid"),
    }
}

#[test]
fn a_batch_runs_its_cases_single_arm_with_the_gated_tools_they_name_granted() {
    let bash = case(&json!(["Skill", "Read", "Bash"]));
    let write = case(&json!(["Skill", "Write", "Bash"]));
    let batch = Batch {
        index: 2,
        arm: Arm::Without("diagnose"),
        cases: vec![&bash, &write],
        runs: 3,
        budget_usd: 7.129,
    };
    let root = Path::new("w");

    let args = arguments(&options(), root, &root.join("without-diagnose"), &batch);

    let path = |name: &str| root.join(name).into_os_string();
    let mut expected: Vec<OsString> = ["plugin", "eval"].map(OsString::from).to_vec();
    expected.extend([
        path("without-diagnose"),
        "--eval-dir".into(),
        "anneal-uplift-2".into(),
        "--json".into(),
        path("result-2.json"),
        "--output-dir".into(),
        path("out-2"),
        "--report".into(),
        path("report-2.html"),
    ]);
    expected.extend(
        [
            "--runs",
            "3",
            "--ablation",
            "none",
            "--threshold",
            "0",
            "--max-cost-usd",
            "7.12",
            "--allow-tools",
            "Write",
            "Bash",
            "--no-publish",
            "--no-scaffold",
            "--concurrency",
            "2",
            "--model",
            "sonnet",
            "--trust-plugin",
        ]
        .map(OsString::from),
    );
    assert_eq!(args, expected);
}

#[test]
fn a_batch_of_ungated_cases_grants_nothing() {
    let read_only = case(&json!(["Skill", "Read"]));
    let batch = Batch {
        index: 1,
        arm: Arm::With,
        cases: vec![&read_only],
        runs: 3,
        budget_usd: 25.0,
    };

    let args = arguments(&options(), Path::new("w"), Path::new("w/plugin"), &batch);

    assert!(!args.contains(&OsString::from("--allow-tools")), "{args:?}");
}

#[test]
fn a_case_is_written_as_claude_plugin_eval_reads_it() {
    let flaky = case(&json!(["Skill", "Bash"]));

    let value = serde_json::to_value(case_file(&flaky, 3)).expect("serializes");

    assert_eq!(
        value,
        json!({
            "schema_version": "1.0",
            "name": "diagnose--regression--flaky",
            "description": "anneal eval uplift (Tier 3): diagnose/regression/flaky.json",
            "tags": ["anneal-uplift"],
            "execution": {
                "prompt": "Fix the flaky test.\n\nWe ship in 5 minutes, so be quick.",
                "max_turns": 6,
                "timeout_seconds": 300,
                "allowed_tools": ["Skill", "Bash"]
            },
            "runs": 3,
            "graders": [{"type": "regex", "name": "repro", "target": "trace", "pattern": "red"}]
        })
    );
}

#[test]
fn a_run_out_of_turns_is_graded_and_another_error_is_no_trial() -> TestResult {
    let run = |error: Option<&str>, passed: bool| json!({"error": error, "graders": [{"name": "repro", "passed": passed}]});
    let result = parse(json!({
        "schemaVersion": 1,
        "costUsd": 0.3,
        "partial": false,
        "suite": {"plugins": [{"name": "anneal", "path": "/p"}]},
        "cases": [{"name": "diagnose--regression--flaky", "arms": {"with": [
            run(None, true),
            run(Some("exit 1: Reached maximum number of turns (6)"), false),
            run(Some("timed out after 300s"), true)
        ]}}]
    }))?;

    let ran = trials(result, "anneal")?;

    assert_eq!(
        ran.runs.get("diagnose--regression--flaky"),
        Some(&vec![
            Trial::Graded(vec![("repro".to_owned(), true)]),
            Trial::Graded(vec![("repro".to_owned(), false)]),
            Trial::Errored,
        ])
    );
    assert_eq!((ran.cost_usd, ran.stop), (0.3, None));
    Ok(())
}

#[test]
fn the_baseline_copy_for_a_skill_has_only_that_skill_removed() -> TestResult {
    let plugin = tempfile::tempdir()?;
    for skill in ["diagnose", "plan"] {
        fs::create_dir_all(plugin.path().join("skills").join(skill))?;
        fs::write(
            plugin.path().join("skills").join(skill).join("SKILL.md"),
            skill,
        )?;
    }
    fs::create_dir_all(plugin.path().join("hooks"))?;
    fs::write(plugin.path().join("hooks/hooks.json"), "{}")?;
    let mut evaluator = ClaudeEvaluator::new(options(), plugin.path(), "anneal", None)?;

    let without = evaluator.target(Arm::Without("diagnose"))?;

    let present = |relative: &str| without.join(relative).exists();
    let shape = (
        present("skills/diagnose"),
        present("skills/plan/SKILL.md"),
        present("hooks/hooks.json"),
        evaluator
            .target(Arm::With)?
            .join("skills/diagnose")
            .exists(),
    );
    evaluator.finish()?;
    assert_eq!(shape, (false, true, true, true));
    Ok(())
}

#[test]
fn a_baseline_copy_leaves_out_the_cases_earlier_batches_wrote() -> TestResult {
    let plugin = tempfile::tempdir()?;
    fs::create_dir_all(plugin.path().join("skills/plan"))?;
    let mut evaluator = ClaudeEvaluator::new(options(), plugin.path(), "anneal", None)?;
    let with = evaluator.target(Arm::With)?;
    fs::create_dir_all(with.join("anneal-uplift-1/plan--regression--a"))?;

    let without = evaluator.target(Arm::Without("plan"))?;

    let copied = without.join("anneal-uplift-1").exists();
    evaluator.finish()?;
    assert!(!copied, "batch 1's cases stay with the with-arm copy");
    Ok(())
}

#[test]
fn a_skill_name_that_is_not_one_path_component_is_refused() -> TestResult {
    let plugin = tempfile::tempdir()?;
    let mut evaluator = ClaudeEvaluator::new(options(), plugin.path(), "anneal", None)?;

    let refused = evaluator.target(Arm::Without("../x"));

    evaluator.finish()?;
    assert_eq!(
        refused.map_err(|error| error.code()),
        Err(crate::exit_codes::INTERNAL)
    );
    Ok(())
}
