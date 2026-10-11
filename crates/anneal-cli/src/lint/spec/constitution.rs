//! The constitution and the spec's check against it.
//!
//! A constitution (`constitution.md`) has a version line (`Version: 2`) and gates, list items
//! that open with an id in brackets (`- [G1] Library first.`). A spec's `## Constitution check`
//! names the version it was checked against, lists every gate with `pass`, `violated` or `n/a`,
//! and justifies each violated gate in a `| Gate | Violation | Justification |` table. See
//! `docs/spec.md`.

use std::collections::BTreeSet;

use super::markdown::{Document, Row, Section, Table, items, tables};
use super::{At, Rule, SpecFinding};

/// The header of the complexity table, in order (matched in any letter case).
const COMPLEXITY_HEADER: [&str; 3] = ["Gate", "Violation", "Justification"];

/// A constitution: its version and its gates, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Constitution {
    /// The first version line's value, if any.
    pub(crate) version: Option<String>,
    /// Every gate, repeats included (a repeat is a finding).
    pub(crate) gates: Vec<Gate>,
}

/// One gate of the constitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gate {
    /// Its id, without the brackets.
    pub(crate) id: String,
    /// Its line.
    pub(crate) line: usize,
}

impl Constitution {
    /// Reads the version and the gates of `text`. Never fails: what is missing is a finding.
    pub(crate) fn parse(text: &str) -> Self {
        let doc = Document::parse(text);
        let version = doc.lines().iter().find_map(|line| version_of(&line.text));
        let gates = items(doc.lines())
            .into_iter()
            .filter_map(|item| {
                gate_ref(&item.text).map(|(id, _)| Gate {
                    id: id.to_owned(),
                    line: item.line,
                })
            })
            .collect();
        Self { version, gates }
    }
}

/// What the spec says about a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Violated,
    NotApplicable,
}

impl Status {
    /// The status opening `rest` (`pass`, `violated` or `n/a`, any letter case, trailing
    /// punctuation ignored).
    fn of(rest: &str) -> Option<Self> {
        let word = rest
            .split_whitespace()
            .next()?
            .trim_end_matches([':', ',', '.', ';'])
            .to_ascii_lowercase();
        match word.as_str() {
            "pass" => Some(Self::Pass),
            "violated" => Some(Self::Violated),
            "n/a" => Some(Self::NotApplicable),
            _ => None,
        }
    }
}

/// The value of a version line: `Version: <v>` or `Constitution version: <v>`, in any letter
/// case, after an optional list marker, with `*` and `_` emphasis ignored and anything after
/// a `|` dropped. The value is the first word.
fn version_of(text: &str) -> Option<String> {
    let plain: String = text.chars().filter(|c| !matches!(c, '*' | '_')).collect();
    let plain = plain.trim().trim_start_matches(['-', '+']).trim_start();
    let rest = ["constitution version:", "version:"]
        .into_iter()
        .find_map(|prefix| strip_prefix_any_case(plain, prefix))?;
    let value = rest.split('|').next()?.split_whitespace().next()?;
    Some(value.to_owned())
}

fn strip_prefix_any_case<'t>(text: &'t str, prefix: &str) -> Option<&'t str> {
    let head = text.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        text.get(prefix.len()..)
    } else {
        None
    }
}

/// The gate id opening a list item (`[G1] ...`) and the text after it. An id is two or more
/// ASCII letters, digits, `-`, `_` or `.`, starting with a letter, so a `[x]` checkbox and a
/// `[NEEDS CLARIFICATION]` marker are not gates. Brackets followed directly by `(` are a link
/// (`[Library-first](https://...)`), not a gate.
fn gate_ref(text: &str) -> Option<(&str, &str)> {
    let (id, after) = text.strip_prefix('[')?.split_once(']')?;
    let mut chars = id.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && id.len() >= 2
        && !after.starts_with('(');
    valid.then_some((id, after.trim()))
}

/// A constitution finding at `line` of `file`.
const fn at(file: &str, line: usize) -> At<'_> {
    Rule::Constitution.at(file, line)
}

