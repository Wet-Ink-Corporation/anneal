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

/// How many characters of an unknown status the violation shows.
const SHOWN_CHARS: usize = 64;

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
                let shown = shortened(word);
                problems.push(format!("unknown status {shown:?}; {expected}"));
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

/// Each section of [`SECTIONS`] with its text in the last status block of `message`, in report
/// order: the text after its key, then its continuation lines up to a blank line, a code fence
/// or the next key, joined by spaces; empty for a section the block lacks. `None` when
/// `message` has no `Status:` line, so it is not a report.
pub(crate) fn section_texts(message: &str) -> Option<Vec<(&'static str, String)>> {
    let raw: Vec<&str> = message.lines().collect();
    let lines: Vec<String> = raw.iter().map(|line| plain(line)).collect();
    let status_at = lines
        .iter()
        .rposition(|line| value(line, "Status").is_some())?;
    let rest = lines.get(status_at.saturating_add(1)..).unwrap_or_default();
    let raw_rest = raw.get(status_at.saturating_add(1)..).unwrap_or_default();
    let is_key = |line: &str| {
        value(line, "Status").is_some() || SECTIONS.iter().any(|s| value(line, s).is_some())
    };
    let texts = SECTIONS
        .iter()
        .map(|section| {
            let Some(at) = rest.iter().position(|line| value(line, section).is_some()) else {
                return (*section, String::new());
            };
            let first = rest
                .get(at)
                .and_then(|line| value(line, section))
                .unwrap_or_default();
            let following = rest
                .iter()
                .zip(raw_rest)
                .skip(at.saturating_add(1))
                .take_while(|(line, raw)| {
                    !line.trim().is_empty() && !is_fence(raw) && !is_key(line)
                })
                .map(|(line, _)| line.as_str());
            let words: Vec<&str> = std::iter::once(first)
                .chain(following)
                .flat_map(str::split_whitespace)
                .collect();
            (*section, words.join(" "))
        })
        .collect();
    Some(texts)
}

/// Whether `line` opens or closes a code fence.
fn is_fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
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

/// `word`'s first [`SHOWN_CHARS`] characters, with `…` when it is longer, so a violation stays
/// short.
fn shortened(word: &str) -> String {
    let mut chars = word.chars();
    let mut shown: String = chars.by_ref().take(SHOWN_CHARS).collect();
    if chars.next().is_some() {
        shown.push('…');
    }
    shown
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

    use super::{Parsed, parse, section_texts};

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
    fn a_long_unknown_status_is_cut_to_64_characters_on_a_char_boundary() {
        let word = format!("{}{}", "é".repeat(63), "x".repeat(500));

        let parsed = parse(&format!("Status: {word}\n{FULL}"));

        let violation = parsed.violation.unwrap_or_default();
        let shown = format!("unknown status \"{}…\";", "é".repeat(63) + "x");
        assert!(violation.starts_with(&shown), "{violation}");
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

    #[test]
    fn section_texts_reads_each_section_of_the_last_block_in_order() {
        let message = "Earlier: Gate: ignored\n\n```text\nStatus: DONE\nGate: fmt ok | test ok\n\
                       - **Changed:** crates/x (+1/-0)\n  and crates/y\nNot verified: none\n```\n";

        let texts = section_texts(message).expect("a status line");

        assert_eq!(
            texts,
            vec![
                ("Gate", "fmt ok | test ok".to_owned()),
                ("Review", String::new()),
                ("Changed", "crates/x (+1/-0) and crates/y".to_owned()),
                ("Decisions needed", String::new()),
                ("Not verified", "none".to_owned()),
            ],
            "every section in report order, its continuation lines joined, absent ones empty"
        );
    }

    #[test]
    fn section_texts_is_none_without_a_status_line() {
        assert_eq!(
            section_texts("Gate: green\nChanged: a"),
            None,
            "sections without a Status: line are not a report"
        );
    }
}
