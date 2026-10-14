use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::Duration;

use clap::Parser;
use serde_json::{Value, json};

use super::runner::{RunError, Runner};
use super::{PerfArgs, run};
use crate::cli::{Cli, Command};
use crate::error::CliError;

/// A runner that hands back scripted run times (or failures) and never starts a process.
struct FakeRunner {
    runs: VecDeque<Result<Duration, RunError>>,
    seen: Vec<(Vec<OsString>, Duration)>,
}

impl FakeRunner {
    fn timed(micros: &[u64]) -> Self {
        Self {
            runs: micros
                .iter()
                .map(|micros| Ok(Duration::from_micros(*micros)))
                .collect(),
            seen: Vec::new(),
        }
    }
}

impl Runner for FakeRunner {
    fn run(&mut self, command: &[OsString], timeout: Duration) -> Result<Duration, RunError> {
        self.seen.push((command.to_vec(), timeout));
        self.runs.pop_front().expect("a scripted run is left")
    }
}

/// `SplitMix64` run times around `mean_micros`, up to `spread` either way: seeded, so the same
/// seed gives the same runs on every machine.
fn seeded_micros(seed: u64, n: usize, mean_micros: u64, spread: u64) -> Vec<u64> {
    let mut state = seed;
    (0..n)
        .map(|_| {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            mean_micros - spread + z % (2 * spread + 1)
        })
        .collect()
}

fn perf_args(args: &[&str]) -> PerfArgs {
    let cli = Cli::try_parse_from(["anneal", "perf"].iter().chain(args)).expect("parses");
    match cli.command {
        Some(Command::Perf(args)) => args,
        other => panic!("not perf: {other:?}"),
    }
}

/// Writes a baseline sample file of `micros` run times into `dir`.
fn baseline(dir: &Path, micros: &[u64]) -> String {
    let path = dir.join("baseline.json");
    let secs: Vec<f64> = micros
        .iter()
        .map(|micros| Duration::from_micros(*micros).as_secs_f64())
        .collect();
    let file = json!({"version": 1, "command": ["bench"], "samples_secs": secs});
    std::fs::write(&path, file.to_string()).expect("writes the baseline");
    path.to_str().expect("a UTF-8 temp path").to_owned()
}

fn run_perf(args: &[&str], runner: &mut FakeRunner) -> (Result<u8, CliError>, Value) {
    let mut out = Vec::new();
    let code = run(&perf_args(args), runner, &mut out).map(crate::exit_codes::Code::value);
    let envelope = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).expect("one JSON envelope")
    };
    (code, envelope)
}

#[test]
fn a_neutral_delta_from_seeded_runs_reverts() {
    let dir = tempfile::tempdir().expect("temp dir");
    // Baseline and candidate come from one seeded distribution: the change did nothing.
    let file = baseline(dir.path(), &seeded_micros(36, 10, 200_000, 10_000));
    let mut runner = FakeRunner::timed(&seeded_micros(35, 10, 200_000, 10_000));

    let (code, envelope) = run_perf(&["noise", "--baseline", &file, "--", "bench"], &mut runner);

    assert_eq!(code.ok(), Some(1), "neutral means revert: {envelope}");
    assert!(
        envelope["data"]["delta_secs"].as_f64() < Some(0.0),
        "the candidate's mean is lower, but within the noise: {envelope}"
    );
    assert_eq!(
        (
            &envelope["data"]["decision"],
            &envelope["data"]["verdict"],
            &envelope["findings"][0]["key"],
        ),
        (&json!("revert"), &json!("neutral"), &json!("perf/neutral")),
        "{envelope}"
    );
}

#[test]
fn a_delta_that_clears_the_interval_downwards_keeps_the_change() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = baseline(dir.path(), &seeded_micros(1, 10, 200_000, 2_000));
    let mut runner = FakeRunner::timed(&seeded_micros(2, 10, 150_000, 2_000));

    let (code, envelope) = run_perf(&["noise", "--baseline", &file, "--", "bench"], &mut runner);

    assert_eq!(code.ok(), Some(0), "faster beyond the noise: {envelope}");
    assert_eq!(
        (&envelope["ok"], &envelope["data"]["decision"]),
        (&json!(true), &json!("keep")),
        "{envelope}"
    );
}

#[test]
fn a_regression_beyond_the_interval_reverts() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = baseline(dir.path(), &seeded_micros(3, 10, 150_000, 2_000));
    let mut runner = FakeRunner::timed(&seeded_micros(4, 10, 200_000, 2_000));

    let (code, envelope) = run_perf(&["noise", "--baseline", &file, "--", "bench"], &mut runner);

    assert_eq!(
        (code.ok(), &envelope["findings"][0]["key"]),
        (Some(1), &json!("perf/slower")),
        "{envelope}"
    );
}

