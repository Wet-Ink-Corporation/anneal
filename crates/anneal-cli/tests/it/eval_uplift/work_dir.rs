//! Where a Tier 3 run's workspace lives (`--work-dir`, else `$TMPDIR`), and the `TMPDIR` the
//! `claude` child gets: a fresh directory inside the workspace, so `claude plugin eval`'s run
//! roots are made there rather than under the system's `/tmp`, where its Linux sandbox denies
//! Bash every write.

#![cfg(unix)]

use std::error::Error;
use std::fs;
use std::path::Path;

use super::{FakeRun, TestResult, envelope, eval_uplift};

/// Runs the fixture suites against the fake with `args`, and returns the run and each batch's
/// `TMPDIR` line (`<path> dir|missing`).
fn tmpdir_run(args: &[&std::ffi::OsStr]) -> Result<(FakeRun, Vec<String>), Box<dyn Error>> {
    tmpdir_run_from(None, &[], args)
}

/// As [`tmpdir_run`], with `env` set too, from the current directory `cwd` if given.
fn tmpdir_run_from(
    cwd: Option<&Path>,
    env: &[(&str, &str)],
    args: &[&std::ffi::OsStr],
) -> Result<(FakeRun, Vec<String>), Box<dyn Error>> {
    let logs = tempfile::tempdir()?;
    let log = logs.path().join("tmpdir.log");
    let mut env = env.to_vec();
    env.push(("FAKE_TMPDIR_LOG", log.to_str().ok_or("UTF-8")?));
    let run = match cwd {
        Some(cwd) => FakeRun::in_cwd(cwd, &env, args)?,
        None => FakeRun::with_args(&env, args)?,
    };
    let lines = fs::read_to_string(&log)?
        .lines()
        .map(str::to_owned)
        .collect();
    Ok((run, lines))
}

/// Whether `line` names an existing directory `tmp` directly inside an
/// `anneal-eval-uplift-*` workspace directly inside `base`.
fn is_workspace_tmp(line: &str, base: &Path) -> bool {
    let Some(path) = line.strip_suffix(" dir").map(Path::new) else {
        return false;
    };
    let workspace = path.parent();
    path.file_name() == Some("tmp".as_ref())
        && workspace.and_then(Path::parent) == Some(base)
        && workspace
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("anneal-eval-uplift-"))
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_batch_gets_a_tmpdir_inside_the_workspace() -> TestResult {
    let (run, lines) = tmpdir_run(&[])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            lines.len(),
            lines
                .iter()
                .all(|line| is_workspace_tmp(line, run.tmp.path()))
        ),
        (3, true),
        "{lines:?}"
    );
    assert_eq!(
        run.left_in_tmp()?,
        Vec::<String>::new(),
        "workspace removed"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn work_dir_places_the_workspace_and_is_left_empty() -> TestResult {
    let work = tempfile::tempdir()?;
    let base = work.path().join("made/for/eval");

    let (run, lines) = tmpdir_run(&["--work-dir".as_ref(), base.as_os_str()])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            lines.iter().all(|line| is_workspace_tmp(line, &base)),
            fs::read_dir(&base)?.count(),
            run.left_in_tmp()?,
        ),
        (true, 0, Vec::<String>::new()),
        "{lines:?}"
    );
    Ok(())
}

