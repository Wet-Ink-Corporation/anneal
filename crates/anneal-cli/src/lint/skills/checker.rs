//! The rule catalogue: every checker id and the design rule it enforces.
//!
//! It also holds the data the checks use (entry skills, allowed frontmatter fields, budgets,
//! word lists). `docs/skill-rules.md` documents the same table; a test fails when a checker is
//! missing from it.

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

/// A checker: one mechanical check (or one explicit review item) for one design rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Checker {
    /// Rule 1: the frontmatter parses (and the file is UTF-8).
    Frontmatter,
    /// Rule 1: `name` matches the directory, is kebab-case, has no reserved prefix.
    Name,
    /// Rule 1: `description` is present; `description` + `when_to_use` <= 300 characters.
    DescriptionBudget,
    /// Rule 1: model-invoked skills' descriptions total <= 2,100 characters.
    ListingBudget,
    /// Rule 2: body <= 600 words and <= 500 lines; file <= 8 KB.
    BodyBudget,
    /// Rule 2: `references/` one level deep, every file linked, no dangling links.
    References,
    /// Rule 2: every MUST sits in the first 20 lines.
    HardRulesFirst,
    /// Rule 3: the six anatomy sections, in order.
    Anatomy,
    /// Rule 4: the bootstrap index is <= 200 words.
    BootstrapIndex,
    /// Rule 4: only entry skills are model-invoked.
    ModelInvocation,
    /// Rule 5: the description starts with the trigger, imperative ("Use ...").
    DescriptionTrigger,
    /// Rule 5: the description ends with "Do NOT use for ... (use `<sibling>`)".
    DescriptionSibling,
    /// Rule 5: sibling descriptions do not overlap too much (feeds Tier 1).
    Overlap,
    /// Rule 6: the description carries a skip-rationalization clause ("even if ...").
    SkipRationalization,
    /// Rule 7 (review): the form matches the failure.
    FormMatchesFailure,
    /// Rule 8: every Process step has an "It's working if" criterion.
    CompletionCriteria,
    /// Rule 9: no adjective thresholds next to a gate.
    AdjectiveThreshold,
    /// Rule 10: every MUST names its enforcer.
    Enforcer,
    /// Rule 11 (review): facts looked up, decisions asked with a recommendation.
    FactsAndDecisions,
    /// Rule 12 (review): bundled scripts follow the script contract.
    ScriptsContract,
    /// Rule 13 (review): gates fail only on new findings.
    GateBaseline,
    /// Rule 14 (review): subagents get files, not pasted text.
    SubagentHandoff,
    /// Rule 15 (review): hooks declare a failure policy and a timeout.
    HookFailurePolicy,
    /// Rule 16 (review): Stop-hook blocks are capped at 2 in a row.
    StopBlockCap,
    /// Rule 17 (review): deterministic check, then Clef, then a model judge.
    CheckOrder,
    /// Rule 18: frontmatter uses spec fields only.
    FrontmatterFields,
    /// Rule 19 (review): hooks are silent on pass, one `ERROR` line on fail.
    HookSilence,
    /// Rule 20: no `model` in frontmatter.
    ModelField,
    /// Rule 21 (review): stable content first, volatile last.
    StableFirst,
}

impl Checker {
    /// Every checker, in rule order.
    pub const ALL: [Self; 29] = [
        Self::Frontmatter,
        Self::Name,
        Self::DescriptionBudget,
        Self::ListingBudget,
        Self::BodyBudget,
        Self::References,
        Self::HardRulesFirst,
        Self::Anatomy,
        Self::BootstrapIndex,
        Self::ModelInvocation,
        Self::DescriptionTrigger,
        Self::DescriptionSibling,
        Self::Overlap,
        Self::SkipRationalization,
        Self::FormMatchesFailure,
        Self::CompletionCriteria,
        Self::AdjectiveThreshold,
        Self::Enforcer,
        Self::FactsAndDecisions,
        Self::ScriptsContract,
        Self::GateBaseline,
        Self::SubagentHandoff,
        Self::HookFailurePolicy,
        Self::StopBlockCap,
        Self::CheckOrder,
        Self::FrontmatterFields,
        Self::HookSilence,
        Self::ModelField,
        Self::StableFirst,
    ];

