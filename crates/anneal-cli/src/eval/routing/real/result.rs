//! Tier 2's reading of `claude plugin eval`'s result into runs: pure, no I/O. The header
//! checks and the DTO are both tiers' (`eval::real::result`).

use std::collections::BTreeMap;

use super::suite::{ANSWERED, Graders};
use super::{Ran, Run};
use crate::eval::real::result::{ResultDto, ResultError, RunDto, check, ran_out_of_turns};

/// The runs of `result`, by case name, for the plugin named `plugin` judged by `graders`.
///
/// # Errors
/// [`ResultError`] when the schema version is not 1, the cost is negative, authentication
/// failed, or the plugin did not load.
pub(crate) fn read(
    result: ResultDto,
    plugin: &str,
    graders: &Graders<'_>,
) -> Result<Ran, ResultError> {
    let checked = check(result, plugin)?;
    let mut counted_errors: u32 = 0;
    let mut runs: BTreeMap<String, Vec<Run>> = BTreeMap::new();
    for (name, case_runs) in checked.cases {
        let case_runs = case_runs
            .iter()
            .map(|run| {
                let judged = to_run(run, graders);
                if run.error().is_some() && matches!(judged, Run::Done { .. }) {
                    counted_errors = counted_errors.saturating_add(1);
                }
                judged
            })
            .collect();
        runs.insert(name, case_runs);
    }
    Ok(Ran {
        runs,
        cost_usd: checked.cost_usd,
        cost_is_lower_bound: false,
        counted_errors,
        stop: checked.stop,
    })
}

