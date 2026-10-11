use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use super::{Cache, RULE_SOURCES, content_hash, hash_with_rules};
use crate::baseline::BaselineError;
use crate::lint::skills::LintError;
use crate::lint::skills::discover::{EntryKind, Surroundings};
use crate::lint::skills::document::{test_dir, test_skill};

const TEXT: &str = "---\nname: plan\n---\n# Plan\n";

fn surroundings(has_scripts: bool, references: &[(&str, EntryKind)]) -> Surroundings {
    Surroundings {
        has_scripts,
        references: references
            .iter()
            .map(|(name, kind)| (OsString::from(name), *kind))
            .collect(),
    }
}

/// The hash of a `plan` skill with `text`, `around` it and `hooks`.
fn hash(text: &str, around: &Surroundings, hooks: &[&str]) -> String {
    let dir = test_dir(Path::new("skills/plan"), "plan");
    let hooks: BTreeSet<String> = hooks.iter().map(|h| (*h).to_owned()).collect();
    content_hash(&test_skill(&dir, text), around, &hooks)
}

#[test]
fn the_hash_covers_every_input_the_checks_consume() {
    let none = surroundings(false, &[]);
    let first = hash(TEXT, &none, &[]);

    assert_eq!(first, hash(TEXT, &none, &[]), "stable");
    let variants = [
        (
            "SKILL.md bytes",
            hash(&TEXT.replace("# Plan", "# Plan!"), &none, &[]),
        ),
        ("hook ids", hash(TEXT, &none, &["guard"])),
        ("scripts/", hash(TEXT, &surroundings(true, &[]), &[])),
        (
            "a reference",
            hash(
                TEXT,
                &surroundings(false, &[("x.md", EntryKind::File)]),
                &[],
            ),
        ),
        (
            "its kind",
            hash(TEXT, &surroundings(false, &[("x.md", EntryKind::Dir)]), &[]),
        ),
    ];
    let mut seen = BTreeSet::from([first]);
    for (input, variant) in variants {
        assert!(seen.insert(variant), "{input} changes the hash");
    }
}

#[test]
fn reference_names_cannot_run_together() {
    let split = surroundings(false, &[("ab", EntryKind::File), ("c", EntryKind::File)]);
    let joined = surroundings(false, &[("a", EntryKind::File), ("bc", EntryKind::File)]);

    assert_ne!(
        hash(TEXT, &split, &[]),
        hash(TEXT, &joined, &[]),
        "names are length-prefixed"
    );
}

#[test]
fn the_rule_sources_are_part_of_the_hash() {
    let dir = test_dir(Path::new("skills/plan"), "plan");
    let skill = test_skill(&dir, TEXT);
    let none = surroundings(false, &[]);
    let hooks = BTreeSet::new();

    assert_ne!(
        hash_with_rules(&[("a.rs", "fn check() {}")], &skill, &none, &hooks),
        hash_with_rules(&[("a.rs", "fn check() { changed }")], &skill, &none, &hooks),
        "editing a check invalidates every cached entry"
    );
    assert!(
        RULE_SOURCES.len() >= 8,
        "checker, checks_*, document, frontmatter"
    );
}

/// The hash of the skill directory at `root` (its `SKILL.md` and surroundings) with `hooks`.
fn hash_of_dir(root: &Path, hooks: &BTreeSet<String>) -> String {
    let dir = test_dir(root, "plan");
    let text = std::fs::read_to_string(root.join("SKILL.md")).expect("read SKILL.md");
    let around = Surroundings::read(&dir).expect("surroundings");
    content_hash(&test_skill(&dir, &text), &around, hooks)
}

#[test]
fn the_hash_changes_with_any_file_and_with_the_hook_ids() {
    let root = tempfile::tempdir().expect("temp dir");
    std::fs::write(root.path().join("SKILL.md"), TEXT).expect("write");
    let none = BTreeSet::new();
    let first = hash_of_dir(root.path(), &none);

    assert_eq!(first, hash_of_dir(root.path(), &none), "stable");
    let hooks: BTreeSet<String> = ["guard".to_owned()].into();
    assert_ne!(first, hash_of_dir(root.path(), &hooks), "hook ids count");
    std::fs::create_dir(root.path().join("references")).expect("mkdir");
    std::fs::write(root.path().join("references/x.md"), "b").expect("write");
    assert_ne!(first, hash_of_dir(root.path(), &none), "new file counts");
}

#[test]
fn file_boundaries_are_part_of_the_hash() {
    // {a: "", b: "Q"} and {a: "b\0\0Q"} under references/ must never hash alike.
    let split = tempfile::tempdir().expect("temp dir");
    let joined = tempfile::tempdir().expect("temp dir");
    for root in [&split, &joined] {
        std::fs::write(root.path().join("SKILL.md"), TEXT).expect("write");
        std::fs::create_dir(root.path().join("references")).expect("mkdir");
    }
    std::fs::write(split.path().join("references/a"), "").expect("write");
    std::fs::write(split.path().join("references/b"), "Q").expect("write");
    std::fs::write(joined.path().join("references/a"), "b\0\0Q").expect("write");
    let none = BTreeSet::new();

    assert_ne!(
        hash_of_dir(split.path(), &none),
        hash_of_dir(joined.path(), &none),
        "two files never hash like one"
    );
}

