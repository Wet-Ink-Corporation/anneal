//! The plan rules: the header's spec link, the required sections, the tasks and their fields,
//! and the blocking edges between tasks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::parse::{self, Key};
use super::{PlanFinding, Rule, Task};
use crate::lint::spec::markdown::{Line, Section, heading};

/// The `## Tasks` section's title.
pub(super) const TASKS: &str = "Tasks";

/// The required level-2 sections, in template order. Titles match in any letter case.
const REQUIRED: [&str; 3] = ["Goal", TASKS, "Review focus"];

/// The required sections that must hold visible text (Tasks is checked by its tasks).
const NOT_EMPTY: [&str; 2] = ["Goal", "Review focus"];

const TEMPLATE: &str = "playbooks/plan.template.md";

/// The first section titled `title`, in any letter case.
pub(super) fn find<'s, 'd>(sections: &'s [Section<'d>], title: &str) -> Option<&'s Section<'d>> {
    sections
        .iter()
        .find(|section| section.title.eq_ignore_ascii_case(title))
}

/// One finding per missing required section (line 0), per repeat, and per empty one.
pub(super) fn sections(file: &str, sections: &[Section<'_>]) -> Vec<PlanFinding> {
    let mut findings = Vec::new();
    for title in REQUIRED {
        let mut matching = sections
            .iter()
            .filter(|section| section.title.eq_ignore_ascii_case(title));
        match matching.next() {
            None => findings.push(Rule::Sections.at(file, 0).finding(
                format!("no `## {title}` section"),
                format!("add a `## {title}` section (see `{TEMPLATE}`)."),
                Some(title),
            )),
            Some(first) if NOT_EMPTY.contains(&title) && is_empty(first.body) => {
                findings.push(Rule::Sections.at(file, first.line).finding(
                    format!("`## {title}` is empty"),
                    format!("write the {} under `## {title}`.", what(title)),
                    Some(title),
                ));
            }
            Some(_) => {}
        }
        findings.extend(matching.map(|repeat| {
            Rule::Sections.at(file, repeat.line).finding(
                format!("`## {title}` appears again; only the first is read"),
                format!("merge it into the first `## {title}` section."),
                Some(title),
            )
        }));
    }
    findings
}

fn is_empty(body: &[Line<'_>]) -> bool {
    body.iter().all(|line| line.text.trim().is_empty())
}

fn what(title: &str) -> &'static str {
    if title.eq_ignore_ascii_case("Goal") {
        "one-sentence goal"
    } else {
        "places a reviewer should look hardest, and why"
    }
}

/// The spec link from the header (the lines before the first `##` section): its target and
/// line, and the findings when there is none, it names nothing, or there are two.
pub(super) fn spec<'d>(
    file: &str,
    header: impl Iterator<Item = &'d Line<'d>>,
) -> (Option<(&'d str, usize)>, Vec<PlanFinding>) {
    let mut found = None;
    let mut findings = Vec::new();
    for line in header {
        let Some((Key::Spec, value)) = parse::field(&line.text) else {
            continue;
        };
        if found.is_some() {
            findings.push(Rule::Spec.at(file, line.number).finding(
                "a second `Spec:` line; a plan implements one spec",
                "keep one `Spec:` line, or split the plan per spec.",
                None,
            ));
            continue;
        }
        match parse::spec_target(value) {
            Some(target) => found = Some((target, line.number)),
            None => findings.push(Rule::Spec.at(file, line.number).finding(
                "the `Spec:` line names no file",
                "name the spec: `Spec: [<feature>](../specs/<feature>.md)`.",
                None,
            )),
        }
    }
    if found.is_none() && findings.is_empty() {
        findings.push(Rule::Spec.at(file, 0).finding(
            "no `Spec:` line before the first `##` section",
            "link the spec under the title: `- Spec: [<feature>](../specs/<feature>.md)`.",
            None,
        ));
    }
    (found, findings)
}

/// A task as read, with the lines its findings point at.
#[derive(Debug)]
pub(super) struct Read {
    /// The task.
    pub(super) task: Task,
    /// The `Blocked by:` line, or the heading's when there is none.
    pub(super) blocked_line: usize,
}

/// The tasks of the `## Tasks` section (level-3 headings and the lines up to the next one),
/// and a finding for no task, a malformed heading, a repeated id or a field missing, repeated
/// or malformed.
pub(super) fn tasks(file: &str, section: Option<&Section<'_>>) -> (Vec<Read>, Vec<PlanFinding>) {
    let Some(section) = section else {
        return (Vec::new(), Vec::new());
    };
    let starts: Vec<usize> = section
        .body
        .iter()
        .enumerate()
        .filter(|(_, line)| heading(&line.text).is_some_and(|(level, _)| level == 3))
        .map(|(index, _)| index)
        .collect();
    let mut findings = Vec::new();
    if starts.is_empty() {
        findings.push(Rule::Tasks.at(file, section.line).finding(
            format!("`## {TASKS}` has no `### <id>: <title>` task"),
            format!("add one `### T1: <title>` heading per task (see `{TEMPLATE}`)."),
            None,
        ));
    }
    let mut read: Vec<Read> = Vec::new();
    for (position, start) in starts.iter().enumerate() {
        let end = starts
            .get(position.saturating_add(1))
            .copied()
            .unwrap_or(section.body.len());
        let (Some(head), Some(body)) = (
            section.body.get(*start),
            section.body.get(start.saturating_add(1)..end),
        ) else {
            continue;
        };
        let title = heading(&head.text).map_or("", |(_, title)| title);
        let Some((id, title)) = parse::task_heading(title) else {
            findings.push(Rule::Tasks.at(file, head.number).finding(
                format!("`### {title}` is not a task heading `### <id>: <title>`"),
                "write `### T<n>: <title>`, an id of letters, digits, `-`, `_` or `.` (at most \
                 64), or make it a `####` heading inside the task above.",
                None,
            ));
            continue;
        };
        if read.iter().any(|earlier| earlier.task.id == id) {
            findings.push(Rule::Tasks.at(file, head.number).finding(
                format!("task id `{id}` is used again; only the first is read"),
                "give each task its own id.",
                Some(id),
            ));
            continue;
        }
        let head = Head {
            id,
            title,
            line: head.number,
        };
        read.push(task(file, head, body, &mut findings));
    }
    (read, findings)
}

/// A task heading as read.
#[derive(Debug, Clone, Copy)]
struct Head<'t> {
    id: &'t str,
    title: &'t str,
    line: usize,
}

