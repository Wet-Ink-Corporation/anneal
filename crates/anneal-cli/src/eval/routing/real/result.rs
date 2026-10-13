//! Reading `claude plugin eval --json`'s result (`aggregate-result.json`, schema version 1)
//! into runs: pure, no I/O.
//!
//! The DTO names only the fields read and ignores the rest, so additions to the format do not
//! break the reader. [`parse`] reads the schema version alone first, so a result in another
//! version is reported as such whatever its shape.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::suite::{ANSWERED, Graders};
use super::{Ran, Run, Stop};

/// The result schema version this reads.
const SCHEMA_VERSION: u32 = 1;
/// Plugin problems that mean the with-plugin arm ran without the plugin.
const NOT_LOADED: [&str; 3] = ["manifest_invalid", "disabled_by_default", "will_not_load"];
/// What a run's error holds when the run used its turns up (Claude Code's `error_max_turns`
/// result, which `claude plugin eval` reports as `exit <code>: Reached maximum number of
/// turns (<n>)`). Matching text is fragile: if the wording changes, such runs fall back to
/// the rule for other errors.
const OUT_OF_TURNS: &str = "Reached maximum number of turns (";

/// The schema version alone, read before the rest.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionDto {
    schema_version: u32,
}

/// `aggregate-result.json`, the fields read.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResultDto {
    pub(crate) schema_version: u32,
    cost_usd: f64,
    partial: bool,
    #[serde(default)]
    partial_reason: Option<String>,
    suite: SuiteDto,
    cases: Vec<CaseDto>,
}

#[derive(Debug, Deserialize)]
struct SuiteDto {
    plugins: Vec<PluginDto>,
}

#[derive(Debug, Deserialize)]
struct PluginDto {
    name: String,
    #[serde(default)]
    problem: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CaseDto {
    name: String,
    arms: ArmsDto,
}

#[derive(Debug, Deserialize)]
struct ArmsDto {
    with: Vec<RunDto>,
}

/// One run of a case, the fields read.
#[derive(Debug, Deserialize)]
pub(crate) struct RunDto {
    #[serde(default)]
    error: Option<String>,
    graders: Vec<GraderDto>,
}

impl RunDto {
    /// Why the run ended in error, if it did.
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Each grader's name and whether it passed.
    pub(crate) fn graders(&self) -> impl Iterator<Item = (&str, bool)> + '_ {
        self.graders.iter().map(|g| (g.name.as_str(), g.passed))
    }
}

#[derive(Debug, Deserialize)]
struct GraderDto {
    name: String,
    passed: bool,
}

/// What every tier reads of a result once its header checks pass: its cost, why it stopped
/// short, and each case's runs (the `with` arm: every eval anneal runs is single-arm).
#[derive(Debug)]
pub(crate) struct Checked {
    /// What the batch cost, in US dollars (never negative).
    pub(crate) cost_usd: f64,
    /// Why the batch stopped short, if it did.
    pub(crate) stop: Option<Stop>,
    /// Each case's name and runs, in the result's order.
    pub(crate) cases: Vec<(String, Vec<RunDto>)>,
}

/// Whether a run's error says it used its turns up ([`OUT_OF_TURNS`]).
pub(crate) fn ran_out_of_turns(error: &str) -> bool {
    error.contains(OUT_OF_TURNS)
}

