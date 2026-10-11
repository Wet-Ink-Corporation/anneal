use super::{Report, Rule, Source, SpecFinding, check};
use crate::lint::terms::Glossary;

/// A spec with every rule satisfied against [`CONSTITUTION`].
pub(super) const CLEAN: &str = "\
# Spec: export

## Problem

Users cannot take their data out.

## Users

Analysts.

## Requirements

- Export a report as CSV.

## Acceptance criteria

- WHEN the user asks for an export, THE exporter SHALL write a CSV file.
- **AC-2**: WHILE offline, THE exporter SHALL queue the export.

## Out of Scope

- PDF export.

## Open questions

- Which delimiter? [NEEDS CLARIFICATION: comma or semicolon]

## Constitution check

Constitution version: 2

- [G1] pass
- [G2] violated: a second file format

### Complexity

| Gate | Violation | Justification |
|---|---|---|
| G2 | CSV beside JSON | analysts' tools read only CSV |
";

/// The constitution [`CLEAN`] checks against.
pub(super) const CONSTITUTION: &str = "\
# Constitution

**Version**: 2 | **Ratified**: 2026-10-11

## Gates

- [G1] Library first.
- [G2] One output format per command.
";

fn spec(text: &str) -> Source<'_> {
    Source {
        file: "spec.md",
        text,
    }
}

fn constitution(text: &str) -> Source<'_> {
    Source {
        file: "constitution.md",
        text,
    }
}

/// Each finding as `(rule, line, subject)`, the identity a test usually needs.
pub(super) fn found(report: &Report) -> Vec<(Rule, usize, Option<&str>)> {
    report
        .findings
        .iter()
        .map(|finding| (finding.rule, finding.line, finding.subject.as_deref()))
        .collect()
}

fn edited(from: &str, to: &str) -> String {
    assert!(CLEAN.contains(from), "the clean spec holds {from:?}");
    CLEAN.replacen(from, to, 1)
}

#[test]
fn a_complete_spec_has_no_finding_and_counts_criteria_and_markers() {
    let report = check(spec(CLEAN), None, Some(constitution(CONSTITUTION)));

    assert_eq!(report.findings, [], "clean");
    assert_eq!(report.criteria, 2, "two criteria");
    assert_eq!(report.needs_clarification, 1, "one marker");
    assert_eq!(
        report.constitution_version.as_deref(),
        Some("2"),
        "version read through the bold markers"
    );
    assert_eq!(report.gates, 2, "two gates");
}

#[test]
fn each_missing_section_is_one_finding_on_the_whole_file() {
    let text =
        edited("## Users\n\nAnalysts.\n\n", "").replacen("## Open questions", "## Questions", 1);

    let report = check(spec(&text), None, None);

    assert_eq!(
        found(&report),
        [
            (Rule::Sections, 0, Some("Open questions")),
            (Rule::Sections, 0, Some("Users")),
        ],
        "line 0, sorted by section"
    );
    assert_eq!(
        report.findings.last().map(|finding| finding.fix.as_str()),
        Some("add a `## Users` section (see `playbooks/spec.template.md`)."),
        "the fix names the heading"
    );
}

#[test]
fn section_titles_match_in_any_letter_case_and_a_repeat_is_a_finding() {
    let text = edited("## Out of Scope", "## OUT OF SCOPE") + "\n## Users\n\nAgain.\n";

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(
        found(&report),
        [(Rule::Sections, 41, Some("Users"))],
        "case ignored; the second Users heading is the finding"
    );
}

#[test]
fn a_criterion_that_is_not_ears_names_the_closest_template() {
    let text = edited(
        "- WHEN the user asks for an export, THE exporter SHALL write a CSV file.",
        "- When the user asks for an export, the exporter writes a CSV file.",
    );

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(
        report.findings,
        [SpecFinding {
            rule: Rule::Ears,
            file: "spec.md".to_owned(),
            line: 17,
            message: "acceptance criterion is not EARS: starts with `When`, not an EARS keyword \
                      (`WHERE`, `WHILE`, `WHEN`, `IF` or `THE`, in capitals)."
                .to_owned(),
            fix: "write it as `WHEN <trigger>, THE <system> SHALL <response>.`".to_owned(),
            subject: None,
        }],
        "line, message and template"
    );
}