/// The task under `head`, read from its body, with a finding per field missing, repeated or
/// malformed.
fn task(file: &str, head: Head<'_>, body: &[Line<'_>], findings: &mut Vec<PlanFinding>) -> Read {
    let Head { id, title, line } = head;
    let seen = field_lines(file, head, body, findings);
    let value = |key: Key| seen.get(key.as_str()).copied();
    let files: Vec<String> = value(Key::Files).map_or_else(Vec::new, |(at, value)| {
        let files = parse::code_spans(value);
        if files.is_empty() {
            findings.push(malformed(file, at, id, Key::Files));
        }
        files.into_iter().map(str::to_owned).collect()
    });
    let test = value(Key::Test).and_then(|(at, value)| {
        let test = parse::code_spans(value)
            .first()
            .map(|test| (*test).to_owned());
        if test.is_none() {
            findings.push(malformed(file, at, id, Key::Test));
        }
        test
    });
    let (blocked_line, blocked_by) =
        value(Key::BlockedBy).map_or((line, Vec::new()), |(at, value)| {
            if let Ok(ids) = parse::blockers(value) {
                (at, ids.into_iter().map(str::to_owned).collect())
            } else {
                findings.push(malformed(file, at, id, Key::BlockedBy));
                (at, Vec::new())
            }
        });
    let task = Task {
        id: id.to_owned(),
        title: title.to_owned(),
        line,
        files,
        test,
        blocked_by,
    };
    Read { task, blocked_line }
}

/// The first line of each task field in `body`, by key, with a finding per repeated or
/// missing field.
fn field_lines<'d>(
    file: &str,
    head: Head<'_>,
    body: &'d [Line<'d>],
    findings: &mut Vec<PlanFinding>,
) -> BTreeMap<&'static str, (usize, &'d str)> {
    let id = head.id;
    let mut seen: BTreeMap<&'static str, (usize, &str)> = BTreeMap::new();
    for body_line in body {
        let Some((key, value)) = parse::field(&body_line.text) else {
            continue;
        };
        if seen.contains_key(key.as_str()) {
            findings.push(Rule::TaskFields.at(file, body_line.number).finding(
                format!("task `{id}` has a second `{}:` line", key.as_str()),
                format!("merge it into the first `{}:` line.", key.as_str()),
                Some(key.as_str()),
            ));
        } else {
            seen.insert(key.as_str(), (body_line.number, value));
        }
    }
    for key in Key::TASK {
        if !seen.contains_key(key.as_str()) {
            findings.push(Rule::TaskFields.at(file, head.line).finding(
                format!("task `{id}` has no `{}:` line", key.as_str()),
                format!("add `- {}` to the task.", example(key)),
                Some(key.as_str()),
            ));
        }
    }
    seen
}

