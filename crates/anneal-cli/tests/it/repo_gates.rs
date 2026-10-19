//! The repository's own skills against the Tier 0 and Tier 1 merge bars (`evals/README.md`).
//! CI runs `cargo test`, so these tests are what enforce the bars there.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anneal_cli::eval::routing::{RoutingArgs, evaluate};
use anneal_cli::exit_codes;
use anneal_cli::lint::finding::Severity;
use anneal_cli::lint::skills::{SkillsArgs, lint};
use anneal_cli::repro::debug_leftovers;

type TestResult = Result<(), Box<dyn Error>>;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn plugin_skills() -> PathBuf {
    repo().join("plugin/skills")
}

#[test]
fn the_plugin_skills_route_at_100_percent_rank_1() -> TestResult {
    let outcome = evaluate(&RoutingArgs {
        skills: plugin_skills(),
        fixtures: repo().join("evals/routing"),
    })?;

    let mut report = outcome.confusion().render().join("\n");
    for problem in &outcome.problems {
        write!(report, "\nproblem: {problem:?}")?;
    }
    for miss in outcome.misses() {
        write!(
            report,
            "\nmiss: {}:{} expected {} got {:?}: {:?}",
            miss.file, miss.line, miss.expected, miss.top, miss.prompt
        )?;
    }
    assert_eq!(
        outcome.code(),
        exit_codes::OK,
        "Tier 1 needs every fixture valid and 100% rank-1 (run `anneal eval routing`):\n{report}"
    );
    Ok(())
}

#[test]
fn the_plugin_skills_lint_clean_and_the_snapshot_replays_the_fresh_result() -> TestResult {
    let work = tempfile::tempdir()?;
    let fresh_args = SkillsArgs {
        paths: vec![plugin_skills()],
        index: None,
        hooks: None,
        cache: None,
    };
    let cached_args = SkillsArgs {
        cache: Some(work.path().join("lint-cache.json")),
        ..fresh_args.clone()
    };

    let fresh = lint(&fresh_args)?;
    let first = lint(&cached_args)?;
    let replayed = lint(&cached_args)?;

    let errors: Vec<String> = fresh
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .map(|f| format!("{}: {}", f.at, f.message))
        .collect();
    assert_eq!(errors, Vec::<String>::new(), "Tier 0: zero lint errors");
    assert_eq!(
        first.findings, fresh.findings,
        "a snapshot run reports the fresh result"
    );
    assert_eq!(
        replayed.findings, fresh.findings,
        "a replayed run reports the fresh result"
    );
    assert_eq!(
        replayed.cached, fresh.skills,
        "every unchanged skill is replayed"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn every_entry_skill_has_valid_regression_and_capability_suites() -> TestResult {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["eval", "uplift", "--check", "--skills"])
        .arg(plugin_skills())
        .arg("--suites")
        .arg(repo().join("evals/uplift"))
        .output()?;
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    assert_eq!(
        output.status.code(),
        Some(0),
        "Tier 3 suites must all be valid (run `anneal eval uplift --check`): {}",
        envelope["findings"]
    );
    let thin: Vec<String> = [
        "grill", "spec", "plan", "build", "diagnose", "verify", "ship",
    ]
    .into_iter()
    .filter(|skill| {
        !envelope["data"]["coverage"].as_array().is_some_and(|rows| {
            rows.iter().any(|row| {
                row["skill"] == *skill
                    && row["regression"].as_u64().is_some_and(|n| n >= 1)
                    && row["capability"].as_u64().is_some_and(|n| n >= 1)
            })
        })
    })
    .map(str::to_owned)
    .collect();
    assert_eq!(
        thin,
        Vec::<String>::new(),
        "each entry skill (#41) needs a regression and a capability case: {}",
        envelope["data"]["coverage"]
    );
    Ok(())
}

/// Every directory under `dir`, recursively, without following symlinks.
fn directories(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let path = entry.path();
            found.extend(directories(&path)?);
            found.push(path);
        }
    }
    Ok(found)
}

#[test]
fn the_packaged_plugin_holds_no_evals_directory() -> TestResult {
    let evals: Vec<PathBuf> = directories(&repo().join("plugin"))?
        .into_iter()
        .filter(|dir| dir.ends_with("evals"))
        .collect();

    assert_eq!(
        evals,
        Vec::<PathBuf>::new(),
        "evals/ stays outside the packaged plugin"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs git in a subprocess")]
fn the_repository_holds_no_debug_leftover() -> TestResult {
    let scan = debug_leftovers(&repo())?;

    let found: Vec<String> = scan
        .leftovers()
        .iter()
        .map(|leftover| format!("{}:{}", leftover.file(), leftover.line()))
        .collect();
    assert!(
        scan.is_clean(),
        "the Stop hook would block every session in this repository; build a test's tag \
         at run time, as `format!(\"[{{}}-x1]\", \"DEBUG\")`: {found:?}"
    );
    Ok(())
}