#[test]
fn labels_and_checkboxes_before_a_criterion_are_not_part_of_it() {
    let text = edited("- **AC-2**: WHILE offline,", "- [x] AC-2.1: WHILE offline,");

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(found(&report), [], "label and checkbox skipped");
}

#[test]
fn a_fence_indented_four_spaces_inside_a_criterion_is_hidden() {
    let text = edited(
        "- WHEN the user asks for an export, THE exporter SHALL write a CSV file.\n",
        "- THE exporter SHALL log each row, for example:\n\
         \n\
         \x20   ```text\n\
         \x20   - row 1\n\
         \x20   ## Not a heading\n\
         \x20   ```\n",
    );

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(
        found(&report),
        [],
        "the fence's lines are not criteria or sections"
    );
    assert_eq!(report.criteria, 2, "the fenced item is not a criterion");
}

#[test]
fn a_bold_label_with_its_colon_inside_is_skipped() {
    let text = edited("- **AC-2**: WHILE offline,", "- **AC-2:** WHILE offline,");

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(found(&report), [], "`**AC-2:**` is a label");
}

#[test]
fn markers_inside_double_backtick_spans_do_not_count() {
    let text = edited(
        "- PDF export.",
        "- PDF ``[NEEDS CLARIFICATION]`` and `` a `[NEEDS CLARIFICATION]` b ``",
    );

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(
        report.needs_clarification, 1,
        "only the open question's marker"
    );
}

#[test]
fn acceptance_criteria_without_a_list_item_is_a_finding() {
    let text = edited(
        "- WHEN the user asks for an export, THE exporter SHALL write a CSV file.\n\
         - **AC-2**: WHILE offline, THE exporter SHALL queue the export.\n",
        "Criteria to come.\n",
    );

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(found(&report), [(Rule::Ears, 15, None)], "at the heading");
    assert_eq!(report.criteria, 0, "none");
}

#[test]
fn an_out_of_scope_holding_only_a_comment_is_empty() {
    let text = edited("- PDF export.", "<!-- what this will not do -->");

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(
        found(&report),
        [(Rule::OutOfScope, 20, None)],
        "at the heading"
    );
}

#[test]
fn four_markers_are_a_finding_at_the_fourth_and_hidden_ones_do_not_count() {
    let text = edited(
        "- PDF export.",
        "- PDF [NEEDS CLARIFICATION] and [NEEDS CLARIFICATION: which]\n\
         - `[NEEDS CLARIFICATION]` <!-- [NEEDS CLARIFICATION] --> [NEEDS CLARIFICATIONS]\n\
         - Excel [NEEDS CLARIFICATION]",
    );

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    assert_eq!(
        report.needs_clarification, 4,
        "inline code, comments, other words skipped"
    );
    assert_eq!(
        found(&report),
        [(Rule::NeedsClarification, 28, None)],
        "the fourth marker, in the open questions"
    );
}

#[test]
fn avoided_terms_are_findings_with_the_term_as_subject() {
    let glossary = Glossary::parse("**Exporter**: x\n_Avoid_: dumper, data dump\n");
    let text = edited("Analysts.", "Analysts who run a Data Dump with the dumper.");

    let report = check(
        spec(&text),
        Some(&glossary),
        Some(constitution(CONSTITUTION)),
    );

    assert_eq!(
        found(&report),
        [
            (Rule::AvoidedTerm, 9, Some("data dump")),
            (Rule::AvoidedTerm, 9, Some("dumper")),
        ],
        "one per term"
    );
}

#[test]
fn findings_come_sorted_by_line_then_rule() {
    let text = edited("- PDF export.", "") + "- [G3] pass\n";

    let report = check(spec(&text), None, Some(constitution(CONSTITUTION)));

    let lines: Vec<usize> = report.findings.iter().map(|finding| finding.line).collect();
    assert_eq!(lines, [20, 40], "Out of Scope, then the unknown gate");
}
