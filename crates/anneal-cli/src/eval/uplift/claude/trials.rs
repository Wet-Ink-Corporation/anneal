//! Reading one batch's checked `claude plugin eval` result into its runs.

use std::collections::{BTreeMap, BTreeSet};

use crate::eval::routing::real::result::{self, ResultDto, ResultError, RunDto};
use crate::eval::uplift::Trial;
use crate::eval::uplift::drive::Ran;
use crate::eval::uplift::suites::Case;

/// The runs of `result` for the plugin named `plugin`, for the batch's `cases`. A run that
/// finished or ran out of turns is graded (running out of turns is the agent's behaviour,
/// not an accident); any other error is no trial. A case the batch did not ask for says
/// nothing about the suites and is dropped. A result whose header passed but whose runs
/// cannot be read keeps its cost and no run, with why in [`Ran::refused`]: a case listed
/// twice ([`ResultError::DuplicateCase`]), a run naming a grader its case does not have
/// ([`ResultError::UnknownGrader`]) or the same grader twice
/// ([`ResultError::DuplicateGrader`]).
///
/// # Errors
/// [`ResultError`] when the header checks fail.
pub(crate) fn trials(result: ResultDto, plugin: &str, cases: &[&Case]) -> Result<Ran, ResultError> {
    let checked = result::check(result, plugin)?;
    let (runs, refused) = match runs_of(checked.cases, cases) {
        Ok(runs) => (runs, None),
        Err(refused) => (BTreeMap::new(), Some(refused)),
    };
    Ok(Ran {
        runs,
        cost_usd: checked.cost_usd,
        cost_is_lower_bound: false,
        stop: checked.stop,
        refused,
    })
}

/// The trials of each case of `listed` that the batch's `cases` asked for, by name.
///
/// # Errors
/// [`ResultError::DuplicateCase`], [`ResultError::UnknownGrader`] or
/// [`ResultError::DuplicateGrader`], as [`trials`] says.
fn runs_of(
    listed: Vec<(String, Vec<RunDto>)>,
    cases: &[&Case],
) -> Result<BTreeMap<String, Vec<Trial>>, ResultError> {
    let asked: BTreeMap<String, &Case> = cases.iter().map(|case| (case.name(), *case)).collect();
    let mut runs: BTreeMap<String, Vec<Trial>> = BTreeMap::new();
    let mut dropped: BTreeSet<String> = BTreeSet::new();
    for (name, case_runs) in listed {
        if runs.contains_key(&name) || dropped.contains(&name) {
            return Err(ResultError::DuplicateCase { case: name });
        }
        let Some(case) = asked.get(&name) else {
            dropped.insert(name);
            continue;
        };
        let trials = case_runs
            .iter()
            .map(|run| trial(case, &name, run))
            .collect::<Result<Vec<_>, _>>()?;
        runs.insert(name, trials);
    }
    Ok(runs)
}

/// `run` of the case `case`, named `name` in the result.
fn trial(case: &Case, name: &str, run: &RunDto) -> Result<Trial, ResultError> {
    if let Some(error) = run.error()
        && !result::ran_out_of_turns(error)
    {
        return Ok(Trial::Errored);
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut graded = Vec::new();
    for (grader, passed) in run.graders() {
        // The errors outlive the result, so they own both names.
        if !case.body.graders.iter().any(|known| known.name() == grader) {
            return Err(ResultError::UnknownGrader {
                case: name.to_owned(),
                grader: grader.to_owned(),
            });
        }
        if !seen.insert(grader) {
            return Err(ResultError::DuplicateGrader {
                case: name.to_owned(),
                grader: grader.to_owned(),
            });
        }
        graded.push((grader.to_owned(), passed));
    }
    Ok(Trial::Graded(graded))
}
