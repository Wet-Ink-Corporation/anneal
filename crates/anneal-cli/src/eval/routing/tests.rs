use crate::eval::routing::fixture::{Fixture, parse};
use crate::eval::routing::{Case, CaseKind, Fault, List, Outcome, Problem, Source, Top, judge};
use crate::exit_codes;
use crate::lint::finding::{LintFinding, Location};
use crate::lint::skills::checker::Checker;
use crate::lint::skills::listing::Listing;

fn listing(name: &str, text: &str) -> Listing {
    Listing {
        name: name.to_owned(),
        text: text.to_owned(),
        model_invoked: true,
    }
}

fn user_invoked(name: &str) -> Listing {
    Listing {
        name: name.to_owned(),
        text: name.to_owned(),
        model_invoked: false,
    }
}

fn source(skill: &str, text: &str) -> Source {
    Source {
        file: format!("{skill}.yaml"),
        skill: Some(skill.to_owned()),
        parsed: parse(text).map_err(|e| (e.line, Fault::Parse { reason: e.reason })),
    }
}

/// `count` numbered prompts under `key`, indented for a fixture.
fn items(count: usize, prompt: &str) -> String {
    (1..=count)
        .map(|n| format!("    - {prompt} {n}\n"))
        .collect::<Vec<_>>()
        .concat()
}

/// Two skills whose triggers share no word.
fn plan_and_spec() -> Vec<Result<Listing, LintFinding>> {
    vec![
        Ok(listing("plan", "plan: break work into ordered tasks")),
        Ok(listing(
            "spec",
            "spec: write requirements and acceptance criteria",
        )),
    ]
}

/// A fixture for `plan` with exactly eight positives and eight near misses owned by `spec`.
fn full_plan_fixture() -> String {
    format!(
        "positives:\n{}near_misses:\n  spec:\n{}",
        items(8, "break into tasks"),
        items(8, "write requirements"),
    )
    .replace("\n    - break", "\n  - break")
}

fn full_spec_fixture() -> String {
    format!(
        "positives:\n{}near_misses:\n  plan:\n{}",
        items(8, "write acceptance criteria"),
        items(8, "order the tasks"),
    )
    .replace("\n    - write", "\n  - write")
}

fn passed(outcome: &Outcome) -> Vec<(usize, bool)> {
    outcome.cases.iter().map(|c| (c.line, c.passed)).collect()
}

#[test]
fn a_complete_passing_set_exits_zero_with_every_case_passed() {
    let outcome = judge(
        plan_and_spec(),
        vec![
            source("plan", &full_plan_fixture()),
            source("spec", &full_spec_fixture()),
        ],
    );

    assert_eq!(outcome.problems, [], "no problem");
    assert_eq!(outcome.cases.len(), 32, "16 per fixture");
    assert!(
        outcome.cases.iter().all(|c| c.passed),
        "{:?}",
        outcome.cases
    );
    assert_eq!(outcome.code(), exit_codes::OK, "100% rank-1");
}

#[test]
fn a_positive_passes_only_when_its_skill_is_the_unique_top() {
    let fixture = "positives:\n  - break into tasks\n  - write requirements\n  - plan the spec\n";

    let outcome = judge(plan_and_spec(), vec![source("plan", fixture)]);

    assert_eq!(
        outcome.cases.first(),
        Some(&Case {
            file: "plan.yaml".to_owned(),
            line: 2,
            kind: CaseKind::Positive,
            prompt: "break into tasks".to_owned(),
            expected: "plan".to_owned(),
            top: Top::Skill("plan".to_owned()),
            expected_score: 2,
            rival: Some(("spec".to_owned(), 0)),
            passed: true,
        }),
        "the full record of a passing positive"
    );
    let tops: Vec<&Top> = outcome.cases.iter().map(|c| &c.top).collect();
    assert_eq!(
        tops,
        [
            &Top::Skill("plan".to_owned()),
            &Top::Skill("spec".to_owned()),
            &Top::Tie,
        ],
        "own skill, another skill, a tie"
    );
    assert_eq!(
        passed(&outcome),
        [(2, true), (3, false), (4, false)],
        "a tie is a miss"
    );
}

#[test]
fn a_near_miss_passes_when_its_owner_outscores_the_skill_it_mimics() {
    let fixture = "near_misses:\n  spec:\n    - write requirements\n    - break requirements into tasks\n    - deploy it\n";

    let outcome = judge(plan_and_spec(), vec![source("plan", fixture)]);

    assert_eq!(
        outcome
            .cases
            .first()
            .map(|c| (&c.kind, c.expected.as_str(), &c.rival)),
        Some((
            &CaseKind::NearMiss {
                mimics: "plan".to_owned()
            },
            "spec",
            &Some(("plan".to_owned(), 0)),
        )),
        "expected is the owner, the rival is the mimicked skill"
    );
    assert_eq!(
        passed(&outcome),
        [(3, true), (4, false), (5, false)],
        "owner ahead passes; behind or level (0 = 0) misses"
    );
}

