//! The nine fields of a brief: the one table `brief build`, `lint brief`, the brief hook and
//! `docs/brief.md` follow. Rename a field here and every reader follows.
//!
//! The set is re-authored from the public pstack's worker brief
//! (`kashyab12/uni-pstack`, `pstack/references/delegation.md`): goal, repository, owned scope,
//! do not touch, the data shape and invariants, success criteria and verification, plus its
//! asks that the worker be told whether it may edit and what to return. It is a default chosen
//! while pstack's `orchestrate.md` was out of reach, recorded as an open decision for Ryan.

/// What `brief build` writes for a field it was not given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// Nothing: the field stays empty until its author fills it, so `lint brief` fails.
    Empty,
    /// This text.
    Text(&'static str),
    /// The worktree root and the base the work starts from, as `brief build` resolved them.
    Repository,
}

/// One field of a brief.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// The key after `brief build --field`, as `key=text`.
    pub key: &'static str,
    /// The section heading in the brief, `## <heading>`.
    pub heading: &'static str,
    /// What the field is for, in one line (`docs/brief.md`, and the placeholder of an empty
    /// field).
    pub purpose: &'static str,
    /// What `brief build` writes when the field is not given.
    pub default: Fill,
    /// A rule `brief build` adds after a given value, as its own paragraph.
    pub appendix: Option<&'static str>,
}

/// The no-revert rule every `Edits allowed` field carries.
const NO_REVERT: &str = "Never revert, reset or overwrite a change you did not make; if one is \
                         in your way, stop and report it.";

/// What a subagent's final report holds, unless the brief says otherwise.
const RETURN_CONTRACT: &str = "End with: the files you changed, what you did, the verification \
                               you ran and its result, and any risks or blockers.";

/// The nine fields, in the order a brief lists them.
pub const FIELDS: [Field; 9] = [
    Field {
        key: "goal",
        heading: "Goal",
        purpose: "The outcome the subagent delivers, in one or two sentences.",
        default: Fill::Empty,
        appendix: None,
    },
    Field {
        key: "repository",
        heading: "Repository",
        purpose: "The worktree it works in and the base its change starts from.",
        default: Fill::Repository,
        appendix: None,
    },
    Field {
        key: "owned-scope",
        heading: "Owned scope",
        purpose: "The files, modules or commands it owns and may change.",
        default: Fill::Empty,
        appendix: None,
    },
    Field {
        key: "do-not-touch",
        heading: "Do not touch",
        purpose: "The paths, interfaces and decisions it must leave as they are.",
        default: Fill::Empty,
        appendix: None,
    },
    Field {
        key: "invariants",
        heading: "Invariants",
        purpose: "The data shapes and rules that must still hold when it is done.",
        default: Fill::Empty,
        appendix: None,
    },
    Field {
        key: "success-criteria",
        heading: "Success criteria",
        purpose: "How the caller judges the work finished.",
        default: Fill::Empty,
        appendix: None,
    },
    Field {
        key: "verification",
        heading: "Verification",
        purpose: "The commands it runs to check its work, and what they must show.",
        default: Fill::Empty,
        appendix: None,
    },
    Field {
        key: "edits-allowed",
        heading: "Edits allowed",
        purpose: "Whether it may change files (yes or no), with the no-revert rule.",
        default: Fill::Empty,
        appendix: Some(NO_REVERT),
    },
    Field {
        key: "return-contract",
        heading: "Return contract",
        purpose: "What its final report must hold.",
        default: Fill::Text(RETURN_CONTRACT),
        appendix: None,
    },
];

/// The field whose key is `key`, if any.
#[must_use]
pub fn by_key(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|field| field.key == key)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{FIELDS, Field};

    #[test]
    fn a_brief_has_nine_fields_with_distinct_keys_and_headings() {
        let keys: BTreeSet<&str> = FIELDS.iter().map(|field| field.key).collect();
        let headings: BTreeSet<String> = FIELDS
            .iter()
            .map(|field| field.heading.to_ascii_lowercase())
            .collect();

        assert_eq!(
            (FIELDS.len(), keys.len(), headings.len()),
            (9, 9, 9),
            "nine fields, no key or heading (in any letter case) used twice"
        );
    }

    #[test]
    fn every_field_has_a_one_line_purpose_and_a_flag_safe_key() {
        for Field {
            key,
            heading,
            purpose,
            ..
        } in FIELDS
        {
            assert!(
                !purpose.trim().is_empty() && !purpose.contains('\n'),
                "{heading}: one line of purpose"
            );
            assert!(
                key.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
                "{key}: written after --field as key=text"
            );
        }
    }

    #[test]
    fn the_field_lookup_finds_a_key_and_refuses_an_unknown_one() {
        assert_eq!(
            super::by_key("goal").map(|field| field.heading),
            Some("Goal"),
            "a known key"
        );
        assert!(super::by_key("Goal").is_none(), "keys are lower case");
    }
}
