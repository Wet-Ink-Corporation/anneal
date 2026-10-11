//! `anneal lint skills` against the fixture pairs in `tests/fixtures/lint-skills/`: every
//! checker has a failing fixture that trips it (and nothing else) and a passing one that is
//! clean.

use std::collections::BTreeSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use anneal_cli::lint::finding::Severity;
use anneal_cli::lint::skills::checker::Checker;
use anneal_cli::lint::skills::{Outcome, SkillsArgs, lint};
use serde_json::Value;

use crate::support::copy_dir;

type TestResult = Result<(), Box<dyn Error>>;

/// One fixture pair: `<dir>/fail` must trip `checker` at `severity` and no other error or
/// warning beyond `also`; `<dir>/pass` must have no error or warning at all.
struct Case {
    dir: &'static str,
    checker: Checker,
    severity: Severity,
    also: &'static [Checker],
}

const fn case(dir: &'static str, checker: Checker, severity: Severity) -> Case {
    Case {
        dir,
        checker,
        severity,
        also: &[],
    }
}

const CASES: [Case; 29] = [
    case("rule-01-frontmatter", Checker::Frontmatter, Severity::Error),
    case("rule-01-name", Checker::Name, Severity::Error),
    case(
        "rule-01-description-budget",
        Checker::DescriptionBudget,
        Severity::Error,
    ),
    Case {
        // Seven entry skills cannot exceed the budget alone, so the fixture adds a
        // model-invoked non-entry skill, which is also a rule-4 error.
        also: &[Checker::ModelInvocation],
        ..case(
            "rule-01-listing-budget",
            Checker::ListingBudget,
            Severity::Error,
        )
    },
    case("rule-02-body-budget", Checker::BodyBudget, Severity::Error),
    case("rule-02-references", Checker::References, Severity::Error),
    case(
        "rule-02-hard-rules-first",
        Checker::HardRulesFirst,
        Severity::Warning,
    ),
    case("rule-03-anatomy", Checker::Anatomy, Severity::Error),
    case(
        "rule-04-bootstrap-index",
        Checker::BootstrapIndex,
        Severity::Error,
    ),
    case(
        "rule-04-model-invocation",
        Checker::ModelInvocation,
        Severity::Error,
    ),
    case(
        "rule-05-description-trigger",
        Checker::DescriptionTrigger,
        Severity::Error,
    ),
    case(
        "rule-05-description-sibling",
        Checker::DescriptionSibling,
        Severity::Error,
    ),
    case("rule-05-overlap", Checker::Overlap, Severity::Warning),
    case(
        "rule-06-skip-rationalization",
        Checker::SkipRationalization,
        Severity::Warning,
    ),
    case(
        "rule-07-form-matches-failure",
        Checker::FormMatchesFailure,
        Severity::Review,
    ),
    case(
        "rule-08-completion-criteria",
        Checker::CompletionCriteria,
        Severity::Warning,
    ),
    case(
        "rule-09-adjective-threshold",
        Checker::AdjectiveThreshold,
        Severity::Warning,
    ),
    case("rule-10-enforcer", Checker::Enforcer, Severity::Warning),
    case(
        "rule-11-facts-and-decisions",
        Checker::FactsAndDecisions,
        Severity::Review,
    ),
    case(
        "rule-12-scripts-contract",
        Checker::ScriptsContract,
        Severity::Review,
    ),
    case(
        "rule-13-gate-baseline",
        Checker::GateBaseline,
        Severity::Review,
    ),
    case(
        "rule-14-subagent-handoff",
        Checker::SubagentHandoff,
        Severity::Review,
    ),
    case(
        "rule-15-hook-failure-policy",
        Checker::HookFailurePolicy,
        Severity::Review,
    ),
    case(
        "rule-16-stop-block-cap",
        Checker::StopBlockCap,
        Severity::Review,
    ),
    case("rule-17-check-order", Checker::CheckOrder, Severity::Review),
    case(
        "rule-18-frontmatter-fields",
        Checker::FrontmatterFields,
        Severity::Error,
    ),
    case(
        "rule-19-hook-silence",
        Checker::HookSilence,
        Severity::Review,
    ),
    case("rule-20-model-field", Checker::ModelField, Severity::Error),
    case(
        "rule-21-stable-first",
        Checker::StableFirst,
        Severity::Review,
    ),
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lint-skills")
}

/// Lints `<fixture>/skills`, passing `<fixture>/index.md` as the index when present.
fn lint_fixture(fixture: &Path) -> Result<Outcome, Box<dyn Error>> {
    let index = fixture.join("index.md");
    let args = SkillsArgs {
        paths: vec![fixture.join("skills")],
        index: index.is_file().then_some(index),
        hooks: None,
        cache: None,
    };
    Ok(lint(&args)?)
}

/// The checkers with an error or warning finding.
fn failing_checkers(outcome: &Outcome) -> BTreeSet<Checker> {
    outcome
        .findings
        .iter()
        .filter(|f| f.severity != Severity::Review)
        .map(|f| f.checker)
        .collect()
}

#[test]
fn every_checker_has_a_fixture_pair() {
    let covered: BTreeSet<Checker> = CASES.iter().map(|c| c.checker).collect();
    let all: BTreeSet<Checker> = Checker::ALL.into_iter().collect();
    assert_eq!(
        covered, all,
        "one fixture pair per checker, so per rule 1 to 21"
    );
}