#[test]
fn any_miss_exits_below_bound() {
    let outcome = judge(
        plan_and_spec(),
        vec![
            source(
                "plan",
                &full_plan_fixture().replace("break into tasks 8", "write requirements"),
            ),
            source("spec", &full_spec_fixture()),
        ],
    );

    assert_eq!(outcome.problems, [], "the fixtures are valid");
    assert_eq!(outcome.misses().count(), 1, "one positive ranks spec first");
    assert_eq!(outcome.code(), exit_codes::EVAL_BELOW_BOUND, "exit 1");
}

fn at(file: &str, line: usize, fault: Fault) -> Problem {
    Problem::Fixture {
        file: file.to_owned(),
        line,
        fault,
    }
}

#[test]
fn file_level_problems_are_reported_once_per_file_and_exit_invalid() {
    let broken = LintFinding::error(
        Checker::Frontmatter,
        Location::new("skills/broken/SKILL.md", 1),
        "frontmatter does not parse.",
        "fix it.",
    );
    let mut skills = plan_and_spec();
    skills.push(Ok(user_invoked("release")));
    skills.push(Err(broken.clone()));
    skills.push(Ok(listing(
        "diagnose",
        "diagnose: find the root cause of a bug",
    )));
    let bad_name = Source {
        file: "bad\u{fffd}.yaml".to_owned(),
        skill: None,
        parsed: Ok(Fixture::default()),
    };

    let outcome = judge(
        skills,
        vec![
            source("plan", &full_plan_fixture()),
            source("spec", "positives:\n  - [a]\n"),
            source("release", ""),
            source("nobody", ""),
            bad_name,
        ],
    );

    let flow = "a flow collection is not a prompt; quote it";
    assert_eq!(
        outcome.problems,
        [
            Problem::Skill(broken),
            at("spec.yaml", 2, Fault::Parse { reason: flow }),
            at("release.yaml", 0, Fault::NotRoutable),
            at("nobody.yaml", 0, Fault::UnknownSkill),
            at("bad\u{fffd}.yaml", 0, Fault::NameNotUtf8),
            Problem::MissingFixture {
                skill: "diagnose".to_owned()
            },
        ],
        "skills first, then fixtures in order, then missing fixtures; spec is not also missing"
    );
    assert_eq!(outcome.code(), exit_codes::EVAL_INVALID, "exit 2");
}

#[test]
fn count_and_owner_problems_point_at_their_lines() {
    let short =
        "positives:\n  - break into tasks\nnear_misses:\n  plan:\n    - a\n  ghost:\n    - b\n";

    let outcome = judge(
        plan_and_spec(),
        vec![source("plan", short), source("spec", &full_spec_fixture())],
    );

    let count = |list, count| Fault::Count { list, count };
    let ghost = Fault::UnknownOwner {
        owner: "ghost".to_owned(),
    };
    assert_eq!(
        outcome.problems,
        [
            at("plan.yaml", 1, count(List::Positives, 1)),
            at("plan.yaml", 3, count(List::NearMisses, 2)),
            at("plan.yaml", 5, Fault::SelfOwned),
            at("plan.yaml", 7, ghost),
        ],
        "counts at their keys, owners at their prompts"
    );
    assert_eq!(
        passed(&outcome).first(),
        Some(&(2, true)),
        "valid prompts are still scored"
    );
    assert_eq!(outcome.code(), exit_codes::EVAL_INVALID, "exit 2");
}

#[test]
fn no_skills_and_no_fixtures_is_a_vacuous_pass() {
    let outcome = judge(Vec::new(), Vec::new());

    assert_eq!(
        (
            outcome.skills.len(),
            outcome.cases.len(),
            outcome.problems.len()
        ),
        (0, 0, 0),
        "nothing to route"
    );
    assert_eq!(outcome.code(), exit_codes::OK, "vacuous");
}

#[test]
fn the_confusion_table_has_expected_rows_and_top_ranked_columns() {
    let fixture = "positives:\n  - break into tasks\n  - write requirements\n  - plan the spec\nnear_misses:\n  spec:\n    - write requirements\n";
    let outcome = judge(plan_and_spec(), vec![source("plan", fixture)]);

    let table = outcome.confusion().render();

    assert_eq!(
        table,
        [
            "expected \\ top  plan  spec  (tie)",
            "plan               1     1      1",
            "spec               0     1      0",
        ],
        "rows: expected skill; columns: every skill, then (tie) as it occurs"
    );
}
