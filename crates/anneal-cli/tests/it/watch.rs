//! `anneal watch` through the built binary, with a fake `gh` first on PATH that answers from
//! the recorded fixtures in `tests/fixtures/watch/`: no network. The fake is a shell script,
//! so these run on unix only; every exit code is also covered in process by
//! `watch::tests`, on every platform.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

use crate::support::anneal_in;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The fake `gh`: answers each call with the fixture its environment names for that kind of
/// call, or fails as a logged-out gh does when `$FAKE_GH_FAIL` is set. With
/// `$FAKE_GH_INTERRUPT` set, after answering for the pull request it sends SIGINT to its
/// parent, the watch, as Ctrl-C in a terminal would.
const FAKE_GH: &str = r#"#!/bin/sh
if [ -n "$FAKE_GH_FAIL" ]; then
  echo 'gh: HTTP 401: Bad credentials (https://api.github.com)' >&2
  exit 1
fi
case "$*" in
  *graphql*) cat "$FAKE_GH_THREADS" ;;
  */check-runs*) cat "$FAKE_GH_CHECKS" ;;
  */status*) cat "$FAKE_GH_STATUS" ;;
  */commits/*) cat "$FAKE_GH_COMMIT" ;;
  */pulls/*)
    cat "$FAKE_GH_PULL"
    if [ -n "$FAKE_GH_INTERRUPT" ]; then
      kill -INT "$PPID"
    fi
    ;;
  *) echo "gh: the fake has no fixture for: $*" >&2; exit 1 ;;
esac
"#;

/// A scratch directory holding the fake `gh` in `bin/`.
struct FakeGhDir {
    dir: TempDir,
}

impl FakeGhDir {
    fn new() -> Result<Self, std::io::Error> {
        let dir = tempfile::tempdir()?;
        let bin = dir.path().join("bin");
        fs::create_dir(&bin)?;
        let gh = bin.join("gh");
        fs::write(&gh, FAKE_GH)?;
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755))?;
        Ok(Self { dir })
    }

    /// `anneal <args>` in the scratch directory, the fake `gh` first on PATH, answering with
    /// `fixtures` as `(variable, file)` pairs.
    fn anneal(&self, args: &[&str], fixtures: &[(&str, &str)]) -> Command {
        let mut path = OsString::from(self.dir.path().join("bin"));
        if let Some(system) = std::env::var_os("PATH") {
            path.push(":");
            path.push(system);
        }
        let mut command = anneal_in(self.dir.path());
        command
            .args(args)
            .env("PATH", path)
            .env_remove("FAKE_GH_FAIL")
            .env_remove("FAKE_GH_INTERRUPT")
            .env_remove("GH_REPO")
            .env("FAKE_GH_COMMIT", fixture_path("commit.json"));
        for (variable, file) in fixtures {
            command.env(variable, fixture_path(file));
        }
        command
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/watch")
        .join(name)
}

fn envelope(output: &Output) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(&output.stdout)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and a fake gh")]
fn failing_checks_exit_4_with_an_envelope_naming_them() -> TestResult {
    let gh = FakeGhDir::new()?;

    let output = gh
        .anneal(
            &["watch", "ci", "main", "--repo", "octo/app"],
            &[
                ("FAKE_GH_CHECKS", "check-runs-cancelled.json"),
                ("FAKE_GH_STATUS", "status-failure.json"),
            ],
        )
        .output()?;

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let envelope = envelope(&output)?;
    let names: Vec<&str> = envelope["data"]["blockers"]
        .as_array()
        .ok_or("blockers is a list")?
        .iter()
        .filter_map(|blocker| blocker["name"].as_str())
        .collect();
    assert_eq!(names, ["build", "deploy/preview"]);
    assert!(output.stderr.is_empty(), "{output:?}");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and a fake gh")]
fn a_zero_timeout_polls_once_and_exits_5_naming_what_is_pending() -> TestResult {
    let gh = FakeGhDir::new()?;

    let output = gh
        .anneal(
            &["watch", "ci", "main", "--timeout", "0"],
            &[
                ("FAKE_GH_CHECKS", "check-runs-queued.json"),
                ("FAKE_GH_STATUS", "status-none.json"),
            ],
        )
        .output()?;

    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let data = &envelope(&output)?["data"];
    assert_eq!(
        (&data["state"], &data["polls"], &data["pending"][0]["name"]),
        (
            &Value::from("timeout"),
            &Value::from(1),
            &Value::from("build")
        )
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and a fake gh")]
fn a_gh_that_cannot_query_exits_7_with_one_error_line_and_no_envelope() -> TestResult {
    let gh = FakeGhDir::new()?;

    let output = gh
        .anneal(&["watch", "pr", "7"], &[])
        .env("FAKE_GH_FAIL", "1")
        .output()?;

    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(output.stdout.is_empty(), "no envelope for an error");
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("ERROR watch: gh api repos/{owner}/{repo}/pulls/7 failed: gh: HTTP 401"),
        "{stderr}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary and a fake gh")]
fn ctrl_c_during_a_watch_ends_it_by_the_signal_with_nothing_on_stdout() -> TestResult {
    let gh = FakeGhDir::new()?;

    // The fake gh answers for the pull request, then interrupts the watch. Were SIGINT
    // ignored, the watch would poll to its 15 s timeout and exit 5, failing this test rather
    // than hanging it.
    let output = gh
        .anneal(
            &["watch", "pr", "7", "--timeout", "15"],
            &[
                ("FAKE_GH_PULL", "pull-unknown.json"),
                ("FAKE_GH_CHECKS", "check-runs-running.json"),
                ("FAKE_GH_STATUS", "status-pending.json"),
                ("FAKE_GH_THREADS", "threads-none.json"),
            ],
        )
        .env("FAKE_GH_INTERRUPT", "1")
        .output()?;

    assert_eq!(
        (output.status.code(), output.status.signal()),
        (None, Some(2)),
        "ended by SIGINT, which a shell reports as 130: {output:?}"
    );
    assert!(output.stdout.is_empty(), "no envelope: {output:?}");
    Ok(())
}
