//! Body checks for one skill: budget and references (rule 2), anatomy (rule 3) and completion
//! criteria (rule 8).

use std::collections::BTreeSet;

use crate::lint::finding::{LintFinding, Location};
use crate::lint::skills::LintError;
use crate::lint::skills::checker::{
    ANATOMY, ANATOMY_EXEMPT, BODY_MAX_LINES, BODY_MAX_WORDS, COMPLETION_PHRASES, Checker,
    FILE_MAX_BYTES,
};
use crate::lint::skills::discover::{EntryKind, list_dir};
use crate::lint::skills::document::Skill;

const REFERENCES: &str = "references";

/// Every body check for `skill`.
///
/// # Errors
/// [`LintError::Io`] when `references/` exists but cannot be listed.
pub(crate) fn check(skill: &Skill<'_>) -> Result<Vec<LintFinding>, LintError> {
    let mut findings = references(skill)?;
    findings.extend(budget(skill));
    findings.extend(anatomy(skill));
    findings.extend(completion_criteria(skill));
    Ok(findings)
}

/// Rule 2: the file within 8 KB, the body within 600 words and 500 lines. One finding lists
/// every limit broken.
fn budget(skill: &Skill<'_>) -> Option<LintFinding> {
    let words = skill.doc.body.split_whitespace().count();
    let lines = skill.lines.len();
    let breaches: Vec<String> = [
        (skill.size > FILE_MAX_BYTES)
            .then(|| format!("{} bytes (max {FILE_MAX_BYTES})", skill.size)),
        (words > BODY_MAX_WORDS).then(|| format!("{words} words (max {BODY_MAX_WORDS})")),
        (lines > BODY_MAX_LINES).then(|| format!("{lines} lines (max {BODY_MAX_LINES})")),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!breaches.is_empty()).then(|| oversized(skill.at(1), &breaches.join(", ")))
}

/// The rule-2 finding for a file over budget by `what`.
pub(crate) fn oversized(at: Location, what: &str) -> LintFinding {
    LintFinding::error(
        Checker::BodyBudget,
        at,
        format!("SKILL.md is over budget: {what}."),
        "move depth into `references/<topic>.md`, linked from the body.",
    )
}

/// Rule 2: `references/` holds files only, each linked from the body, and every
/// `references/...` link resolves to one of them.
fn references(skill: &Skill<'_>) -> Result<Vec<LintFinding>, LintError> {
    let dir = skill.dir.dir.join(REFERENCES);
    let entries = list_dir(&dir)?;
    let links = linked_references(skill);
    let mut findings = Vec::new();
    let files: BTreeSet<&str> = entries
        .iter()
        .filter(|(_, kind)| *kind != EntryKind::Dir)
        .map(|(name, _)| name.as_str())
        .collect();
    for (name, kind) in &entries {
        let at = Location::new(dir.join(name).display().to_string(), 0);
        let (message, fix) = if *kind == EntryKind::Dir {
            (
                format!("`references/{name}/` nests references two levels deep."),
                "flatten it: one level of `references/<topic>.md` files.",
            )
        } else if !links.iter().any(|(target, _)| target == name) {
            (
                format!("`references/{name}` is not linked from the body."),
                "link it from the step that needs it, or delete it.",
            )
        } else {
            continue;
        };
        findings.push(LintFinding::error(Checker::References, at, message, fix));
    }
    for (target, line) in links {
        if !files.contains(target.as_str()) {
            findings.push(
                LintFinding::error(Checker::References, skill.at(line),
                        format!("the link `references/{target}` does not resolve to a file in `references/`."),
                        "fix the link, or add the file one level deep in `references/`.",
                    )
                    .with_subject(target),
            );
        }
    }
    Ok(findings)
}

/// Every `references/<target>` mentioned in the body, with its line.
fn linked_references(skill: &Skill<'_>) -> Vec<(String, usize)> {
    let mut links = Vec::new();
    for line in &skill.lines {
        for (index, matched) in line.text.match_indices("references/") {
            let rest = line
                .text
                .get(index.saturating_add(matched.len())..)
                .unwrap_or("");
            let target: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
                .collect();
            let target = target.trim_end_matches(['.', '/']);
            if !target.is_empty() {
                links.push((target.to_owned(), line.no));
            }
        }
    }
    links.dedup();
    links
}

