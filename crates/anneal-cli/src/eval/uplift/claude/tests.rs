use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::json;

use super::{
    BaselinePlugin, ClaudeEvaluator, LauncherTarget, Placement, Plugins, arguments, case_file,
    scaffold, traces, trials, write_cases,
};
use crate::eval::real::claude::Options;
use crate::eval::real::result::{ResultError, parse};
use crate::eval::real::{RouterError, Stop};
use crate::eval::uplift::Trial;
use crate::eval::uplift::case::parse as parse_case;
use crate::eval::uplift::drive::{Arm, Batch, Evaluator};
use crate::eval::uplift::fixture::Fixture;
use crate::eval::uplift::suites::{Case, SuiteKind};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// An evaluator over the plugin at `dir`, named `anneal`, whose skills are in `skills_dir`.
fn evaluator<'a>(
    options: Options<'a>,
    dir: &'a Path,
    skills_dir: &'a str,
    baseline: Option<BaselinePlugin<'a>>,
) -> Result<ClaudeEvaluator<'a>, RouterError> {
    ClaudeEvaluator::new(
        options,
        Plugins {
            dir,
            skills_dir: OsStr::new(skills_dir),
            name: "anneal",
            baseline,
            anneal: None,
        },
        &std::env::temp_dir(),
    )
}

#[test]
fn every_plugin_copy_holds_the_binary_where_the_launcher_looks_first() -> TestResult {
    let plugin = tempfile::tempdir()?;
    let baseline = tempfile::tempdir()?;
    fs::create_dir_all(plugin.path().join("skills/diagnose"))?;
    let tools = tempfile::tempdir()?;
    let binary = tools.path().join("anneal");
    fs::write(&binary, "the running anneal")?;
    let target = LauncherTarget::of("linux", "x86_64").ok_or("a target")?;
    let mut evaluator = ClaudeEvaluator::new(
        options(),
        Plugins {
            dir: plugin.path(),
            skills_dir: OsStr::new("skills"),
            name: "anneal",
            baseline: Some(BaselinePlugin {
                dir: baseline.path(),
                name: "older",
            }),
            anneal: Some(Placement {
                binary: &binary,
                target,
            }),
        },
        &std::env::temp_dir(),
    )?;

    let placed = [Arm::With, Arm::Baseline, Arm::Without("diagnose")].map(|arm| {
        evaluator.target(arm).and_then(|copy| {
            fs::read_to_string(copy.join("libexec/x86_64-unknown-linux-musl/anneal"))
                .map_err(RouterError::Workspace)
        })
    });

    evaluator.finish()?;
    assert_eq!(
        placed.map(Result::ok),
        [(); 3].map(|()| Some("the running anneal".to_owned()))
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_skill_s_baseline_copy_inherits_an_executable_binary() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let plugin = tempfile::tempdir()?;
    fs::create_dir_all(plugin.path().join("skills/diagnose"))?;
    let tools = tempfile::tempdir()?;
    let binary = tools.path().join("anneal");
    fs::write(&binary, "the running anneal")?;
    let target = LauncherTarget::of("linux", "x86_64").ok_or("a target")?;
    let mut evaluator = ClaudeEvaluator::new(
        options(),
        Plugins {
            dir: plugin.path(),
            skills_dir: OsStr::new("skills"),
            name: "anneal",
            baseline: None,
            anneal: Some(Placement {
                binary: &binary,
                target,
            }),
        },
        &std::env::temp_dir(),
    )?;

    let mode = evaluator.target(Arm::Without("diagnose")).and_then(|copy| {
        fs::metadata(copy.join("libexec/x86_64-unknown-linux-musl/anneal"))
            .map(|meta| meta.permissions().mode() & 0o111)
            .map_err(RouterError::Workspace)
    });

    evaluator.finish()?;
    assert_eq!(mode.ok(), Some(0o111));
    Ok(())
}

#[test]
fn with_no_target_for_the_host_no_binary_is_placed() -> TestResult {
    let plugin = tempfile::tempdir()?;
    fs::create_dir_all(plugin.path().join("skills/diagnose"))?;
    let mut evaluator = evaluator(options(), plugin.path(), "skills", None)?;

    let copies = [Arm::With, Arm::Without("diagnose")].map(|arm| {
        evaluator
            .target(arm)
            .map(|copy| copy.join("libexec").exists())
    });

    evaluator.finish()?;
    assert_eq!(copies.map(Result::ok), [Some(false), Some(false)]);
    Ok(())
}

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
        fixture: None,
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
fn each_case_withholds_the_gated_tools_its_batch_grants_that_it_does_not_name() -> TestResult {
    let bash = Case {
        slug: "bash".to_owned(),
        ..case(&json!(["Skill", "Bash"]))
    };
    let write = Case {
        slug: "write".to_owned(),
        ..case(&json!(["Skill", "Write", "Edit"]))
    };
    let read = Case {
        slug: "read".to_owned(),
        ..case(&json!(["Skill", "Read"]))
    };
    let batch = Batch {
        index: 1,
        arm: Arm::With,
        cases: vec![&bash, &write, &read],
        runs: 3,
        budget_usd: 1.0,
    };
    let dir = tempfile::tempdir()?;

    write_cases(dir.path(), &batch)?;

    let withheld = |slug: &str| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let text = fs::read(
            dir.path()
                .join(format!("diagnose--regression--{slug}/case.yaml")),
        )?;
        let mut value: serde_json::Value = serde_json::from_slice(&text)?;
        Ok(value["execution"]["disallowed_tools"].take())
    };
    assert_eq!(
        [withheld("bash")?, withheld("write")?, withheld("read")?],
        [
            json!(["Write", "Edit"]),
            json!(["Bash"]),
            json!(["Write", "Edit", "Bash"])
        ]
    );
    Ok(())
}

