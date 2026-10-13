//! The sequential stopping rule: pure, no I/O.
//!
//! Each prompt runs [`BATCH`] times. While its should-fire interval straddles the bar it runs
//! another batch, up to [`MAX_RUNS`] runs in all; once the interval wholly meets or wholly
//! misses the bar it stops.

use serde::Serialize;

use super::Run;
use crate::eval::real::wilson::{Bar, Position, wilson};

/// Runs per batch.
pub(crate) const BATCH: u32 = 3;
/// The most runs one prompt gets.
pub(crate) const MAX_RUNS: u32 = 12;

/// What the runs of one prompt say about whether to run it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Next {
    /// Its interval meets the bar.
    Met,
    /// Its interval misses the bar.
    Missed,
    /// It still straddles the bar after [`MAX_RUNS`] runs.
    MaxRuns,
    /// It straddles the bar (or has no finished run) with runs to spare.
    RunMore,
}

/// How often a skill fired over the finished runs of a set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Count {
    /// Finished runs in which the skill fired.
    pub(crate) fired: u32,
    /// Finished runs (errored runs are no evidence either way and are not counted).
    pub(crate) trials: u32,
}

impl Count {
    /// Adds one finished run in which `skill` did or did not fire.
    pub(crate) fn add(&mut self, fired: bool) {
        self.trials = self.trials.saturating_add(1);
        if fired {
            self.fired = self.fired.saturating_add(1);
        }
    }

    /// How often `skill` fired over `runs`.
    pub(crate) fn of<'r>(runs: impl IntoIterator<Item = &'r Run>, skill: &str) -> Self {
        let mut count = Self::default();
        for run in runs {
            if let Some(fired) = run.fired(skill) {
                count.add(fired);
            }
        }
        count
    }
}

/// Whether a prompt whose own skill is `skill` should run again after `runs`.
pub(crate) fn next(runs: &[Run], skill: &str) -> Next {
    let count = Count::of(runs, skill);
    match wilson(count.fired, count.trials).map(|interval| Bar::ShouldFire.position(interval)) {
        Some(Position::Met) => Next::Met,
        Some(Position::Missed) => Next::Missed,
        Some(Position::Straddles) | None => {
            let ran = u32::try_from(runs.len()).unwrap_or(u32::MAX);
            if ran >= MAX_RUNS {
                Next::MaxRuns
            } else {
                Next::RunMore
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Next, next};
    use crate::eval::routing::real::Run;

    fn runs(fired: usize, missed: usize, errored: usize) -> Vec<Run> {
        let done = |skills: &[&str]| Run::Done {
            fired: skills.iter().map(|s| (*s).to_owned()).collect(),
            answered: true,
        };
        std::iter::repeat_with(|| done(&["plan"]))
            .take(fired)
            .chain(std::iter::repeat_with(|| done(&["spec"])).take(missed))
            .chain(std::iter::repeat_n(Run::Errored, errored))
            .collect()
    }

    #[test]
    fn a_prompt_with_no_runs_runs() {
        assert_eq!(next(&[], "plan"), Next::RunMore);
    }

    #[test]
    fn three_of_three_straddles_and_runs_again() {
        assert_eq!(next(&runs(3, 0, 0), "plan"), Next::RunMore, "lower 0.439");
    }

    #[test]
    fn nine_of_nine_meets_the_bar_and_stops() {
        assert_eq!(next(&runs(9, 0, 0), "plan"), Next::Met, "lower 0.7009");
    }

    #[test]
    fn none_of_three_misses_the_bar_and_stops() {
        assert_eq!(next(&runs(0, 3, 0), "plan"), Next::Missed, "upper 0.561");
    }

    #[test]
    fn eleven_of_twelve_still_straddles_and_stops_at_twelve_runs() {
        assert_eq!(next(&runs(11, 1, 0), "plan"), Next::MaxRuns, "lower 0.646");
    }

    #[test]
    fn errored_runs_are_no_evidence_but_count_toward_the_twelve() {
        assert_eq!(
            (next(&runs(0, 0, 3), "plan"), next(&runs(0, 0, 12), "plan")),
            (Next::RunMore, Next::MaxRuns)
        );
    }

    #[test]
    fn another_skill_firing_is_a_miss_for_the_prompt() {
        assert_eq!(
            next(&runs(3, 0, 0), "spec"),
            Next::Missed,
            "spec never fired"
        );
    }
}
