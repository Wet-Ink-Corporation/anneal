//! The lint finding model shared by every `anneal lint` target: severity, location, message and
//! the fix string, and its conversion to the envelope's [`Finding`].

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::envelope::Finding;
use crate::lint::skills::checker::Checker;

/// How much a finding matters. Only [`Severity::Error`] fails the run (exit 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A rule is broken; the run exits 1.
    Error,
    /// A rule is probably broken (heuristic); listed, exit unaffected.
    Warning,
    /// The rule cannot be checked mechanically; a human or reviewer must look. Listed, exit
    /// unaffected, never silently passed.
    Review,
}

impl Severity {
    /// The lowercase name used in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Review => "review",
        }
    }
}

/// Where a finding points. `line` is 1-based; 0 means the path as a whole (a directory, or a
/// file with no single offending line).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Location {
    /// The file or directory, as given on the command line plus the path inside it.
    pub file: String,
    /// 1-based line, or 0 for the whole path.
    pub line: usize,
}

impl Location {
    /// A location at `line` of `file`.
    #[must_use]
    pub fn new(file: impl Into<String>, line: usize) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// One lint finding. Every finding names its rule (through its checker), its location and a
/// fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintFinding {
    /// The checker that produced it; [`Checker::rule`] gives the design rule.
    pub checker: Checker,
    /// How much it matters.
    pub severity: Severity,
    /// Where it is.
    #[serde(flatten)]
    pub at: Location,
    /// What is wrong, one line.
    pub message: String,
    /// What to do about it, one line.
    pub fix: String,
    /// Disambiguates findings of one checker on one line (e.g. the other skill of an overlap
    /// pair); part of the stable key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl LintFinding {
    /// An [`Severity::Error`] finding from `checker` at `at`.
    #[must_use]
    pub fn error(
        checker: Checker,
        at: Location,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self::build(checker, Severity::Error, at, (message.into(), fix.into()))
    }

    /// A [`Severity::Warning`] finding from `checker` at `at`.
    #[must_use]
    pub fn warning(
        checker: Checker,
        at: Location,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self::build(checker, Severity::Warning, at, (message.into(), fix.into()))
    }

    /// A [`Severity::Review`] finding from `checker` at `at`.
    #[must_use]
    pub fn review(
        checker: Checker,
        at: Location,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self::build(checker, Severity::Review, at, (message.into(), fix.into()))
    }

    fn build(
        checker: Checker,
        severity: Severity,
        at: Location,
        (message, fix): (String, String),
    ) -> Self {
        Self {
            checker,
            severity,
            at,
            message,
            fix,
            subject: None,
        }
    }

    /// The same finding, disambiguated by `subject`.
    #[must_use]
    pub fn with_subject(self, subject: impl Into<String>) -> Self {
        Self {
            subject: Some(subject.into()),
            ..self
        }
    }

    /// Stable identity for baselines: `lint/<checker>:<file>:<line>[:<subject>]`.
    #[must_use]
    pub fn key(&self) -> String {
        match &self.subject {
            Some(subject) => format!("lint/{}:{}:{subject}", self.checker, self.at),
            None => format!("lint/{}:{}", self.checker, self.at),
        }
    }

    /// The envelope finding: the key, and one line
    /// `<file>:<line>: <severity> rule <n> [<checker>] <message> fix: <fix>`.
    #[must_use]
    pub fn to_envelope(&self) -> Finding {
        Finding::new(
            self.key(),
            format!(
                "{}: {} rule {} [{}] {} fix: {}",
                self.at,
                self.severity.as_str(),
                self.checker.rule(),
                self.checker,
                self.message,
                self.fix
            ),
        )
    }

    /// Sort order for output: file, line, checker, subject.
    pub(crate) fn sort_key(&self) -> (&Location, Checker, Option<&str>) {
        (&self.at, self.checker, self.subject.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::{LintFinding, Location};
    use crate::envelope::Finding;
    use crate::lint::skills::checker::Checker;

    #[test]
    fn the_envelope_line_names_location_severity_rule_checker_and_fix() {
        let finding = LintFinding::error(
            Checker::Name,
            Location::new("skills/Plan/SKILL.md", 2),
            "name `Plan` is not kebab-case.",
            "rename it to `plan`.",
        );

        assert_eq!(
            finding.to_envelope(),
            Finding::new(
                "lint/name:skills/Plan/SKILL.md:2",
                "skills/Plan/SKILL.md:2: error rule 1 [name] name `Plan` is not kebab-case. \
                 fix: rename it to `plan`."
            ),
            "one line with every part"
        );
    }

    #[test]
    fn a_subject_makes_the_key_unique() {
        let at = Location::new("s/a/SKILL.md", 3);
        let finding = LintFinding::warning(Checker::Overlap, at, "m", "f").with_subject("b");
        assert_eq!(
            finding.key(),
            "lint/overlap:s/a/SKILL.md:3:b",
            "subject last"
        );
    }
}