#[test]
fn a_case_alone_in_its_batch_withholds_nothing() -> TestResult {
    let bash = case(&json!(["Skill", "Bash"]));
    let dir = tempfile::tempdir()?;

    write_cases(dir.path(), &batch(vec![&bash]))?;

    let text = fs::read(dir.path().join("diagnose--regression--flaky/case.yaml"))?;
    let value: serde_json::Value = serde_json::from_slice(&text)?;
    assert_eq!(value["execution"].get("disallowed_tools"), None);
    Ok(())
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

/// The `flaky` case seeded from the fixture at `/repo/evals/uplift/fixtures/crate`.
fn seeded() -> Case {
    Case {
        fixture: Some(Fixture::sample("crate")),
        ..case(&json!(["Skill", "Read"]))
    }
}

#[test]
fn a_case_with_a_fixture_is_written_at_schema_1_1_naming_its_scaffold_script() {
    let seeded = seeded();

    let value = serde_json::to_value(case_file(&seeded, 3)).expect("serializes");

    assert_eq!(
        value,
        json!({
            "schema_version": "1.1",
            "name": "diagnose--regression--flaky",
            "description": "anneal eval uplift (Tier 3): diagnose/regression/flaky.json",
            "tags": ["anneal-uplift"],
            "context": {"scaffold_script": "scaffold.sh"},
            "execution": {
                "prompt": "Fix the flaky test.\n\nWe ship in 5 minutes, so be quick.",
                "max_turns": 6,
                "timeout_seconds": 300,
                "allowed_tools": ["Skill", "Read"]
            },
            "runs": 3,
            "graders": [{"type": "regex", "name": "repro", "target": "trace", "pattern": "red"}]
        })
    );
}

#[test]
fn a_fixture_case_s_scaffold_script_is_written_beside_its_case_file() -> TestResult {
    let seeded = seeded();
    let plain = Case {
        slug: "plain".to_owned(),
        ..case(&json!(["Skill"]))
    };
    let batch = Batch {
        index: 1,
        arm: Arm::With,
        cases: vec![&seeded, &plain],
        runs: 3,
        budget_usd: 1.0,
    };
    let dir = tempfile::tempdir()?;

    write_cases(dir.path(), &batch)?;

    assert_eq!(
        (
            fs::read_to_string(dir.path().join("diagnose--regression--flaky/scaffold.sh"))?,
            dir.path()
                .join("diagnose--regression--plain/scaffold.sh")
                .exists(),
        ),
        (scaffold::script("/repo/evals/uplift/fixtures/crate"), false)
    );
    Ok(())
}

#[test]
fn a_batch_with_a_fixture_case_runs_scaffolds_and_one_without_does_not() {
    let seeded = seeded();
    let plain = case(&json!(["Skill"]));

    let mixed = scaffold_flags(&batch(vec![&plain, &seeded]));
    let unseeded = scaffold_flags(&batch(vec![&plain]));

    assert_eq!((mixed, unseeded), ([true, false], [false, true]));
}

/// The first batch, against the whole plugin, of `cases`.
fn batch(cases: Vec<&Case>) -> Batch<'_> {
    Batch {
        index: 1,
        arm: Arm::With,
        cases,
        runs: 3,
        budget_usd: 1.0,
    }
}

