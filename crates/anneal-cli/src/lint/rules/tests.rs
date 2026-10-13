//! `lint rules` over documents written in each test, against a scratch root.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::CommandFactory;

use super::enforcer::{self, Enforcer, Repository, Root, Unresolved};
use super::{RulesArgs, check_document, lint, named_enforcers};
use crate::cli::Cli;
use crate::lint::finding::LintFinding;
use crate::lint::skills::LintError;
use crate::lint::skills::discover::MAX_READ_BYTES;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Resolves `span` against `root`, with `guard` as the one hook id: the outer error is a test
/// file that cannot be read, the inner one why the enforcer cannot be found.
fn resolve_in(root: &Path, span: &str) -> Result<Result<(), Unresolved>, LintError> {
    let hooks: BTreeSet<String> = ["guard".to_owned()].into();
    let commands = Cli::command();
    let root = Root::resolve(root)?;
    let repository = Repository {
        root: &root,
        hook_ids: &hooks,
        commands: &commands,
    };
    match enforcer::parse(span) {
        Ok(named) => enforcer::resolve(&named, &repository),
        Err(why) => Ok(Err(why)),
    }
}

/// The findings for `text`, resolved against `root`.
fn check(root: &Path, text: &str) -> Vec<LintFinding> {
    let hooks: BTreeSet<String> = ["guard".to_owned()].into();
    let commands = Cli::command();
    let root = Root::resolve(root).expect("the scratch root resolves");
    let repository = Repository {
        root: &root,
        hook_ids: &hooks,
        commands: &commands,
    };
    check_document("rules.md", text, &repository).expect("the root is readable")
}

#[test]
fn the_span_after_each_enforcer_label_is_named() {
    let named: Vec<&str> =
        named_enforcers("Do X. Enforcer: `hook guard` and Enforcer:`anneal lint skills`").collect();

    assert_eq!(named, ["hook guard", "anneal lint skills"]);
}

#[test]
fn every_span_in_a_comma_separated_run_is_named() {
    let named: Vec<&str> = named_enforcers(
        "Do X. Enforcer: `hook guard`, `test gone.rs::x` ,`anneal lint skills`, and `not this`",
    )
    .collect();

    assert_eq!(
        named,
        ["hook guard", "test gone.rs::x", "anneal lint skills"]
    );
}

#[test]
fn a_label_without_a_backticked_span_names_nothing() {
    let named: Vec<&str> = named_enforcers("Do X. Enforcer: the guard hook").collect();

    assert!(named.is_empty(), "{named:?}");
}

#[test]
fn the_three_kinds_parse() {
    assert_eq!(
        enforcer::parse("anneal lint skills plugin/skills"),
        Ok(Enforcer::Command(vec!["lint", "skills", "plugin/skills"]))
    );
    assert_eq!(enforcer::parse("hook guard"), Ok(Enforcer::Hook("guard")));
    assert_eq!(
        enforcer::parse("test tests/it/a.rs::a_case"),
        Ok(Enforcer::Test {
            file: "tests/it/a.rs",
            name: "a_case"
        })
    );
}

#[test]
fn an_unknown_kind_or_a_malformed_one_does_not_parse() {
    assert_eq!(
        enforcer::parse("clippy::unwrap_used"),
        Err(Unresolved::UnknownKind)
    );
    assert_eq!(
        enforcer::parse("hook guard extra"),
        Err(Unresolved::NotAHookId)
    );
    assert_eq!(
        enforcer::parse("test tests/it/a.rs"),
        Err(Unresolved::NotATestName)
    );
    assert_eq!(
        enforcer::parse("test a.rs::not-an-identifier"),
        Err(Unresolved::NotATestName)
    );
}

#[test]
fn a_command_resolves_only_down_to_a_command_with_no_subcommands() -> TestResult {
    let root = tempfile::tempdir()?;

    assert_eq!(
        resolve_in(root.path(), "anneal lint rules docs/x.md")?,
        Ok(())
    );
    assert_eq!(resolve_in(root.path(), "anneal decide --point D1")?, Ok(()));
    assert_eq!(
        resolve_in(root.path(), "anneal lint")?,
        Err(Unresolved::IncompleteCommand {
            path: "anneal lint".to_owned()
        })
    );
    assert_eq!(
        resolve_in(root.path(), "anneal lint frobnicate")?,
        Err(Unresolved::NoCommand {
            path: "anneal lint".to_owned(),
            word: "frobnicate".to_owned()
        })
    );
    Ok(())
}

