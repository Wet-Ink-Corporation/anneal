//! `anneal lint skills` on awkward inputs: symlinks, wrong path kinds, oversized and
//! non-UTF-8 files, broken hooks metadata.

use std::error::Error;
use std::path::{Path, PathBuf};

use anneal_cli::lint::finding::{LintFinding, Severity};
use anneal_cli::lint::skills::checker::Checker;
use anneal_cli::lint::skills::{LintError, SkillsArgs, lint};

use crate::support::copy_dir;

type TestResult = Result<(), Box<dyn Error>>;

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lint-skills")
        .join(relative)
}

fn args(paths: Vec<PathBuf>) -> SkillsArgs {
    SkillsArgs {
        paths,
        index: None,
        hooks: None,
        cache: None,
    }
}

/// A scratch copy of a clean skill pair (`plan`, `spec`) at `<tmp>/skills`.
fn clean_skills() -> Result<(tempfile::TempDir, PathBuf), Box<dyn Error>> {
    let work = tempfile::tempdir()?;
    let skills = work.path().join("skills");
    copy_dir(&fixture("rule-01-name/pass/skills"), &skills)?;
    Ok((work, skills))
}

fn errors_from(findings: &[LintFinding], checker: Checker) -> Vec<&LintFinding> {
    findings
        .iter()
        .filter(|f| f.checker == checker && f.severity == Severity::Error)
        .collect()
}

#[test]
#[cfg(unix)]
fn symlinks_in_a_skill_give_the_same_outcome_with_and_without_the_cache() -> TestResult {
    let (work, skills) = clean_skills()?;
    let plan = skills.join("plan");
    std::os::unix::fs::symlink("does-not-exist", plan.join("dangling"))?;
    std::os::unix::fs::symlink("../spec", plan.join("linked-dir"))?;
    std::os::unix::fs::symlink("/dev/zero", plan.join("endless"))?;
    let uncached_args = args(vec![skills.clone()]);
    let cached_args = SkillsArgs {
        cache: Some(work.path().join("cache.json")),
        ..args(vec![skills])
    };

    let without = lint(&uncached_args)?;
    let with = lint(&cached_args)?;
    let replayed = lint(&cached_args)?;

    assert_eq!(with.findings, without.findings, "the cache changes nothing");
    assert_eq!(replayed.findings, without.findings, "nor does a replay");
    assert_eq!(replayed.cached, 2, "symlinked skills still cache");
    Ok(())
}

#[test]
fn a_path_of_the_wrong_kind_is_a_usage_error() -> TestResult {
    let (_work, skills) = clean_skills()?;
    let cases = [
        args(vec![skills.join("plan/SKILL.md")]),
        SkillsArgs {
            index: Some(skills.clone()),
            ..args(vec![skills.clone()])
        },
        SkillsArgs {
            hooks: Some(skills.clone()),
            ..args(vec![skills.clone()])
        },
        SkillsArgs {
            cache: Some(skills.clone()),
            ..args(vec![skills])
        },
    ];
    for case in &cases {
        let result = lint(case);
        assert!(
            matches!(result, Err(LintError::WrongKind { .. })),
            "{case:?} is the wrong kind of path: {result:?}"
        );
        let code = result.map_or_else(|e| e.code().value(), |_| 0);
        assert_eq!(code, 2, "{case:?} is a usage error, not internal (70)");
    }
    Ok(())
}

/// Lints `skills` without a cache, then twice with one at `cache`; returns the three outcomes'
/// findings.
fn fresh_and_cached(skills: &Path, cache: &Path) -> Result<[Vec<LintFinding>; 3], Box<dyn Error>> {
    let uncached_args = args(vec![skills.to_path_buf()]);
    let cached_args = SkillsArgs {
        cache: Some(cache.to_path_buf()),
        ..args(vec![skills.to_path_buf()])
    };
    Ok([
        lint(&uncached_args)?.findings,
        lint(&cached_args)?.findings,
        lint(&cached_args)?.findings,
    ])
}