/// The findings of the constitution check: the constitution's own (no version, a repeated
/// gate), then the spec's section against it.
///
/// Without a constitution, only the spec's violations and their justifications are checked.
/// Without the section, only the constitution's own findings (the missing section is a
/// `sections` finding).
pub(crate) fn check(
    spec_file: &str,
    section: Option<&Section<'_>>,
    constitution: Option<(&str, &Constitution)>,
) -> Vec<SpecFinding> {
    let mut findings = constitution.map_or_else(Vec::new, |(file, parsed)| own(file, parsed));
    let Some(section) = section else {
        return findings;
    };
    findings.extend(version(spec_file, section, constitution));
    let known: Option<BTreeSet<&str>> =
        constitution.map(|(_, parsed)| parsed.gates.iter().map(|gate| gate.id.as_str()).collect());
    let entries = entries(spec_file, section, known.as_ref());
    findings.extend(entries.findings);
    let unchecked = known
        .iter()
        .flatten()
        .filter(|id| !entries.listed.contains(**id))
        .map(|id| {
            at(spec_file, section.line)
                .finding(
                    format!("gate `{id}` of the constitution is not checked"),
                    format!("add `- [{id}] pass`, `- [{id}] violated: <why>` or `- [{id}] n/a`."),
                )
                .about(*id)
        });
    findings.extend(unchecked);
    findings.extend(complexity(spec_file, section, &entries.violated));
    findings
}

/// The constitution's own findings: no version line, a gate id used twice.
fn own(file: &str, constitution: &Constitution) -> Vec<SpecFinding> {
    let mut findings = Vec::new();
    if constitution.version.is_none() {
        findings.push(at(file, 0).finding(
            format!("{file} has no version line"),
            "add a line `Version: <N>` and raise it whenever a gate changes.",
        ));
    }
    if constitution.gates.is_empty() {
        findings.push(
            at(file, 0)
                .finding(
                    format!("{file} lists no gate"),
                    "list each gate as an item opening with its id, such as `- [G1] Library first.`",
                )
                .about("gates"),
        );
    }
    let mut seen = BTreeSet::new();
    for gate in &constitution.gates {
        if !seen.insert(gate.id.as_str()) {
            findings.push(
                at(file, gate.line)
                    .finding(
                        format!("gate `{}` is listed twice", gate.id),
                        "give each gate its own id; never reuse one.",
                    )
                    .about(&gate.id),
            );
        }
    }
    findings
}

/// The spec names the constitution's current version.
fn version(
    spec_file: &str,
    section: &Section<'_>,
    constitution: Option<(&str, &Constitution)>,
) -> Option<SpecFinding> {
    let (file, expected) = constitution
        .and_then(|(file, parsed)| parsed.version.as_deref().map(|version| (file, version)))?;
    let fix = format!(
        "check the spec against version {expected} of {file}, then write \
         `Constitution version: {expected}`."
    );
    let named = section
        .body
        .iter()
        .find_map(|line| version_of(&line.text).map(|version| (version, line.number)));
    match named {
        None => Some(at(spec_file, section.line).finding(
            format!("the constitution check does not name the version of {file}"),
            fix,
        )),
        Some((version, line)) if version != expected => Some(at(spec_file, line).finding(
            format!(
                "checked against constitution version {version}, but {file} is version {expected}"
            ),
            fix,
        )),
        Some(_) => None,
    }
}

/// The gate entries of the section, read.
#[derive(Debug, Default)]
struct Entries {
    findings: Vec<SpecFinding>,
    /// Every gate id the section lists.
    listed: BTreeSet<String>,
    /// The gates marked violated, with their lines.
    violated: Vec<(String, usize)>,
}

