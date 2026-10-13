//! `anneal eval uplift` (Tier 3) against `tests/fixtures/eval-uplift/`, whose plugin is
//! `tests/fixtures/eval-routing/` (its manifest names it `fixture`). No test reaches a model:
//! the evaluator is a missing program, or on unix a fake `claude` script that answers every
//! batch from its arguments.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn plugin_skills() -> PathBuf {
    fixtures().join("eval-routing/skills")
}

/// `anneal eval uplift --skills <fixture skills> --suites <suites>` plus `args`.
fn eval_uplift(suites: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["eval", "uplift", "--skills"])
        .arg(plugin_skills())
        .arg("--suites")
        .arg(suites)
        .args(args)
        .output()
}

fn envelope(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    })
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_reads_the_suites_reports_their_coverage_and_runs_nothing() -> TestResult {
    let output = eval_uplift(&fixtures().join("eval-uplift/suites"), &["--check"])?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            &envelope["command"],
            &envelope["data"]["mode"],
            &envelope["data"]["cases"],
            &envelope["data"]["runs"],
            &envelope["data"]["coverage"],
        ),
        (
            &json!("eval uplift"),
            &json!("check"),
            &json!(3),
            &json!(0),
            &json!([
                {"skill": "plan", "regression": 1, "capability": 1},
                {"skill": "spec", "regression": 1, "capability": 0}
            ]),
        )
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn invalid_suites_exit_2_before_anything_runs() -> TestResult {
    let work = tempfile::tempdir()?;
    let missing = work.path().join("no-such-claude");

    let output = eval_uplift(
        &fixtures().join("eval-uplift/invalid"),
        &["--claude", &missing.to_string_lossy()],
    )?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(
        (&envelope["data"]["runs"], &envelope["findings"]),
        (
            &json!(0),
            &json!([{
                "key": "eval/uplift/suite:plan/regression/llm-first.json",
                "message": "plan/regression/llm-first.json: llm grader `judge` comes before a deterministic grader; llm graders go last"
            }])
        ),
        "nothing ran: a missing claude would have exited 4"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_skill_with_no_suite_named_by_skill_is_a_usage_error() -> TestResult {
    let output = eval_uplift(
        &fixtures().join("eval-uplift/suites"),
        &["--check", "--skill", "diagnose"],
    )?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR eval: --skill diagnose names no skill with suites under the suites directory\n"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_run_flag_with_check_is_a_usage_error() -> TestResult {
    let output = eval_uplift(
        &fixtures().join("eval-uplift/suites"),
        &["--check", "--max-cost-usd", "2"],
    )?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR --check runs nothing, so --max-cost-usd does not apply\n"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_claude_that_cannot_start_exits_4_with_one_error_line() -> TestResult {
    let work = tempfile::tempdir()?;
    let missing = work.path().join("no-such-claude");

    let output = eval_uplift(
        &fixtures().join("eval-uplift/suites"),
        &["--claude", &missing.to_string_lossy()],
    )?;

    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(4), "{stderr}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert!(
        stderr.starts_with("ERROR eval: ")
            && stderr.contains("no-such-claude")
            && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git and the binary")]
fn with_nothing_changed_since_the_base_nothing_runs_and_it_exits_0() -> TestResult {
    let repo = tempfile::tempdir()?;
    crate::support::copy_dir(
        &fixtures().join("eval-routing"),
        &repo.path().join("plugin"),
    )?;
    crate::support::copy_dir(
        &fixtures().join("eval-uplift/suites"),
        &repo.path().join("evals/uplift"),
    )?;
    crate::support::git(repo.path(), &["init", "-q"]);
    crate::support::git(repo.path(), &["add", "-A"]);
    crate::support::git(repo.path(), &["commit", "-q", "-m", "base"]);
    let missing = repo.path().join("no-such-claude");

    let output = Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["eval", "uplift", "--skills"])
        .arg(repo.path().join("plugin/skills"))
        .arg("--suites")
        .arg(repo.path().join("evals/uplift"))
        .args(["--changed-since", "HEAD", "--claude"])
        .arg(&missing)
        .output()?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (&envelope["data"]["skipped"], &envelope["data"]["cases"]),
        (&json!(true), &json!(0)),
        "a missing claude would have exited 4"
    );
    Ok(())
}

/// A `claude` that logs `<plugin copy> <eval dir> <budget> <runs> <allowed tools>` per batch to
/// `$FAKE_CLAUDE_LOG`, and answers that `always` passed in every run of every case and
/// `outcome` passed only against the whole plugin (`plugin/`), or in every run when
/// `$FAKE_BASE_PASSES` is set, at $0.25 a batch. With `$FAKE_FAILS` set it writes no result
/// and exits 1. A batch against the `baseline/` copy lists its plugin as `fixture-baseline`,
/// the name in `eval-uplift/baseline-plugin`'s manifest; any other as `fixture`.
#[cfg(unix)]
const FAKE_CLAUDE: &str = r#"#!/bin/sh
set -eu
target=$3
tools=''
while [ $# -gt 0 ]; do
  case "$1" in
    --eval-dir) evaldir=$2; shift ;;
    --json) out=$2; shift ;;
    --max-cost-usd) budget=$2; shift ;;
    --runs) runs=$2; shift ;;
    --allow-tools) tools=$2; shift ;;
  esac
  shift
