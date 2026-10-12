//! The sequential run: batches of prompts through the [`Router`] port until each settles.

use super::schedule::{BATCH, MAX_RUNS, Next, next};
use super::tally::Probe;
use super::{Ran, RouterError, Stop};
use crate::error_chain;

/// The least budget, in US dollars, worth launching a batch for.
const MIN_BUDGET_USD: f64 = 0.01;
/// The most batches: enough for [`MAX_RUNS`] runs of a prompt.
const MAX_ROUNDS: u32 = MAX_RUNS / BATCH;

/// The real router: runs a batch of prompts and reports each run.
pub(crate) trait Router {
    /// Runs each case of `batch` `batch.runs` times, spending at most `batch.budget_usd`.
    ///
    /// # Errors
    /// [`RouterError`] when the batch could not run or its result says nothing about routing.
    fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError>;
}

/// One batch for the router.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Batch<'a> {
    /// 1 for the first batch.
    pub(crate) round: u32,
    /// The prompts to run.
    pub(crate) cases: Vec<BatchCase<'a>>,
    /// Runs per prompt.
    pub(crate) runs: u32,
    /// The most the batch may spend, in US dollars.
    pub(crate) budget_usd: f64,
}

/// One prompt of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchCase<'a> {
    /// The case's name, stable across batches: `prompt-<n>`, 1-based.
    pub(crate) name: String,
    /// The prompt.
    pub(crate) prompt: &'a str,
}

/// How the sequential run went.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drove {
    /// What the batches cost, in US dollars, as the router reported it.
    pub(crate) spent_usd: f64,
    /// Batches run.
    pub(crate) rounds: u32,
    /// Why the run stopped before every prompt settled, if it did.
    pub(crate) stop: Option<Stop>,
}

/// The name of the case for the prompt at `index` of the probes.
fn case_name(index: usize) -> String {
    format!("prompt-{:03}", index.saturating_add(1))
}

/// The indexes of the probes that should run again.
fn pending(probes: &[Probe]) -> Vec<usize> {
    probes
        .iter()
        .enumerate()
        .filter(|(_, probe)| next(&probe.runs, &probe.skill) == Next::RunMore)
        .map(|(index, _)| index)
        .collect()
}

/// The batch cases of the probes at `indexes`.
fn cases<'a>(probes: &'a [Probe], indexes: &[usize]) -> Vec<BatchCase<'a>> {
    indexes
        .iter()
        .filter_map(|&index| {
            probes.get(index).map(|probe| BatchCase {
                name: case_name(index),
                prompt: &probe.text,
            })
        })
        .collect()
}