#[test]
fn each_failing_fixture_trips_its_checker_and_nothing_else() -> TestResult {
    for case in &CASES {
        let outcome = lint_fixture(&fixtures().join(case.dir).join("fail"))?;

        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.checker == case.checker && f.severity == case.severity),
            "{}: expected a {:?} from {}: {:#?}",
            case.dir,
            case.severity,
            case.checker,
            outcome.findings
        );
        let mut expected: BTreeSet<Checker> = case.also.iter().copied().collect();
        if case.severity != Severity::Review {
            expected.insert(case.checker);
        }
        assert_eq!(
            failing_checkers(&outcome),
            expected,
            "{}: only its own rule breaks",
            case.dir
        );
    }
    Ok(())
}

#[test]
fn each_passing_fixture_has_no_error_or_warning() -> TestResult {
    for case in &CASES {
        let outcome = lint_fixture(&fixtures().join(case.dir).join("pass"))?;

        assert_eq!(
            failing_checkers(&outcome),
            BTreeSet::new(),
            "{} passes",
            case.dir
        );
        assert!(
            outcome.findings.iter().all(|f| f.checker != case.checker),
            "{}: {} is silent on the passing fixture",
            case.dir,
            case.checker
        );
    }
    Ok(())
}

#[test]
fn a_skills_dir_without_skills_is_clean() -> TestResult {
    let outcome = lint_fixture(&fixtures().join("rule-07-form-matches-failure/pass"))?;

    assert_eq!(outcome.skills, 0, "README.md is not a skill");
    assert_eq!(outcome.findings, [], "an empty set has nothing to report");
    Ok(())
}

fn anneal(args: &[&str]) -> Result<(Option<i32>, Value, String), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(args)
        .output()?;
    let stdout: Value = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    };
    Ok((
        output.status.code(),
        stdout,
        String::from_utf8(output.stderr)?,
    ))
}

fn fixture_arg(relative: &str) -> String {
    fixtures().join(relative).display().to_string()
}

#[test]
fn an_error_exits_1_and_every_finding_prints_its_rule_location_and_fix() -> TestResult {
    let (code, envelope, stderr) =
        anneal(&["lint", "skills", &fixture_arg("rule-01-name/fail/skills")])?;

    assert_eq!(code, Some(1), "an error finding fails the run");
    assert_eq!(stderr, "", "findings are results, not errors");
    assert_eq!(
        envelope["ok"],
        Value::Bool(false),
        "ok follows the exit code"
    );
    let findings = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?;
    assert!(!findings.is_empty(), "findings are listed");
    for finding in findings {
        let message = finding["message"].as_str().ok_or("message is a string")?;
        assert!(
            message.contains(" rule ") && message.contains(" fix: "),
            "rule and fix: {message}"
        );
        assert!(
            message.contains("SKILL.md:") || message.contains("skills:0"),
            "file:line: {message}"
        );
    }
    assert_eq!(
        envelope["data"]["errors"],
        Value::from(1),
        "one error counted"
    );
    Ok(())
}

#[test]
fn warnings_and_review_items_are_listed_but_exit_0() -> TestResult {
    let (code, envelope, _) = anneal(&[
        "lint",
        "skills",
        &fixture_arg("rule-02-hard-rules-first/fail/skills"),
    ])?;

    assert_eq!(code, Some(0), "warnings do not fail the run");
    assert_eq!(
        envelope["data"]["warnings"],
        Value::from(1),
        "the warning is counted"
    );
    assert!(
        envelope["data"]["reviews"].as_u64().is_some_and(|n| n > 0),
        "review items are listed, never silently passed"
    );
    Ok(())
}

#[test]
fn a_missing_path_is_a_usage_error_with_nothing_on_stdout() -> TestResult {
    let (code, envelope, stderr) = anneal(&["lint", "skills", &fixture_arg("does-not-exist")])?;

    assert_eq!(code, Some(2), "a bad path is a usage error");
    assert_eq!(envelope, Value::Null, "nothing on stdout");
    assert!(
        stderr.starts_with("ERROR ") && stderr.lines().count() == 1,
        "one line: {stderr}"
    );
    Ok(())
}

#[test]
fn the_cache_replays_unchanged_skills_and_rechecks_changed_ones() -> TestResult {
    let work = tempfile::tempdir()?;
    let skills = work.path().join("skills");
    copy_dir(
        &fixtures().join("rule-08-completion-criteria/fail/skills"),
        &skills,
    )?;
    let args = SkillsArgs {
        paths: vec![skills.clone()],
        index: None,
        hooks: None,
        cache: Some(work.path().join("cache/lint-skills.json")),
    };

    let fresh = lint(&args)?;
    let replayed = lint(&args)?;
    std::fs::write(skills.join("spec/notes.txt"), "changed")?;
    let after_change = lint(&args)?;

    assert_eq!(
        (fresh.skills, fresh.cached),
        (2, 0),
        "first run checks both"
    );
    assert_eq!(replayed.cached, 2, "second run replays both");
    assert_eq!(
        replayed.findings, fresh.findings,
        "replayed findings equal fresh ones"
    );
    assert_eq!(after_change.cached, 1, "a changed skill is checked again");
    Ok(())
}

#[test]
fn the_rules_doc_lists_every_checker_id() -> TestResult {
    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/skill-rules.md");
    let doc = std::fs::read_to_string(doc_path)?;

    for checker in Checker::ALL {
        assert!(
            doc.contains(&format!("`{checker}`")),
            "docs/skill-rules.md names `{checker}`"
        );
    }
    Ok(())
}
