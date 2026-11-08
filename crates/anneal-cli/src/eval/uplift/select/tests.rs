use std::path::{Path, PathBuf};

use super::select;
use crate::eval::uplift::UpliftError;
use crate::eval::uplift::args::{BaselineArm, GatedTools, Scaffolds, UpliftArgs};
use crate::exit_codes;

/// The fixture suites, relative to the crate root, where cargo runs unit tests.
const SUITES: &str = "tests/fixtures/eval-uplift/suites";

/// The fixture suites and plugin, with `--case` given as `case` and `--skill` as `skill`.
fn args(case: &str, skill: &[&str]) -> UpliftArgs {
    UpliftArgs {
        skills: PathBuf::from("tests/fixtures/eval-routing/skills"),
        suites: PathBuf::from(SUITES),
        skill: skill.iter().map(|name| (*name).to_owned()).collect(),
        case: Some(case.to_owned()),
        runs: None,
        baseline_arm: BaselineArm::Run,
        check: false,
        baseline: None,
        claude: None,
        max_cost_usd: None,
        timeout: None,
        concurrency: None,
        model: None,
        trust_plugin: false,
        changed_since: None,
        gated_tools: GatedTools::Refuse,
        scaffolds: Scaffolds::Refuse,
        work_dir: None,
        keep_results: None,
    }
}

/// The files and coverage rows `--case case` selects.
fn selected(case: &str) -> (Vec<String>, Vec<String>) {
    let suites = select(&args(case, &[])).expect("a case");
    (
        suites.cases.into_iter().map(|case| case.file).collect(),
        suites.coverage.into_iter().map(|row| row.skill).collect(),
    )
}

#[test]
fn case_selects_one_case_by_its_file_its_name_or_its_path_and_keeps_its_skill_s_coverage() {
    let path = Path::new(SUITES).join("plan/regression/break-down.json");
    let one = || {
        (
            vec!["plan/regression/break-down.json".to_owned()],
            vec!["plan".to_owned()],
        )
    };

    assert_eq!(
        [
            selected("plan/regression/break-down.json"),
            selected("plan--regression--break-down"),
            selected(path.to_str().expect("UTF-8")),
        ],
        [one(), one(), one()]
    );
}

#[test]
fn a_case_naming_no_case_is_a_usage_error_as_is_one_outside_the_skill_given() {
    let unknown = select(&args("nope.json", &[]));
    let elsewhere = select(&args("plan/regression/break-down.json", &["spec"]));

    assert!(
        matches!(&unknown, Err(error @ UpliftError::UnknownCase { case, .. })
            if case == "nope.json" && error.code() == exit_codes::USAGE),
        "{unknown:?}"
    );
    assert_eq!(
        elsewhere.map(|_| ()).map_err(|error| error.to_string()),
        Err(
            "--case plan/regression/break-down.json names no case under the suites directory \
             for the --skill given"
                .to_owned()
        )
    );
}
