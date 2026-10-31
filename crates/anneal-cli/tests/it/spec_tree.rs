//! A spec's `Tree:` line (#77): `lint spec` checks the design tree it names, and `plan
//! approve` refuses while that tree has no `grill_confirmed` at its current hash. Each test
//! runs the built binary on a temporary copy of the dry run in `evals/fixtures/dry-run/`, its
//! spec given a `Tree:` line, with the ledger at `ledger.sqlite3` there (`--store`). The
//! fixture itself is left as recorded.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::{anneal_in, copy_dir};

type TestResult = Result<(), Box<dyn Error>>;

const STORE: &str = "ledger.sqlite3";
const TREE: &str = "design-tree.json";
const SPEC: &str = "spec.md";
const PLAN: &str = "plan.md";
const QUOTE: &str = "Yes, that's it: only notes list, compact JSON, go ahead.";

fn fixture() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "../../evals/fixtures/dry-run"]
        .iter()
        .collect()
}

/// A temporary copy of the dry run whose spec has `tree_line` as its third line, under the
/// title.
fn workspace(tree_line: &str) -> Result<TempDir, Box<dyn Error>> {
    let work = tempfile::tempdir()?;
    copy_dir(&fixture(), work.path())?;
    let spec = fs::read_to_string(work.path().join(SPEC))?;
    let linked = spec.replacen(
        "\n\n## Problem",
        &format!("\n\n{tree_line}\n\n## Problem"),
        1,
    );
    assert_ne!(linked, spec, "the spec has a `## Problem` section");
    fs::write(work.path().join(SPEC), linked)?;
    Ok(work)
}

/// Runs `anneal <args>` in `cwd` and parses stdout as the envelope (`Null` when empty).
fn anneal(cwd: &Path, args: &[&str]) -> Result<(Output, Value), Box<dyn Error>> {
    let output = anneal_in(cwd).args(args).output()?;
    let envelope = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)?
    };
    Ok((output, envelope))
}

fn approve(cwd: &Path) -> Result<(Output, Value), Box<dyn Error>> {
    anneal(cwd, &["plan", "approve", PLAN, "--store", STORE])
}

fn confirm(cwd: &Path) -> Result<(Output, Value), Box<dyn Error>> {
    anneal(
        cwd,
        &[
            "grill", "confirm", "--tree", TREE, "--quote", QUOTE, "--store", STORE,
        ],
    )
}