/// Runs `probes` through `router` in batches of [`BATCH`] until each settles (see
/// [`next`]) or `cap_usd` is spent, appending each batch's runs to its probe.
///
/// # Errors
/// The first batch's [`RouterError`]: nothing has run, so there is nothing to report. A later
/// batch's failure ends the run with [`Stop::Failed`] instead, keeping the runs so far.
pub(crate) fn drive<R: Router>(
    router: &mut R,
    probes: &mut [Probe],
    cap_usd: f64,
) -> Result<Drove, RouterError> {
    let mut drove = Drove {
        spent_usd: 0.0,
        rounds: 0,
        stop: None,
    };
    for round in 1..=MAX_ROUNDS {
        let pending = pending(probes);
        if pending.is_empty() {
            break;
        }
        let budget_usd = cap_usd - drove.spent_usd;
        if budget_usd < MIN_BUDGET_USD {
            drove.stop = Some(Stop::CostCap);
            break;
        }
        let batch = Batch {
            round,
            cases: cases(probes, &pending),
            runs: BATCH,
            budget_usd,
        };
        let ran = match router.run(&batch) {
            Ok(ran) => ran,
            Err(error) if round == 1 => return Err(error),
            Err(error) => {
                drove.stop = Some(Stop::Failed {
                    reason: error_chain::reason(&error),
                });
                break;
            }
        };
        drove.rounds = round;
        if ran.cost_usd.is_finite() && ran.cost_usd > 0.0 {
            drove.spent_usd += ran.cost_usd;
        }
        let Ran { mut runs, stop, .. } = ran;
        for index in pending {
            if let (Some(probe), Some(new)) =
                (probes.get_mut(index), runs.remove(&case_name(index)))
            {
                probe.runs.extend(new);
            }
        }
        if stop.is_some() {
            drove.stop = stop;
            break;
        }
    }
    Ok(drove)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Batch, Router, drive};
    use crate::eval::routing::real::tally::Probe;
    use crate::eval::routing::real::{Ran, RouterError, Run, Stop};

    /// A router that answers each batch with `answer`, recording every batch it got as
    /// `(case names, runs, budget)`.
    struct Fake<F> {
        answer: F,
        batches: Vec<(Vec<String>, u32, f64)>,
    }

    impl<F: FnMut(usize, &str) -> Option<Vec<Run>>> Fake<F> {
        fn new(answer: F) -> Self {
            Self {
                answer,
                batches: Vec::new(),
            }
        }
    }

    /// Each batch costs this much unless a test says otherwise.
    const BATCH_COST: f64 = 0.5;

    impl<F: FnMut(usize, &str) -> Option<Vec<Run>>> Router for Fake<F> {
        fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError> {
            let round = self.batches.len();
            self.batches.push((
                batch.cases.iter().map(|c| c.name.clone()).collect(),
                batch.runs,
                batch.budget_usd,
            ));
            let mut runs = BTreeMap::new();
            for case in &batch.cases {
                if let Some(case_runs) = (self.answer)(round, &case.name) {
                    runs.insert(case.name.clone(), case_runs);
                }
            }
            Ok(Ran {
                runs,
                cost_usd: BATCH_COST,
                stop: None,
            })
        }
    }

    fn fired(skill: &str, n: usize) -> Vec<Run> {
        vec![
            Run::Done {
                fired: vec![skill.to_owned()],
                answered: true
            };
            n
        ]
    }

    fn probes() -> Vec<Probe> {
        ["plan", "spec"]
            .iter()
            .map(|skill| Probe {
                skill: (*skill).to_owned(),
                file: format!("{skill}.yaml"),
                line: 2,
                text: format!("a {skill} prompt"),
                runs: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn prompts_that_always_fire_run_three_batches_of_three() -> Result<(), RouterError> {
        let mut probes = probes();
        let mut router = Fake::new(|_, name: &str| {
            Some(fired(if name == "prompt-001" { "plan" } else { "spec" }, 3))
        });

        let drove = drive(&mut router, &mut probes, 5.0)?;

        let names = vec!["prompt-001".to_owned(), "prompt-002".to_owned()];
        assert_eq!(
            router.batches,
            [
                (names.clone(), 3, 5.0),
                (names.clone(), 3, 4.5),
                (names, 3, 4.0)
            ],
            "3/3 and 6/6 straddle 0.70; 9/9 meets it; each batch gets the cap minus the spend"
        );
        assert_eq!(
            (
                drove.spent_usd,
                drove.stop,
                probes.iter().map(|p| p.runs.len()).sum::<usize>()
            ),
            (1.5, None, 18)
        );
        Ok(())
    }

    #[test]
    fn a_prompt_that_never_fires_stops_after_one_batch() -> Result<(), RouterError> {
        let mut probes = probes();
        // Every prompt fires spec, so plan's prompt (prompt-001) misses.
        let mut router = Fake::new(|_, _: &str| Some(fired("spec", 3)));

        drive(&mut router, &mut probes, 5.0)?;

        let rounds: Vec<Vec<String>> = router.batches.into_iter().map(|b| b.0).collect();
        assert_eq!(
            rounds,
            [
                vec!["prompt-001".to_owned(), "prompt-002".to_owned()],
                vec!["prompt-002".to_owned()],
                vec!["prompt-002".to_owned()],
            ],
            "plan's prompt missed (0/3, upper 0.56) and ran once"
        );
        Ok(())
    }

    #[test]
    fn a_spent_cap_stops_before_the_next_batch() -> Result<(), RouterError> {
        let mut probes = probes();
        let mut router = Fake::new(|_, name: &str| {
            Some(fired(if name == "prompt-001" { "plan" } else { "spec" }, 3))
        });

        let drove = drive(&mut router, &mut probes, 1.0)?;

        assert_eq!(
            (router.batches.len(), drove.stop),
            (2, Some(Stop::CostCap)),
            "two batches of 0.50 spend the 1.00 cap"
        );
        Ok(())
    }

    #[test]
    fn a_reported_stop_ends_the_run_and_keeps_the_batch() -> Result<(), RouterError> {
        struct Capped;
        impl Router for Capped {
            fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError> {
                let first = batch.cases.first().map(|c| c.name.clone());
                Ok(Ran {
                    runs: first
                        .into_iter()
                        .map(|name| (name, fired("plan", 1)))
                        .collect(),
                    cost_usd: 5.0,
                    stop: Some(Stop::CostCap),
                })
            }
        }
        let mut probes = probes();

        let drove = drive(&mut Capped, &mut probes, 5.0)?;

        assert_eq!(
            (
                drove.stop,
                probes.iter().map(|p| p.runs.len()).collect::<Vec<_>>()
            ),
            (Some(Stop::CostCap), vec![1, 0])
        );
        Ok(())
    }

    #[test]
    fn a_first_batch_failure_is_an_error_and_a_later_one_stops_the_run() {
        struct FailsOn(usize, usize);
        impl Router for FailsOn {
            fn run(&mut self, batch: &Batch<'_>) -> Result<Ran, RouterError> {
                self.1 += 1;
                if self.1 == self.0 {
                    return Err(RouterError::Failed {
                        status: "exit status: 1".to_owned(),
                        stderr: "boom".to_owned(),
                    });
                }
                Ok(Ran {
                    runs: batch
                        .cases
                        .iter()
                        .map(|c| (c.name.clone(), fired("plan", 3)))
                        .collect(),
                    cost_usd: 0.1,
                    stop: None,
                })
            }
        }

        let first = drive(&mut FailsOn(1, 0), &mut probes(), 5.0);
        let mut later_probes = probes();
        let later = drive(&mut FailsOn(2, 0), &mut later_probes, 5.0);

        assert!(
            matches!(first, Err(RouterError::Failed { .. })),
            "{first:?}"
        );
        assert!(
            matches!(&later, Ok(drove) if matches!(&drove.stop, Some(Stop::Failed { reason }) if reason.contains("boom"))),
            "{later:?}"
        );
        assert_eq!(
            later_probes
                .iter()
                .map(|p| p.runs.len())
                .collect::<Vec<_>>(),
            [3, 3],
            "the first batch's runs stand"
        );
    }
}
