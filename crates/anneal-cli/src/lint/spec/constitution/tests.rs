use std::fmt::Write;

use super::{Constitution, Gate};
use crate::lint::spec::tests::{CLEAN, CONSTITUTION, found};
use crate::lint::spec::{Report, Rule, Source, check};

fn run(spec: &str, constitution: Option<&str>) -> Report {
    check(
        Source {
            file: "spec.md",
            text: spec,
        },
        None,
        constitution.map(|text| Source {
            file: "constitution.md",
            text,
        }),
    )
}

fn spec_with(from: &str, to: &str) -> String {
    assert!(CLEAN.contains(from), "the clean spec holds {from:?}");
    CLEAN.replacen(from, to, 1)
}

fn constitution_with(from: &str, to: &str) -> String {
    assert!(
        CONSTITUTION.contains(from),
        "the constitution holds {from:?}"
    );
    CONSTITUTION.replacen(from, to, 1)
}

#[test]
fn the_version_and_the_gates_are_read_with_their_lines() {
    let parsed = Constitution::parse(
        "Version: 1.2.0\n- [ ] not a gate\n- [G1] One.\n- plain\n```\n- [G9] in code\n```\n1. [I-2] Two.\n",
    );

    assert_eq!(
        parsed.version.as_deref(),
        Some("1.2.0"),
        "first version line"
    );
    assert_eq!(
        parsed.gates,
        [
            Gate {
                id: "G1".to_owned(),
                line: 3
            },
            Gate {
                id: "I-2".to_owned(),
                line: 8
            },
        ],
        "list items opening with [id], outside code"
    );
}

#[test]
fn a_link_list_item_is_not_a_gate() {
    let parsed = Constitution::parse(
        "Version: 1\n- [Library-first](https://example.com/library-first)\n- [G1] One.\n",
    );

    assert_eq!(
        parsed.gates,
        [Gate {
            id: "G1".to_owned(),
            line: 3
        }],
        "`[text](url)` is a link"
    );
}

#[test]
fn a_constitution_with_no_gates_is_a_finding_on_the_constitution() {
    let report = run(CLEAN, Some("**Version**: 2\n"));

    assert_eq!(
        report
            .findings
            .first()
            .map(|f| (f.file.as_str(), f.line, f.subject.as_deref())),
        Some(("constitution.md", 0, Some("gates"))),
        "no gate to check against"
    );
    assert_eq!(report.gates, 0, "none read");
}

#[test]
fn thousands_of_violated_gates_are_each_matched_to_their_row() {
    const GATES: usize = 3000;
    let (head, _) = CLEAN
        .split_once("## Constitution check")
        .expect("the clean spec has the section");
    let mut spec = format!("{head}## Constitution check\n\nConstitution version: 2\n\n");
    let mut constitution = String::from("Version: 2\n\n");
    let mut table = String::from("| Gate | Violation | Justification |\n|---|---|---|\n");
    for n in 0..GATES {
        writeln!(constitution, "- [G{n}] Gate {n}.").expect("a String takes writes");
        writeln!(spec, "- [G{n}] violated: reason {n}").expect("a String takes writes");
        writeln!(table, "| G{n} | breaks {n} | needed {n} |").expect("a String takes writes");
    }
    spec.push('\n');
    spec.push_str(&table);

    let report = run(&spec, Some(&constitution));

    assert_eq!(report.findings, [], "every violation justified");
    assert_eq!(report.gates, GATES, "all gates read");
}

#[test]
fn a_version_other_than_the_constitutions_is_a_finding_at_the_spec_line() {
    let constitution = constitution_with("**Version**: 2", "**Version**: 3");

    let report = run(CLEAN, Some(&constitution));

    assert_eq!(found(&report), [(Rule::Constitution, 30, None)], "mismatch");
    assert_eq!(
        report.findings.first().map(|finding| finding.fix.as_str()),
        Some(
            "check the spec against version 3 of constitution.md, then write `Constitution version: 3`."
        ),
        "fix names the current version"
    );
}

#[test]
fn a_spec_that_names_no_version_is_a_finding_at_the_section() {
    let spec = spec_with("Constitution version: 2\n", "\n");

    let report = run(&spec, Some(CONSTITUTION));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 28, None)],
        "at the heading"
    );
}

#[test]
fn a_constitution_without_a_version_is_a_finding_on_the_constitution() {
    let constitution = constitution_with("**Version**: 2 | **Ratified**: 2026-10-11\n", "");

    let report = run(CLEAN, Some(&constitution));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 0, None)],
        "whole file"
    );
    assert_eq!(
        report.findings.first().map(|finding| finding.file.as_str()),
        Some("constitution.md"),
        "on the constitution"
    );
    assert_eq!(report.constitution_version, None, "no version");
}

#[test]
fn a_gate_listed_twice_in_the_constitution_is_a_finding_there() {
    let constitution = format!("{CONSTITUTION}- [G1] Again.\n");

    let report = run(CLEAN, Some(&constitution));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 9, Some("G1"))],
        "the repeat"
    );
}

#[test]
fn every_gate_of_the_constitution_must_be_checked() {
    let constitution = format!("{CONSTITUTION}- [G3] No globals.\n");

    let report = run(CLEAN, Some(&constitution));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 28, Some("G3"))],
        "at the section heading"
    );
}

#[test]
fn a_gate_the_constitution_lacks_or_a_repeated_entry_is_a_finding() {
    let spec = spec_with("- [G1] pass\n", "- [G1] pass\n- [G7] pass\n- [G1] n/a\n");

    let report = run(&spec, Some(CONSTITUTION));

    assert_eq!(
        found(&report),
        [
            (Rule::Constitution, 33, Some("G7")),
            (Rule::Constitution, 34, Some("G1")),
        ],
        "unknown gate, then the repeat"
    );
}

#[test]
fn a_gate_needs_pass_violated_or_n_a() {
    let spec = spec_with("- [G1] pass", "- [G1] passed");

    let report = run(&spec, Some(CONSTITUTION));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 32, Some("G1"))],
        "status"
    );
}

#[test]
fn a_violated_gate_without_a_complexity_row_is_a_finding() {
    let spec = spec_with(
        "| G2 | CSV beside JSON | analysts' tools read only CSV |\n",
        "",
    );

    let report = run(&spec, Some(CONSTITUTION));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 33, Some("G2"))],
        "at the gate"
    );
}

#[test]
fn a_complexity_row_with_an_empty_justification_is_a_finding() {
    let spec = spec_with(
        "| G2 | CSV beside JSON | analysts' tools read only CSV |",
        "| [G2] | CSV beside JSON |  |",
    );

    let report = run(&spec, Some(CONSTITUTION));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 39, Some("G2"))],
        "at the row"
    );
}

#[test]
fn a_complexity_row_for_a_gate_not_violated_is_a_finding() {
    let spec = format!("{CLEAN}| G1 | none | none |\n");

    let report = run(&spec, Some(CONSTITUTION));

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 40, Some("G1"))],
        "at the row"
    );
}

#[test]
fn without_a_constitution_violations_still_need_rows_but_no_version() {
    let spec = spec_with("Constitution version: 2\n", "\n").replacen(
        "| G2 | CSV beside JSON | analysts' tools read only CSV |\n",
        "",
        1,
    );

    let report = run(&spec, None);

    assert_eq!(
        found(&report),
        [(Rule::Constitution, 33, Some("G2"))],
        "row only"
    );
    assert_eq!(report.gates, 0, "no constitution, no gates");
}
