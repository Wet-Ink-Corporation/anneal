//! The spec's own rules: required sections, EARS criteria, a non-empty Out of Scope, the
//! clarification-marker cap and avoided terms. The constitution check is in `constitution`.

use super::ears::{self, template};
use super::markdown::{Line, Section, items};
use super::{MAX_CLARIFICATIONS, Rule, SpecFinding};
use crate::lint::terms::TermHit;

/// The `## Acceptance criteria` section's title.
pub(crate) const ACCEPTANCE_CRITERIA: &str = "Acceptance criteria";
/// The `## Out of Scope` section's title.
pub(crate) const OUT_OF_SCOPE: &str = "Out of Scope";
/// The `## Constitution check` section's title.
pub(crate) const CONSTITUTION_CHECK: &str = "Constitution check";

/// The required level-2 sections, in template order. Titles match in any letter case.
pub(crate) const REQUIRED: [&str; 7] = [
    "Problem",
    "Users",
    "Requirements",
    ACCEPTANCE_CRITERIA,
    OUT_OF_SCOPE,
    "Open questions",
    CONSTITUTION_CHECK,
];

/// A clarification marker: `[NEEDS CLARIFICATION]` or `[NEEDS CLARIFICATION: question]`.
const MARKER: &str = "[NEEDS CLARIFICATION";

const TEMPLATE: &str = "playbooks/spec.template.md";

/// The first section titled `title`, in any letter case.
pub(crate) fn find<'s, 'd>(sections: &'s [Section<'d>], title: &str) -> Option<&'s Section<'d>> {
    sections
        .iter()
        .find(|section| section.title.eq_ignore_ascii_case(title))
}

/// One finding per missing required section (line 0) and per repeat of one.
pub(crate) fn sections(file: &str, sections: &[Section<'_>]) -> Vec<SpecFinding> {
    let mut findings = Vec::new();
    for title in REQUIRED {
        let mut matching = sections
            .iter()
            .filter(|section| section.title.eq_ignore_ascii_case(title));
        if matching.next().is_none() {
            findings.push(
                Rule::Sections
                    .at(file, 0)
                    .finding(
                        format!("no `## {title}` section"),
                        format!("add a `## {title}` section (see `{TEMPLATE}`)."),
                    )
                    .about(title),
            );
        }
        findings.extend(matching.map(|repeat| {
            Rule::Sections
                .at(file, repeat.line)
                .finding(
                    format!("`## {title}` appears again; only the first is checked"),
                    format!("merge it into the first `## {title}` section."),
                )
                .about(title)
        }));
    }
    findings
}

/// How many criteria the section lists, and a finding for each that is not EARS (or for a
/// section with none).
pub(crate) fn criteria(file: &str, section: Option<&Section<'_>>) -> (usize, Vec<SpecFinding>) {
    let Some(section) = section else {
        return (0, Vec::new());
    };
    let criteria = items(section.body);
    if criteria.is_empty() {
        let finding = Rule::Ears.at(file, section.line).finding(
            format!("`## {ACCEPTANCE_CRITERIA}` has no list item"),
            "add one list item per criterion, such as \
             `WHEN <trigger>, THE <system> SHALL <response>.`",
        );
        return (0, vec![finding]);
    }
    let findings = criteria
        .iter()
        .filter_map(|item| {
            let text = criterion_text(&item.text);
            ears::parse(text).err().map(|error| {
                Rule::Ears.at(file, item.line).finding(
                    format!("acceptance criterion is not EARS: {error}."),
                    format!("write it as `{}`", template(text)),
                )
            })
        })
        .collect();
    (criteria.len(), findings)
}

/// A criterion without its checkbox (`[ ]`, `[x]`) and label (`**AC-1**:` or a first word
/// with a digit, ending in `:`, such as `AC-1:`).
fn criterion_text(item: &str) -> &str {
    let mut text = item.trim();
    if let Some(rest) = ["[ ]", "[x]", "[X]"]
        .into_iter()
        .find_map(|checkbox| text.strip_prefix(checkbox))
    {
        text = rest.trim_start();
    }
    if let Some(rest) = text.strip_prefix("**")
        && let Some((_, after)) = rest.split_once("**")
        && let Some(after) = after.strip_prefix(':')
    {
        return after.trim_start();
    }
    if let Some((first, rest)) = text.split_once(char::is_whitespace)
        && let Some(label) = first.strip_suffix(':')
        && label.chars().any(|c| c.is_ascii_digit())
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return rest.trim_start();
    }
    text
}

/// A finding when the section has no visible content.
pub(crate) fn out_of_scope(file: &str, section: Option<&Section<'_>>) -> Option<SpecFinding> {
    let section = section?;
    if section.body.iter().any(|line| !line.text.trim().is_empty()) {
        return None;
    }
    Some(Rule::OutOfScope.at(file, section.line).finding(
        format!("`## {OUT_OF_SCOPE}` is empty"),
        "list what this spec will not do, one item each, with the reason.",
    ))
}

/// The number of clarification markers in visible text outside inline code, and a finding at
/// the marker that takes the count over [`MAX_CLARIFICATIONS`].
pub(crate) fn clarifications(file: &str, lines: &[Line<'_>]) -> (usize, Option<SpecFinding>) {
    let mut count: usize = 0;
    let mut over: Option<usize> = None;
    for line in lines {
        count = count.saturating_add(markers(&line.text));
        if over.is_none() && count > MAX_CLARIFICATIONS {
            over = Some(line.number);
        }
    }
    let finding = over.map(|line| {
        Rule::NeedsClarification.at(file, line).finding(
            format!(
                "{count} `[NEEDS CLARIFICATION]` markers; at most {MAX_CLARIFICATIONS} are allowed"
            ),
            format!(
                "resolve open questions with the human until at most {MAX_CLARIFICATIONS} markers \
                 remain; plan approval needs none."
            ),
        )
    });
    (count, finding)
}

/// Markers in `text`, outside backtick code spans: the marker followed by `]` or `:`.
fn markers(text: &str) -> usize {
    text.split('`')
        .step_by(2)
        .map(|outside| {
            outside
                .match_indices(MARKER)
                .filter(|(at, _)| {
                    outside
                        .get(at.saturating_add(MARKER.len())..)
                        .is_some_and(|rest| rest.starts_with([']', ':']))
                })
                .count()
        })
        .fold(0, usize::saturating_add)
}

/// The finding for a glossary hit.
pub(crate) fn avoided(hit: TermHit) -> SpecFinding {
    Rule::AvoidedTerm
        .at(&hit.file, hit.line)
        .finding(
            format!("avoided term `{}`", hit.term),
            "use the term the glossary defines instead.",
        )
        .about(hit.term)
}