done
name=$(basename "$target")
echo "$name $evaldir $budget $runs $tools" >> "$FAKE_CLAUDE_LOG"
if [ -n "${FAKE_FAILS:-}" ]; then echo 'Error: not logged in' >&2; exit 1; fi
if [ "$name" = baseline ]; then listed=fixture-baseline; else listed=fixture; fi
if [ "$name" = plugin ] || [ -n "${FAKE_BASE_PASSES:-}" ]; then passed=true; else passed=false; fi
run="{\"error\":null,\"graders\":[{\"name\":\"outcome\",\"passed\":$passed},{\"name\":\"always\",\"passed\":true}]}"
cases=''
for dir in "$target/$evaldir"/*/; do
  case_name=$(basename "$dir")
  one="{\"name\":\"$case_name\",\"arms\":{\"with\":[$run,$run,$run]}}"
  if [ -z "$cases" ]; then cases=$one; else cases="$cases,$one"; fi
done
printf '{"schemaVersion":1,"costUsd":0.25,"partial":false,"suite":{"plugins":[{"name":"%s","path":"%s"}]},"cases":[%s]}' "$listed" "$target" "$cases" > "$out"
"#;

/// A run of `anneal eval uplift` against a fake `claude`, with its own `TMPDIR` so the test
/// can see what the run left behind there.
#[cfg(unix)]
struct FakeRun {
    output: Output,
    /// Holds the fake and its log.
    dir: tempfile::TempDir,
    /// The run's `TMPDIR`.
    tmp: tempfile::TempDir,
}

#[cfg(unix)]
impl FakeRun {
    /// Runs the fixture suites against the fake, with `env` set.
    fn new(env: &[(&str, &str)]) -> Result<Self, Box<dyn Error>> {
        Self::with_args(env, &[])
    }

    /// Runs the fixture suites against the fake, with `env` set and `args` added.
    fn with_args(env: &[(&str, &str)], args: &[&std::ffi::OsStr]) -> Result<Self, Box<dyn Error>> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir()?;
        let tmp = tempfile::tempdir()?;
        let claude = dir.path().join("claude");
        std::fs::write(&claude, FAKE_CLAUDE)?;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755))?;
        let output = Command::new(env!("CARGO_BIN_EXE_anneal"))
            .args(["eval", "uplift", "--skills"])
            .arg(plugin_skills())
            .arg("--suites")
            .arg(fixtures().join("eval-uplift/suites"))
            .arg("--claude")
            .arg(&claude)
            .args(args)
            .env("FAKE_CLAUDE_LOG", dir.path().join("claude.log"))
            .env("TMPDIR", tmp.path())
            .envs(env.iter().copied())
            .output()?;
        Ok(Self { output, dir, tmp })
    }

    fn log(&self) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir.path().join("claude.log"))
    }

    fn left_in_tmp(&self) -> std::io::Result<Vec<String>> {
        std::fs::read_dir(self.tmp.path())?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect()
    }
}

/// The `fields` of each entry of the array at `pointer`, as one JSON array per entry.
#[cfg(unix)]
fn project(envelope: &Value, pointer: &str, fields: &[&str]) -> Result<Vec<Value>, Box<dyn Error>> {
    Ok(envelope
        .pointer(pointer)
        .and_then(Value::as_array)
        .ok_or("an array")?
        .iter()
        .map(|entry| {
            json!(
                fields
                    .iter()
                    .map(|field| entry.get(field))
                    .collect::<Vec<_>>()
            )
        })
        .collect())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_skill_that_beats_its_baseline_passes_and_the_non_discriminating_assertions_are_listed()
-> TestResult {
    let run = FakeRun::new(&[])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        run.left_in_tmp()?,
        Vec::<String>::new(),
        "workspace removed"
    );
    assert_eq!(
        run.log()?,
        "plugin anneal-uplift-1 12.50 3 \n\
         without-plan anneal-uplift-2 24.75 3 \n\
         without-spec anneal-uplift-3 24.50 3 \n",
        "the with arm with half the $25 cap, then each skill's baseline with what is left"
    );
    assert_eq!(
        project(
            &envelope,
            "/data/per_skill",
            &["skill", "with", "base", "passed"]
        )?,
        [
            json!([
                "plan",
                {"passed": 6, "trials": 6},
                {"passed": 0, "trials": 6},
                true
            ]),
            json!([
                "spec",
                {"passed": 3, "trials": 3},
                {"passed": 0, "trials": 3},
                true
            ]),
        ]
    );
    assert_eq!(
        project(&envelope, "/data/non_discriminating", &["case", "grader"])?,
        [
            json!(["plan/capability/estimate.json", "always"]),
            json!(["plan/regression/break-down.json", "always"]),
            json!(["spec/regression/criteria.json", "always"]),
        ]
    );
    assert_eq!(
        (
            &envelope["data"]["graduates"],
            &envelope["data"]["cost_usd"],
            &envelope["data"]["complete"]
        ),
        (&json!(["plan"]), &json!(0.75), &json!(true))
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_skill_that_does_no_better_than_its_baseline_is_below_bound() -> TestResult {
    let run = FakeRun::new(&[("FAKE_BASE_PASSES", "1")])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(1), "{envelope}");
    let keys: Vec<&str> = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?
        .iter()
        .filter_map(|f| f["key"].as_str())
        .filter(|key| key.starts_with("eval/uplift/uplift:"))
        .collect();
    assert_eq!(keys, ["eval/uplift/uplift:plan", "eval/uplift/uplift:spec"]);
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_first_batch_that_fails_after_claude_started_exits_4_saying_its_spend_is_unknown() -> TestResult
{
    let run = FakeRun::new(&[("FAKE_FAILS", "1")])?;

    let stderr = std::str::from_utf8(&run.output.stderr)?;
    assert_eq!(run.output.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.starts_with("ERROR eval: ")
            && stderr.contains("spend is unknown")
            && stderr.contains("Error: not logged in")
            && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    assert_eq!(
        run.left_in_tmp()?,
        Vec::<String>::new(),
        "workspace removed"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_baseline_plugin_runs_every_case_against_its_copy_checked_under_its_own_name() -> TestResult {
    let baseline = fixtures().join("eval-uplift/baseline-plugin");
    let run = FakeRun::with_args(&[], &["--baseline".as_ref(), baseline.as_os_str()])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(
        run.output.status.code(),
        Some(0),
        "a result checked under `fixture` would not list the baseline plugin: {envelope}"
    );
    assert_eq!(
        run.log()?,
        "plugin anneal-uplift-1 12.50 3 \n\
         baseline anneal-uplift-2 24.75 3 \n",
        "the with arm, then one batch of every case against the baseline copy"
    );
    assert_eq!(
        (
            &envelope["data"]["baseline"],
            &envelope["data"]["complete"],
            &envelope["data"]["runs"]
        ),
        (&json!("plugin"), &json!(true), &json!(18))
    );
    assert_eq!(
        run.left_in_tmp()?,
        Vec::<String>::new(),
        "workspace removed"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_baseline_that_cannot_be_copied_exits_70_and_leaves_no_workspace() -> TestResult {
    let baseline = tempfile::tempdir()?;
    crate::support::copy_dir(
        &fixtures().join("eval-uplift/baseline-plugin"),
        baseline.path(),
    )?;
    // The copy refuses a symlink, so the baseline copy fails after the workspace was made.
    std::os::unix::fs::symlink("/", baseline.path().join("link"))?;

    let run = FakeRun::with_args(&[], &["--baseline".as_ref(), baseline.path().as_os_str()])?;

    let stderr = std::str::from_utf8(&run.output.stderr)?;
    assert_eq!(run.output.status.code(), Some(70), "{stderr}");
    assert_eq!(
        (
            run.left_in_tmp()?,
            std::fs::read_to_string(run.dir.path().join("claude.log")).ok()
        ),
        (Vec::<String>::new(), None),
        "workspace removed and claude never started"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_baseline_without_a_plugin_manifest_is_a_usage_error() -> TestResult {
    let work = tempfile::tempdir()?;
    let missing = work.path().join("no-such-claude");
    let baseline = work.path().join("not-a-plugin");
    std::fs::create_dir(&baseline)?;

    let output = eval_uplift(
        &fixtures().join("eval-uplift/suites"),
        &[
            "--claude",
            &missing.to_string_lossy(),
            "--baseline",
            &baseline.to_string_lossy(),
        ],
    )?;

    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert!(
        stderr.starts_with("ERROR eval: reading the --baseline plugin")
            && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    Ok(())
}