/// A current directory for a run, resolved so it compares equal to what the run sees.
fn resolved_cwd() -> Result<(tempfile::TempDir, std::path::PathBuf), Box<dyn Error>> {
    let cwd = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let resolved = fs::canonicalize(cwd.path())?;
    Ok((cwd, resolved))
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_relative_work_dir_is_taken_from_the_current_directory() -> TestResult {
    let (_cwd, cwd) = resolved_cwd()?;
    let base = cwd.join("rel/base");

    let (run, lines) = tmpdir_run_from(
        Some(&cwd),
        &[],
        &["--work-dir".as_ref(), "rel/base".as_ref()],
    )?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            lines.len(),
            lines.iter().all(|line| is_workspace_tmp(line, &base)),
            fs::read_dir(&base)?.count(),
        ),
        (3, true, 0),
        "{lines:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_relative_tmpdir_is_taken_from_the_current_directory() -> TestResult {
    let (_cwd, cwd) = resolved_cwd()?;
    let base = cwd.join("rel-tmp");

    let (run, lines) = tmpdir_run_from(Some(&cwd), &[("TMPDIR", "rel-tmp")], &[])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            lines.len(),
            lines.iter().all(|line| is_workspace_tmp(line, &base)),
            fs::read_dir(&base)?.count(),
        ),
        (3, true, 0),
        "{lines:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_relative_anneal_home_the_default_work_dir_needs_stops_the_run() -> TestResult {
    let run = FakeRun::with_args(&[("TMPDIR", ""), ("ANNEAL_HOME", "rel/home")], &[])?;

    assert_eq!(
        (
            run.output.status.code(),
            std::str::from_utf8(&run.output.stderr)?,
            run.log().is_err(),
        ),
        (
            Some(70),
            "ERROR eval: finding the default work directory: ANNEAL_HOME must be an absolute \
             path, got rel/home\n",
            true
        )
    );
    Ok(())
}

/// The `eval/uplift/read-only-sandbox` findings of a run of the gated suites (one case names
/// Bash) with its workspace made in `work_dir`.
#[cfg(target_os = "linux")]
fn read_only_findings(work_dir: &Path) -> Result<Vec<serde_json::Value>, Box<dyn Error>> {
    let run = FakeRun::with_suites(
        &super::fixtures().join("eval-uplift/gated"),
        &[],
        &[
            "--allow-gated-tools".as_ref(),
            "--work-dir".as_ref(),
            work_dir.as_os_str(),
        ],
    )?;
    let mut envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    let Some(serde_json::Value::Array(findings)) =
        envelope.get_mut("findings").map(serde_json::Value::take)
    else {
        return Err("findings is an array".into());
    };
    Ok(findings
        .into_iter()
        .filter(|finding| {
            finding.get("key").and_then(serde_json::Value::as_str)
                == Some("eval/uplift/read-only-sandbox")
        })
        .collect())
}

#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(miri, ignore = "runs the binary")]
fn on_linux_a_bash_case_run_under_tmp_is_flagged_and_one_elsewhere_is_not() -> TestResult {
    let under_tmp = tempfile::Builder::new().tempdir_in("/tmp")?;
    let elsewhere = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;

    let flagged = read_only_findings(under_tmp.path())?;
    let unflagged = read_only_findings(elsewhere.path())?;

    assert_eq!(
        (
            flagged
                .iter()
                .map(
                    |finding| finding["message"].as_str().is_some_and(|message| {
                        message.contains(&format!("made in {}, ", under_tmp.path().display()))
                    })
                )
                .collect::<Vec<_>>(),
            unflagged.len()
        ),
        (vec![true], 0)
    );
    Ok(())
}

/// What a [`socket_run`] showed.
#[cfg(target_os = "linux")]
struct SocketRun {
    /// The exit code.
    code: Option<i32>,
    /// Everything on stderr.
    stderr: String,
    /// The `eval/uplift/socket-path` findings' messages.
    messages: Vec<String>,
    /// The `TMPDIR` each batch got.
    tmpdirs: Vec<String>,
}

