use std::borrow::Cow;

use super::{MAX_FINDINGS, Plan, Report, Rule, Task, check, spec_path};
use crate::lint::spec::Source;
use crate::path_text::slashed;

/// A plan with every rule satisfied.
const CLEAN: &str = "\
# Plan: export

- Spec: [export](../specs/export.md)

## Goal

Analysts can export a report as CSV.

## Architecture

A CSV writer behind the report port.

## Tasks

Tasks run top to bottom.

### T1: CSV writer

- Files: `src/csv.rs`, `tests/csv.rs`
- Test: `cargo test -p report csv`
- Blocked by: none

1. Write the failing test.

#### Notes

A level-4 heading stays inside its task.

### Task T2: Export command

- **Files:** `src/export.rs`
- **Test:** `cargo test -p report export`
- **Blocked by:** T1

## Review focus

- The quoting of fields with commas.
";

fn plan(text: &str) -> Source<'_> {
    Source {
        file: "plan.md",
        text,
    }
}

/// Each finding as `(rule, line, subject)`.
fn found(report: &Report) -> Vec<(Rule, usize, Option<&str>)> {
    report
        .findings
        .iter()
        .map(|finding| (finding.rule, finding.line, finding.subject.as_deref()))
        .collect()
}

/// A task from its heading (`id`, `title`, `line`), files, test command and blockers.
fn task(heading: (&str, &str, usize), files: &[&str], test: &str, blocked_by: &[&str]) -> Task {
    let (id, title, line) = heading;
    Task {
        id: id.to_owned(),
        title: title.to_owned(),
        line,
        files: files.iter().map(|file| (*file).to_owned()).collect(),
        test: Some(test.to_owned()),
        blocked_by: blocked_by.iter().map(|id| (*id).to_owned()).collect(),
    }
}

#[test]
fn a_clean_plan_has_no_findings_and_reads_its_spec_and_tasks() {
    let report = check(plan(CLEAN));

    assert_eq!(found(&report), vec![], "clean");
    assert_eq!(
        report.plan,
        Plan {
            spec: Some("../specs/export.md".to_owned()),
            tasks: vec![
                task(
                    ("T1", "CSV writer", 17),
                    &["src/csv.rs", "tests/csv.rs"],
                    "cargo test -p report csv",
                    &[]
                ),
                task(
                    ("T2", "Export command", 29),
                    &["src/export.rs"],
                    "cargo test -p report export",
                    &["T1"]
                ),
            ],
        },
        "spec link and tasks"
    );
    assert_eq!(report.spec_line, 3, "the Spec line");
}

#[test]
fn the_shipped_template_checks_clean() {
    let template = include_str!("../../../../../plugin/playbooks/plan.template.md");

    let report = check(plan(template));

    assert_eq!(found(&report), vec![], "the template is a clean plan");
    assert!(report.plan.spec.is_some(), "the template links a spec");
}

#[test]
fn a_missing_repeated_or_empty_section_is_a_sections_finding() {
    let text = CLEAN
        .replace(
            "## Goal\n\nAnalysts can export a report as CSV.\n",
            "## Goal\n\n<!-- later -->\n",
        )
        .replace(
            "## Review focus\n\n- The quoting of fields with commas.\n",
            "## Tasks\n",
        );

    let report = check(plan(&text));

    assert_eq!(
        found(&report),
        vec![
            (Rule::Sections, 0, Some("Review focus")),
            (Rule::Sections, 5, Some("Goal")),
            (Rule::Sections, 35, Some("Tasks")),
        ],
        "missing Review focus, empty Goal, repeated Tasks"
    );
}

#[test]
fn the_spec_link_may_be_a_link_code_or_plain_text_and_must_be_there() {
    for (line, target) in [
        ("Spec: `docs/specs/export.md`", "docs/specs/export.md"),
        ("**Spec:** docs/specs/export.md", "docs/specs/export.md"),
        ("- **Spec**: [the spec](specs/export.md)", "specs/export.md"),
    ] {
        let text = CLEAN.replace("- Spec: [export](../specs/export.md)", line);
        let report = check(plan(&text));
        assert_eq!(found(&report), vec![], "{line}");
        assert_eq!(report.plan.spec.as_deref(), Some(target), "{line}");
    }

    let missing = check(plan(
        &CLEAN.replace("- Spec: [export](../specs/export.md)", ""),
    ));
    assert_eq!(found(&missing), vec![(Rule::Spec, 0, None)], "no Spec line");
    assert_eq!(missing.plan.spec, None, "no spec read");

    let blank = check(plan(&CLEAN.replace("[export](../specs/export.md)", "")));
    assert_eq!(
        found(&blank),
        vec![(Rule::Spec, 3, None)],
        "a Spec line naming nothing"
    );

    let twice = CLEAN.replace("## Goal", "Spec: other.md\n\n## Goal");
    assert_eq!(
        found(&check(plan(&twice))),
        vec![(Rule::Spec, 5, None)],
        "a second Spec line"
    );
}

