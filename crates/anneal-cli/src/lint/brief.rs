//! `anneal lint brief`: each of the nine fields once, with content.
//!
//! A brief has each field of `brief::fields::FIELDS` as one `## <heading>` section with
//! content. Text inside code fences and HTML comments is not content, so the placeholders
//! `brief build` writes for a field it was not given fail. Headings match in any ASCII letter
//! case; other sections are allowed. See `docs/brief.md`.

use std::fmt::{self, Display, Formatter};
use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::brief::fields::{FIELDS, Field};
use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::read_whole;
use crate::lint::skills::LintError;
use crate::lint::spec::markdown::{Document, Section};

/// Arguments of `anneal lint brief`.
#[derive(Debug, Clone, Args)]
pub struct BriefArgs {
    /// The brief to check
    #[arg(value_name = "FILE")]
    pub file: PathBuf,
}

/// The rule a brief finding breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Rule {
    /// A field has no section.
    #[serde(rename = "field-missing")]
    Missing,
    /// A field's section has no content.
    #[serde(rename = "field-empty")]
    Empty,
    /// A field has more than one section.
    #[serde(rename = "field-repeated")]
    Repeated,
}

impl Rule {
    /// The rule's id in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "field-missing",
            Self::Empty => "field-empty",
            Self::Repeated => "field-repeated",
        }
    }
}