/// Rule 3: the six sections, as level-2 headings, in order. Other sections may sit between.
fn anatomy(skill: &Skill<'_>) -> Option<LintFinding> {
    if ANATOMY_EXEMPT.contains(&skill.dir.name.as_str()) {
        return None;
    }
    let position = |wanted: &str| {
        skill
            .sections
            .iter()
            .find(|s| s.title.eq_ignore_ascii_case(wanted))
    };
    let missing: Vec<&str> = ANATOMY
        .iter()
        .copied()
        .filter(|wanted| position(wanted).is_none())
        .collect();
    let fix = format!("use `## ` headings in this order: {}.", ANATOMY.join(", "));
    if !missing.is_empty() {
        return Some(LintFinding::error(
            Checker::Anatomy,
            skill.at(skill.doc.body_line),
            format!("missing sections: {}.", missing.join(", ")),
            fix,
        ));
    }
    let found: Vec<_> = ANATOMY
        .iter()
        .filter_map(|wanted| position(wanted))
        .collect();
    let misplaced = found.windows(2).find_map(|pair| match pair {
        [before, after] if after.line < before.line => Some(*after),
        _ => None,
    })?;
    Some(LintFinding::error(
        Checker::Anatomy,
        skill.at(misplaced.line),
        format!("`## {}` is out of order.", misplaced.title),
        fix,
    ))
}

/// Rule 8: every numbered step in Process carries "It's working if ...".
fn completion_criteria(skill: &Skill<'_>) -> Vec<LintFinding> {
    let Some(process) = skill
        .sections
        .iter()
        .find(|s| s.title.eq_ignore_ascii_case("Process"))
    else {
        return Vec::new();
    };
    let mut steps: Vec<(usize, String)> = Vec::new();
    for line in skill
        .lines
        .iter()
        .filter(|l| l.section == Some(process.title))
    {
        if is_numbered_step(line.text) && !line.in_fence {
            steps.push((line.no, line.text.to_lowercase()));
        } else if let Some((_, text)) = steps.last_mut() {
            text.push('\n');
            text.push_str(&line.text.to_lowercase());
        }
    }
    let fix = "end the step with \"It's working if <observable result>.\"";
    if steps.is_empty() {
        return vec![LintFinding::warning(
            Checker::CompletionCriteria,
            skill.at(process.line),
            "Process has no numbered steps, so no step has a completion criterion.",
            fix,
        )];
    }
    steps
        .iter()
        .filter(|(_, text)| !COMPLETION_PHRASES.iter().any(|p| text.contains(p)))
        .map(|(line, _)| {
            LintFinding::warning(
                Checker::CompletionCriteria,
                skill.at(*line),
                "this step has no observable completion criterion.",
                fix,
            )
        })
        .collect()
}

/// `1. text` or `1) text` at the start of the line.
fn is_numbered_step(text: &str) -> bool {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0
        && text
            .get(digits..)
            .is_some_and(|rest| rest.starts_with(". ") || rest.starts_with(") "))
}

#[cfg(test)]
mod tests {
    use super::{budget, is_numbered_step, references};
    use crate::lint::skills::checker::{BODY_MAX_LINES, FILE_MAX_BYTES};
    use crate::lint::skills::document::{test_dir, test_skill};
    use std::path::Path;

    #[test]
    fn the_budget_reports_too_many_lines_and_too_many_bytes() {
        let dir = test_dir(Path::new("skills/plan"), "plan");
        let many_lines = format!("---\nname: plan\n---\n{}", "x\n".repeat(BODY_MAX_LINES + 1));
        let wide = format!("---\nname: plan\n---\n{}\n", "y".repeat(9 * 1024));

        let by_lines = budget(&test_skill(&dir, &many_lines)).map(|f| f.message);
        let by_bytes = budget(&test_skill(&dir, &wide)).map(|f| f.message);

        assert!(
            by_lines
                .as_deref()
                .is_some_and(|m| m.contains(&format!("{} lines", BODY_MAX_LINES + 1))),
            "the line limit: {by_lines:?}"
        );
        assert!(
            by_bytes
                .as_deref()
                .is_some_and(|m| m.contains(&format!("(max {FILE_MAX_BYTES})"))),
            "the byte limit: {by_bytes:?}"
        );
    }

    #[test]
    fn references_flag_nested_directories_and_dangling_links() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path().join("references/deeper")).expect("mkdir");
        std::fs::write(root.path().join("references/guide.md"), "# Guide\n").expect("write");
        let dir = test_dir(root.path(), "plan");
        let text = "---\nname: plan\n---\nSee references/guide.md and references/missing.md.\n";

        let findings = references(&test_skill(&dir, text)).expect("references/ is listable");

        let messages: Vec<&str> = findings.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(
            findings.len(),
            2,
            "nested dir and dangling link: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`references/deeper/` nests")),
            "nested: {messages:?}"
        );
        assert!(
            findings.iter().any(|f| f.at.line == 4
                && f.message
                    .contains("`references/missing.md` does not resolve")),
            "dangling, at its line: {messages:?}"
        );
    }

    #[test]
    fn numbered_steps_need_digits_a_marker_and_a_space() {
        assert!(is_numbered_step("1. Read"), "dot marker");
        assert!(is_numbered_step("12) Run"), "paren marker");
        assert!(
            !is_numbered_step("  1. nested"),
            "indented items are not steps"
        );
        assert!(!is_numbered_step("1.5 KB"), "a number is not a step");
        assert!(!is_numbered_step("- bullet"), "bullets are not steps");
    }
}