/// A run of the gated suites (one case names Bash) against the fake, with its workspace made
/// in `work_dir` and `env` set.
#[cfg(target_os = "linux")]
fn socket_run(work_dir: &Path, env: &[(&str, &str)]) -> Result<SocketRun, Box<dyn Error>> {
    let logs = tempfile::tempdir()?;
    let log = logs.path().join("tmpdir.log");
    let mut env = env.to_vec();
    env.push(("FAKE_TMPDIR_LOG", log.to_str().ok_or("UTF-8")?));
    let run = FakeRun::with_suites(
        &super::fixtures().join("eval-uplift/gated"),
        &env,
        &[
            "--allow-gated-tools".as_ref(),
            "--work-dir".as_ref(),
            work_dir.as_os_str(),
        ],
    )?;
    let envelope = envelope(&run.output)?;
    let messages = envelope
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding.get("key").and_then(serde_json::Value::as_str)
                == Some("eval/uplift/socket-path")
        })
        .filter_map(|finding| finding.get("message").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect();
    let tmpdirs = fs::read_to_string(&log)?
        .lines()
        .filter_map(|line| line.strip_suffix(" dir").map(str::to_owned))
        .collect();
    Ok(SocketRun {
        code: run.output.status.code(),
        stderr: std::str::from_utf8(&run.output.stderr)?.to_owned(),
        messages,
        tmpdirs,
    })
}

#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_child_tmpdir_over_the_socket_budget_is_warned_before_any_batch() -> TestResult {
    let work = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let long = fs::canonicalize(work.path())?.join("a-work-directory-long-enough-to-overrun");

    let SocketRun {
        code,
        stderr,
        messages,
        tmpdirs,
    } = socket_run(&long, &[])?;

    assert_eq!(code, Some(0), "{stderr}");
    let tmpdir = tmpdirs.first().ok_or("a batch ran")?;
    let warning = stderr.lines().next().unwrap_or_default();
    assert_eq!(
        (
            warning.strip_prefix("WARN eval: "),
            stderr.lines().count(),
            messages.len(),
            warning.contains(&format!("is {} bytes", tmpdir.len())),
            warning.contains(&long.display().to_string()),
        ),
        (messages.first().map(String::as_str), 1, 1, true, true),
        "{stderr}"
    );
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(miri, ignore = "runs the binary")]
fn the_socket_warning_comes_before_a_first_batch_that_fails() -> TestResult {
    let work = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
    let long = work.path().join("a-work-directory-long-enough-to-overrun");

    let SocketRun { code, stderr, .. } = socket_run(&long, &[("FAKE_FAILS", "1")])?;

    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        (
            code,
            lines.len(),
            lines
                .first()
                .is_some_and(|line| line.starts_with("WARN eval: ")),
            lines
                .get(1)
                .is_some_and(|line| line.starts_with("ERROR eval: ")),
        ),
        (Some(4), 2, true, true),
        "{stderr}"
    );
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_short_work_dir_is_not_warned() -> TestResult {
    let short = tempfile::Builder::new().tempdir_in("/tmp")?;

    let SocketRun {
        code,
        stderr,
        messages,
        tmpdirs,
    } = socket_run(short.path(), &[])?;

    assert_eq!(
        (
            code,
            stderr.as_str(),
            messages.len(),
            tmpdirs.len(),
            tmpdirs.iter().all(|tmpdir| tmpdir.len() <= 49),
        ),
        (Some(0), "", 0, 3, true),
        "{tmpdirs:?}"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn keep_results_keeps_each_batch_s_result_named_by_its_arm() -> TestResult {
    let kept = tempfile::tempdir()?;
    let dir = kept.path().join("r5");

    let run = FakeRun::with_args(&[], &["--keep-results".as_ref(), dir.as_os_str()])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    let mut names: Vec<String> = fs::read_dir(&dir)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    names.sort();
    let first: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("result-1-with.json"))?)?;
    assert_eq!(
        (names, first["schemaVersion"].as_u64(), run.left_in_tmp()?),
        (
            vec![
                "result-1-with.json".to_owned(),
                "result-2-without-plan.json".to_owned(),
                "result-3-without-spec.json".to_owned(),
            ],
            Some(1),
            Vec::<String>::new()
        )
    );
    Ok(())
}

