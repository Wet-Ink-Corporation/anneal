use std::path::PathBuf;

use super::{args, reason};
use crate::eval::uplift::args::{BaselineArm, Mode, UpliftArgs};
use crate::eval::uplift::runs::Runs;

#[test]
fn runs_default_to_a_round_and_the_runs_given_reach_the_settings() {
    let default = args();
    let ten = UpliftArgs {
        runs: Some(10),
        ..args()
    };

    let modes = (default.mode(), ten.mode());

    assert!(
        matches!(
            &modes,
            (Ok(Mode::Run(default)), Ok(Mode::Run(ten)))
                if default.runs == Runs::ROUND
                    && default.baseline_arm == BaselineArm::Run
                    && ten.runs.get() == 10
        ),
        "{modes:?}"
    );
}

#[test]
fn runs_outside_3_to_20_are_a_usage_error() {
    let args = UpliftArgs {
        runs: Some(25),
        ..args()
    };

    assert_eq!(
        reason(args.mode()),
        Some("--runs 25 is not 3 to 20".to_owned())
    );
}

#[test]
fn check_refuses_runs_and_no_baseline_but_not_case() {
    let runs = UpliftArgs {
        check: true,
        runs: Some(10),
        ..args()
    };
    let no_baseline = UpliftArgs {
        check: true,
        baseline_arm: BaselineArm::Skip,
        ..args()
    };
    let case = UpliftArgs {
        check: true,
        case: Some("plan/regression/a.json".to_owned()),
        ..args()
    };

    assert_eq!(
        (reason(runs.mode()), reason(no_baseline.mode())),
        (
            Some("--check runs nothing, so --runs does not apply".to_owned()),
            Some("--check runs nothing, so --no-baseline does not apply".to_owned())
        )
    );
    assert!(matches!(case.mode(), Ok(Mode::Check)));
}

#[test]
fn no_baseline_with_a_baseline_and_case_with_changed_since_are_usage_errors() {
    let baseline = UpliftArgs {
        baseline_arm: BaselineArm::Skip,
        baseline: Some(PathBuf::from(".")),
        ..args()
    };
    let changed = UpliftArgs {
        case: Some("plan/regression/a.json".to_owned()),
        changed_since: Some("main".to_owned()),
        ..args()
    };

    assert_eq!(
        (reason(baseline.mode()), reason(changed.mode())),
        (
            Some("--no-baseline runs no baseline, so --baseline does not apply".to_owned()),
            Some("--case runs one case, so --changed-since does not apply".to_owned())
        )
    );
}

#[test]
fn a_no_baseline_run_skips_the_baseline_arm() {
    let args = UpliftArgs {
        baseline_arm: BaselineArm::Skip,
        ..args()
    };

    let mode = args.mode();

    assert!(
        matches!(&mode, Ok(Mode::Run(settings)) if settings.baseline_arm == BaselineArm::Skip),
        "{mode:?}"
    );
}

#[test]
fn a_confirmation_carries_the_run_s_settings_and_drops_its_selection_and_kept_results() {
    let args = UpliftArgs {
        suites: PathBuf::from("my suites"),
        model: Some("opus".to_owned()),
        trust_plugin: true,
        skill: vec!["plan".to_owned()],
        keep_results: Some(PathBuf::from("kept")),
        changed_since: Some("main".to_owned()),
        ..args()
    };

    assert_eq!(
        args.confirm_words(),
        ["--suites", "my suites", "--model", "opus", "--trust-plugin"]
    );
}

#[test]
fn a_confirmation_carries_nothing_for_the_defaults() {
    assert_eq!(args().confirm_words(), Vec::<String>::new());
}