#[test]
fn a_spec_link_with_an_angle_bracket_destination_names_the_spec_inside_them() {
    for (line, target) in [
        ("- Spec: [s](<spec.md>)", "spec.md"),
        (
            "- Spec: [s](<specs/my spec (v2).md>)",
            "specs/my spec (v2).md",
        ),
    ] {
        let text = CLEAN.replace("- Spec: [export](../specs/export.md)", line);

        let report = check(plan(&text));

        assert_eq!(
            (found(&report), report.plan.spec.as_deref()),
            (vec![], Some(target)),
            "{line}"
        );
    }
}

#[test]
fn a_spec_line_after_the_first_section_does_not_count() {
    let text = CLEAN
        .replace("- Spec: [export](../specs/export.md)\n", "")
        .replace(
            "## Architecture\n",
            "## Architecture\n\nSpec: docs/specs/export.md\n",
        );

    assert_eq!(
        found(&check(plan(&text))),
        vec![(Rule::Spec, 0, None)],
        "only the header counts"
    );
}

#[test]
fn tasks_need_well_formed_headings_and_unique_ids() {
    let none = CLEAN
        .split("### T1")
        .next()
        .map(|head| format!("{head}## Review focus\n\n- x\n"))
        .expect("split yields a head");
    assert_eq!(
        found(&check(plan(&none))),
        vec![(Rule::Tasks, 13, None)],
        "no task"
    );

    let malformed = CLEAN.replace("### Task T2: Export command", "### Export command");
    assert_eq!(
        found(&check(plan(&malformed))),
        vec![(Rule::Tasks, 29, None)],
        "a level-3 heading that is not `<id>: <title>`"
    );

    let duplicate = CLEAN
        .replace("### Task T2: Export command", "### T1: Export command")
        .replace("Blocked by:** T1", "Blocked by:** none");
    let report = check(plan(&duplicate));
    assert_eq!(
        found(&report),
        vec![(Rule::Tasks, 29, Some("T1"))],
        "a repeated id"
    );
    assert_eq!(report.plan.tasks.len(), 1, "only the first T1 is read");
}

#[test]
fn every_task_needs_files_a_test_command_and_its_blockers() {
    let text = CLEAN
        .replace(
            "- Files: `src/csv.rs`, `tests/csv.rs`\n",
            "- Files: src/csv.rs\n",
        )
        .replace("- Test: `cargo test -p report csv`\n", "")
        .replace("- **Blocked by:** T1\n", "")
        .replace(
            "- **Test:** `cargo test -p report export`\n",
            "- **Test:** `cargo test`\n- Test: `again`\n",
        );

    let report = check(plan(&text));

    assert_eq!(
        found(&report),
        vec![
            (Rule::TaskFields, 17, Some("Test")),
            (Rule::TaskFields, 19, Some("Files")),
            (Rule::TaskFields, 28, Some("Blocked by")),
            (Rule::TaskFields, 32, Some("Test")),
        ],
        "T1: files not in backticks, no test; T2: no blockers, a second Test line"
    );
}

#[test]
fn blockers_must_name_other_tasks_of_the_plan_without_a_cycle() {
    let dangling = CLEAN.replace("Blocked by:** T1", "Blocked by:** T1, `T9`");
    assert_eq!(
        found(&check(plan(&dangling))),
        vec![(Rule::Blockers, 33, Some("T9"))],
        "an unknown blocker"
    );

    let own = CLEAN.replace("Blocked by:** T1", "Blocked by:** T2");
    assert_eq!(
        found(&check(plan(&own))),
        vec![(Rule::Blockers, 33, Some("T2"))],
        "a task blocking itself"
    );

    let cycle = CLEAN.replace("- Blocked by: none", "- Blocked by: T2");
    assert_eq!(
        found(&check(plan(&cycle))),
        vec![(Rule::Blockers, 17, None)],
        "T1 and T2 block each other"
    );

    let garbled = CLEAN.replace("Blocked by:** T1", "Blocked by:** T1,, T2?");
    assert_eq!(
        found(&check(plan(&garbled))),
        vec![(Rule::TaskFields, 33, Some("Blocked by"))],
        "a blocker list that is not ids"
    );
}

