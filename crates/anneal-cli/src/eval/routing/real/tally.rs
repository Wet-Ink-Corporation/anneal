//! Per-skill verdicts and the confusion table: pure, no I/O.
//!
//! Every run of a skill's positive prompt is a should-fire trial for that skill and a
//! should-not trial for every other skill, so the negatives are the siblings' positives.
//! Should-fire is judged per prompt (each must settle [`Next::Met`]), with the pooled rate
//! reported beside it; should-not is pooled, since one prompt's 0 of 12 has an upper bound of
//! 0.242 and could never meet 0.20.

use serde::Serialize;

use super::Run;
use super::schedule::{Count, Next, next};
use super::wilson::{Bar, Interval, Position, wilson};
use crate::eval::routing::Confusion;

/// The row-header cell of the rendered table.
const CORNER: &str = "expected \\ fired";
/// The column of finished runs that invoked no skill.
const NONE: &str = "(none)";
/// The column of finished runs that invoked more than one skill.
const SEVERAL: &str = "(several)";
/// The column of runs that did not finish.
const ERROR: &str = "(error)";

/// One positive prompt and its runs under the real router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Probe {
    /// The skill whose fixture lists it, which should fire.
    pub(crate) skill: String,
    /// The fixture file, for display.
    pub(crate) file: String,
    /// 1-based line of the prompt.
    pub(crate) line: usize,
    /// The prompt.
    pub(crate) text: String,
    /// Its runs so far, oldest first.
    pub(crate) runs: Vec<Run>,
}

/// A pooled firing rate held to a bar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Rate {
    /// Which bar.
    pub(crate) bar: Bar,
    /// The bar's threshold.
    pub(crate) threshold: f64,
    /// Fired runs and finished runs.
    #[serde(flatten)]
    pub(crate) count: Count,
    /// The 95% Wilson interval, or `None` with no finished run.
    pub(crate) interval: Option<Interval>,
    /// Where the interval lies against the bar, or `None` with no finished run.
    pub(crate) position: Option<Position>,
}

impl Rate {
    /// `count` held to `bar`.
    pub(crate) fn new(bar: Bar, count: Count) -> Self {
        let interval = wilson(count.fired, count.trials);
        Self {
            bar,
            threshold: bar.threshold(),
            count,
            interval,
            position: interval.map(|interval| bar.position(interval)),
        }
    }

    /// Whether the interval wholly meets the bar.
    pub(crate) fn met(self) -> bool {
        self.position == Some(Position::Met)
    }
}

/// One skill's result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SkillResult<'a> {
    /// The skill.
    pub(crate) skill: &'a str,
    /// Its own prompts' runs pooled: did it fire? Reported; the bar is per prompt.
    pub(crate) should_fire: Rate,
    /// Its own prompts that settled with the should-fire bar met.
    pub(crate) prompts_met: u32,
    /// Its own prompts that settled missing the bar, or still straddled it after the most
    /// runs.
    pub(crate) prompts_failed: u32,
    /// The other skills' prompts' runs pooled: did it fire anyway?
    pub(crate) should_not: Rate,
    /// Its own prompts' finished runs whose outcome grader passed (a non-empty reply);
    /// reported, not held to a bar.
    pub(crate) answered: u32,
    /// Every one of its own prompts met the should-fire bar (and it has at least one), and
    /// its pooled should-not bar is met.
    pub(crate) passed: bool,
}

/// Whether the prompt `probe` ended failing its should-fire bar: settled missing it, or
/// still straddling it after the most runs.
pub(crate) fn failed(probe: &Probe) -> bool {
    matches!(
        next(&probe.runs, &probe.skill),
        Next::Missed | Next::MaxRuns
    )
}