#[test]
fn a_hook_resolves_only_to_a_known_id() -> TestResult {
    let root = tempfile::tempdir()?;

    assert_eq!(resolve_in(root.path(), "hook guard")?, Ok(()));
    assert_eq!(
        resolve_in(root.path(), "hook preconditions")?,
        Err(Unresolved::UnknownHook)
    );
    Ok(())
}

#[test]
fn a_test_resolves_only_to_a_function_its_file_defines() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("tests"))?;
    fs::write(root.path().join("tests/a.rs"), "#[test]\nfn a_case() {}\n")?;

    assert_eq!(resolve_in(root.path(), "test tests/a.rs::a_case")?, Ok(()));
    assert_eq!(
        resolve_in(root.path(), "test tests/a.rs::another_case")?,
        Err(Unresolved::NoTestFn)
    );
    assert_eq!(
        resolve_in(root.path(), "test tests/b.rs::a_case")?,
        Err(Unresolved::NoTestFile)
    );
    assert_eq!(
        resolve_in(root.path(), "test ../a.rs::a_case")?,
        Err(Unresolved::OutsideRoot)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_test_file_symlinked_from_outside_the_root_is_outside_it() -> TestResult {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("a.rs"), "#[test]\nfn a_case() {}\n")?;
    std::os::unix::fs::symlink(outside.path().join("a.rs"), root.path().join("a.rs"))?;
    std::os::unix::fs::symlink(outside.path(), root.path().join("out"))?;

    assert_eq!(
        (
            resolve_in(root.path(), "test a.rs::a_case")?,
            resolve_in(root.path(), "test out/a.rs::a_case")?,
        ),
        (Err(Unresolved::OutsideRoot), Err(Unresolved::OutsideRoot)),
        "a link to a file or a directory outside the root leads out of it"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_test_file_symlinked_within_the_root_resolves() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("tests"))?;
    fs::write(root.path().join("tests/a.rs"), "#[test]\nfn a_case() {}\n")?;
    std::os::unix::fs::symlink(root.path().join("tests/a.rs"), root.path().join("link.rs"))?;

    assert_eq!(resolve_in(root.path(), "test link.rs::a_case")?, Ok(()));
    Ok(())
}

/// `lint` over one document holding a rule enforced by `hook guard`, with `--root` `root`.
fn lint_hook_guard_rule(root: &Path) -> Result<Vec<LintFinding>, Box<dyn std::error::Error>> {
    let document = root.join("rules.md");
    fs::write(
        &document,
        "# R\n\n## Rules\n\n- Guard Bash.\n  Enforcer: `hook guard`\n",
    )?;
    let args = RulesArgs {
        paths: vec![document],
        root: root.to_path_buf(),
        hooks: None,
    };
    Ok(lint(&args)?.findings)
}

/// The repository's own hooks directory, which holds a `guard` hook id.
fn shipped_hooks_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/hooks")
}

#[cfg(unix)]
#[test]
fn default_hooks_metadata_linked_from_outside_the_root_is_ignored() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("plugin"))?;
    std::os::unix::fs::symlink(shipped_hooks_dir(), root.path().join("plugin/hooks"))?;

    let findings = lint_hook_guard_rule(root.path())?;

    assert_eq!(
        findings
            .iter()
            .map(|finding| finding.subject.as_deref())
            .collect::<Vec<_>>(),
        [Some("hook guard")],
        "the outside metadata is not read, so `guard` is unknown: {findings:#?}"
    );
    Ok(())
}

/// A root whose default `plugin/hooks/hooks.metadata.json` is a symlink to itself, which
/// cannot be resolved.
#[cfg(unix)]
fn root_with_a_looping_default_hooks_metadata() -> std::io::Result<tempfile::TempDir> {
    let root = tempfile::tempdir()?;
    let hooks = root.path().join("plugin/hooks");
    fs::create_dir_all(&hooks)?;
    std::os::unix::fs::symlink(
        hooks.join("hooks.metadata.json"),
        hooks.join("hooks.metadata.json"),
    )?;
    Ok(root)
}