#[test]
fn finding_keys_name_the_rule_file_line_and_subject() {
    let report = check(plan(&CLEAN.replace("Blocked by:** T1", "Blocked by:** T9")));
    let keys: Vec<String> = report
        .findings
        .iter()
        .map(super::PlanFinding::key)
        .collect();

    assert_eq!(
        keys,
        vec!["lint/plan/blockers:plan.md:33:T9".to_owned()],
        "stable key"
    );
}

#[test]
fn findings_stop_at_the_cap_and_the_rest_are_counted() {
    let repeats: String = (0..5_000).map(|_| "### T1: again\n\n").collect();
    let text = CLEAN.replace("## Review focus", &format!("{repeats}## Review focus"));

    let report = check(plan(&text));

    assert_eq!(report.findings.len(), MAX_FINDINGS, "capped");
    assert_eq!(report.omitted, 5_000 - MAX_FINDINGS, "the rest counted");
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.rule == Rule::Tasks),
        "all repeated ids"
    );
    assert_eq!(
        report.findings.first().map(|finding| finding.line),
        Some(35),
        "the first ones in line order are kept: the first repeat replaces `## Review focus`"
    );
    assert_eq!(report.plan.tasks.len(), 2, "each id read once");
}

#[test]
fn thousands_of_chained_tasks_check_clean() {
    let tasks: Vec<String> = (1..=5_000)
        .map(|n| {
            let blocked = if n == 1 {
                "none".to_owned()
            } else {
                format!("T{}", n - 1)
            };
            format!(
                "### T{n}: step\n\n- Files: `src/{n}.rs`\n- Test: `cargo test t{n}`\n- Blocked by: \
                 {blocked}\n\n"
            )
        })
        .collect();
    let text = format!(
        "# Plan\n\n- Spec: [s](s.md)\n\n## Goal\n\nGo.\n\n## Tasks\n\n{}## Review focus\n\n- \
         Order.\n",
        tasks.concat()
    );

    let report = check(plan(&text));

    assert_eq!(found(&report), Vec::new(), "clean");
    assert_eq!(report.omitted, 0, "nothing omitted");
    assert_eq!(report.plan.tasks.len(), 5_000, "every task read");
}

#[test]
fn a_relative_spec_joins_the_plan_directory_with_forward_slashes() {
    assert_eq!(
        spec_path("docs/plans/p.md", "../specs/s.md", '/'),
        "docs/plans/../specs/s.md",
        "unix"
    );
    assert_eq!(
        spec_path(r"docs\plans\p.md", "../specs/s.md", '\\'),
        "docs/plans/../specs/s.md",
        "Windows separators become `/`"
    );
    assert_eq!(spec_path("p.md", "s.md", '/'), "s.md", "no directory");
    assert_eq!(spec_path("/p.md", "s.md", '/'), "/s.md", "the root");
}

#[test]
fn an_absolute_spec_is_used_as_written() {
    assert_eq!(
        spec_path("docs/p.md", "/srv/specs/s.md", '/'),
        "/srv/specs/s.md",
        "unix absolute"
    );
    assert_eq!(
        spec_path(r"docs\p.md", r"C:\specs\s.md", '\\'),
        r"C:\specs\s.md",
        "a Windows drive"
    );
    assert_eq!(
        spec_path(r"docs\p.md", r"\specs\s.md", '\\'),
        r"\specs\s.md",
        "a Windows rooted path"
    );
    assert_eq!(
        spec_path("docs/p.md", "C:/specs/s.md", '/'),
        "docs/C:/specs/s.md",
        "a drive means nothing on unix"
    );
}

#[test]
fn slashed_replaces_only_a_foreign_separator() {
    assert_eq!(slashed(r"a\b\c.md", '\\'), "a/b/c.md", "Windows");
    assert_eq!(
        slashed(r"a\b.md", '/'),
        r"a\b.md",
        "a backslash is a name on unix"
    );
    assert!(
        matches!(slashed("a/b.md", '\\'), Cow::Borrowed(_)),
        "nothing to replace"
    );
}