/// Each skill's verdict over `probes`, in `skills` order.
pub(crate) fn per_skill<'a>(skills: &'a [String], probes: &[Probe]) -> Vec<SkillResult<'a>> {
    skills
        .iter()
        .map(|skill| {
            let own_runs = || {
                probes
                    .iter()
                    .filter(|probe| probe.skill == *skill)
                    .flat_map(|probe| &probe.runs)
            };
            let other_runs = probes
                .iter()
                .filter(|probe| probe.skill != *skill)
                .flat_map(|probe| &probe.runs);
            let should_fire = Rate::new(Bar::ShouldFire, Count::of(own_runs(), skill));
            let should_not = Rate::new(Bar::ShouldNot, Count::of(other_runs, skill));
            let answered = own_runs()
                .filter(|run| matches!(run, Run::Done { answered: true, .. }))
                .count();
            let (mut prompts, mut met, mut failing) = (0_usize, 0_usize, 0_usize);
            for probe in probes.iter().filter(|probe| probe.skill == *skill) {
                prompts = prompts.saturating_add(1);
                match next(&probe.runs, skill) {
                    Next::Met => met = met.saturating_add(1),
                    Next::Missed | Next::MaxRuns => failing = failing.saturating_add(1),
                    Next::RunMore => {}
                }
            }
            SkillResult {
                skill,
                should_fire,
                prompts_met: saturating_u32(met),
                prompts_failed: saturating_u32(failing),
                should_not,
                answered: saturating_u32(answered),
                passed: prompts > 0 && met == prompts && should_not.met(),
            }
        })
        .collect()
}

/// `n` as a `u32`, or `u32::MAX` past it (a count for display).
fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The confusion table of every run of `probes`: the prompt's skill against the skill that
/// fired, `(none)`, `(several)` or `(error)`.
pub(crate) fn confusion<'a>(skills: &'a [String], probes: &'a [Probe]) -> Confusion<'a> {
    let pairs = probes
        .iter()
        .flat_map(|probe| {
            probe
                .runs
                .iter()
                .map(|run| (probe.skill.as_str(), label(run)))
        })
        .collect();
    Confusion::tally(CORNER, skills, &[NONE, SEVERAL, ERROR], pairs)
}