#[cfg(unix)]
#[test]
fn an_unresolvable_default_hooks_metadata_is_not_read_when_no_rule_names_a_hook() -> TestResult {
    let root = root_with_a_looping_default_hooks_metadata()?;
    let document = root.path().join("rules.md");
    fs::write(
        &document,
        "# R\n\n## Rules\n\n- Lint skills.\n  Enforcer: `anneal lint skills`\n",
    )?;
    let args = RulesArgs {
        paths: vec![document],
        root: root.path().to_path_buf(),
        hooks: None,
    };

    let outcome = lint(&args)?;

    assert_eq!(
        (outcome.findings, outcome.documents),
        (Vec::new(), 1),
        "no rule names a hook, so the metadata is never read"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn an_unresolvable_default_hooks_metadata_fails_when_a_rule_names_a_hook() -> TestResult {
    let root = root_with_a_looping_default_hooks_metadata()?;

    let failed = lint_hook_guard_rule(root.path());

    let error = failed.expect_err("the metadata a hook rule needs cannot be resolved");
    assert!(
        matches!(
            error.downcast_ref::<LintError>(),
            Some(LintError::Io { .. })
        ),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn default_hooks_metadata_inside_the_root_is_read() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("plugin/hooks"))?;
    fs::copy(
        shipped_hooks_dir().join("hooks.metadata.json"),
        root.path().join("plugin/hooks/hooks.metadata.json"),
    )?;

    assert_eq!(lint_hook_guard_rule(root.path())?, []);
    Ok(())
}

#[test]
fn a_test_file_over_the_read_cap_is_too_large() -> TestResult {
    let root = tempfile::tempdir()?;
    let cap = usize::try_from(MAX_READ_BYTES)?;
    let mut text = b"fn a_case() {}\n".to_vec();
    text.resize(cap.saturating_add(1), b' ');
    fs::write(root.path().join("big.rs"), text)?;

    assert_eq!(
        resolve_in(root.path(), "test big.rs::a_case")?,
        Err(Unresolved::TestFileTooLarge)
    );
    Ok(())
}

#[test]
fn a_test_path_that_is_a_directory_is_no_test_file() -> TestResult {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("tests/a.rs"))?;

    assert_eq!(
        resolve_in(root.path(), "test tests/a.rs::a_case")?,
        Err(Unresolved::NoTestFile)
    );
    Ok(())
}

#[test]
fn each_unknown_enforcer_is_its_own_finding_on_the_rule_line() -> TestResult {
    let root = tempfile::tempdir()?;
    let text = "# R\n\n## Rules\n\n- Do X.\n  Enforcer: `hook nope` Enforcer: `anneal nope`\n";

    let findings = check(root.path(), text);

    let keys: Vec<String> = findings.iter().map(LintFinding::key).collect();
    assert_eq!(
        keys,
        [
            "lint/enforcer:rules.md:5:hook nope",
            "lint/enforcer:rules.md:5:anneal nope"
        ]
    );
    Ok(())
}

#[test]
fn a_document_with_no_rules_section_or_an_empty_one_is_a_finding() -> TestResult {
    let root = tempfile::tempdir()?;

    let none = check(root.path(), "# R\n\nprose\n");
    let empty = check(root.path(), "# R\n\n## Rules\n\nprose only\n");

    let at = |findings: &[LintFinding]| -> Vec<String> {
        findings.iter().map(|f| f.at.to_string()).collect()
    };
    assert_eq!(at(&none), ["rules.md:0"], "the file as a whole");
    assert_eq!(at(&empty), ["rules.md:3"], "the section heading");
    Ok(())
}

#[test]
fn an_enforcer_in_a_code_fence_does_not_count() -> TestResult {
    let root = tempfile::tempdir()?;
    let text = "## Rules\n\n- Do X.\n```\n- Enforcer: `hook guard`\n```\n";

    let findings = check(root.path(), text);

    let messages: Vec<&str> = findings.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(messages, ["this rule names no enforcer."]);
    Ok(())
}