    /// The checker id as printed: kebab-case.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Frontmatter => "frontmatter",
            Self::Name => "name",
            Self::DescriptionBudget => "description-budget",
            Self::ListingBudget => "listing-budget",
            Self::BodyBudget => "body-budget",
            Self::References => "references",
            Self::HardRulesFirst => "hard-rules-first",
            Self::Anatomy => "anatomy",
            Self::BootstrapIndex => "bootstrap-index",
            Self::ModelInvocation => "model-invocation",
            Self::DescriptionTrigger => "description-trigger",
            Self::DescriptionSibling => "description-sibling",
            Self::Overlap => "overlap",
            Self::SkipRationalization => "skip-rationalization",
            Self::FormMatchesFailure => "form-matches-failure",
            Self::CompletionCriteria => "completion-criteria",
            Self::AdjectiveThreshold => "adjective-threshold",
            Self::Enforcer => "enforcer",
            Self::FactsAndDecisions => "facts-and-decisions",
            Self::ScriptsContract => "scripts-contract",
            Self::GateBaseline => "gate-baseline",
            Self::SubagentHandoff => "subagent-handoff",
            Self::HookFailurePolicy => "hook-failure-policy",
            Self::StopBlockCap => "stop-block-cap",
            Self::CheckOrder => "check-order",
            Self::FrontmatterFields => "frontmatter-fields",
            Self::HookSilence => "hook-silence",
            Self::ModelField => "model-field",
            Self::StableFirst => "stable-first",
        }
    }

    /// The skill design rule (1 to 21, `docs/skill-rules.md`) this checker enforces.
    #[must_use]
    pub const fn rule(self) -> u8 {
        match self {
            Self::Frontmatter | Self::Name | Self::DescriptionBudget | Self::ListingBudget => 1,
            Self::BodyBudget | Self::References | Self::HardRulesFirst => 2,
            Self::Anatomy => 3,
            Self::BootstrapIndex | Self::ModelInvocation => 4,
            Self::DescriptionTrigger | Self::DescriptionSibling | Self::Overlap => 5,
            Self::SkipRationalization => 6,
            Self::FormMatchesFailure => 7,
            Self::CompletionCriteria => 8,
            Self::AdjectiveThreshold => 9,
            Self::Enforcer => 10,
            Self::FactsAndDecisions => 11,
            Self::ScriptsContract => 12,
            Self::GateBaseline => 13,
            Self::SubagentHandoff => 14,
            Self::HookFailurePolicy => 15,
            Self::StopBlockCap => 16,
            Self::CheckOrder => 17,
            Self::FrontmatterFields => 18,
            Self::HookSilence => 19,
            Self::ModelField => 20,
            Self::StableFirst => 21,
        }
    }
}

impl Display for Checker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The model-invoked entry skills (blueprint "Packaging against the listing budget"). Every
/// other skill must set `disable-model-invocation: true`.
pub(crate) const ENTRY_SKILLS: [&str; 7] = [
    "grill", "spec", "plan", "build", "diagnose", "verify", "ship",
];

/// Frontmatter keys from the Agent Skills spec (agentskills.io).
pub(crate) const SPEC_FIELDS: [&str; 6] = [
    "name",
    "description",
    "license",
    "compatibility",
    "metadata",
    "allowed-tools",
];

/// Claude Code keys the design rules themselves call for.
///
/// `when_to_use` (rule 1), `disable-model-invocation` (rule 4), `context` and `agent` (rule
/// 20's fork), and `argument-hint` for user-invoked skills. Skills still read correctly where
/// they are ignored.
pub(crate) const EXTENSION_FIELDS: [&str; 5] = [
    "when_to_use",
    "disable-model-invocation",
    "argument-hint",
    "context",
    "agent",
];

/// Prefixes a skill name may not start with (rule 1).
pub(crate) const RESERVED_PREFIXES: [&str; 2] = ["claude", "anthropic"];