/// The files below `dir`, as paths relative to it, sorted.
#[cfg(target_os = "linux")]
fn files_below(dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir)?.to_str().ok_or("UTF-8")?;
                files.push(relative.to_owned());
            }
        }
    }
    files.sort();
    Ok(files)
}

#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(miri, ignore = "runs the binary")]
fn keep_results_keeps_each_run_s_trace_from_inside_the_workspace() -> TestResult {
    let kept = tempfile::tempdir()?;
    let dir = kept.path().join("r6");
    let outside = kept.path().join("outside.jsonl");
    fs::write(&outside, "not this run's")?;

    let run = FakeRun::with_args(
        &[("FAKE_TRACES", outside.to_str().ok_or("UTF-8")?)],
        &["--keep-results".as_ref(), dir.as_os_str()],
    )?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    let traces: Vec<String> = files_below(&dir)?
        .into_iter()
        .filter(|file| file.starts_with("traces/"))
        .collect();
    let expected: Vec<String> = [
        ("1-with", "plan--capability--estimate"),
        ("1-with", "plan--regression--break-down"),
        ("1-with", "spec--regression--criteria"),
        ("2-without-plan", "plan--capability--estimate"),
        ("2-without-plan", "plan--regression--break-down"),
        ("3-without-spec", "spec--regression--criteria"),
    ]
    .iter()
    .flat_map(|(batch, case)| [1, 2].map(|i| format!("traces/{batch}/{case}-{i}.jsonl")))
    .collect();
    let unkept = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?
        .iter()
        .filter(|finding| {
            finding["key"]
                .as_str()
                .is_some_and(|key| key.starts_with("eval/uplift/keep-results"))
        })
        .count();
    assert_eq!(
        (
            traces,
            fs::read_to_string(
                dir.join("traces/2-without-plan/plan--regression--break-down-2.jsonl")
            )?,
            unkept,
            run.left_in_tmp()?,
        ),
        (
            expected,
            "{\"eval\":\"anneal-uplift-2\",\"case\":\"plan--regression--break-down\",\"run\":2}\n"
                .to_owned(),
            0,
            Vec::<String>::new()
        ),
        "the third run's trace is outside the workspace and is skipped"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_result_that_cannot_be_kept_keeps_its_batch_s_runs_and_says_so() -> TestResult {
    let kept = tempfile::tempdir()?;
    // A directory where the first batch's result would go, so copying it there fails.
    fs::create_dir(kept.path().join("result-1-with.json"))?;
    let full = FakeRun::new(&[])?;

    let run = FakeRun::with_args(&[], &["--keep-results".as_ref(), kept.path().as_os_str()])?;

    let full = envelope(&full.output)?;
    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    let unkept: Vec<&str> = envelope["findings"]
        .as_array()
        .ok_or("findings is an array")?
        .iter()
        .filter_map(|finding| finding["key"].as_str())
        .filter(|key| key.starts_with("eval/uplift/keep-results"))
        .collect();
    assert_eq!(
        (
            &envelope["data"]["runs"],
            &envelope["data"]["cost_usd"],
            &envelope["data"]["cost_is_lower_bound"],
            unkept,
            kept.path().join("result-2-without-plan.json").is_file(),
        ),
        (
            &full["data"]["runs"],
            &full["data"]["cost_usd"],
            &full["data"]["cost_is_lower_bound"],
            vec!["eval/uplift/keep-results:result-1-with.json"],
            true
        )
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_refuses_work_dir_as_a_run_flag() -> TestResult {
    let work = tempfile::tempdir()?;

    let output = eval_uplift(
        &super::fixtures().join("eval-uplift/suites"),
        &[
            "--check",
            "--work-dir",
            work.path().to_str().ok_or("UTF-8")?,
        ],
    )?;

    assert_eq!(
        (output.status.code(), std::str::from_utf8(&output.stderr)?),
        (
            Some(2),
            "ERROR --check runs nothing, so --work-dir does not apply\n"
        )
    );
    Ok(())
}
