//! Checks across the whole skill set, run on every invocation (they are not cached): the listing
//! budget (rule 1), sibling clauses and overlap (rule 5), the bootstrap index (rule 4), and the
//! review items for rules about hooks and gates rather than skills (13, 15, 16, 17, 19, 21).

use std::collections::BTreeSet;

use crate::lint::finding::{LintFinding, Location};
use crate::lint::skills::checker::{
    Checker, INDEX_MAX_WORDS, LISTING_MAX_CHARS, NOT_FOR_CLAUSE, OVERLAP_PERCENT,
};
use crate::lint::skills::document::{Skill, words};

/// Words too common to signal overlap between two triggers.
const STOPWORDS: [&str; 24] = [
    "the", "and", "for", "use", "when", "with", "this", "that", "you", "are", "not", "any", "from",
    "into", "before", "after", "even", "its", "your", "has", "have", "was", "but", "all",
];

/// Rule 1: the model-invoked skills' descriptions (plus `when_to_use`) fit the shared listing
/// budget. Reported at the skill that pushes the running total over it.
pub(crate) fn listing_budget(skills: &[Skill<'_>]) -> Option<LintFinding> {
    let mut total: usize = 0;
    let mut crossed = None;
    for skill in skills.iter().filter(|s| !s.model_invocation_disabled()) {
        total = total
            .saturating_add(skill.doc.value("description").chars().count())
            .saturating_add(skill.doc.value("when_to_use").chars().count());
        if total > LISTING_MAX_CHARS && crossed.is_none() {
            crossed = Some(skill);
        }
    }
    let skill = crossed?;
    Some(LintFinding::error(
        Checker::ListingBudget,
        skill.at(skill.field_line("description")),
        format!(
            "model-invoked descriptions total {total} characters (listing budget \
             {LISTING_MAX_CHARS}); the total crosses it here."
        ),
        "shorten descriptions, or set `disable-model-invocation: true` on non-entry skills.",
    ))
}

/// Rule 5: each description ends with "Do NOT use for ... (use `<sibling>`)", naming at least
/// one other skill of the set, and every named sibling exists. `names` holds every skill
/// directory found, including those whose `SKILL.md` did not parse.
pub(crate) fn siblings(skills: &[Skill<'_>], names: &BTreeSet<&str>) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    for skill in skills {
        let description = skill.doc.value("description");
        if description.is_empty() {
            continue;
        }
        let line = skill.field_line("description");
        let fix = format!(
            "end with \"{NOT_FOR_CLAUSE} <case> (use `<sibling>`).\" naming a skill in this set."
        );
        let Some(siblings) = named_siblings(description).filter(|n| !n.is_empty()) else {
            findings.push(LintFinding::error(
                Checker::DescriptionSibling,
                skill.at(line),
                format!("the description does not end with a \"{NOT_FOR_CLAUSE} ... (use `<sibling>`)\" clause."),
                fix,
            ));
            continue;
        };
        for sibling in siblings {
            if sibling == skill.dir.name || !names.contains(sibling) {
                findings.push(
                    LintFinding::error(Checker::DescriptionSibling, skill.at(line),
                            format!("the Do NOT clause names `{sibling}`, which is not a sibling skill in this set."),
                            fix.as_str(),
                        )
                        .with_subject(sibling),
                );
            }
        }
    }
    findings
}

/// The `(use `x`)` names after the Do NOT clause, or `None` when there is no clause.
fn named_siblings(description: &str) -> Option<Vec<&str>> {
    let (_, clause) = description.split_once(NOT_FOR_CLAUSE)?;
    let names = clause
        .split("(use `")
        .skip(1)
        .filter_map(|rest| rest.split_once('`').map(|(name, _)| name))
        .collect();
    Some(names)
}

/// Rule 5 (feeds Tier 1): sibling triggers sharing at least [`OVERLAP_PERCENT`] of their
/// content words (Jaccard) will compete for the same prompts.
pub(crate) fn overlap(skills: &[Skill<'_>]) -> Vec<LintFinding> {
    let vocabularies: Vec<BTreeSet<String>> =
        skills.iter().map(|s| vocabulary(s.trigger())).collect();
    let mut findings = Vec::new();
    for (later, skill) in skills.iter().enumerate() {
        for (earlier, other) in skills.iter().enumerate().take(later) {
            let (Some(a), Some(b)) = (vocabularies.get(earlier), vocabularies.get(later)) else {
                continue;
            };
            let Some(percent) = jaccard_percent(a, b) else {
                continue;
            };
            if percent >= OVERLAP_PERCENT {
                findings.push(
                    LintFinding::warning(
                        Checker::Overlap,
                        skill.at(skill.field_line("description")),
                        format!(
                            "the trigger shares {percent}% of its words with `{}`.",
                            other.dir.name
                        ),
                        "make the triggers disjoint and name each other in the Do NOT clauses.",
                    )
                    .with_subject(other.dir.name.as_str()),
                );
            }
        }
    }
    findings
}

/// Content words of a trigger: lowercase, three letters or more, stopwords removed.
fn vocabulary(text: &str) -> BTreeSet<String> {
    words(text)
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// `|a ∩ b| / |a ∪ b|` as a whole percentage (rounded down), or `None` when both are empty.
fn jaccard_percent(a: &BTreeSet<String>, b: &BTreeSet<String>) -> Option<usize> {
    let union = a.union(b).count();
    let shared = a.intersection(b).count();
    shared.saturating_mul(100).checked_div(union)
}

/// Rule 4: the bootstrap index is at most [`INDEX_MAX_WORDS`] words. `index` is the file's
/// printed path and text; without one, the rule is listed for review.
pub(crate) fn bootstrap_index(index: Option<(&str, &str)>, root: &str) -> Option<LintFinding> {
    let Some((path, text)) = index else {
        return Some(LintFinding::review(
            Checker::BootstrapIndex,
            Location::new(root, 0),
            "the bootstrap index was not checked.",
            "pass `--index <file>` with the SessionStart index text.",
        ));
    };
    let count = text.split_whitespace().count();
    (count > INDEX_MAX_WORDS).then(|| {
        LintFinding::error(
            Checker::BootstrapIndex,
            Location::new(path, 1),
            format!("the bootstrap index is {count} words (budget {INDEX_MAX_WORDS})."),
            "keep only what anneal is, the entry skills with triggers, and the user-invoked commands.",
        )
    })
}

/// The rules that govern hooks, gates and injections rather than `SKILL.md` files. `lint
/// skills` cannot see them, so each is listed for review at the skills root.
pub(crate) fn outside_skills(root: &str) -> Vec<LintFinding> {
    [
        (
            Checker::GateBaseline,
            "gates fail only on new findings (snapshot at session start).",
        ),
        (
            Checker::HookFailurePolicy,
            "every hook declares onFailure and an explicit timeout (guards 5 s, Clef 2 s).",
        ),
        (
            Checker::StopBlockCap,
            "Stop-hook blocks are capped at 2 in a row per session.",
        ),
        (
            Checker::CheckOrder,
            "a deterministic check runs before Clef, and Clef before any model judge.",
        ),
        (
            Checker::HookSilence,
            "hooks are silent on pass and print one `ERROR <reason>` line on fail.",
        ),
        (
            Checker::StableFirst,
            "stable content comes first and volatile content (SHAs, times, counters) last.",
        ),
    ]
    .into_iter()
    .map(|(checker, what)| {
        LintFinding::review(
            checker,
            Location::new(root, 0),
            format!("not checkable from skills: {what}"),
            "confirm it in the hooks, scripts and briefs the skills rely on (docs/skill-rules.md).",
        )
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{jaccard_percent, named_siblings, vocabulary};

    #[test]
    fn the_do_not_clause_yields_its_named_siblings() {
        let description =
            "Use when planning. Do NOT use for specs (use `spec`) or code (use `build`).";
        assert_eq!(
            named_siblings(description),
            Some(vec!["spec", "build"]),
            "both names"
        );
        assert_eq!(named_siblings("Use when planning."), None, "no clause");
    }

    #[test]
    fn overlap_is_the_rounded_down_jaccard_percentage_of_content_words() {
        let a = vocabulary("Use when writing a plan for the change");
        let b = vocabulary("Use when writing a spec for the change");
        assert_eq!(a.len(), 3, "writing, plan, change: {a:?}");
        assert_eq!(jaccard_percent(&a, &b), Some(50), "2 shared of 4");
        assert_eq!(
            jaccard_percent(&vocabulary(""), &vocabulary("a")),
            None,
            "nothing to compare"
        );
    }
}
