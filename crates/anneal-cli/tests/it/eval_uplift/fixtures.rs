//! Tier 3 cases seeded from a fixture (`--allow-scaffolds`), and the running `anneal` placed in
//! every plugin copy, against a repository built in a temporary directory: its suites at
//! `evals/uplift/`, where `plan/regression/seeded.json` names the fixture `crate`.

use std::error::Error;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::{TestResult, envelope, eval_uplift};

/// The refusal a run of the seeded suites gives without `--allow-scaffolds`.
const SCAFFOLD_REFUSAL: &str = "ERROR eval: fixtures are named by 1 selected case \
                                (plan/regression/seeded.json), whose scaffold scripts run \
                                outside the eval sandbox with your rights; review them, then \
                                pass --allow-scaffolds to run them\n";

/// A case of the fake's graders (`outcome`, `always`), seeded from `fixture` when given (a
/// `null` fixture reads as none).
fn case_json(fixture: Option<&str>) -> String {
    json!({
        "behaviour": "outcome",
        "seed": "test fixture",
        "prompt": "Break the export feature into tasks.",
        "fixture": fixture,
        "graders": [
            {"type": "regex", "name": "outcome", "pattern": "task"},
            {"type": "regex", "name": "always", "pattern": "\\S"}
        ]
    })
    .to_string()
}

/// A repository holding `docs/guide.md` and suites at `evals/uplift/`: `plan`'s case seeded
/// from the fixture `crate` (`src/lib.rs`, and on unix a `docs` link to the repository's
/// `docs/`), `spec`'s case seeded from nothing.
fn seeded_repo() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let repo = tempfile::tempdir()?;
    let suites = repo.path().join("evals/uplift");
    fs::create_dir_all(repo.path().join("docs"))?;
    fs::write(repo.path().join("docs/guide.md"), "guide\n")?;
    let fixture = suites.join("fixtures/crate");
    fs::create_dir_all(fixture.join("src"))?;
    fs::write(fixture.join("src/lib.rs"), "pub fn f() {}\n")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../../../docs", fixture.join("docs"))?;
    for (case, fixture) in [
        ("plan/regression/seeded.json", Some("crate")),
        ("spec/regression/plain.json", None),
    ] {
        let path = suites.join(case);
        fs::create_dir_all(path.parent().ok_or("a parent")?)?;
        fs::write(path, case_json(fixture))?;
    }
    Ok(repo)
}

