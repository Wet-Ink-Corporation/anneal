use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Ruling, RulingError, list, parse};

type TestResult = Result<(), Box<dyn Error>>;

const FULL: &str = "---\nid: no-gui\nruled_at: 2026-10-11\nsummary: No GUI in v0.1; the CLI is the surface.\n\
                    weigh_in_ref: wi-2415fc\n---\n\n# No GUI\n\nWhy.\n";

fn ruling(id: &str, ruled_at: &str, summary: &str, file: &str) -> Ruling {
    Ruling {
        id: id.to_owned(),
        ruled_at: ruled_at.to_owned(),
        summary: summary.to_owned(),
        weigh_in_ref: None,
        file: PathBuf::from(file),
    }
}

fn text(id: &str) -> String {
    format!("---\nid: {id}\nruled_at: 2026-10-11\nsummary: Ruling {id}.\n---\n")
}

#[test]
fn a_ruling_reads_its_four_fields_and_names_its_file() -> TestResult {
    let read = parse(Path::new(".out-of-scope/no-gui.md"), FULL)?;

    assert_eq!(
        read,
        Ruling {
            weigh_in_ref: Some("wi-2415fc".to_owned()),
            ..ruling(
                "no-gui",
                "2026-10-11",
                "No GUI in v0.1; the CLI is the surface.",
                ".out-of-scope/no-gui.md"
            )
        },
        "every field, the body ignored"
    );
    Ok(())
}

#[test]
fn weigh_in_ref_is_optional_and_ruled_at_may_carry_a_time() -> TestResult {
    let text =
        "---\nid: x\nruled_at: \"2026-10-11T09:30:00Z\"\nsummary: >\n  Folded\n  summary.\n---\n";

    let read = parse(Path::new("x.md"), text)?;

    assert_eq!(
        read,
        ruling("x", "2026-10-11T09:30:00Z", "Folded summary.", "x.md"),
        "no weigh-in, a timestamp, a folded summary"
    );
    Ok(())
}

#[test]
fn an_invalid_ruling_is_an_error_naming_the_file_and_what_is_wrong() {
    let cases = [
        ("# no frontmatter\n", "frontmatter, line 1"),
        ("---\nid: x\n", "frontmatter, no closing fence"),
        ("---\nid: x\nruled_at: 2026-10-11\n---\n", "missing summary"),
        (
            "---\nid: x\nruled_at: 2026-10-11\nsummary: s\nowner: me\n---\n",
            "unknown key",
        ),
        (
            "---\nid: two words\nruled_at: 2026-10-11\nsummary: s\n---\n",
            "id with a space",
        ),
        (
            "---\nid: x\nruled_at: 11/10/2026\nsummary: s\n---\n",
            "not a date",
        ),
        (
            "---\nid: x\nruled_at: 2026-13-01\nsummary: s\n---\n",
            "no month 13",
        ),
        (
            "---\nid: x\nruled_at: 2026-10-11\nsummary: \"\"\n---\n",
            "empty summary",
        ),
        (
            "---\nid: x\nruled_at: 2026-10-11\nsummary: s\nweigh_in_ref:\n---\n",
            "empty ref",
        ),
    ];

    for (text, why) in cases {
        let result = parse(Path::new(".out-of-scope/bad.md"), text);
        let error = result.err();
        let named = error.as_ref().map(ToString::to_string);
        assert!(
            named
                .as_deref()
                .is_some_and(|m| m.starts_with(".out-of-scope/bad.md")),
            "{why}: {error:?}"
        );
    }
}

#[test]
fn a_specific_fault_has_its_own_variant() {
    let missing = parse(Path::new("a.md"), "---\nid: x\nsummary: s\n---\n");
    let unknown = parse(
        Path::new("a.md"),
        "---\nid: x\nruled_at: 2026-10-11\nsummary: s\nby: me\n---\n",
    );

    assert!(
        matches!(
            missing,
            Err(RulingError::MissingField {
                field: "ruled_at",
                ..
            })
        ),
        "{missing:?}"
    );
    assert!(
        matches!(&unknown, Err(RulingError::UnknownField { field, line: 5, .. }) if field == "by"),
        "{unknown:?}"
    );
}

#[test]
fn the_list_is_sorted_by_id_whatever_the_file_names() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = dir.path().join(".out-of-scope");
    fs::create_dir(&store)?;
    fs::write(store.join("a.md"), text("zeta"))?;
    fs::write(store.join("b.md"), text("Alpha"))?;
    fs::write(store.join("c.md"), text("alpha"))?;
    fs::write(store.join("notes.txt"), "not a ruling")?;
    fs::create_dir(store.join("drafts.md"))?;

    let ids: Vec<String> = list(&store)?.into_iter().map(|r| r.id).collect();

    assert_eq!(
        ids,
        ["Alpha", "alpha", "zeta"],
        "byte order; only .md files"
    );
    Ok(())
}

#[test]
fn a_missing_store_has_no_rulings() -> TestResult {
    let dir = tempfile::tempdir()?;

    let rulings = list(&dir.path().join(".out-of-scope"))?;

    assert_eq!(rulings, [], "none yet");
    Ok(())
}

#[test]
fn a_duplicate_id_names_both_files() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("one.md"), text("same"))?;
    fs::write(dir.path().join("two.md"), text("same"))?;

    let result = list(dir.path());

    assert!(
        matches!(
            &result,
            Err(RulingError::DuplicateId { id, first, second })
                if id == "same" && first.ends_with("one.md") && second.ends_with("two.md")
        ),
        "{result:?}"
    );
    Ok(())
}

#[test]
fn one_invalid_file_fails_the_whole_list() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("good.md"), text("good"))?;
    fs::write(dir.path().join("bad.md"), "no frontmatter\n")?;

    let result = list(dir.path());

    assert!(
        matches!(&result, Err(RulingError::Frontmatter { file, .. }) if file.ends_with("bad.md")),
        "never skipped silently: {result:?}"
    );
    Ok(())
}