/// One run: the skills whose grader passed. `claude plugin eval` grades the trace of a run
/// that ended in error too, so an error is judged by its kind:
/// - a run that ran out of turns ([`ran_out_of_turns`]) has decided routing, since the `Skill`
///   call comes first or not at all, so it counts with whatever fired, nothing included;
/// - any other error (a timeout, a crash) counts only when a skill fired, its `Skill` call
///   being in the trace, and otherwise says nothing either way (weigh-in default wi-b4a3b0).
fn to_run(run: &RunDto, graders: &Graders<'_>) -> Run {
    let passed = || {
        run.graders()
            .filter(|(_, passed)| *passed)
            .map(|(name, _)| name)
    };
    let mut fired: Vec<String> = passed()
        .filter_map(|name| graders.skill_of(name))
        .map(str::to_owned)
        .collect();
    let decided = run
        .error()
        .is_none_or(|error| ran_out_of_turns(error) || !fired.is_empty());
    if !decided {
        return Run::Errored;
    }
    fired.sort_unstable();
    fired.dedup();
    Run::Done {
        fired,
        answered: passed().any(|name| name == ANSWERED),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::read;
    use crate::eval::real::result::{ResultDto, ResultError, parse};
    use crate::eval::routing::real::suite::Graders;
    use crate::eval::routing::real::{Run, Stop};

    fn run(error: Option<&str>, passed: &[&str]) -> Value {
        let graders: Vec<Value> = ["fired-plan", "fired-spec", "answered"]
            .iter()
            .map(|name| json!({"name": name, "passed": passed.contains(name), "weight": 1, "explanation": "", "withOnly": false}))
            .collect();
        json!({"score": 0.5, "passed": false, "turns": 1, "costUsd": 0.01, "judgeCostUsd": 0, "error": error, "tracePath": "t", "skippedPaidGraders": false, "graders": graders})
    }

    fn result(
        partial: Option<&str>,
        plugins: Value,
        cases: Value,
    ) -> Result<ResultDto, serde_json::Error> {
        let mut value = json!({
            "schemaVersion": 1,
            "claudeVersion": "2.1.296",
            "startedAt": "2026-10-12T00:00:00Z",
            "durationSeconds": 30,
            "costUsd": 0.42,
            "partial": partial.is_some(),
            "suite": {"root": "/tmp/x", "ablation": "none", "threshold": 0},
            "aggregates": {"casesTotal": 1, "casesPassed": 0, "overallScore": 0, "overallPassRate": 0}
        });
        value["suite"]["plugins"] = plugins;
        value["cases"] = cases;
        if let Some(reason) = partial {
            value["partialReason"] = json!(reason);
        }
        serde_json::from_value(value)
    }

    fn loaded() -> Value {
        json!([{"name": "anneal", "version": "0.0.1", "path": "/tmp/x/plugin"}])
    }

    fn one_case(runs: Vec<Value>) -> Value {
        json!([{"name": "prompt-001", "dir": "d", "source": "s", "promptMarkdown": "p", "runsPerCase": 3, "timeoutSeconds": 180, "maxTurns": 2, "graders": [], "arms": {"with": Value::Array(runs)}, "aggregates": {"score": 0, "passRate": 0}}])
    }

    fn skills() -> Vec<String> {
        ["plan", "spec"].map(str::to_owned).to_vec()
    }

    #[test]
    fn each_run_reports_the_skills_whose_grader_passed_and_whether_it_answered()
    -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let dto = result(
            None,
            loaded(),
            one_case(vec![
                run(None, &["fired-plan", "answered"]),
                run(None, &["fired-plan", "fired-spec"]),
                run(Some("timed out after 180s"), &[]),
            ]),
        )?;

        let ran = read(dto, "anneal", &graders)?;

        assert_eq!(
            ran.runs.get("prompt-001"),
            Some(&vec![
                Run::Done {
                    fired: vec!["plan".to_owned()],
                    answered: true
                },
                Run::Done {
                    fired: vec!["plan".to_owned(), "spec".to_owned()],
                    answered: false
                },
                Run::Errored,
            ])
        );
        assert_eq!((ran.cost_usd, ran.stop), (0.42, None));
        Ok(())
    }

    #[test]
    fn a_run_that_fired_and_then_ended_in_error_still_counts()
    -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let dto = result(
            None,
            loaded(),
            one_case(vec![run(
                Some("exit 1: Reached maximum number of turns (2)"),
                &["fired-spec"],
            )]),
        )?;

        let ran = read(dto, "anneal", &graders)?;

        assert_eq!(
            ran.runs.get("prompt-001"),
            Some(&vec![Run::Done {
                fired: vec!["spec".to_owned()],
                answered: false
            }])
        );
        Ok(())
    }

    #[test]
    fn a_run_that_ran_out_of_turns_is_a_trial_whatever_fired_and_is_counted_as_such()
    -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let dto = result(
            None,
            loaded(),
            one_case(vec![
                run(Some("exit 1: Reached maximum number of turns (2)"), &[]),
                run(
                    Some("exit 1: Reached maximum number of turns (2) · stderr: x"),
                    &["fired-plan"],
                ),
                run(Some("timed out after 180s"), &["fired-spec"]),
                run(Some("timed out after 180s"), &[]),
                run(None, &["fired-plan"]),
            ]),
        )?;

        let ran = read(dto, "anneal", &graders)?;

        let done = |fired: &[&str]| Run::Done {
            fired: fired.iter().map(|s| (*s).to_owned()).collect(),
            answered: false,
        };
        assert_eq!(
            ran.runs.get("prompt-001"),
            Some(&vec![
                done(&[]),
                done(&["plan"]),
                done(&["spec"]),
                Run::Errored,
                done(&["plan"]),
            ]),
            "out of turns: routing was decided; another error counts only when a skill fired"
        );
        assert_eq!(ran.counted_errors, 3);
        Ok(())
    }

    #[test]
    fn a_negative_cost_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let dto = parse(json!({
            "schemaVersion": 1,
            "costUsd": -0.5,
            "partial": false,
            "suite": {"plugins": loaded()},
            "cases": []
        }))?;

        assert!(matches!(
            read(dto, "anneal", &graders),
            Err(ResultError::NegativeCost { .. })
        ));
        Ok(())
    }

    #[test]
    fn a_cost_ceiling_stops_the_batch_and_keeps_its_runs() -> Result<(), Box<dyn std::error::Error>>
    {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let dto = result(
            Some("cost_ceiling"),
            loaded(),
            one_case(vec![run(None, &["fired-plan"])]),
        )?;

        let ran = read(dto, "anneal", &graders)?;

        assert_eq!(
            (ran.stop, ran.runs.get("prompt-001").map(Vec::len)),
            (Some(Stop::CostCap), Some(1))
        );
        Ok(())
    }

    #[test]
    fn failed_authentication_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let dto = result(Some("auth_failed"), loaded(), json!([]))?;

        assert!(matches!(
            read(dto, "anneal", &graders),
            Err(ResultError::AuthFailed)
        ));
        Ok(())
    }

    #[test]
    fn a_plugin_that_did_not_load_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let missing = result(None, json!([]), json!([]))?;
        let disabled = result(
            None,
            json!([{"name": "anneal", "path": "/p", "problem": "will_not_load"}]),
            json!([]),
        )?;
        let unverified = result(
            None,
            json!([{"name": "anneal", "path": "/p", "problem": "identity_unverified"}]),
            json!([]),
        )?;

        assert!(
            matches!(
                read(missing, "anneal", &graders),
                Err(ResultError::PluginNotLoaded { problem: None })
            ),
            "no plugin in the run"
        );
        assert!(
            matches!(
                read(disabled, "anneal", &graders),
                Err(ResultError::PluginNotLoaded { problem: Some(p) }) if p == "will_not_load"
            ),
            "a loading problem"
        );
        assert!(
            read(unverified, "anneal", &graders).is_ok(),
            "says nothing about loading"
        );
        Ok(())
    }

    #[test]
    fn another_schema_version_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let skills = skills();
        let graders = Graders::new("anneal", &skills);
        let mut dto = result(None, loaded(), json!([]))?;
        dto.schema_version = 2;

        assert!(matches!(
            read(dto, "anneal", &graders),
            Err(ResultError::Schema { version: 2 })
        ));
        Ok(())
    }
}
