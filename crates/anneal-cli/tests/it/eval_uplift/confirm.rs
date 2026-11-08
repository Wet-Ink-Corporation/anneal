//! Confirming a guard: one case (`--case`), the with arm alone (`--no-baseline`), and the runs
//! asked for (`--runs`).

use std::ffi::OsStr;

use serde_json::json;

use super::{TestResult, envelope, eval_uplift, fixtures};

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn one_case_without_a_baseline_runs_once_with_the_whole_cap_and_computes_no_uplift() -> TestResult {
    let args = [
        "--case",
        "plan/regression/break-down.json",
        "--no-baseline",
        "--runs",
        "10",
    ]
    .map(OsStr::new);

    let run = super::FakeRun::with_args(&[], &args)?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    assert_eq!(run.log()?, "with anneal-uplift-1 25.00 10 \n");
    assert_eq!(
        (
            &envelope["data"]["baseline"],
            &envelope["data"]["runs_per_arm"],
            &envelope["data"]["complete"],
            &envelope["data"]["per_skill"][0]["uplift_met"],
            &envelope["data"]["results"][0]["with"],
        ),
        (
            &json!("none"),
            &json!(10),
            &json!(true),
            &json!(null),
            &json!({"passed": 10, "trials": 10})
        )
    );
    let keys: Vec<&str> = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?
        .iter()
        .filter_map(|finding| finding["key"].as_str())
        .collect();
    assert!(keys.contains(&"eval/uplift/no-baseline"), "{keys:?}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_case_naming_no_case_exits_2_with_one_error_line() -> TestResult {
    let output = eval_uplift(
        &fixtures().join("eval-uplift/suites"),
        &["--check", "--case", "nope.json"],
    )?;

    assert_eq!(
        (
            output.status.code(),
            String::from_utf8(output.stderr)?,
            output.stdout.is_empty()
        ),
        (
            Some(2),
            "ERROR eval: --case nope.json names no case under the suites directory\n".to_owned(),
            true
        )
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn runs_outside_3_to_20_exit_2() -> TestResult {
    let suites = fixtures().join("eval-uplift/suites");

    let codes = ["2", "21"]
        .into_iter()
        .map(|runs| Ok(eval_uplift(&suites, &["--runs", runs])?.status.code()))
        .collect::<Result<Vec<_>, std::io::Error>>()?;

    assert_eq!(codes, [Some(2), Some(2)]);
    Ok(())
}
