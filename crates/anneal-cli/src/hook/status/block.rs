//! The status block a subagent ends with: a `Status:` line and the required sections.
//!
//! The block is temper's (`references/process.md` §5): `Status: <value>`, then `Gate:`,
//! `Review:`, `Changed:`, `Decisions needed:` and `Not verified:` lines. The values and the
//! sections are data ([`STATUSES`], [`SECTIONS`]), so a playbook can extend them. The last
//! `Status:` line is the report; code fences do not hide it (temper's template is fenced).
//! A key is read through list markers, quote markers, emphasis (`*`) and code spans, in any
//! letter case.

use anneal_ledger::event::AgentStatus;

/// The values a `Status:` line may carry, as written, and what each records.
pub(crate) const STATUSES: [(&str, AgentStatus); 4] = [
    ("DONE", AgentStatus::Done),
    ("DONE_WITH_CONCERNS", AgentStatus::DoneWithConcerns),
    ("NEEDS_CONTEXT", AgentStatus::NeedsContext),
    ("BLOCKED", AgentStatus::Blocked),
];

/// The sections that must each start a line after the `Status:` line, in report order.
pub(crate) const SECTIONS: [&str; 5] = [
    "Gate",
    "Review",
    "Changed",
    "Decisions needed",
    "Not verified",
];

/// The status block as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    /// The status, when the last `Status:` line carries a known one.
    pub(crate) status: Option<AgentStatus>,
    /// What is wrong, in one line, when anything is.
    pub(crate) violation: Option<String>,
    /// The sections not found after the `Status:` line (after nothing, without one).
    pub(crate) missing: Vec<String>,
}

/// Reads the status block of `message`. Never fails: whatever is wrong is the violation.
pub(crate) fn parse(message: &str) -> Parsed {
    let lines: Vec<String> = message.lines().map(plain).collect();
    let status_at = lines
        .iter()
        .rposition(|line| value(line, "Status").is_some());
    let after = status_at.map_or(0, |at| at.saturating_add(1));
    let rest = lines.get(after..).unwrap_or_default();
    let missing: Vec<String> = SECTIONS
        .iter()
        .filter(|section| !rest.iter().any(|line| value(line, section).is_some()))
        .map(|section| (*section).to_owned())
        .collect();
    let mut problems = Vec::new();
    let status = match status_at
        .and_then(|at| lines.get(at))
        .and_then(|line| value(line, "Status"))
    {
        None => {
            problems.push("no Status: line".to_owned());
            None
        }
        Some(found) => {
            let word = first_word(found);
            let known = STATUSES
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(word))
                .map(|(_, status)| *status);
            if known.is_none() {
                let expected = "expected DONE, DONE_WITH_CONCERNS, NEEDS_CONTEXT or BLOCKED";
                problems.push(format!("unknown status {word:?}; {expected}"));
            }
            known
        }
    };
    if !missing.is_empty() {
        problems.push(format!("missing sections: {}", missing.join(", ")));
    }
    Parsed {
        status,
        violation: (!problems.is_empty()).then(|| problems.join("; ")),
        missing,
    }
}

/// `line` without emphasis and code-span marks, and without leading list or quote markers.
fn plain(line: &str) -> String {
    let unmarked: String = line.chars().filter(|c| !matches!(c, '*' | '`')).collect();
    unmarked
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '+' | '>' | '#'))
        .to_owned()
}

/// The text after `<key>:` when `line` starts with it (in any letter case).
fn value<'l>(line: &'l str, key: &str) -> Option<&'l str> {
    let head = line.get(..key.len())?;
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    line.get(key.len()..)?.strip_prefix(':')
}

/// The first word of `text`, without trailing punctuation.
fn first_word(text: &str) -> &str {
    text.split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ',', ';', ':', '!'])
}

#[cfg(test)]
mod tests {
    use anneal_ledger::event::AgentStatus;

    use super::{Parsed, parse};

    const FULL: &str = "\
Gate: fmt ok | clippy ok | test ok (3 passed) | doc ok
Review: approved
Changed: crates/x (+1/-0)
Decisions needed: none
Not verified: none";

    #[test]
    fn each_known_status_with_every_section_is_clean() {
        let statuses = [
            ("DONE", AgentStatus::Done),
            ("DONE_WITH_CONCERNS", AgentStatus::DoneWithConcerns),
            ("NEEDS_CONTEXT", AgentStatus::NeedsContext),
            ("BLOCKED", AgentStatus::Blocked),
        ];
        for (word, status) in statuses {
            let message = format!("Report.\n\n```text\nStatus: {word}\n{FULL}\n```\n");

            assert_eq!(
                parse(&message),
                Parsed {
                    status: Some(status),
                    violation: None,
                    missing: Vec::new(),
                },
                "{word}: a fenced block counts"
            );
        }
    }

    #[test]
    fn markdown_emphasis_and_list_markers_around_the_keys_are_read_through() {
        let message = "- **Status:** `DONE`\n- **Gate:** green\n- **Review:** approved\n\
                       - **Changed:** a\n- **Decisions needed:** none\n- **Not verified:** none";

        assert_eq!(
            parse(message),
            Parsed {
                status: Some(AgentStatus::Done),
                violation: None,
                missing: Vec::new(),
            },
            "emphasis, code spans and list markers are not part of the key"
        );
    }

    #[test]
    fn missing_sections_are_listed_in_order_and_make_a_violation() {
        let message = "Status: DONE\nGate: ok\nChanged: a\n";

        assert_eq!(
            parse(message),
            Parsed {
                status: Some(AgentStatus::Done),
                violation: Some(
                    "missing sections: Review, Decisions needed, Not verified".to_owned()
                ),
                missing: vec![
                    "Review".to_owned(),
                    "Decisions needed".to_owned(),
                    "Not verified".to_owned(),
                ],
            },
            "the status is kept, the gap recorded"
        );
    }

    #[test]
    fn an_unknown_status_is_a_violation_without_a_status() {
        let parsed = parse(&format!("Status: WIP\n{FULL}"));

        assert_eq!(
            parsed,
            Parsed {
                status: None,
                violation: Some(
                    "unknown status \"WIP\"; expected DONE, DONE_WITH_CONCERNS, NEEDS_CONTEXT \
                     or BLOCKED"
                        .to_owned()
                ),
                missing: Vec::new(),
            },
            "WIP is not one of the four"
        );
    }

    #[test]
    fn a_message_without_a_status_line_is_a_violation_listing_every_section() {
        let parsed = parse("I looked around; the parser lives in src/parse.rs.");

        assert_eq!(parsed.status, None, "no status");
        assert_eq!(
            parsed.violation.as_deref(),
            Some(
                "no Status: line; missing sections: Gate, Review, Changed, Decisions needed, Not verified"
            ),
            "both gaps in one line"
        );
        assert_eq!(parsed.missing.len(), 5, "every section is missing");
    }

    #[test]
    fn the_last_status_line_wins_and_sections_count_only_after_it() {
        let message = format!("{FULL}\nStatus: BLOCKED\n\nlater:\nStatus: DONE\n{FULL}");

        assert_eq!(
            parse(&message).status,
            Some(AgentStatus::Done),
            "the final block is the report"
        );
        assert_eq!(
            parse(&format!("{FULL}\nStatus: DONE\n")).missing.len(),
            5,
            "sections before the status line belong to something else"
        );
    }
}
