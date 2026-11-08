use serde_json::{Value, json};

use super::{case, judged, outcome, trials};
use crate::eval::uplift::report::report;
use crate::eval::uplift::runs::Runs;
use crate::eval::uplift::suites::SuiteKind;
use crate::eval::uplift::tally::Arms;
use crate::eval::uplift::{BaselineKind, Scope, UpliftOutcome};

/// `outcome`'s envelope.
fn envelope(outcome: &UpliftOutcome) -> Value {
    let mut out = Vec::new();
    report(outcome, &mut out).expect("written");
    serde_json::from_slice(&out).expect("json")
}

/// A finished round of `plan`'s outcome case `a`, passing every run with the skill and none
/// without, and guard `g`, passing `guard` with the skill.
fn guarded(guard: &[bool]) -> UpliftOutcome {
    outcome(
        vec![
            case(SuiteKind::Regression, "a", "outcome"),
            case(SuiteKind::Capability, "g", "guard"),
        ],
        vec![
            (
                "plan--regression--a",
                Arms {
                    with: trials(&[true, true, true]),
                    base: trials(&[false, false, false]),
                },
            ),
            (
                "plan--capability--g",
                Arms {
                    with: trials(guard),
                    base: trials(&[false, false, false]),
                },
            ),
        ],
    )
}

#[test]
fn a_watched_guard_s_finding_holds_its_confirm_command_and_its_skill_lists_it() {
    let envelope = envelope(&guarded(&[true, false, true]));

    assert_eq!(
        (
            &envelope["findings"][0]["message"],
            &envelope["data"]["per_skill"][0]["guards_watch"],
            &envelope["data"]["per_skill"][0]["guards_failed"],
            &envelope["data"]["results"][1]["guard_band"],
        ),
        (
            &json!(
                "guard case plan/capability/g.json passed 2 of 3 runs with the skill: on watch, \
                 not failed; confirm it (it holds at 9 of 10): anneal eval uplift --case \
                 plan/capability/g.json --no-baseline --runs 10"
            ),
            &json!(["plan/capability/g.json"]),
            &json!([]),
            &json!("watch")
        )
    );
}

#[test]
fn a_guard_that_passes_one_of_three_runs_is_broken_and_below_bound_with_its_finding() {
    let outcome = guarded(&[false, false, true]);

    let envelope = envelope(&outcome);

    assert_eq!(
        judged(&outcome),
        (
            1,
            vec!["eval/uplift/guard:plan/capability/g.json".to_owned()],
            json!(true)
        )
    );
    assert_eq!(
        (
            &envelope["findings"][0]["message"],
            &envelope["data"]["watch"]
        ),
        (
            &json!(
                "guard case plan/capability/g.json passed 1 of 3 runs with the skill: broken (a \
                 guard may slip one run, which puts it on watch)"
            ),
            &json!([])
        )
    );
}

/// A finished `--no-baseline` run of `plan`'s guard `g`, asked for 10 runs, with `with` runs
/// with the skill and none without.
fn confirmation(with: &[bool]) -> UpliftOutcome {
    let mut outcome = outcome(
        vec![case(SuiteKind::Capability, "g", "guard")],
        vec![(
            "plan--capability--g",
            Arms {
                with: trials(with),
                base: Vec::new(),
            },
        )],
    );
    outcome.runs = Runs::CONFIRMATION;
    outcome.baseline = Some(BaselineKind::Skipped);
    outcome
}

#[test]
fn a_no_baseline_run_is_complete_without_a_baseline_arm_and_computes_no_uplift() {
    let outcome = confirmation(&[true; 10]);

    let envelope = envelope(&outcome);

    assert_eq!(
        (
            judged(&outcome).0,
            &envelope["data"]["complete"],
            &envelope["data"]["baseline"],
            &envelope["data"]["runs_per_arm"],
            &envelope["data"]["per_skill"][0]["uplift_met"],
        ),
        (0, &json!(true), &json!("none"), &json!(10), &Value::Null)
    );
    assert_eq!(
        envelope["findings"],
        json!([{
            "key": "eval/uplift/no-baseline",
            "message": "the run had no baseline arm (--no-baseline), so no uplift was computed; \
                        each guard was held to 9 in 10 of its runs"
        }])
    );
}

#[test]
fn a_no_baseline_run_fails_a_guard_below_9_in_10() {
    let mut with = [true; 10];
    with[..2].fill(false);
    let outcome = confirmation(&with);

    let envelope = envelope(&outcome);

    assert_eq!(
        judged(&outcome),
        (
            1,
            vec![
                "eval/uplift/no-baseline".to_owned(),
                "eval/uplift/guard:plan/capability/g.json".to_owned()
            ],
            json!(true)
        )
    );
    assert_eq!(
        envelope["findings"][1]["message"],
        json!(
            "guard case plan/capability/g.json passed 8 of 10 runs with the skill alone, below \
             the 9-in-10 confirmation bar"
        )
    );
}

#[test]
fn a_run_of_10_with_9_runs_with_the_skill_is_incomplete() {
    let outcome = confirmation(&[true; 9]);

    let envelope = envelope(&outcome);

    let incomplete: Vec<&Value> = envelope["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| finding["key"] == "eval/uplift/incomplete")
        .map(|finding| &finding["message"])
        .collect();
    assert_eq!(
        (judged(&outcome).0, incomplete),
        (
            3,
            vec![&json!(
                "1 cases have fewer than 10 runs in an arm: claude plugin eval returned fewer \
                 runs than asked"
            )]
        )
    );
}

#[test]
fn a_round_s_uplift_met_is_still_true_or_false() {
    let met = guarded(&[true, true, true]);
    let missed = outcome(
        vec![case(SuiteKind::Regression, "a", "outcome")],
        vec![(
            "plan--regression--a",
            Arms {
                with: trials(&[false, false, false]),
                base: trials(&[false, false, false]),
            },
        )],
    );

    let uplift_met = [met, missed]
        .map(|outcome| envelope(&outcome)["data"]["per_skill"][0]["uplift_met"].take());

    assert_eq!(uplift_met, [json!(true), json!(false)]);
}

/// `data.graduates` and whether `plan`'s capability suite graduates, in `outcome`'s envelope.
fn graduation(outcome: &UpliftOutcome) -> (Value, Value) {
    let mut envelope = envelope(outcome);
    let mut suites = envelope["data"]["per_skill"][0]["suites"].take();
    let capability = suites
        .as_array_mut()
        .expect("suites")
        .iter_mut()
        .find(|suite| suite["suite"] == "capability")
        .map(|suite| suite["graduates"].take())
        .expect("a capability suite");
    (envelope["data"]["graduates"].take(), capability)
}

#[test]
fn a_confirmation_of_a_capability_guard_at_10_of_10_graduates_nothing() {
    assert_eq!(
        graduation(&confirmation(&[true; 10])),
        (json!([]), json!(false))
    );
}

#[test]
fn only_a_round_of_whole_suites_graduates_a_capability_suite_not_one_case_of_it() {
    let whole = guarded(&[true, true, true]);
    let mut one_case = guarded(&[true, true, true]);
    one_case.scope = Scope::OneCase;

    assert_eq!(
        [graduation(&whole), graduation(&one_case)],
        [(json!(["plan"]), json!(true)), (json!([]), json!(false))]
    );
}
