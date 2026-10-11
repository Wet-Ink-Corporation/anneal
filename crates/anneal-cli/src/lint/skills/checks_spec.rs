//! Frontmatter checks for one skill: spec conformance (rule 1), spec-only fields (rule 18), no
//! `model` (rule 20), model invocation (rule 4) and the description's shape (rules 5, 6).

use crate::lint::finding::LintFinding;
use crate::lint::skills::checker::{
    Checker, DESCRIPTION_MAX_CHARS, ENTRY_SKILLS, EXTENSION_FIELDS, NAME_MAX_CHARS,
    RESERVED_PREFIXES, SKIP_CLAUSES, SPEC_FIELDS,
};
use crate::lint::skills::document::Skill;

/// Every frontmatter check for `skill`.
pub(crate) fn check(skill: &Skill<'_>) -> Vec<LintFinding> {
    let mut findings = fields(skill);
    findings.extend(name(skill));
    findings.extend(description_budget(skill));
    findings.extend(description_trigger(skill));
    findings.extend(skip_rationalization(skill));
    findings.extend(model_invocation(skill));
    findings
}

/// Rules 18 and 20: only spec fields (plus the documented extensions); never `model`.
fn fields(skill: &Skill<'_>) -> Vec<LintFinding> {
    skill
        .doc
        .fields
        .iter()
        .filter_map(|field| {
            let key = field.key.as_str();
            if key == "model" {
                return Some(LintFinding::error(
                    Checker::ModelField,
                    skill.at(field.line),
                    "`model` is set; a model switch re-reads the whole history uncached.",
                    "remove `model`; to run on another model use `context: fork` with an \
                     `agent` that names it.",
                ));
            }
            let allowed = SPEC_FIELDS.contains(&key) || EXTENSION_FIELDS.contains(&key);
            (!allowed).then(|| {
                LintFinding::error(
                    Checker::FrontmatterFields,
                    skill.at(field.line),
                    format!("`{key}` is not a spec frontmatter field."),
                    format!(
                        "remove `{key}` or move it under `metadata:`; allowed: {}, {}.",
                        SPEC_FIELDS.join(", "),
                        EXTENSION_FIELDS.join(", ")
                    ),
                )
            })
        })
        .collect()
}

/// Rule 1: `name` matches the directory, is kebab-case, and has no reserved prefix.
fn name(skill: &Skill<'_>) -> Option<LintFinding> {
    let line = skill.field_line("name");
    let Some(field) = skill.doc.field("name") else {
        return Some(LintFinding::error(
            Checker::Name,
            skill.at(line),
            "frontmatter has no `name`.",
            format!("add `name: {}`.", skill.dir.name),
        ));
    };
    let name = field.value.as_str();
    let problem = if name != skill.dir.name {
        format!(
            "`name: {name}` does not match its directory `{}`",
            skill.dir.name
        )
    } else if !is_kebab_case(name) || name.chars().count() > NAME_MAX_CHARS {
        format!("`{name}` is not kebab-case of at most {NAME_MAX_CHARS} characters")
    } else {
        let prefix = RESERVED_PREFIXES.iter().find(|p| name.starts_with(**p))?;
        format!("`{name}` starts with the reserved prefix `{prefix}`")
    };
    Some(LintFinding::error(
        Checker::Name,
        skill.at(line),
        format!("{problem}."),
        "use a lowercase kebab-case name without a `claude`/`anthropic` prefix, and name the \
         directory the same.",
    ))
}

/// Lowercase ASCII letters and digits in runs joined by single hyphens.
fn is_kebab_case(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Rule 1: a description exists, and with `when_to_use` it fits the per-skill budget.
fn description_budget(skill: &Skill<'_>) -> Option<LintFinding> {
    let line = skill.field_line("description");
    let description = skill.doc.value("description");
    if description.trim().is_empty() {
        return Some(LintFinding::error(
            Checker::DescriptionBudget,
            skill.at(line),
            "frontmatter has no `description`.",
            "add a description: the trigger first, ending with \"Do NOT use for ... (use \
             `<sibling>`)\".",
        ));
    }
    let total = description
        .chars()
        .count()
        .saturating_add(skill.doc.value("when_to_use").chars().count());
    (total > DESCRIPTION_MAX_CHARS).then(|| {
        LintFinding::error(
            Checker::DescriptionBudget,
            skill.at(line),
            format!(
                "`description` + `when_to_use` is {total} characters (budget \
                 {DESCRIPTION_MAX_CHARS})."
            ),
            "cut how-it-works detail; keep the trigger, the skip clause and the Do NOT clause.",
        )
    })
}

/// Rule 5: the description opens with its trigger in the imperative: "Use when/before/...".
fn description_trigger(skill: &Skill<'_>) -> Option<LintFinding> {
    let description = skill.doc.value("description");
    let first = description.split_whitespace().next()?;
    (first != "Use").then(|| {
        LintFinding::error(
            Checker::DescriptionTrigger,
            skill.at(skill.field_line("description")),
            format!("the description opens with `{first}`, not with its trigger."),
            "start with the trigger in the imperative: \"Use when ...\" (say when, never how).",
        )
    })
}

/// Rule 6: the description names the rationalization the model uses to skip the skill.
fn skip_rationalization(skill: &Skill<'_>) -> Option<LintFinding> {
    let description = skill.doc.value("description").to_lowercase();
    if description.is_empty() || SKIP_CLAUSES.iter().any(|c| description.contains(c)) {
        return None;
    }
    Some(LintFinding::warning(
        Checker::SkipRationalization,
        skill.at(skill.field_line("description")),
        "the description has no skip-rationalization clause.",
        "add the excuse the model uses to skip it, e.g. \"even if the diff looks small\".",
    ))
}

/// Rule 4: entry skills are model-invoked; every other skill sets
/// `disable-model-invocation: true`.
fn model_invocation(skill: &Skill<'_>) -> Option<LintFinding> {
    let value = skill.doc.value("disable-model-invocation");
    let line = skill.field_line("disable-model-invocation");
    let is_entry = ENTRY_SKILLS.contains(&skill.dir.name.as_str());
    let (message, fix) = match (value, is_entry) {
        ("" | "false", true) | ("true", false) => return None,
        ("true", true) => (
            format!(
                "entry skill `{}` disables model invocation, so it never routes.",
                skill.dir.name
            ),
            "remove `disable-model-invocation`.".to_owned(),
        ),
        ("" | "false", false) => (
            format!(
                "`{}` is not an entry skill ({}) but is model-invoked.",
                skill.dir.name,
                ENTRY_SKILLS.join(", ")
            ),
            "set `disable-model-invocation: true`, or fold it into an entry skill's \
             `references/`."
                .to_owned(),
        ),
        (other, _) => (
            format!("`disable-model-invocation: {other}` is not `true` or `false`."),
            "write `true` or `false`.".to_owned(),
        ),
    };
    Some(LintFinding::error(
        Checker::ModelInvocation,
        skill.at(line),
        message,
        fix,
    ))
}

#[cfg(test)]
mod tests {
    use super::is_kebab_case;

    #[test]
    fn kebab_case_is_lowercase_runs_joined_by_single_hyphens() {
        for good in ["plan", "write-skill", "tier-2"] {
            assert!(is_kebab_case(good), "{good} is kebab-case");
        }
        for bad in ["", "Plan", "write_skill", "-plan", "plan-", "a--b"] {
            assert!(!is_kebab_case(bad), "{bad:?} is not kebab-case");
        }
    }
}
