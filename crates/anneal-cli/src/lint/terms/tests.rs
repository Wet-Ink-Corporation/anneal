use std::error::Error;
use std::fs::{self, File};
use std::path::Path;

use super::{AvoidedTerm, Glossary, Skipped, TermHit, TermsArgs, lint};
use crate::lint::skills::LintError;
use crate::lint::skills::discover::MAX_READ_BYTES;

type TestResult = Result<(), Box<dyn Error>>;

fn term(term: &str, line: usize) -> AvoidedTerm {
    AvoidedTerm {
        term: term.to_owned(),
        line,
    }
}

fn hit(file: &str, line: usize, term: &str) -> TermHit {
    TermHit {
        file: file.to_owned(),
        line,
        term: term.to_owned(),
    }
}

const GLOSSARY: &str = "# Context\n\n**Order**: what a customer asks for.\n\
                        _Avoid_: purchase, Purchase Order , `txn`.\n\n\
                        **Ruling**: a scope decision.\n- _Avoid_: verdict, PURCHASE,\n";

#[test]
fn avoid_lines_give_trimmed_terms_with_their_lines_and_no_duplicates() {
    let glossary = Glossary::parse(GLOSSARY);

    assert_eq!(
        glossary.terms(),
        [
            term("purchase", 4),
            term("Purchase Order", 4),
            term("txn", 4),
            term("verdict", 7),
        ],
        "terms split on `,`, trimmed of spaces, backticks and a final `.`; empty ones and \
         a term already listed in any case are dropped"
    );
}

#[test]
fn a_term_is_found_in_any_letter_case_with_its_file_and_line() {
    let glossary = Glossary::parse(GLOSSARY);

    let hits = glossary.scan("spec.md", "# Spec\nThe VERDICT is final.\nA Txn, then.\n");

    assert_eq!(
        hits,
        [hit("spec.md", 2, "verdict"), hit("spec.md", 3, "txn")],
        "case-insensitive whole words, 1-based lines"
    );
}

#[test]
fn a_term_inside_another_word_is_not_a_hit() {
    let glossary = Glossary::parse("_Avoid_: txn, verdict\n");

    let hits = glossary.scan(
        "spec.md",
        "txns\nmytxn\ntxn_id\nverdicts\nprevérdict\nverdict2\nVerdictÉ\n",
    );

    assert_eq!(
        hits,
        [],
        "prefixes, suffixes, `_`, digits and letters join words"
    );
}

#[test]
fn a_term_next_to_punctuation_or_a_line_edge_is_a_hit() {
    let glossary = Glossary::parse("_Avoid_: txn\n");

    let hits = glossary.scan("f", "txn\n(txn)\n`txn`.\n\"Txn\"\na-txn-b\n");

    let lines: Vec<usize> = hits.iter().map(|h| h.line).collect();
    assert_eq!(lines, [1, 2, 3, 4, 5], "punctuation and edges bound a word");
}

#[test]
fn a_multi_word_term_matches_across_a_single_space_in_any_case() {
    let glossary = Glossary::parse("_Avoid_: purchase  order\n");

    let hits = glossary.scan(
        "f",
        "a Purchase ORDER here\npurchase orders\npurchase\norder\npurchase  order\n",
    );

    assert_eq!(
        hits,
        [hit("f", 1, "purchase order")],
        "the term's inner whitespace is one space; a plural or a split is not a hit"
    );
}

#[test]
fn a_rejected_candidate_does_not_hide_an_overlapping_hit() {
    let glossary = Glossary::parse("_Avoid_: ab ab\n");

    let hits = glossary.scan("f", "xab ab ab\n");

    assert_eq!(hits, [hit("f", 1, "ab ab")], "the second `ab ab` is whole");
}

#[test]
fn a_line_is_one_hit_per_term_and_avoid_entries_are_not_hits() {
    let glossary = Glossary::parse(GLOSSARY);

    let hits = glossary.scan(
        "CONTEXT.md",
        "_Avoid_: verdict, txn\ntxn and txn, verdict\n",
    );

    assert_eq!(
        hits,
        [hit("CONTEXT.md", 2, "txn"), hit("CONTEXT.md", 2, "verdict")],
        "glossary order within a line; the entry line itself is skipped"
    );
}

/// A scratch directory holding `CONTEXT.md` (the shared glossary) and `files`.
fn workspace(files: &[(&str, &str)]) -> Result<tempfile::TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("CONTEXT.md"), GLOSSARY)?;
    for (name, text) in files {
        fs::write(dir.path().join(name), text)?;
    }
    Ok(dir)
}

fn args(dir: &Path, files: &[&str]) -> TermsArgs {
    TermsArgs {
        glossary: dir.join("CONTEXT.md"),
        files: files.iter().map(|f| dir.join(f)).collect(),
    }
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

#[test]
fn lint_reads_the_glossary_and_sorts_hits_by_file_line_and_term() -> TestResult {
    let dir = workspace(&[("b.md", "a txn\n"), ("a.md", "ok\nverdict txn\n")])?;

    let outcome = lint(&args(dir.path(), &["b.md", "a.md", "b.md"]))?;

    let a = shown(&dir.path().join("a.md"));
    let b = shown(&dir.path().join("b.md"));
    assert_eq!(
        outcome.hits,
        [hit(&a, 2, "txn"), hit(&a, 2, "verdict"), hit(&b, 1, "txn"),],
        "sorted, and a file named twice is scanned once"
    );
    assert_eq!(
        (outcome.terms, outcome.checked),
        (4, 2),
        "four terms, two files"
    );
    Ok(())
}

#[test]
fn an_oversized_file_is_listed_as_skipped_not_scanned() -> TestResult {
    let dir = workspace(&[])?;
    let big = dir.path().join("big.md");
    File::create(&big)?.set_len(MAX_READ_BYTES + 1)?;

    let outcome = lint(&args(dir.path(), &["big.md"]))?;

    assert_eq!(
        outcome.skipped,
        [Skipped {
            file: shown(&big),
            size: MAX_READ_BYTES + 1,
        }],
        "listed with its size"
    );
    assert_eq!((outcome.hits.len(), outcome.checked), (0, 0), "not scanned");
    Ok(())
}

#[test]
fn a_missing_or_unreadable_input_is_an_error_naming_it() -> TestResult {
    let dir = workspace(&[("a.md", "x\n")])?;
    let big = dir.path().join("big.md");
    File::create(&big)?.set_len(MAX_READ_BYTES + 1)?;
    let cases = [
        (args(dir.path(), &["missing.md"]), "missing file"),
        (args(dir.path(), &["."]), "a directory"),
        (
            TermsArgs {
                glossary: dir.path().join("NONE.md"),
                ..args(dir.path(), &["a.md"])
            },
            "missing glossary",
        ),
        (
            TermsArgs {
                glossary: big,
                ..args(dir.path(), &["a.md"])
            },
            "oversized glossary",
        ),
    ];

    for (case, why) in cases {
        let error = lint(&case).map(|o| o.hits);
        assert!(
            matches!(
                error,
                Err(LintError::MissingPath { .. }
                    | LintError::WrongKind { .. }
                    | LintError::TooLarge { .. })
            ),
            "{why}: {error:?}"
        );
        let code = error.err().map(|e| e.code().value());
        assert_eq!(code, Some(2), "{why} is a usage error");
    }
    Ok(())
}