/// Each finding of a `lint spec` envelope as `(rule, line)`.
fn rules_and_lines(envelope: &Value) -> Vec<(String, u64)> {
    envelope
        .pointer("/data/findings")
        .and_then(Value::as_array)
        .map(|findings| {
            findings
                .iter()
                .map(|finding| {
                    (
                        finding["rule"].as_str().unwrap_or("?").to_owned(),
                        finding["line"].as_u64().unwrap_or(u64::MAX),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The keys of the envelope's findings.
fn keys(envelope: &Value) -> Vec<&str> {
    envelope["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|finding| finding["key"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_line_naming_a_valid_design_tree_lints_clean_and_is_reported() -> TestResult {
    let work = workspace("Tree: [design tree](design-tree.json)")?;

    let (linted, envelope) = anneal(work.path(), &["lint", "spec", SPEC])?;

    assert_eq!(linted.status.code(), Some(0), "clean: {envelope}");
    assert_eq!(
        envelope.pointer("/data/tree"),
        Some(&json!(TREE)),
        "the tree, read relative to the spec's directory"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_line_naming_a_missing_file_is_a_tree_finding() -> TestResult {
    let work = workspace("Tree: `trees/missing.json`")?;

    let (linted, envelope) = anneal(work.path(), &["lint", "spec", SPEC])?;

    assert_eq!(linted.status.code(), Some(1), "a finding: {envelope}");
    assert_eq!(
        rules_and_lines(&envelope),
        vec![("tree".to_owned(), 3)],
        "one finding at the `Tree:` line"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_line_naming_a_file_that_is_no_design_tree_is_a_tree_finding() -> TestResult {
    let work = workspace("Tree: CONTEXT.md")?;

    let (linted, envelope) = anneal(work.path(), &["lint", "spec", SPEC])?;

    assert_eq!(linted.status.code(), Some(1), "a finding: {envelope}");
    assert_eq!(
        rules_and_lines(&envelope),
        vec![("tree".to_owned(), 3)],
        "one finding at the `Tree:` line"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_spec_whose_tree_was_never_confirmed_cannot_have_its_plan_approved() -> TestResult {
    let work = workspace("Tree: design-tree.json")?;

    let (refused, envelope) = approve(work.path())?;

    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["plan/approve/unconfirmed-tree:design-tree.json"],
        "one finding naming the tree"
    );
    assert_eq!(
        envelope.pointer("/data/tree"),
        Some(&json!(TREE)),
        "the refusal names the tree"
    );
    assert!(!work.path().join(STORE).exists(), "no ledger created");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_spec_whose_tree_is_confirmed_at_its_hash_has_its_plan_approved() -> TestResult {
    let work = workspace("Tree: design-tree.json")?;

    let (confirmed, envelope) = confirm(work.path())?;
    assert_eq!(confirmed.status.code(), Some(0), "confirmed: {envelope}");
    let (approved, envelope) = approve(work.path())?;

    assert_eq!(approved.status.code(), Some(0), "approved: {envelope}");
    assert_eq!(
        (
            envelope.pointer("/data/appended"),
            envelope.pointer("/data/tree")
        ),
        (Some(&json!(true)), Some(&json!(TREE))),
        "appended, naming the tree"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_edited_after_its_confirmation_refuses_the_approval() -> TestResult {
    let work = workspace("Tree: design-tree.json")?;
    let (confirmed, envelope) = confirm(work.path())?;
    assert_eq!(confirmed.status.code(), Some(0), "confirmed: {envelope}");
    let tree = fs::read_to_string(work.path().join(TREE))?;
    fs::write(work.path().join(TREE), tree.replacen("toy", "small", 1))?;

    let (refused, envelope) = approve(work.path())?;

    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["plan/approve/unconfirmed-tree:design-tree.json"],
        "the confirmation is of other bytes"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_line_left_as_the_template_placeholder_refuses_the_approval() -> TestResult {
    let work = workspace("Tree: <the design tree, relative to this spec>")?;

    let (refused, envelope) = approve(work.path())?;

    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["plan/approve/tree:spec.md"],
        "one finding naming the spec"
    );
    assert!(!work.path().join(STORE).exists(), "no ledger opened");
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_line_naming_a_missing_file_refuses_the_approval() -> TestResult {
    let work = workspace("Tree: trees/missing.json")?;

    let (refused, envelope) = approve(work.path())?;

    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["plan/approve/tree:spec.md"],
        "one finding naming the spec"
    );
    Ok(())
}

/// Makes git fail in `dir` with something other than "not a git repository": a `.git` file
/// that is no gitfile, so locating the ledger without `--store` fails there.
fn break_git(dir: &Path) -> Result<(), Box<dyn Error>> {
    fs::write(dir.join(".git"), "not a gitfile\n")?;
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_missing_tree_is_refused_before_the_ledger_is_located() -> TestResult {
    let located = workspace(&format!("Tree: {TREE}"))?;
    break_git(located.path())?;
    let missing = workspace("Tree: trees/missing.json")?;
    break_git(missing.path())?;

    let (failed, _) = anneal(located.path(), &["plan", "approve", PLAN])?;
    let (refused, envelope) = anneal(missing.path(), &["plan", "approve", PLAN])?;

    assert!(
        !matches!(failed.status.code(), Some(0 | 3)),
        "with a readable tree the approval reaches the ledger and git fails: {failed:?}"
    );
    assert_eq!(refused.status.code(), Some(3), "not approved: {envelope}");
    assert_eq!(
        keys(&envelope),
        vec!["plan/approve/tree:spec.md"],
        "the missing tree is the refusal, decided before git is asked where the ledger is"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_link_with_an_angle_bracket_destination_lints_clean() -> TestResult {
    let work = workspace("Tree: [t](<design-tree.json>)")?;

    let (linted, envelope) = anneal(work.path(), &["lint", "spec", SPEC])?;

    assert_eq!(linted.status.code(), Some(0), "clean: {envelope}");
    assert_eq!(
        envelope.pointer("/data/tree"),
        Some(&json!(TREE)),
        "the destination inside `<` and `>`, not a placeholder"
    );
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_tree_link_with_an_angle_bracket_destination_approves_once_confirmed() -> TestResult {
    let work = workspace("Tree: [t](<design-tree.json>)")?;

    let (refused, envelope) = approve(work.path())?;
    assert_eq!(
        (refused.status.code(), keys(&envelope)),
        (
            Some(3),
            vec!["plan/approve/unconfirmed-tree:design-tree.json"]
        ),
        "the tree is named but not yet confirmed: {envelope}"
    );
    let (confirmed, envelope) = confirm(work.path())?;
    assert_eq!(confirmed.status.code(), Some(0), "confirmed: {envelope}");

    let (approved, envelope) = approve(work.path())?;

    assert_eq!(approved.status.code(), Some(0), "approved: {envelope}");
    assert_eq!(
        envelope.pointer("/data/tree"),
        Some(&json!(TREE)),
        "naming the tree"
    );
    Ok(())
}

/// A temporary copy of the dry run with its spec at `docs/specs/spec.md` (its `Tree:` line
/// `../trees/t.json`), its plan at `docs/plans/plan.md` (its `Spec:` link `../specs/spec.md`)
/// and its design tree at `docs/trees/t.json`.
fn nested() -> Result<TempDir, Box<dyn Error>> {
    let work = workspace("Tree: ../trees/t.json")?;
    let docs = work.path().join("docs");
    for dir in ["specs", "plans", "trees"] {
        fs::create_dir_all(docs.join(dir))?;
    }
    fs::rename(work.path().join(SPEC), docs.join("specs").join(SPEC))?;
    fs::rename(work.path().join(TREE), docs.join("trees").join("t.json"))?;
    let plan = fs::read_to_string(work.path().join(PLAN))?;
    let relinked = plan.replacen("](spec.md)", "](../specs/spec.md)", 1);
    assert_ne!(relinked, plan, "the plan links `spec.md`");
    fs::write(docs.join("plans").join(PLAN), relinked)?;
    fs::remove_file(work.path().join(PLAN))?;
    Ok(work)
}

#[test]
#[cfg_attr(miri, ignore = "runs the binary")]
fn a_spec_and_plan_in_nested_directories_approve_against_the_resolved_tree() -> TestResult {
    let work = nested()?;
    // The spec's path as the plan links it, then the tree's joined to its directory.
    let resolved = json!("docs/plans/../specs/../trees/t.json");
    let approve_nested = || {
        anneal(
            work.path(),
            &["plan", "approve", "docs/plans/plan.md", "--store", STORE],
        )
    };

    let (refused, envelope) = approve_nested()?;
    assert_eq!(
        (refused.status.code(), envelope.pointer("/data/tree")),
        (Some(3), Some(&resolved)),
        "refused before the confirmation, naming the resolved tree: {envelope}"
    );
    let (confirmed, envelope) = anneal(
        work.path(),
        &[
            "grill",
            "confirm",
            "--tree",
            "docs/trees/t.json",
            "--quote",
            QUOTE,
            "--store",
            STORE,
        ],
    )?;
    assert_eq!(confirmed.status.code(), Some(0), "confirmed: {envelope}");

    let (approved, envelope) = approve_nested()?;

    assert_eq!(approved.status.code(), Some(0), "approved: {envelope}");
    assert_eq!(
        envelope.pointer("/data/tree"),
        Some(&resolved),
        "the tree, joined to the spec's directory"
    );
    Ok(())
}