/// The suites of `repo`.
fn suites(repo: &Path) -> std::path::PathBuf {
    repo.join("evals/uplift")
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_case_naming_a_fixture_is_refused_without_allow_scaffolds() -> TestResult {
    let repo = seeded_repo()?;
    let missing = repo.path().join("no-such-claude");

    let output = eval_uplift(
        &suites(repo.path()),
        &["--claude", &missing.to_string_lossy()],
    )?;

    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing claude would have exited 4"
    );
    assert!(output.stdout.is_empty(), "nothing on stdout");
    assert_eq!(String::from_utf8(output.stderr)?, SCAFFOLD_REFUSAL);
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn check_lists_the_cases_seeded_from_fixtures_and_runs_nothing() -> TestResult {
    let repo = seeded_repo()?;

    let output = eval_uplift(&suites(repo.path()), &["--check"])?;

    let envelope = envelope(&output)?;
    assert_eq!(output.status.code(), Some(0), "{envelope}");
    assert_eq!(
        (
            &envelope["data"]["scaffolded"],
            &envelope["data"]["coverage"],
            &envelope["data"]["libexec_target"],
            &envelope["findings"],
        ),
        (
            &json!([{"case": "plan/regression/seeded.json", "fixture": "crate"}]),
            &json!([
                {"skill": "plan", "regression": 1, "capability": 0},
                {"skill": "spec", "regression": 1, "capability": 0}
            ]),
            &Value::Null,
            &json!([{
                "key": "eval/uplift/scaffolded",
                "message": "a run seeds 1 case from a fixture with a scaffold script that runs \
                            outside the eval sandbox with your rights \
                            (plan/regression/seeded.json); review those cases and fixtures \
                            before running them with --allow-scaffolds"
            }]),
        )
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_fixture_that_cannot_seed_a_run_is_a_suite_problem() -> TestResult {
    let repo = seeded_repo()?;
    fs::remove_dir_all(suites(repo.path()).join("fixtures/crate"))?;

    let output = eval_uplift(&suites(repo.path()), &["--check"])?;

    let envelope = envelope(&output)?;
    assert_eq!(
        (output.status.code(), &envelope["findings"]),
        (
            Some(2),
            &json!([{
                "key": "eval/uplift/suite:plan/regression/seeded.json",
                "message": "plan/regression/seeded.json: no fixture directory fixtures/crate"
            }])
        )
    );
    Ok(())
}

/// The launcher target this host's copies get, as `plugin/libexec/anneal` lists it first.
#[cfg(unix)]
fn host_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

/// Runs `suites` against the fake with `args`, the fake logging each batch's scaffold flag,
/// scaffold scripts and placed binary; returns the run and that log.
#[cfg(unix)]
fn seeds_run(
    suites: &Path,
    args: &[&std::ffi::OsStr],
) -> Result<(super::FakeRun, String), Box<dyn Error>> {
    let logs = tempfile::tempdir()?;
    let log = logs.path().join("seeds.log");
    let run = super::FakeRun::with_suites(
        suites,
        &[
            ("FAKE_SEEDS_LOG", log.to_str().ok_or("UTF-8")?),
            ("FAKE_ANNEAL", env!("CARGO_BIN_EXE_anneal")),
        ],
        args,
    )?;
    let seeds = fs::read_to_string(&log)?;
    Ok((run, seeds))
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_refused_scaffold_run_never_starts_claude_and_leaves_no_workspace() -> TestResult {
    let repo = seeded_repo()?;

    let run = super::FakeRun::with_suites(&suites(repo.path()), &[], &[])?;

    assert_eq!(
        (
            run.output.status.code(),
            run.output.stdout.is_empty(),
            std::str::from_utf8(&run.output.stderr)?,
        ),
        (Some(2), true, SCAFFOLD_REFUSAL)
    );
    assert_eq!(
        (run.left_in_tmp()?, run.log().ok()),
        (Vec::<String>::new(), None),
        "no workspace made and claude never started"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "runs the binary")]
fn with_allow_scaffolds_seeded_batches_run_scaffolds_and_every_copy_holds_the_running_anneal()
-> TestResult {
    let repo = seeded_repo()?;

    let (run, seeds) = seeds_run(&suites(repo.path()), &["--allow-scaffolds".as_ref()])?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    let placed = host_target().unwrap_or("none");
    assert_eq!(
        (seeds, &envelope["data"]["libexec_target"]),
        (
            format!(
                "plugin --scaffold scripts=1 anneal={placed}\n\
                 without-plan --scaffold scripts=1 anneal={placed}\n\
                 without-spec --no-scaffold scripts=0 anneal={placed}\n"
            ),
            &json!(host_target()),
        ),
        "the batches holding the seeded case run its scaffold; every copy has this anneal"
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
fn a_baseline_plugin_copy_holds_the_running_anneal_too() -> TestResult {
    let baseline = super::fixtures().join("eval-uplift/baseline-plugin");

    let (run, seeds) = seeds_run(
        &super::fixtures().join("eval-uplift/suites"),
        &["--baseline".as_ref(), baseline.as_os_str()],
    )?;

    let envelope = envelope(&run.output)?;
    assert_eq!(run.output.status.code(), Some(0), "{envelope}");
    let placed = host_target().unwrap_or("none");
    assert_eq!(
        seeds,
        format!(
            "plugin --no-scaffold scripts=0 anneal={placed}\n\
             baseline --no-scaffold scripts=0 anneal={placed}\n"
        )
    );
    Ok(())
}
