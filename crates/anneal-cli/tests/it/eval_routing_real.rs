//! `anneal eval routing --router claude` (Tier 2) against the fixture sets in
//! `tests/fixtures/eval-routing/`, whose root is also a plugin (`.claude-plugin/plugin.json`
//! names it `fixture`). No test reaches a model: the router is a missing program, or on unix
//! a fake `claude` script that answers every batch from its arguments.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval-routing")
}

/// `anneal eval routing --skills <fixtures>/skills --fixtures <routing>` plus `args`.
fn eval_routing(routing: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["eval", "routing", "--skills"])
        .arg(fixtures().join("skills"))
        .arg("--fixtures")
        .arg(routing)
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
fn a_claude_only_flag_without_the_claude_router_is_a_usage_error() -> TestResult {
    let output = eval_routing(&fixtures().join("pass"), &["--max-cost-usd", "2"])?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(envelope(&output)?, Value::Null, "nothing on stdout");
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "ERROR --max-cost-usd needs --router claude\n"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_cost_cap_that_is_not_a_positive_amount_is_a_usage_error() -> TestResult {
    for cap in ["0", "-1", "NaN", "inf", "five"] {
        let output = eval_routing(
            &fixtures().join("pass"),
            &["--router", "claude", "--max-cost-usd", cap],
        )?;

        assert_eq!(output.status.code(), Some(2), "{cap}");
        assert!(output.stdout.is_empty(), "{cap}: nothing on stdout");
    }
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn invalid_fixtures_exit_2_before_the_router_runs() -> TestResult {
    let work = tempfile::tempdir()?;
    for name in ["spec.yaml", "diagnose.yaml"] {
        std::fs::copy(fixtures().join("pass").join(name), work.path().join(name))?;
    }
    std::fs::write(
        work.path().join("plan.yaml"),
        "positives:\n  - [not, a, prompt]\n",
    )?;
    let missing = work.path().join("no-such-claude");

    let output = eval_routing(
        work.path(),
        &["--router", "claude", "--claude", &missing.to_string_lossy()],
    )?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(
        (
            &envelope["data"]["router"],
            &envelope["data"]["runs"],
            &envelope["data"]["problems"]
        ),
        (&json!("claude"), &json!(0), &json!(1)),
        "nothing ran: a missing claude would have exited 4"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_claude_that_cannot_start_exits_4_with_one_error_line() -> TestResult {
    let work = tempfile::tempdir()?;
    let missing = work.path().join("no-such-claude");

    let output = eval_routing(
        &fixtures().join("pass"),
        &["--router", "claude", "--claude", &missing.to_string_lossy()],
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
fn with_no_description_changed_since_the_base_nothing_runs_and_it_exits_0() -> TestResult {
    let repo = tempfile::tempdir()?;
    crate::support::copy_dir(&fixtures(), &repo.path().join("plugin"))?;
    crate::support::git(repo.path(), &["init", "-q"]);
    crate::support::git(repo.path(), &["add", "-A"]);
    crate::support::git(repo.path(), &["commit", "-q", "-m", "base"]);
    let body = repo.path().join("plugin/skills/plan/SKILL.md");
    std::fs::write(&body, std::fs::read_to_string(&body)? + "\nA body edit.\n")?;
    let missing = repo.path().join("no-such-claude");

    let output = Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["eval", "routing", "--skills"])
        .arg(repo.path().join("plugin/skills"))
        .arg("--fixtures")
        .arg(repo.path().join("plugin/pass"))
        .args(["--router", "claude", "--changed-since", "HEAD", "--claude"])
        .arg(&missing)
        .output()?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (&envelope["data"]["skipped"], &envelope["data"]["runs"]),
        (&json!(true), &json!(0)),
        "a missing claude would have exited 4"
    );
    Ok(())
}

/// A `claude` that logs `<eval dir> <budget> <runs>` per batch to `$FAKE_CLAUDE_LOG` and
/// answers that every run of every case fired `plan` and answered, at $0.25 a batch.
#[cfg(unix)]
const FAKE_CLAUDE: &str = r#"#!/bin/sh
set -eu
plugin=$3
while [ $# -gt 0 ]; do
  case "$1" in
    --eval-dir) evaldir=$2; shift ;;
    --json) out=$2; shift ;;
    --max-cost-usd) budget=$2; shift ;;
    --runs) runs=$2; shift ;;
  esac
  shift
done
echo "$evaldir $budget $runs" >> "$FAKE_CLAUDE_LOG"
run='{"error":null,"graders":[{"name":"fired-plan","passed":true},{"name":"fired-spec","passed":false},{"name":"answered","passed":true}]}'
cases=''
for dir in "$plugin/$evaldir"/*/; do
  name=$(basename "$dir")
  one="{\"name\":\"$name\",\"arms\":{\"with\":[$run,$run,$run]}}"
  if [ -z "$cases" ]; then cases=$one; else cases="$cases,$one"; fi
done
printf '{"schemaVersion":1,"costUsd":0.25,"partial":false,"suite":{"plugins":[{"name":"fixture","path":"%s"}]},"cases":[%s]}' "$plugin" "$cases" > "$out"
"#;

/// A run of `anneal eval routing --router claude` against a fake `claude`, with its own
/// `TMPDIR` so the test can see what the run left behind there.
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
    /// Runs the fixture set `pass` against a `claude` that is the shell script `script`, with
    /// `args` after `--claude <fake>`.
    fn new(script: &str, args: &[&str]) -> Result<Self, Box<dyn Error>> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir()?;
        let tmp = tempfile::tempdir()?;
        let claude = dir.path().join("claude");
        std::fs::write(&claude, script)?;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755))?;
        let output = Command::new(env!("CARGO_BIN_EXE_anneal"))
            .args(["eval", "routing", "--skills"])
            .arg(fixtures().join("skills"))
            .arg("--fixtures")
            .arg(fixtures().join("pass"))
            .args(["--router", "claude", "--claude"])
            .arg(&claude)
            .args(args)
            .env("FAKE_CLAUDE_LOG", dir.path().join("claude.log"))
            .env("TMPDIR", tmp.path())
            .output()?;
        Ok(Self { output, dir, tmp })
    }

    /// What the fake logged.
    fn log(&self) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir.path().join("claude.log"))
    }

    /// The names left in the run's `TMPDIR`: its eval workspace should be gone.
    fn left_in_tmp(&self) -> std::io::Result<Vec<String>> {
        std::fs::read_dir(self.tmp.path())?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect()
    }
}

/// A `claude` that says why on stderr and exits 1 without writing a result.
#[cfg(unix)]
const FAILING_CLAUDE: &str =
    "#!/bin/sh\necho 'starting up' >&2\necho 'Error: not logged in' >&2\nexit 1\n";

/// A `claude` that never finishes, so only the time limit ends it.
#[cfg(unix)]
const HANGING_CLAUDE: &str = "#!/bin/sh\nexec sleep 30\n";

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_claude_that_fails_without_a_result_exits_4_with_its_stderr_and_leaves_no_workspace()
-> TestResult {
    let run = FakeRun::new(FAILING_CLAUDE, &[])?;

    let stderr = std::str::from_utf8(&run.output.stderr)?;
    assert_eq!(run.output.status.code(), Some(4), "{stderr}");
    assert!(run.output.stdout.is_empty(), "nothing on stdout");
    assert!(
        stderr.starts_with("ERROR eval: ")
            && stderr.contains("Error: not logged in")
            && stderr.lines().count() == 1,
        "{stderr:?}"
    );
    assert_eq!(run.left_in_tmp()?, Vec::<String>::new());
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_claude_that_outlives_the_time_limit_is_stopped_and_the_run_is_incomplete() -> TestResult {
    let run = FakeRun::new(HANGING_CLAUDE, &["--timeout", "1"])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(3), "{envelope}");
    assert_eq!(
        (
            &envelope["data"]["stop"],
            &envelope["data"]["runs"],
            &envelope["data"]["complete"],
            &envelope["data"]["cost_is_lower_bound"],
        ),
        (
            &json!({"kind": "time-limit"}),
            &json!(0),
            &json!(false),
            &json!(true)
        ),
        "the killed batch's spend is unknown"
    );
    assert_eq!(run.left_in_tmp()?, Vec::<String>::new());
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_router_that_always_fires_plan_passes_plans_should_fire_bar_only() -> TestResult {
    let run = FakeRun::new(FAKE_CLAUDE, &[])?;
    let output = &run.output;

    let envelope = envelope(output)?;
    assert_eq!(output.status.code(), Some(1), "below bound: {envelope}");
    assert_eq!(run.left_in_tmp()?, Vec::<String>::new());
    assert_eq!(
        run.log()?,
        "anneal-routing-1 25.00 3\nanneal-routing-2 24.75 3\nanneal-routing-3 24.50 3\n",
        "the default $25 cap (wi-92bc1b); 24 prompts, then plan's 8 twice more until 9 of 9; \
         each batch gets what is left"
    );
    let skills: Vec<(Value, Value, Value)> = envelope["data"]["per_skill"]
        .as_array()
        .ok_or("per_skill is an array")?
        .iter()
        .map(|s| {
            (
                s["skill"].clone(),
                s["should_fire"]["position"].clone(),
                s["should_not"]["position"].clone(),
            )
        })
        .collect();
    assert_eq!(
        skills,
        [
            (json!("diagnose"), json!("missed"), json!("met")),
            (json!("plan"), json!("met"), json!("missed")),
            (json!("spec"), json!("missed"), json!("met")),
        ]
    );
    assert_eq!(
        envelope["data"]["table"],
        json!([
            "expected \\ fired  diagnose  plan  spec",
            "diagnose                 0    24     0",
            "plan                     0    72     0",
            "spec                     0    24     0",
        ])
    );
    assert_eq!(
        (&envelope["data"]["cost_usd"], &envelope["data"]["complete"]),
        (&json!(0.75), &json!(true))
    );
    Ok(())
}