/// Whether `batch`'s arguments hold `--scaffold` and `--no-scaffold`.
fn scaffold_flags(batch: &Batch<'_>) -> [bool; 2] {
    let args = arguments(&options(), Path::new("w"), Path::new("w/plugin"), batch);
    ["--scaffold", "--no-scaffold"].map(|flag| args.contains(&OsString::from(flag)))
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

    let flaky = case(&json!(["Skill"]));

    let ran = trials(result, "anneal", &[&flaky])?;

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

/// A result of `cases`, each `(name, grader names)` with one passing run per grader set.
fn result_of(cases: &[(&str, &[&str])]) -> serde_json::Value {
    let cases: Vec<serde_json::Value> = cases
        .iter()
        .map(|(name, graders)| {
            let graders: Vec<serde_json::Value> = graders
                .iter()
                .map(|grader| json!({"name": grader, "passed": true}))
                .collect();
            json!({"name": name, "arms": {"with": [{"error": null, "graders": graders}]}})
        })
        .collect();
    json!({
        "schemaVersion": 1,
        "costUsd": 0.3,
        "partial": false,
        "suite": {"plugins": [{"name": "anneal", "path": "/p"}]},
        "cases": cases
    })
}

#[test]
fn a_result_run_naming_a_grader_the_case_does_not_have_is_refused() -> TestResult {
    let flaky = case(&json!(["Skill"]));
    let result = parse(result_of(&[(
        "diagnose--regression--flaky",
        &["repro", "invented"],
    )]))?;

    let ran = trials(result, "anneal", &[&flaky])?;

    assert_eq!(
        (ran.runs.len(), ran.cost_usd),
        (0, 0.3),
        "no run kept, the cost counted"
    );
    let refused = ran.refused;
    assert!(
        matches!(
            &refused,
            Some(ResultError::UnknownGrader { case, grader })
                if case == "diagnose--regression--flaky" && grader == "invented"
        ),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_result_run_listing_a_grader_twice_is_refused() -> TestResult {
    let flaky = case(&json!(["Skill"]));
    let result = parse(json!({
        "schemaVersion": 1,
        "costUsd": 0.3,
        "partial": false,
        "suite": {"plugins": [{"name": "anneal", "path": "/p"}]},
        "cases": [{"name": "diagnose--regression--flaky", "arms": {"with": [{
            "error": null,
            "graders": [{"name": "repro", "passed": false}, {"name": "repro", "passed": true}]
        }]}}]
    }))?;

    let ran = trials(result, "anneal", &[&flaky])?;

    assert_eq!(
        (ran.runs.len(), ran.cost_usd),
        (0, 0.3),
        "no run kept, the cost counted"
    );
    let refused = ran.refused;
    assert!(
        matches!(
            &refused,
            Some(ResultError::DuplicateGrader { case, grader })
                if case == "diagnose--regression--flaky" && grader == "repro"
        ),
        "a failed and a passed `repro` would count as passed: {refused:?}"
    );
    Ok(())
}

#[test]
fn a_result_listing_a_case_twice_is_refused() -> TestResult {
    let flaky = case(&json!(["Skill"]));
    let result = parse(result_of(&[
        ("diagnose--regression--flaky", &["repro"]),
        ("diagnose--regression--flaky", &["repro"]),
    ]))?;

    let ran = trials(result, "anneal", &[&flaky])?;

    assert_eq!(
        (ran.runs.len(), ran.cost_usd),
        (0, 0.3),
        "no run kept, the cost counted"
    );
    let refused = ran.refused;
    assert!(
        matches!(
            &refused,
            Some(ResultError::DuplicateCase { case }) if case == "diagnose--regression--flaky"
        ),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_result_listing_a_case_the_batch_did_not_ask_for_twice_is_refused() -> TestResult {
    let flaky = case(&json!(["Skill"]));
    let result = parse(result_of(&[
        ("plan--regression--other", &["anything"]),
        ("diagnose--regression--flaky", &["repro"]),
        ("plan--regression--other", &["anything"]),
    ]))?;

    let ran = trials(result, "anneal", &[&flaky])?;

    assert_eq!(
        (ran.runs.len(), ran.cost_usd),
        (0, 0.3),
        "no run kept, the cost counted"
    );
    let refused = ran.refused;
    assert!(
        matches!(
            &refused,
            Some(ResultError::DuplicateCase { case }) if case == "plan--regression--other"
        ),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_result_case_the_batch_did_not_ask_for_is_dropped() -> TestResult {
    let flaky = case(&json!(["Skill"]));
    let result = parse(result_of(&[
        ("diagnose--regression--flaky", &["repro"]),
        ("plan--regression--other", &["anything"]),
    ]))?;

    let ran = trials(result, "anneal", &[&flaky])?;

    assert_eq!(
        ran.runs.keys().collect::<Vec<_>>(),
        ["diagnose--regression--flaky"]
    );
    Ok(())
}

#[test]
fn with_no_time_left_a_batch_launches_nothing_and_its_spend_is_exact() -> TestResult {
    let plugin = tempfile::tempdir()?;
    fs::create_dir_all(plugin.path().join("skills/diagnose"))?;
    let flaky = case(&json!(["Skill"]));
    let options = Options {
        // Never started: no time is left, and a missing program would fail to spawn.
        program: Path::new("no-such-claude"),
        time_limit: Duration::ZERO,
        ..options()
    };
    let mut evaluator = evaluator(options, plugin.path(), "skills", None)?;
    let batch = Batch {
        index: 2,
        arm: Arm::Without("diagnose"),
        cases: vec![&flaky],
        runs: 3,
        budget_usd: 5.0,
    };

    let ran = evaluator.run(&batch);

    let with = evaluator.target(Arm::With)?;
    let copied = with
        .parent()
        .is_some_and(|root| root.join("without-diagnose").exists());
    evaluator.finish()?;
    let ran = ran?;
    assert_eq!(
        (
            ran.runs.len(),
            ran.cost_usd,
            ran.cost_is_lower_bound,
            ran.stop,
            copied
        ),
        (0, 0.0, false, Some(Stop::TimeLimit), false)
    );
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
    let mut evaluator = evaluator(options(), plugin.path(), "skills", None)?;

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
fn the_skill_is_removed_from_the_skills_directory_the_plugin_names_its_own() -> TestResult {
    let plugin = tempfile::tempdir()?;
    for skill in ["diagnose", "plan"] {
        fs::create_dir_all(plugin.path().join("my-skills").join(skill))?;
    }
    let mut evaluator = evaluator(options(), plugin.path(), "my-skills", None)?;

    let without = evaluator.target(Arm::Without("diagnose"));

    let shape = without.as_ref().ok().map(|copy| {
        (
            copy.join("my-skills/diagnose").exists(),
            copy.join("my-skills/plan").exists(),
        )
    });
    evaluator.finish()?;
    without?;
    assert_eq!(shape, Some((false, true)));
    Ok(())
}

#[test]
fn a_baseline_copy_leaves_out_the_cases_earlier_batches_wrote() -> TestResult {
    let plugin = tempfile::tempdir()?;
    fs::create_dir_all(plugin.path().join("skills/plan"))?;
    let mut evaluator = evaluator(options(), plugin.path(), "skills", None)?;
    let with = evaluator.target(Arm::With)?;
    fs::create_dir_all(with.join("anneal-uplift-1/plan--regression--a"))?;

    let without = evaluator.target(Arm::Without("plan"))?;

    let copied = without.join("anneal-uplift-1").exists();
    evaluator.finish()?;
    assert!(!copied, "batch 1's cases stay with the with-arm copy");
    Ok(())
}

#[test]
fn stale_eval_dirs_in_the_plugin_or_the_baseline_are_not_copied() -> TestResult {
    let plugin = tempfile::tempdir()?;
    let baseline = tempfile::tempdir()?;
    for source in [plugin.path(), baseline.path()] {
        fs::create_dir_all(source.join("skills/plan"))?;
        fs::create_dir_all(source.join("anneal-uplift-1/plan--regression--stale"))?;
    }
    let mut evaluator = evaluator(
        options(),
        plugin.path(),
        "skills",
        Some(BaselinePlugin {
            dir: baseline.path(),
            name: "older",
        }),
    )?;

    let stale = [Arm::With, Arm::Baseline].map(|arm| {
        evaluator.target(arm).map(|copy| {
            (
                copy.join("anneal-uplift-1").exists(),
                copy.join("skills/plan").exists(),
            )
        })
    });

    evaluator.finish()?;
    assert_eq!(
        stale.map(Result::ok),
        [Some((false, true)), Some((false, true))],
        "batch 1 would run the stale cases too, against the cap"
    );
    Ok(())
}

#[test]
fn a_baseline_that_cannot_be_copied_is_a_workspace_error() -> TestResult {
    let plugin = tempfile::tempdir()?;
    let missing = plugin.path().join("no-such-baseline");

    let refused = evaluator(
        options(),
        plugin.path(),
        "skills",
        Some(BaselinePlugin {
            dir: &missing,
            name: "older",
        }),
    );

    assert!(
        matches!(&refused, Err(RouterError::Workspace(_))),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_trace_is_named_by_its_batch_arm_case_and_run() {
    let flaky = case(&json!(["Skill"]));
    let batches = [
        (1, Arm::With),
        (2, Arm::Without("plan")),
        (3, Arm::Baseline),
    ]
    .map(|(index, arm)| Batch {
        index,
        arm,
        ..batch(vec![&flaky])
    });

    let names = batches.map(|batch| traces::trace_name(&batch, "plan--regression--x", 2));

    assert_eq!(
        names,
        [
            "traces/1-with/plan--regression--x-2.jsonl",
            "traces/2-without-plan/plan--regression--x-2.jsonl",
            "traces/3-baseline/plan--regression--x-2.jsonl",
        ]
        .map(str::to_owned)
    );
}

#[test]
fn the_traces_kept_are_the_asked_cases_runs_that_name_one() -> TestResult {
    let flaky = case(&json!(["Skill"]));
    let run = |trace: Option<&str>| json!({"error": null, "graders": [], "tracePath": trace});
    let result = parse(json!({
        "schemaVersion": 1,
        "costUsd": 0.3,
        "partial": false,
        "suite": {"plugins": [{"name": "anneal", "path": "/p"}]},
        "cases": [
            {"name": "diagnose--regression--flaky", "arms": {"with": [
                run(Some("/w/tmp/a/out/trace.jsonl")), run(None), run(Some("/w/tmp/c/out/trace.jsonl"))
            ]}},
            {"name": "other--regression--unasked", "arms": {"with": [run(Some("/w/x"))]}}
        ]
    }))?;

    let traces: Vec<(String, String)> = traces::traces(&result, &batch(vec![&flaky]))
        .into_iter()
        .map(|trace| (trace.name, trace.path))
        .collect();

    assert_eq!(
        traces,
        [
            (
                "traces/1-with/diagnose--regression--flaky-1.jsonl",
                "/w/tmp/a/out/trace.jsonl"
            ),
            (
                "traces/1-with/diagnose--regression--flaky-3.jsonl",
                "/w/tmp/c/out/trace.jsonl"
            ),
        ]
        .map(|(name, path)| (name.to_owned(), path.to_owned()))
    );
    Ok(())
}

#[test]
fn a_skill_name_that_is_not_one_path_component_is_refused() -> TestResult {
    let plugin = tempfile::tempdir()?;
    let mut evaluator = evaluator(options(), plugin.path(), "skills", None)?;

    let refused = evaluator.target(Arm::Without("../x"));

    evaluator.finish()?;
    assert_eq!(
        refused.map_err(|error| error.code()),
        Err(crate::exit_codes::INTERNAL)
    );
    Ok(())
}