/// Rule 1: `description` + `when_to_use`, in characters.
pub(crate) const DESCRIPTION_MAX_CHARS: usize = 300;
/// Rule 1: all model-invoked descriptions together, in characters.
pub(crate) const LISTING_MAX_CHARS: usize = 2_100;
/// Rule 1: the Agent Skills spec's name length ceiling.
pub(crate) const NAME_MAX_CHARS: usize = 64;
/// Rule 2: body words.
pub(crate) const BODY_MAX_WORDS: usize = 600;
/// Rule 2: body lines.
pub(crate) const BODY_MAX_LINES: usize = 500;
/// Rule 2: the whole `SKILL.md`, in bytes (wshobson's ~8 KB cap).
pub(crate) const FILE_MAX_BYTES: u64 = 8 * 1024;
/// Rule 2: hard rules (every MUST) sit in this many first lines of the file.
pub(crate) const HARD_RULES_MAX_LINE: usize = 20;
/// Rule 3: the sections, in order (level-2 headings, case-insensitive).
pub(crate) const ANATOMY: [&str; 6] = [
    "Overview",
    "When to use",
    "Process",
    "Rationalizations",
    "Red flags",
    "Verification",
];
/// Rule 3: skills exempt from the anatomy check. Exemptions live here, never in the skill.
pub(crate) const ANATOMY_EXEMPT: [&str; 0] = [];
/// Rule 4: the bootstrap index, in words.
pub(crate) const INDEX_MAX_WORDS: usize = 200;
/// Rule 5: the clause that ends a description.
pub(crate) const NOT_FOR_CLAUSE: &str = "Do NOT use for";
/// Rule 5: siblings whose trigger words overlap at or above this percentage (Jaccard) warn.
pub(crate) const OVERLAP_PERCENT: usize = 50;
/// Rule 6: phrases that mark a skip-rationalization clause.
pub(crate) const SKIP_CLAUSES: [&str; 2] = ["even if", "even when"];
/// Rule 8: the completion-criterion phrase each Process step carries.
pub(crate) const COMPLETION_PHRASES: [&str; 2] = ["it's working if", "it\u{2019}s working if"];
/// Rule 9: adjectives that stand in for a threshold.
pub(crate) const THRESHOLD_ADJECTIVES: [&str; 10] = [
    "small", "large", "big", "quick", "few", "many", "tiny", "huge", "minor", "trivial",
];
/// Rule 9: words that make a line a gate.
pub(crate) const GATE_WORDS: [&str; 19] = [
    "gate",
    "gates",
    "threshold",
    "skip",
    "skips",
    "block",
    "blocks",
    "require",
    "requires",
    "required",
    "allow",
    "allows",
    "allowed",
    "only",
    "limit",
    "cap",
    "budget",
    "must",
    "unless",
];
/// Rule 9: sections where adjectives are quoted on purpose (the rationalizations being refuted).
pub(crate) const QUOTING_SECTIONS: [&str; 2] = ["Rationalizations", "Red flags"];

#[cfg(test)]
mod tests {
    use super::Checker;
    use std::collections::BTreeSet;

    #[test]
    fn every_rule_from_1_to_21_has_a_checker() {
        let covered: BTreeSet<u8> = Checker::ALL.iter().map(|c| c.rule()).collect();
        let expected: BTreeSet<u8> = (1..=21).collect();
        assert_eq!(covered, expected, "each design rule maps to a checker id");
    }

    #[test]
    fn checker_ids_are_distinct_and_match_serde() {
        let ids: BTreeSet<&str> = Checker::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(ids.len(), Checker::ALL.len(), "distinct ids");
        for checker in Checker::ALL {
            let json = serde_json::to_string(&checker).expect("a unit enum serialises");
            assert_eq!(json, format!("\"{checker}\""), "serde id equals as_str");
        }
    }

    #[test]
    fn checkers_are_listed_in_rule_order() {
        let rules: Vec<u8> = Checker::ALL.iter().map(|c| c.rule()).collect();
        assert!(rules.is_sorted(), "ALL is in rule order: {rules:?}");
    }
}
