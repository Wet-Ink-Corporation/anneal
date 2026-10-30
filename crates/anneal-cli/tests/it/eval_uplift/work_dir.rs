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
    let logs = tempfile::tempdir()?;
    let log = logs.path().join("tmpdir.log");
    let run = FakeRun::with_args(&[("FAKE_TMPDIR_LOG", log.to_str().ok_or("UTF-8")?)], args)?;
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
