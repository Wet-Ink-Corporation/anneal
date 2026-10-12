//! Reading `claude plugin eval --json`'s result (`aggregate-result.json`, schema version 1)
//! into runs: pure, no I/O.
//!
//! The DTO names only the fields read and ignores the rest, so additions to the format do not
//! break the reader; the schema version is checked first.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::suite::{ANSWERED, Graders};
use super::{Ran, Run, Stop};

/// The result schema version this reads.
const SCHEMA_VERSION: u32 = 1;
/// Plugin problems that mean the with-plugin arm ran without the plugin.
const NOT_LOADED: [&str; 3] = ["manifest_invalid", "disabled_by_default", "will_not_load"];

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

#[derive(Debug, Deserialize)]
struct RunDto {
    #[serde(default)]
    error: Option<String>,
    graders: Vec<GraderDto>,
}

#[derive(Debug, Deserialize)]
struct GraderDto {
    name: String,
    passed: bool,
}

/// Why a result says nothing about routing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResultError {
    /// The result is in a schema version this does not read.
    #[error("the result is schema version {version}; this anneal reads version 1")]
    Schema {
        /// The version found.
        version: u32,
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

/// The runs of `result`, by case name, for the plugin named `plugin` judged by `graders`.
///
/// # Errors
/// [`ResultError`] when the schema version is not 1, authentication failed, or the plugin did
/// not load.
pub(crate) fn read(
    result: ResultDto,
    plugin: &str,
    graders: &Graders<'_>,
) -> Result<Ran, ResultError> {
    if result.schema_version != SCHEMA_VERSION {
        return Err(ResultError::Schema {
            version: result.schema_version,
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
    let runs: BTreeMap<String, Vec<Run>> = result
        .cases
        .into_iter()
        .map(|case| {
            let runs = case
                .arms
                .with
                .into_iter()
                .map(|run| to_run(&run, graders))
                .collect();
            (case.name, runs)
        })
        .collect();
    Ok(Ran {
        runs,
        cost_usd: result.cost_usd,
        stop,
    })
}

/// One run: the skills whose grader passed. A run that reports an error is evidence only when
/// a skill fired: the `Skill` call is in its trace, and `claude plugin eval` grades the trace
/// of a run that ended in error too (a run that fired a skill and then ran out of turns is the
/// common case). An error with no skill fired says nothing either way.
fn to_run(run: &RunDto, graders: &Graders<'_>) -> Run {
    let passed = || run.graders.iter().filter(|g| g.passed);
    let mut fired: Vec<String> = passed()
        .filter_map(|g| graders.skill_of(&g.name))
        .map(str::to_owned)
        .collect();
    if run.error.is_some() && fired.is_empty() {
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

    use super::{ResultDto, ResultError, read};
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