#[test]
#[cfg(unix)]
fn a_symlinked_skill_file_is_rechecked_when_its_target_changes() -> TestResult {
    let (work, skills) = clean_skills()?;
    let target = work.path().join("plan-source.md");
    std::fs::rename(skills.join("plan/SKILL.md"), &target)?;
    std::os::unix::fs::symlink(&target, skills.join("plan/SKILL.md"))?;
    let cache = work.path().join("cache.json");
    let [before, _, _] = fresh_and_cached(&skills, &cache)?;
    assert!(
        errors_from(&before, Checker::ModelField).is_empty(),
        "clean to start"
    );

    let text = std::fs::read_to_string(&target)?;
    std::fs::write(
        &target,
        text.replacen("name: plan\n", "name: plan\nmodel: opus\n", 1),
    )?;
    let [fresh, cached, replayed] = fresh_and_cached(&skills, &cache)?;

    assert_eq!(
        errors_from(&fresh, Checker::ModelField).len(),
        1,
        "the edit is an error"
    );
    assert_eq!(cached, fresh, "the cache sees the edited target");
    assert_eq!(replayed, fresh, "and replays it faithfully");
    Ok(())
}

#[test]
#[cfg(unix)]
fn a_non_utf8_file_name_gives_the_same_outcome_with_and_without_the_cache() -> TestResult {
    use std::os::unix::ffi::OsStrExt;
    let (work, skills) = clean_skills()?;
    let references = skills.join("plan/references");
    std::fs::create_dir(&references)?;
    let odd = std::ffi::OsStr::from_bytes(b"odd-\xff.md");
    std::fs::write(references.join(odd), "# Odd\n")?;
    std::fs::write(skills.join("plan").join(odd), "loose file\n")?;

    let [fresh, cached, replayed] = fresh_and_cached(&skills, &work.path().join("cache.json"))?;

    assert_eq!(
        errors_from(&fresh, Checker::References).len(),
        1,
        "the unlinked reference"
    );
    assert_eq!(cached, fresh, "the cache changes nothing");
    assert_eq!(replayed, fresh, "nor does a replay");
    Ok(())
}

#[test]
fn a_plain_value_with_colon_space_is_a_frontmatter_error() -> TestResult {
    let outcome = lint(&args(vec![fixture("rule-01-frontmatter/fail/skills")]))?;

    let errors = errors_from(&outcome.findings, Checker::Frontmatter);
    assert!(
        errors
            .iter()
            .any(|f| Path::new(&f.at.file).ends_with("build/SKILL.md") && f.at.line == 3),
        "build's `description: Use when: ...` is refused at line 3: {errors:#?}"
    );
    Ok(())
}

#[test]
fn an_oversized_skill_gets_one_budget_error_and_no_other_check() -> TestResult {
    let (_work, skills) = clean_skills()?;
    let big = "x ".repeat(600 * 1024);
    std::fs::write(skills.join("plan/SKILL.md"), big)?;

    let outcome = lint(&args(vec![skills]))?;

    let plan: Vec<&LintFinding> = outcome
        .findings
        .iter()
        .filter(|f| Path::new(&f.at.file).ends_with("plan/SKILL.md"))
        .collect();
    assert_eq!(plan.len(), 1, "only the budget finding: {plan:#?}");
    assert_eq!(
        errors_from(&outcome.findings, Checker::BodyBudget).len(),
        1,
        "body-budget"
    );
    Ok(())
}

#[test]
fn a_skill_that_is_not_utf8_is_a_frontmatter_error() -> TestResult {
    let (_work, skills) = clean_skills()?;
    std::fs::write(skills.join("plan/SKILL.md"), b"---\nname: plan\xff\n---\n")?;

    let outcome = lint(&args(vec![skills]))?;

    let errors = errors_from(&outcome.findings, Checker::Frontmatter);
    assert_eq!(errors.len(), 1, "one frontmatter error: {errors:#?}");
    assert!(
        errors.iter().all(|f| f.message.contains("not UTF-8")),
        "says why: {errors:#?}"
    );
    Ok(())
}

#[test]
fn hooks_metadata_that_does_not_parse_is_an_enforcer_error() -> TestResult {
    let (work, skills) = clean_skills()?;
    let hooks = work.path().join("hooks");
    std::fs::create_dir(&hooks)?;
    std::fs::write(hooks.join("hooks.metadata.json"), "{\"hooks\": [{}]}")?;

    let outcome = lint(&args(vec![skills]))?;

    let errors = errors_from(&outcome.findings, Checker::Enforcer);
    assert_eq!(
        errors.len(),
        1,
        "the default metadata path was read: {errors:#?}"
    );
    assert!(
        errors
            .iter()
            .all(|f| f.at.file.ends_with("hooks.metadata.json")),
        "points at the metadata file"
    );
    Ok(())
}

#[test]
fn the_envelope_names_the_command_lint_skills() -> TestResult {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_anneal"))
        .args(["lint", "skills"])
        .arg(fixture("rule-01-name/pass/skills"))
        .output()?;
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    assert_eq!(
        envelope["command"], "lint skills",
        "the full command as typed"
    );
    Ok(())
}