/// Why a result says nothing about routing.
#[derive(Debug, thiserror::Error)]
pub enum ResultError {
    /// The result is in a schema version this does not read.
    #[error("the result is schema version {version}; this anneal reads version 1")]
    Schema {
        /// The version found.
        version: u32,
    },
    /// The result is version 1 but not in its shape.
    #[error("the result is not in the shape of schema version 1")]
    Malformed(#[source] serde_json::Error),
    /// A case reports more runs than the batch asked for, which would settle its prompt (or
    /// narrow a sibling's should-not interval) on runs no batch paid for.
    #[error("case {case} reports {runs} runs; the batch asked for {asked}")]
    TooManyRuns {
        /// The case's name.
        case: String,
        /// The runs it reported.
        runs: usize,
        /// The runs the batch asked for.
        asked: u32,
    },
    /// A case is listed twice, so which runs belong to it cannot be told.
    #[error("case {case} is listed twice in the result")]
    DuplicateCase {
        /// The case's name.
        case: String,
    },
    /// A run names a grader its case does not have.
    #[error("a run of case {case} names grader {grader}, which the case does not have")]
    UnknownGrader {
        /// The case's name.
        case: String,
        /// The grader named.
        grader: String,
    },
    /// A run lists the same grader twice, so whether it passed cannot be told.
    #[error("a run of case {case} lists grader {grader} twice")]
    DuplicateGrader {
        /// The case's name.
        case: String,
        /// The grader listed twice.
        grader: String,
    },
    /// The result reports a negative cost, so the spend cannot be tracked.
    #[error("the result reports a negative cost ({cost_usd} USD)")]
    NegativeCost {
        /// The cost reported.
        cost_usd: f64,
    },
    /// The run could not authenticate with the model provider.
    #[error("claude could not authenticate (no credentials or a rejected key)")]
    AuthFailed,
    /// The plugin under test did not load, so every run went without it.
    #[error("the plugin did not load in the eval run ({})", problem.as_deref().unwrap_or("not listed"))]
    PluginNotLoaded {
        /// Claude Code's problem code, or `None` when the plugin is not listed at all.
        problem: Option<String>,
    },
}

/// The result in `value`: its schema version first, then, only for version 1, the rest.
///
/// # Errors
/// [`ResultError::Schema`] for another version, [`ResultError::Malformed`] for no version or
/// a version 1 result of another shape.
pub(crate) fn parse(value: Value) -> Result<ResultDto, ResultError> {
    let version = VersionDto::deserialize(&value).map_err(ResultError::Malformed)?;
    if version.schema_version != SCHEMA_VERSION {
        return Err(ResultError::Schema {
            version: version.schema_version,
        });
    }
    ResultDto::deserialize(value).map_err(ResultError::Malformed)
}

/// The first case in `runs` reporting more than `asked` runs, as the error that refuses it.
pub(crate) fn over_asked<T>(runs: &BTreeMap<String, Vec<T>>, asked: u32) -> Option<ResultError> {
    let asked_runs = usize::try_from(asked).unwrap_or(usize::MAX);
    runs.iter()
        .find(|(_, runs)| runs.len() > asked_runs)
        .map(|(case, runs)| ResultError::TooManyRuns {
            // The error outlives the batch's result, so it owns the case name.
            case: case.clone(),
            runs: runs.len(),
            asked,
        })
}

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
                if run.error.is_some() && matches!(judged, Run::Done { .. }) {
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

/// The header checks every tier makes: the schema version is 1, the cost is not negative,
/// authentication worked and the plugin named `plugin` loaded. Then the cases' runs.
///
/// # Errors
/// [`ResultError`] when a check fails.
pub(crate) fn check(result: ResultDto, plugin: &str) -> Result<Checked, ResultError> {
    if result.schema_version != SCHEMA_VERSION {
        return Err(ResultError::Schema {
            version: result.schema_version,
        });
    }
    if result.cost_usd < 0.0 {
        return Err(ResultError::NegativeCost {
            cost_usd: result.cost_usd,
        });
    }
    let stop = match (result.partial, result.partial_reason.as_deref()) {
        (_, Some("auth_failed")) => return Err(ResultError::AuthFailed),
        (_, Some("cost_ceiling")) => Some(Stop::CostCap),
        (true, _) => Some(Stop::Interrupted),
        (false, _) => None,
    };
    match result.suite.plugins.into_iter().find(|p| p.name == plugin) {
        None => return Err(ResultError::PluginNotLoaded { problem: None }),
        Some(PluginDto {
            problem: Some(problem),
            ..
        }) if NOT_LOADED.contains(&problem.as_str()) => {
            return Err(ResultError::PluginNotLoaded {
                problem: Some(problem),
            });
        }
        Some(_) => {}
    }
    Ok(Checked {
        cost_usd: result.cost_usd,
        stop,
        cases: result
            .cases
            .into_iter()
            .map(|case| (case.name, case.arms.with))
            .collect(),
    })
}

/// One run: the skills whose grader passed. `claude plugin eval` grades the trace of a run
/// that ended in error too, so an error is judged by its kind:
/// - a run that ran out of turns ([`OUT_OF_TURNS`]) has decided routing, since the `Skill`
///   call comes first or not at all, so it counts with whatever fired, nothing included;
/// - any other error (a timeout, a crash) counts only when a skill fired, its `Skill` call
///   being in the trace, and otherwise says nothing either way (weigh-in default wi-b4a3b0).
fn to_run(run: &RunDto, graders: &Graders<'_>) -> Run {
    let passed = || run.graders.iter().filter(|g| g.passed);
    let mut fired: Vec<String> = passed()
        .filter_map(|g| graders.skill_of(&g.name))
        .map(str::to_owned)
        .collect();
    let decided = run
        .error
        .as_deref()
        .is_none_or(|error| ran_out_of_turns(error) || !fired.is_empty());
    if !decided {
        return Run::Errored;
    }
    fired.sort_unstable();
    fired.dedup();
    Run::Done {
        fired,
        answered: passed().any(|g| g.name == ANSWERED),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{ResultDto, ResultError, parse, read};
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
    fn the_schema_version_is_checked_before_the_rest_of_the_result() {
        let other_shape = json!({"schemaVersion": 2, "runs": {"something": "else"}});

        assert!(matches!(
            parse(other_shape),
            Err(ResultError::Schema { version: 2 })
        ));
    }

    #[test]
    fn a_result_of_the_right_version_and_wrong_shape_is_malformed() {
        let wrong = json!({"schemaVersion": 1, "costUsd": "free"});

        assert!(matches!(parse(wrong), Err(ResultError::Malformed(_))));
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