/// The example line for `key`, used in fixes.
const fn example(key: Key) -> &'static str {
    match key {
        Key::Spec => "Spec: [<feature>](../specs/<feature>.md)",
        Key::Files => "Files: <path in backticks>, ...",
        Key::Test => "Test: <command in backticks>",
        Key::BlockedBy => "Blocked by: none",
    }
}

fn malformed(file: &str, line: usize, id: &str, key: Key) -> PlanFinding {
    let (what, fix) = match key {
        Key::Files => (
            "names no file in backticks",
            "put each file in backticks: Files: `src/a.rs`, `tests/a.rs`.",
        ),
        Key::Test => (
            "has no command in backticks",
            "put the command in backticks: Test: `cargo test -p <crate> <name>`.",
        ),
        Key::Spec | Key::BlockedBy => (
            "is not `none` or a comma-separated list of task ids",
            "write it as `- Blocked by: none` or `- Blocked by: T1, T2`.",
        ),
    };
    Rule::TaskFields.at(file, line).finding(
        format!("task `{id}`'s `{}:` line {what}", key.as_str()),
        fix,
        Some(key.as_str()),
    )
}

/// A finding per blocker that is the task itself or no task of the plan, and one for the
/// tasks a cycle leaves unordered (in a cycle or behind one), at the first of them.
pub(super) fn blockers(file: &str, tasks: &[Read]) -> Vec<PlanFinding> {
    let index: BTreeMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .map(|(at, read)| (read.task.id.as_str(), at))
        .collect();
    let mut findings = Vec::new();
    let mut blocks: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    let mut waiting: Vec<usize> = vec![0; tasks.len()];
    for (at, read) in tasks.iter().enumerate() {
        let task = &read.task;
        let distinct: BTreeSet<&str> = task.blocked_by.iter().map(String::as_str).collect();
        for blocker in distinct {
            let finding = |message: String, fix: &str| {
                Rule::Blockers
                    .at(file, read.blocked_line)
                    .finding(message, fix, Some(blocker))
            };
            if blocker == task.id {
                findings.push(finding(
                    format!("task `{blocker}` is blocked by itself"),
                    "remove it from its own `Blocked by:` line.",
                ));
            } else if let Some(&from) = index.get(blocker) {
                if let Some(dependents) = blocks.get_mut(from) {
                    dependents.push(at);
                }
                if let Some(count) = waiting.get_mut(at) {
                    *count = count.saturating_add(1);
                }
            } else {
                findings.push(finding(
                    format!(
                        "task `{}` is blocked by `{blocker}`, which is not a task of this plan",
                        task.id
                    ),
                    "name a task of this plan, or `none`; outside blockers belong in the tracker \
                     (`anneal dag check`).",
                ));
            }
        }
    }
    findings.extend(cycle(file, tasks, &blocks, waiting));
    findings
}

/// Kahn's walk: the tasks never released are in a cycle or behind one.
fn cycle(
    file: &str,
    tasks: &[Read],
    blocks: &[Vec<usize>],
    mut waiting: Vec<usize>,
) -> Option<PlanFinding> {
    let mut ready: VecDeque<usize> = waiting
        .iter()
        .enumerate()
        .filter(|(_, count)| **count == 0)
        .map(|(at, _)| at)
        .collect();
    while let Some(done) = ready.pop_front() {
        for &dependent in blocks.get(done).map_or(&[][..], Vec::as_slice) {
            if let Some(count) = waiting.get_mut(dependent) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push_back(dependent);
                }
            }
        }
    }
    let stuck: Vec<&Read> = tasks
        .iter()
        .zip(&waiting)
        .filter(|(_, count)| **count > 0)
        .map(|(read, _)| read)
        .collect();
    let first = stuck.first()?;
    let ids: Vec<&str> = stuck.iter().map(|read| read.task.id.as_str()).collect();
    Some(Rule::Blockers.at(file, first.task.line).finding(
        format!(
            "tasks {} are blocked in a cycle (or behind one), so no order builds them",
            ids.join(", ")
        ),
        "remove the `Blocked by:` edge that closes the cycle; see `anneal dag check`.",
        None,
    ))
}
