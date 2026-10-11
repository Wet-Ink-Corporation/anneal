//! Prose checks for one skill: hard rules first (rule 2), adjective thresholds (rule 9),
//! enforcers (rule 10), and the review items for rules a machine cannot judge (7, 11, 12, 14).

use std::collections::BTreeSet;

use crate::command::CommandName;
use crate::lint::finding::LintFinding;
use crate::lint::skills::checker::{
    Checker, GATE_WORDS, HARD_RULES_MAX_LINE, QUOTING_SECTIONS, THRESHOLD_ADJECTIVES,
};
use crate::lint::skills::document::{Line, Skill, code_spans, has_token, words};

/// Every prose check for `skill`. `hook_ids` are the ids in `hooks.metadata.json`.
pub(crate) fn check(
    skill: &Skill<'_>,
    hook_ids: &BTreeSet<String>,
    has_scripts: bool,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    for line in skill.prose().filter(|l| has_token(l.text, "MUST")) {
        findings.extend(hard_rule_late(skill, line));
        findings.extend(enforcer(skill, line, hook_ids));
    }
    findings.extend(
        skill
            .prose()
            .filter_map(|line| adjective_threshold(skill, line)),
    );
    findings.extend(reviews(skill, has_scripts));
    findings
}

/// Rule 2: hard rules sit in the first lines, which survive compaction.
fn hard_rule_late(skill: &Skill<'_>, line: &Line<'_>) -> Option<LintFinding> {
    (line.no > HARD_RULES_MAX_LINE).then(|| {
        LintFinding::warning(
            Checker::HardRulesFirst,
            skill.at(line.no),
            format!("a MUST on line {} is past the first {HARD_RULES_MAX_LINE} lines.", line.no),
            format!("move the hard rule into the first {HARD_RULES_MAX_LINE} lines (compaction keeps only the start)."),
        )
    })
}

/// Rule 10: a MUST line names its enforcer: `` `anneal <command>` `` or a hook id in backticks.
fn enforcer(
    skill: &Skill<'_>,
    line: &Line<'_>,
    hook_ids: &BTreeSet<String>,
) -> Option<LintFinding> {
    let enforced = code_spans(line.text).any(|span| is_enforcer(span, hook_ids));
    (!enforced).then(|| {
        LintFinding::warning(
            Checker::Enforcer,
            skill.at(line.no),
            "this MUST names no enforcer.",
            "name the check on the same line: `anneal <command> ...` or a hook id from \
             hooks/hooks.metadata.json, in backticks; or soften the MUST.",
        )
    })
}

fn is_enforcer(span: &str, hook_ids: &BTreeSet<String>) -> bool {
    if hook_ids.contains(span.trim()) {
        return true;
    }
    let mut parts = span.split_whitespace();
    parts.next() == Some("anneal")
        && parts
            .next()
            .is_some_and(|command| CommandName::ALL.iter().any(|c| c.as_str() == command))
}

/// Rule 9: an adjective standing in for a number on a gate line. Tables (the cure) and the
/// sections that quote rationalizations are skipped.
fn adjective_threshold(skill: &Skill<'_>, line: &Line<'_>) -> Option<LintFinding> {
    let quoting = line
        .section
        .is_some_and(|s| QUOTING_SECTIONS.iter().any(|q| s.eq_ignore_ascii_case(q)));
    if quoting || line.text.trim_start().starts_with('|') {
        return None;
    }
    let tokens: Vec<String> = words(line.text).collect();
    let adjective = tokens
        .iter()
        .find(|w| THRESHOLD_ADJECTIVES.contains(&w.as_str()))?;
    tokens
        .iter()
        .any(|w| GATE_WORDS.contains(&w.as_str()))
        .then(|| {
            LintFinding::warning(
                Checker::AdjectiveThreshold,
                skill.at(line.no),
                format!("`{adjective}` stands in for a threshold on a gate line."),
                "replace the adjective with a number, or a table of bands (sizes, rollout bands, \
             retry modes).",
            )
        })
}

/// Rules 7, 11, 12 and 14 need judgment: each is listed for review, never passed silently.
fn reviews(skill: &Skill<'_>, has_scripts: bool) -> Vec<LintFinding> {
    let mut findings = vec![
        LintFinding::review(
            Checker::FormMatchesFailure,
            skill.at(1),
            "check that each instruction's form matches its failure.",
            "broken rule: prohibition plus rationalization table; wrong shape: positive recipe; \
             omission: template slot; no \"unless\" in recipes.",
        ),
        LintFinding::review(
            Checker::FactsAndDecisions,
            skill.at(1),
            "check that facts are looked up and decisions are asked.",
            "each question to the user carries a recommended answer that \"yes\" accepts.",
        ),
    ];
    if has_scripts {
        findings.push(LintFinding::review(
            Checker::ScriptsContract,
            skill.at(1),
            "the skill ships `scripts/`; check each follows the script contract.",
            "`--help` first, JSON on stdout, documented exit codes (2 never reads as 0), and a \
             does-not-prove statement.",
        ));
    }
    let subagent = skill
        .prose()
        .find(|l| l.text.to_lowercase().contains("subagent"));
    if let Some(line) = subagent {
        findings.push(LintFinding::review(
            Checker::SubagentHandoff,
            skill.at(line.no),
            "the skill hands work to subagents; check it hands them files.",
            "pass briefs, review packages and diff files by path, not pasted text.",
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::is_enforcer;
    use std::collections::BTreeSet;

    #[test]
    fn an_enforcer_is_an_anneal_command_or_a_known_hook_id() {
        let hooks: BTreeSet<String> = ["guard".to_owned()].into();
        assert!(is_enforcer("anneal plan check", &hooks), "a real command");
        assert!(is_enforcer("guard", &hooks), "a known hook id");
        assert!(
            !is_enforcer("anneal frobnicate", &hooks),
            "an unknown command"
        );
        assert!(!is_enforcer("preconditions", &hooks), "an unknown hook id");
    }
}