/// Lint sources that do not decide per-skill findings, so they are not in `RULE_SOURCES`:
/// `cache.rs` stores snapshots (a change to its format bumps `FORMAT_VERSION`); `listing.rs`
/// lists skills for `eval routing` and no lint check reads it.
const NOT_RULE_SOURCES: [&str; 2] = ["cache.rs", "listing.rs"];

/// The `.rs` files directly in `src/<dir>`, as paths relative to `src/lint/skills/`.
fn sources_in(dir: &str, prefix: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(dir);
    let mut names: Vec<String> = std::fs::read_dir(path)
        .expect("source dir is listable")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        })
        .map(|name| format!("{prefix}{name}"))
        .collect();
    names.sort();
    names
}

#[test]
fn every_lint_source_is_hashed_or_explicitly_excluded() {
    let hashed: BTreeSet<&str> = RULE_SOURCES.iter().map(|(name, _)| *name).collect();

    for name in sources_in("lint/skills", "")
        .into_iter()
        .chain(sources_in("lint", "../"))
    {
        assert!(
            hashed.contains(name.as_str()) || NOT_RULE_SOURCES.contains(&name.as_str()),
            "{name} is neither in RULE_SOURCES nor in NOT_RULE_SOURCES"
        );
    }
    assert!(
        hashed.contains("../../command.rs"),
        "CommandName::ALL decides enforcer findings"
    );
    for excluded in NOT_RULE_SOURCES {
        assert!(!hashed.contains(excluded), "{excluded} is listed twice");
    }
}

fn write(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
    let path = dir.path().join("cache.json");
    std::fs::write(&path, text).expect("write");
    path
}

#[test]
fn our_snapshot_from_another_version_loads_as_empty() {
    let root = tempfile::tempdir().expect("temp dir");
    for text in [
        r#"{"kind": "anneal-lint-skills", "version": 1, "skills": {}}"#,
        r#"{"kind": "anneal-lint-skills", "version": 99, "skills": []}"#,
    ] {
        let cache = Cache::load(&write(&root, text)).expect("stale, not an error");
        assert_eq!(
            cache.lookup("any", "hash"),
            None,
            "{text}: nothing replayed"
        );
    }
}

#[test]
fn a_snapshot_from_another_version_or_shape_loads_as_empty() {
    // Since B2 only a file carrying our `kind` can be stale; the unmarked inputs this test used
    // to hold (`{"version": 99, ..}`, `{"version": 1, "skills": 5}`) are Format errors now,
    // pinned in `a_file_that_is_not_our_snapshot_is_a_format_error`.
    let root = tempfile::tempdir().expect("temp dir");
    for text in [
        r#"{"kind": "anneal-lint-skills", "version": 99, "skills": []}"#,
        r#"{"kind": "anneal-lint-skills", "version": 1, "skills": 5}"#,
    ] {
        let cache = Cache::load(&write(&root, text)).expect("stale, not an error");
        assert_eq!(
            cache.lookup("any", "hash"),
            None,
            "{text}: nothing replayed"
        );
    }
}

#[test]
fn a_file_that_is_not_a_versioned_snapshot_is_a_format_error() {
    let root = tempfile::tempdir().expect("temp dir");
    for text in [
        "not json",
        "[1, 2]",
        r#"{"skills": {}}"#,
        r#"{"version": "1"}"#,
    ] {
        let result = Cache::load(&write(&root, text));
        assert!(
            matches!(result, Err(LintError::Cache(BaselineError::Format { .. }))),
            "{text}: refused rather than overwritten: {result:?}"
        );
    }
}

#[test]
fn a_saved_snapshot_loads_back() {
    let root = tempfile::tempdir().expect("temp dir");
    let path = root.path().join("cache.json");
    let mut cache = Cache::load(&path).expect("missing is empty");
    cache.store("skills/plan".to_owned(), "h".to_owned(), Vec::new());

    cache.save(&path).expect("save");
    let loaded = Cache::load(&path).expect("load");

    assert_eq!(
        loaded.lookup("skills/plan", "h"),
        Some(&[][..]),
        "round trip"
    );
}

#[test]
fn a_file_that_is_not_our_snapshot_is_a_format_error() {
    let root = tempfile::tempdir().expect("temp dir");
    for text in [
        "not json",
        "[1, 2]",
        r#"{"skills": {}}"#,
        r#"{"version": "1"}"#,
        r#"{"version": 1, "skills": 5}"#,
        // plugin/hooks/hooks.metadata.json
        r#"{"version": 1, "hooks": [{"id": "guard"}]}"#,
        // an `anneal` baseline snapshot
        r#"{"version": 1, "keys": ["lint/name:a:1"]}"#,
        // ours, current version, but corrupt
        r#"{"kind": "anneal-lint-skills", "version": 2, "skills": 5}"#,
        r#"{"kind": "anneal-lint-skills"}"#,
    ] {
        let result = Cache::load(&write(&root, text));
        assert!(
            matches!(result, Err(LintError::Cache(BaselineError::Format { .. }))),
            "{text}: refused rather than overwritten: {result:?}"
        );
    }
}