impl Display for Rule {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One brief finding. Every finding is an error (exit 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BriefFinding {
    /// The broken rule.
    pub rule: Rule,
    /// The field's heading.
    pub field: &'static str,
    /// The 1-based line of the section's heading, or 0 for a missing field.
    pub line: usize,
}

impl BriefFinding {
    /// What is wrong, in one line.
    #[must_use]
    pub fn message(&self) -> String {
        match self.rule {
            Rule::Missing => format!("the brief has no `## {}` section", self.field),
            Rule::Empty => format!("the `## {}` section is empty", self.field),
            Rule::Repeated => format!("the brief has a second `## {}` section", self.field),
        }
    }

    /// What to do about it, naming the field's purpose.
    #[must_use]
    pub fn fix(&self) -> String {
        let purpose = FIELDS
            .iter()
            .find(|field| field.heading == self.field)
            .map_or("", |field| field.purpose);
        match self.rule {
            Rule::Missing | Rule::Empty => format!("write `## {}`: {purpose}", self.field),
            Rule::Repeated => format!("keep one `## {}` section.", self.field),
        }
    }

    /// The finding in the envelope: `<file>:<line>: error [<rule>] <message> fix: <fix>`.
    fn to_envelope(&self, file: &str) -> Finding {
        Finding::new(
            format!("brief/{}:{}", self.rule, self.field),
            format!(
                "{file}:{}: error [{}] {} fix: {}",
                self.line,
                self.rule,
                self.message(),
                self.fix()
            ),
        )
    }
}

/// The findings for the brief `text`, in field order: each field missing, empty or repeated.
#[must_use]
pub fn check(text: &str) -> Vec<BriefFinding> {
    let document = Document::parse(text);
    let sections = document.sections();
    FIELDS
        .iter()
        .flat_map(|field| field_findings(field, &sections))
        .collect()
}

/// The findings for one `field` among `sections`.
fn field_findings(field: &Field, sections: &[Section<'_>]) -> Vec<BriefFinding> {
    let finding = |rule, line| BriefFinding {
        rule,
        field: field.heading,
        line,
    };
    let mut matching = sections
        .iter()
        .filter(|section| section.title.eq_ignore_ascii_case(field.heading));
    let Some(first) = matching.next() else {
        return vec![finding(Rule::Missing, 0)];
    };
    let empty = first.body.iter().all(|line| line.text.trim().is_empty());
    let mut findings: Vec<BriefFinding> = Vec::new();
    if empty {
        findings.push(finding(Rule::Empty, first.line));
    }
    findings.extend(matching.map(|again| finding(Rule::Repeated, again.line)));
    findings
}

/// What `lint brief` found in one file.
#[derive(Debug)]
pub struct Outcome {
    /// The file, as given.
    pub file: String,
    /// Its findings.
    pub findings: Vec<BriefFinding>,
}

/// Reads the brief `args.file` and checks it.
///
/// # Errors
/// The [`LintError`] when the file is missing, a directory, unreadable or over the read cap.
pub fn lint(args: &BriefArgs) -> Result<Outcome, LintError> {
    let text = read_whole(&args.file)?;
    Ok(Outcome {
        file: args.file.display().to_string(),
        findings: check(&text),
    })
}

/// The envelope's `data`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    file: &'a str,
    fields: usize,
    findings: &'a [BriefFinding],
}

/// Writes the envelope for `outcome` and returns its code: 1 with any finding, else 0.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn report<O: Write>(outcome: &Outcome, out: &mut O) -> Result<Code, CliError> {
    let code = if outcome.findings.is_empty() {
        exit_codes::LINT_CLEAN
    } else {
        exit_codes::LINT_FINDINGS
    };
    let data = Data {
        file: &outcome.file,
        fields: FIELDS.len(),
        findings: &outcome.findings,
    };
    let findings: Vec<Finding> = outcome
        .findings
        .iter()
        .map(|finding| finding.to_envelope(&outcome.file))
        .collect();
    Envelope::new(CommandName::Lint, Some("brief"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests {
    use super::{BriefFinding, Rule, check};

    /// A brief with every field filled, in table order.
    const COMPLETE: &str = "# Brief: 36\n\n\
        ## Goal\nShip the round counter.\n\n\
        ## Repository\n/work/anneal, from origin/main.\n\n\
        ## Owned scope\ncrates/anneal-cli/src/round.rs\n\n\
        ## Do not touch\nThe ledger crate.\n\n\
        ## Invariants\nRounds only grow.\n\n\
        ## Success criteria\nround status says park after five rounds.\n\n\
        ## Verification\n`cargo test -p anneal-cli round`\n\n\
        ## Edits allowed\nyes\n\n\
        ## Return contract\nFiles changed and the test result.\n\n\
        ## Volatile\n- Built: 2026-10-12T09:00:00Z\n";

    fn rules(findings: &[BriefFinding]) -> Vec<(Rule, &str, usize)> {
        findings
            .iter()
            .map(|finding| (finding.rule, finding.field, finding.line))
            .collect()
    }

    #[test]
    fn a_brief_with_all_nine_fields_filled_is_clean() {
        assert_eq!(check(COMPLETE), Vec::new(), "nothing to report");
    }

    #[test]
    fn a_missing_field_is_reported_at_line_0() {
        let text = COMPLETE.replace("## Invariants\nRounds only grow.\n\n", "");

        assert_eq!(
            rules(&check(&text)),
            [(Rule::Missing, "Invariants", 0)],
            "the one missing field"
        );
    }

    #[test]
    fn a_field_holding_only_a_comment_or_a_blank_is_empty() {
        let text = COMPLETE
            .replace(
                "Ship the round counter.",
                "<!-- The outcome the subagent delivers. -->",
            )
            .replace("Rounds only grow.", "   ");

        assert_eq!(
            rules(&check(&text)),
            [(Rule::Empty, "Goal", 3), (Rule::Empty, "Invariants", 15)],
            "comments and blanks are not content"
        );
    }

    #[test]
    fn a_field_given_twice_is_repeated_at_its_second_heading() {
        let text = format!("{COMPLETE}\n## goal\nAgain.\n");

        assert_eq!(
            rules(&check(&text)),
            [(Rule::Repeated, "Goal", 33)],
            "headings match in any letter case"
        );
    }

    #[test]
    fn a_heading_inside_a_code_fence_is_not_a_field() {
        let text = COMPLETE.replace(
            "## Goal\nShip the round counter.\n",
            "```\n## Goal\nShip the round counter.\n```\n",
        );

        assert_eq!(
            rules(&check(&text)),
            [(Rule::Missing, "Goal", 0)],
            "fenced text is not read"
        );
    }

    #[test]
    fn every_rule_has_its_stable_id() {
        let ids = [Rule::Missing, Rule::Empty, Rule::Repeated].map(Rule::as_str);

        assert_eq!(
            ids,
            ["field-missing", "field-empty", "field-repeated"],
            "ids in findings"
        );
    }
}