/// The column a run counts in.
fn label(run: &Run) -> &str {
    match run {
        Run::Errored => ERROR,
        Run::Done { fired, .. } => match fired.as_slice() {
            [] => NONE,
            [skill] => skill,
            [_, _, ..] => SEVERAL,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Probe, per_skill};
    use crate::eval::routing::real::Run;
    use crate::eval::routing::real::wilson::Position;

    fn skills() -> Vec<String> {
        ["plan", "spec"].map(str::to_owned).to_vec()
    }

    fn done(fired: &[&str]) -> Run {
        Run::Done {
            fired: fired.iter().map(|s| (*s).to_owned()).collect(),
            answered: true,
        }
    }

    fn probe(skill: &str, runs: Vec<Run>) -> Probe {
        Probe {
            skill: skill.to_owned(),
            file: format!("evals/routing/{skill}.yaml"),
            line: 2,
            text: format!("a {skill} prompt"),
            runs,
        }
    }

    #[test]
    fn should_fire_pools_a_skills_own_prompts_and_should_not_the_others() {
        let skills = skills();
        let probes = [
            probe("plan", vec![done(&["plan"]); 9]),
            probe("plan", vec![done(&["plan"]), done(&["spec"]), Run::Errored]),
            probe("spec", vec![done(&["spec"]); 9]),
        ];

        let results = per_skill(&skills, &probes);

        let [plan, spec] = results.as_slice() else {
            panic!("two skills: {results:?}");
        };
        assert_eq!(
            (plan.should_fire.count.fired, plan.should_fire.count.trials),
            (10, 11),
            "the errored run is not a trial"
        );
        assert_eq!(
            (plan.should_not.count.fired, plan.should_not.count.trials),
            (0, 9),
            "spec's prompts never fired plan"
        );
        assert_eq!(
            (spec.should_not.count.fired, spec.should_not.count.trials),
            (1, 11),
            "one of plan's runs fired spec"
        );
    }

    #[test]
    fn a_skill_passes_only_when_both_bars_are_met() {
        let skills = skills();
        let probes = [
            probe("plan", vec![done(&["plan"]); 12]),
            probe("spec", vec![done(&["spec"]); 12]),
            probe("spec", vec![done(&["spec"]); 12]),
        ];

        let results = per_skill(&skills, &probes);

        let verdicts: Vec<_> = results
            .iter()
            .map(|r| {
                (
                    r.skill,
                    r.should_fire.position,
                    r.should_not.position,
                    r.passed,
                )
            })
            .collect();
        assert_eq!(
            verdicts,
            [
                ("plan", Some(Position::Met), Some(Position::Met), true),
                (
                    "spec",
                    Some(Position::Met),
                    Some(Position::Straddles),
                    false
                ),
            ],
            "spec's should-not rests on plan's 12 runs: 0/12 upper 0.242 > 0.20"
        );
    }

    #[test]
    fn a_skill_fails_when_one_of_its_prompts_misses_even_if_its_pool_meets_the_bar() {
        let skills = skills();
        let probes = [
            probe("plan", vec![done(&["plan"]); 12]),
            probe("plan", vec![done(&["plan"]); 12]),
            probe("plan", vec![done(&[]); 3]),
            probe("spec", vec![done(&["spec"]); 12]),
            probe("spec", vec![done(&["spec"]); 12]),
            probe("spec", vec![done(&["spec"]); 12]),
        ];

        let results = per_skill(&skills, &probes);

        let plan = results.first().expect("plan");
        assert_eq!(
            (
                plan.should_fire.position,
                plan.should_not.position,
                plan.prompts_met,
                plan.prompts_failed,
                plan.passed
            ),
            (Some(Position::Met), Some(Position::Met), 2, 1, false),
            "24 of 27 pooled has lower bound 0.72, but the third prompt's 0 of 3 misses"
        );
    }

    #[test]
    fn a_prompt_that_still_straddles_after_12_runs_fails_its_skill() {
        let skills = skills();
        let mut straddling = vec![done(&["plan"]); 7];
        straddling.extend(vec![done(&[]); 5]);
        let probes = [
            probe("plan", straddling),
            probe("spec", vec![done(&["spec"]); 12]),
            probe("spec", vec![done(&["spec"]); 12]),
            probe("spec", vec![done(&["spec"]); 12]),
        ];

        let results = per_skill(&skills, &probes);

        let plan = results.first().expect("plan");
        assert_eq!(
            (plan.prompts_met, plan.prompts_failed, plan.passed),
            (0, 1, false),
            "7 of 12: lower bound 0.32"
        );
    }

    #[test]
    fn a_skill_with_no_finished_runs_has_no_interval_and_fails() {
        let skills = skills();
        let probes = [
            probe("plan", vec![Run::Errored; 3]),
            probe("spec", vec![done(&["spec"]); 12]),
        ];

        let results = per_skill(&skills, &probes);

        let plan = results.first().expect("plan");
        assert_eq!(
            (
                plan.should_fire.interval,
                plan.should_fire.position,
                plan.passed
            ),
            (None, None, false)
        );
    }

    #[test]
    fn the_confusion_table_counts_finished_runs_by_the_skill_that_fired() {
        let skills = skills();
        let probes = [
            probe(
                "plan",
                vec![
                    done(&["plan"]),
                    done(&["spec"]),
                    done(&[]),
                    done(&["plan", "spec"]),
                    Run::Errored,
                ],
            ),
            probe("spec", vec![done(&["spec"])]),
        ];

        let table = super::confusion(&skills, &probes).render();

        assert_eq!(
            table,
            [
                "expected \\ fired  plan  spec  (none)  (several)  (error)",
                "plan                 1     1       1          1        1",
                "spec                 0     1       0          0        0",
            ]
        );
    }
}