/// Reads the `- [id] status` entries. Each entry gets at most one finding: a repeat, then a
/// gate the constitution lacks, then an unknown status.
fn entries(spec_file: &str, section: &Section<'_>, known: Option<&BTreeSet<&str>>) -> Entries {
    let mut read = Entries::default();
    for item in items(section.body) {
        let Some((id, rest)) = gate_ref(&item.text) else {
            continue;
        };
        let finding =
            |message: String, fix: String| at(spec_file, item.line).finding(message, fix).about(id);
        if !read.listed.insert(id.to_owned()) {
            read.findings.push(finding(
                format!("gate `{id}` is listed twice"),
                format!("keep one `[{id}]` entry."),
            ));
        } else if known.is_some_and(|known| !known.contains(id)) {
            read.findings.push(finding(
                format!("gate `{id}` is not in the constitution"),
                format!("use a gate id the constitution lists, or remove `[{id}]`."),
            ));
        } else {
            match Status::of(rest) {
                None => read.findings.push(finding(
                    format!("gate `{id}` has no status `pass`, `violated` or `n/a`"),
                    format!("write `[{id}] pass`, `[{id}] violated: <why>` or `[{id}] n/a`."),
                )),
                Some(Status::Violated) => read.violated.push((id.to_owned(), item.line)),
                Some(Status::Pass | Status::NotApplicable) => {}
            }
        }
    }
    read
}

/// Every violated gate has a complexity row with a violation and a justification, and every
/// row is for a violated gate.
fn complexity(
    spec_file: &str,
    section: &Section<'_>,
    violated: &[(String, usize)],
) -> Vec<SpecFinding> {
    let rows: Vec<Row<'_>> = tables(section.body)
        .into_iter()
        .filter(is_complexity)
        .flat_map(|table| table.rows)
        .collect();
    // Both sides as sets, so matching is O(n log n), not a scan per gate (PERF-10).
    let row_gates: BTreeSet<&str> = rows.iter().map(row_gate).collect();
    let violated_ids: BTreeSet<&str> = violated.iter().map(|(id, _)| id.as_str()).collect();
    let mut findings: Vec<SpecFinding> = violated
        .iter()
        .filter(|(id, _)| !row_gates.contains(id.as_str()))
        .map(|(id, line)| {
            at(spec_file, *line)
                .finding(
                    format!("gate `{id}` is violated but has no row in the complexity table"),
                    format!(
                        "add `| {id} | <what breaks the gate> | <why it is needed> |` under a \
                         `| Gate | Violation | Justification |` header."
                    ),
                )
                .about(id)
        })
        .collect();
    findings.extend(
        rows.iter()
            .filter_map(|row| row_finding(spec_file, row, &violated_ids)),
    );
    findings
}

/// The finding for one complexity row, if any.
fn row_finding(spec_file: &str, row: &Row<'_>, violated: &BTreeSet<&str>) -> Option<SpecFinding> {
    let gate = row_gate(row);
    let finding = |message: String, fix: String| {
        Some(at(spec_file, row.line).finding(message, fix).about(gate))
    };
    if gate.is_empty() {
        return finding(
            "a complexity row names no gate".to_owned(),
            "put the violated gate's id in the first cell.".to_owned(),
        );
    }
    if !violated.contains(gate) {
        return finding(
            format!("the complexity table has a row for `{gate}`, which is not marked violated"),
            format!("remove the row, or mark the gate `[{gate}] violated: <why>`."),
        );
    }
    let filled = |index: usize| row.cells.get(index).is_some_and(|cell| !cell.is_empty());
    if filled(1) && filled(2) {
        return None;
    }
    finding(
        format!("the complexity row for `{gate}` has an empty violation or justification"),
        "say what breaks the gate and why the simpler option is not enough.".to_owned(),
    )
}

fn is_complexity(table: &Table<'_>) -> bool {
    table.header.len() == COMPLEXITY_HEADER.len()
        && table
            .header
            .iter()
            .zip(COMPLEXITY_HEADER)
            .all(|(cell, name)| cell.eq_ignore_ascii_case(name))
}

/// The gate a row names: its first cell without brackets or backticks.
fn row_gate<'d>(row: &Row<'d>) -> &'d str {
    row.cells
        .first()
        .map_or("", |cell| cell.trim_matches(['[', ']', '`']).trim())
}

#[cfg(test)]
mod tests;