#[test]
fn noise_runs_the_command_n_times_with_the_timeout() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = baseline(dir.path(), &[100, 200, 300]);
    let mut runner = FakeRunner::timed(&[100, 200, 300, 400]);

    let (code, _) = run_perf(
        &[
            "noise",
            "-n",
            "4",
            "--timeout",
            "9",
            "--baseline",
            &file,
            "--",
            "bench",
            "--quick",
        ],
        &mut runner,
    );

    assert!(code.is_ok(), "compared");
    let expected = (
        vec![OsString::from("bench"), OsString::from("--quick")],
        Duration::from_secs(9),
    );
    assert_eq!(runner.seen, vec![expected; 4], "four runs, as given");
}

#[test]
fn sample_saves_the_run_times_and_noise_reads_them_back() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("base.json");
    let out_arg = out.to_str().expect("UTF-8");
    let mut sampler = FakeRunner::timed(&[1_000, 2_000, 3_000]);

    let (code, envelope) = run_perf(
        &["sample", "-n", "3", "--out", out_arg, "--", "bench"],
        &mut sampler,
    );

    assert_eq!(code.ok(), Some(0), "{envelope}");
    let saved: Value = serde_json::from_slice(&std::fs::read(&out).expect("saved")).expect("JSON");
    assert_eq!(
        saved,
        json!({"version": 1, "command": ["bench"], "samples_secs": [0.001, 0.002, 0.003]}),
        "the sample file"
    );
    let mut runner = FakeRunner::timed(&[1_000, 2_000, 3_000]);
    let (code, envelope) = run_perf(
        &["noise", "-n", "3", "--baseline", out_arg, "--", "bench"],
        &mut runner,
    );
    assert_eq!(
        (code.ok(), &envelope["data"]["baseline"]["runs"]),
        (Some(1), &json!(3)),
        "identical runs are neutral: {envelope}"
    );
}

#[test]
fn a_failed_run_is_an_internal_error_and_a_command_that_cannot_start_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = baseline(dir.path(), &[100, 200]);
    let mut failing = FakeRunner {
        runs: VecDeque::from([
            Ok(Duration::from_micros(5)),
            Err(RunError::Failed { exit: Some(1) }),
        ]),
        seen: Vec::new(),
    };
    let mut missing = FakeRunner {
        runs: VecDeque::from([Err(RunError::NotStarted {
            program: "bench".to_owned(),
            source: io::Error::from(io::ErrorKind::NotFound),
        })]),
        seen: Vec::new(),
    };

    let (failed, failed_out) =
        run_perf(&["noise", "--baseline", &file, "--", "bench"], &mut failing);
    let (not_started, _) = run_perf(&["noise", "--baseline", &file, "--", "bench"], &mut missing);

    assert_eq!(
        (
            failed.map_err(|error| error.code().value()),
            not_started.map_err(|error| error.code().value()),
        ),
        (Err(70), Err(2)),
        "no envelope for either"
    );
    assert_eq!(failed_out, Value::Null, "nothing on stdout");
    assert_eq!(failing.seen.len(), 2, "it stops at the failed run");
}

#[test]
fn a_timed_out_run_stops_the_command_as_an_internal_error_naming_the_run() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = baseline(dir.path(), &[100, 200]);
    let mut runner = FakeRunner {
        runs: VecDeque::from([
            Ok(Duration::from_micros(5)),
            Err(RunError::TimedOut { secs: 9 }),
            Ok(Duration::from_micros(5)),
        ]),
        seen: Vec::new(),
    };

    let (code, out) = run_perf(
        &["noise", "-n", "3", "--baseline", &file, "--", "bench"],
        &mut runner,
    );

    let error = code.expect_err("a timed-out run is an error");
    assert_eq!(
        (error.code().value(), crate::error_chain::reason(&error)),
        (
            70,
            "perf: run 2 of 3: the command ran past its 9 s timeout and was killed".to_owned()
        ),
        "the run and its timeout are named"
    );
    assert_eq!(
        (out, runner.seen.len()),
        (Value::Null, 2),
        "nothing on stdout, no run after it"
    );
}

#[test]
fn a_baseline_of_another_version_or_with_one_run_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let other = dir.path().join("v2.json");
    std::fs::write(
        &other,
        r#"{"version": 2, "command": [], "samples_secs": [1, 2]}"#,
    )
    .expect("writes");
    let one = baseline(dir.path(), &[100]);

    for file in [other.to_str().expect("UTF-8"), &one] {
        let mut runner = FakeRunner::timed(&[]);
        let (code, _) = run_perf(&["noise", "--baseline", file, "--", "bench"], &mut runner);
        assert_eq!(
            code.map_err(|error| error.code().value()),
            Err(2),
            "{file} is refused"
        );
        assert!(runner.seen.is_empty(), "{file}: nothing runs");
    }
}

#[test]
fn runs_outside_2_to_100_are_a_usage_error() {
    for n in ["1", "101"] {
        let parsed = Cli::try_parse_from([
            "anneal",
            "perf",
            "noise",
            "-n",
            n,
            "--baseline",
            "b",
            "--",
            "bench",
        ]);
        assert!(parsed.is_err(), "-n {n} is refused");
    }
}
