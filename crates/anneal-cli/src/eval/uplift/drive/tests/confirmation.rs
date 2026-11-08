use super::{Answer, Fake, cases, names};
use crate::eval::real::{ResultError, RouterError};
use crate::eval::uplift::drive::{Ask, Baseline, drive};
use crate::eval::uplift::runs::Runs;

#[test]
fn with_the_baseline_skipped_only_the_with_arm_runs_with_the_whole_cap() {
    let cases = cases();
    let mut fake = Fake::new(vec![Answer::Costs(1.0)]);
    let ask = Ask {
        baseline: Baseline::Skipped,
        runs: Runs::CONFIRMATION,
        cap_usd: 10.0,
    };

    let drove = drive(&mut fake, &cases, ask).expect("runs");

    assert_eq!(
        fake.batches,
        [(
            "with".to_owned(),
            names(&[
                "plan--regression--a",
                "plan--regression--b",
                "spec--regression--c"
            ]),
            10,
            10.0
        )]
    );
    assert_eq!((drove.batches, drove.stop), (1, None));
}

#[test]
fn the_runs_given_reach_every_batch_and_more_runs_than_that_are_refused() {
    let cases = cases();
    let ask = Ask {
        baseline: Baseline::SkillRemoved,
        runs: Runs::new(10).expect("in range"),
        cap_usd: 10.0,
    };
    let mut fake = Fake::new(vec![Answer::Costs(1.0)]);
    let mut padded = Fake::new(vec![Answer::Pads(1.0)]);

    drive(&mut fake, &cases, ask).expect("runs");
    let refused = drive(&mut padded, &cases, ask);

    assert_eq!(
        fake.batches
            .iter()
            .map(|(_, _, runs, _)| *runs)
            .collect::<Vec<_>>(),
        [10, 10, 10]
    );
    assert!(
        matches!(
            &refused,
            Err(RouterError::Result(ResultError::TooManyRuns {
                runs: 11,
                asked: 10,
                ..
            }))
        ),
        "{refused:?}"
    );
}
