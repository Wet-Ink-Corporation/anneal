//! `anneal lint spec`: checks a spec written from `plugin/playbooks/spec.template.md`.
//!
//! Seven rules, each finding with a fix (see `docs/spec.md`): the required sections are present
//! (`sections`), every acceptance criterion is EARS (`ears`), Out of Scope is not empty
//! (`out-of-scope`), at most [`MAX_CLARIFICATIONS`] `[NEEDS CLARIFICATION]` markers
//! (`needs-clarification`), the constitution check covers the constitution's gates and
//! justifies each violation (`constitution`), no avoided glossary term is used
//! (`avoided-term`), and a `Tree:` header line, when there is one, names a valid design tree
//! (`tree`, #77). [`check`] is the pure check, [`lint`] the edge that reads the files.

mod checks;
mod constitution;
pub mod ears;
pub(crate) mod markdown;
mod tree;

use std::fmt::{self, Display, Formatter};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::read_whole;
use crate::lint::skills::LintError;
use crate::lint::terms::Glossary;
use checks::{ACCEPTANCE_CRITERIA, CONSTITUTION_CHECK, OUT_OF_SCOPE, find};
use constitution::Constitution;
use markdown::{Document, Line, Section};
pub use tree::{TreeLink, tree_link, tree_path};

/// The most `[NEEDS CLARIFICATION]` markers a spec may hold.
pub const MAX_CLARIFICATIONS: usize = 3;

/// The glossary read when `--glossary` is not given, if it exists.
pub const DEFAULT_GLOSSARY: &str = "CONTEXT.md";
/// The constitution read when `--constitution` is not given, if it exists.
pub const DEFAULT_CONSTITUTION: &str = "constitution.md";

/// Arguments of `anneal lint spec`.
#[derive(Debug, Clone, Args)]
pub struct SpecArgs {
    /// The spec to check
    #[arg(value_name = "FILE")]
    pub file: PathBuf,
    /// The glossary whose `_Avoid_:` terms the spec must not use [default: CONTEXT.md, read
    /// only when it exists]
    #[arg(long, value_name = "FILE")]
    pub glossary: Option<PathBuf>,
    /// The constitution whose gates the spec's constitution check must cover [default:
    /// constitution.md, read only when it exists]
    #[arg(long, value_name = "FILE")]
    pub constitution: Option<PathBuf>,
}

/// The rule a spec finding breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// A required section is missing or repeated.
    Sections,
    /// An acceptance criterion is not EARS, or there is none.
    Ears,
    /// Out of Scope is empty.
    OutOfScope,
    /// More than [`MAX_CLARIFICATIONS`] clarification markers.
    NeedsClarification,
    /// The constitution check is incomplete, stale or unjustified, or the constitution is
    /// malformed.
    Constitution,
    /// A term the glossary avoids.
    AvoidedTerm,
    /// The `Tree:` line is repeated or names nothing, or its file is no valid design tree.
    Tree,
}

impl Rule {
    /// The rule's id in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sections => "sections",
            Self::Ears => "ears",
            Self::OutOfScope => "out-of-scope",
            Self::NeedsClarification => "needs-clarification",
            Self::Constitution => "constitution",
            Self::AvoidedTerm => "avoided-term",
            Self::Tree => "tree",
        }
    }
}

impl Display for Rule {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One spec finding. Every finding is an error (exit 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecFinding {
    /// The broken rule.
    pub rule: Rule,
    /// The file: the spec, or the constitution for its own findings.
    pub file: String,
    /// 1-based line, or 0 for the file as a whole.
    pub line: usize,
    /// What is wrong, one line.
    pub message: String,
    /// What to do about it, one line.
    pub fix: String,
    /// What the finding is about (a section, a gate, a term) when one line can hold several.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl Rule {
    /// A finding of this rule at `line` of `file` (0 for the whole file), still to be worded.
    const fn at(self, file: &str, line: usize) -> At<'_> {
        At {
            rule: self,
            file,
            line,
        }
    }
}

/// Where a finding of a rule points, before it has a message and a fix.
#[derive(Debug, Clone, Copy)]
struct At<'a> {
    rule: Rule,
    file: &'a str,
    line: usize,
}

impl At<'_> {
    /// The finding, with what is wrong and what to do about it.
    fn finding(self, message: impl Into<String>, fix: impl Into<String>) -> SpecFinding {
        SpecFinding {
            rule: self.rule,
            file: self.file.to_owned(),
            line: self.line,
            message: message.into(),
            fix: fix.into(),
            subject: None,
        }
    }
}

impl SpecFinding {
    fn about(self, subject: impl Into<String>) -> Self {
        Self {
            subject: Some(subject.into()),
            ..self
        }
    }

    /// Stable identity for baselines: `lint/spec/<rule>:<file>:<line>[:<subject>]`.
    #[must_use]
    pub fn key(&self) -> String {
        match &self.subject {
            Some(subject) => format!(
                "lint/spec/{}:{}:{}:{subject}",
                self.rule, self.file, self.line
            ),
            None => format!("lint/spec/{}:{}:{}", self.rule, self.file, self.line),
        }
    }

    /// The envelope finding: the key, and one line
    /// `<file>:<line>: error [<rule>] <message> fix: <fix>`.
    #[must_use]
    pub fn to_envelope(&self) -> Finding {
        Finding::new(
            self.key(),
            format!(
                "{}:{}: error [{}] {} fix: {}",
                self.file, self.line, self.rule, self.message, self.fix
            ),
        )
    }
}

/// A file to check: its name in output and its text.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    /// The file name, as shown in findings.
    pub file: &'a str,
    /// Its contents.
    pub text: &'a str,
}

/// What the checks found in one spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Every finding, sorted by file, line, rule and subject.
    pub findings: Vec<SpecFinding>,
    /// The acceptance criteria (list items) read.
    pub criteria: usize,
    /// The `[NEEDS CLARIFICATION]` markers outside code and comments, however many.
    pub needs_clarification: usize,
    /// The constitution's version, when a constitution with a version line was read.
    pub constitution_version: Option<String>,
    /// The constitution's gates (0 without a constitution).
    pub gates: usize,
    /// The design tree the first `Tree:` line of the header links, if there is one.
    pub tree: Option<TreeLink>,
}

/// Checks `spec`, with the avoided terms of `glossary` and the gates of `constitution` when
/// given. Never fails: every problem is a finding.
#[must_use]
pub fn check(
    spec: Source<'_>,
    glossary: Option<&Glossary>,
    constitution: Option<Source<'_>>,
) -> Report {
    let doc = Document::parse(spec.text);
    let sections = doc.sections();
    let mut findings = checks::sections(spec.file, &sections);
    let (tree, tree_findings) = tree::link(spec.file, header(&doc, &sections));
    findings.extend(tree_findings);
    let (criteria, ears) = checks::criteria(spec.file, find(&sections, ACCEPTANCE_CRITERIA));
    findings.extend(ears);
    findings.extend(checks::out_of_scope(
        spec.file,
        find(&sections, OUT_OF_SCOPE),
    ));
    let (needs_clarification, over) = checks::clarifications(spec.file, doc.lines());
    findings.extend(over);
    let parsed = constitution.map(|source| (source.file, Constitution::parse(source.text)));
    findings.extend(constitution::check(
        spec.file,
        find(&sections, CONSTITUTION_CHECK),
        parsed.as_ref().map(|(file, parsed)| (*file, parsed)),
    ));
    if let Some(glossary) = glossary {
        findings.extend(
            glossary
                .scan(spec.file, spec.text)
                .into_iter()
                .map(checks::avoided),
        );
    }
    sort(&mut findings);
    let (constitution_version, gates) = parsed.map_or((None, 0), |(_, parsed)| {
        (parsed.version, parsed.gates.len())
    });
    Report {
        findings,
        criteria,
        needs_clarification,
        constitution_version,
        gates,
        tree,
    }
}

/// The header's lines: those before the first `##` section, where the `Tree:` line goes.
fn header<'d>(
    doc: &'d Document<'d>,
    sections: &[Section<'_>],
) -> impl Iterator<Item = &'d Line<'d>> {
    let end = sections.first().map_or(usize::MAX, |section| section.line);
    doc.lines().iter().take_while(move |line| line.number < end)
}

/// Sorts findings by file, line, rule and subject.
fn sort(findings: &mut [SpecFinding]) {
    findings.sort_by(|a, b| {
        (&a.file, a.line, a.rule, &a.subject).cmp(&(&b.file, b.line, b.rule, &b.subject))
    });
}

/// What a `lint spec` run read and found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The spec, as named.
    pub file: String,
    /// The glossary read, if any.
    pub glossary: Option<String>,
    /// The constitution read, if any.
    pub constitution: Option<String>,
    /// The design tree the `Tree:` line names, resolved against the spec's directory.
    pub tree: Option<PathBuf>,
    /// What the checks found, including a design tree that cannot be read or is not valid.
    pub report: Report,
}

/// Reads and checks `args.file`.
///
/// The glossary and constitution are the named files, or the defaults ([`DEFAULT_GLOSSARY`],
/// [`DEFAULT_CONSTITUTION`]) when those exist; an absent default is skipped. Files that are
/// not UTF-8 are read lossily. The design tree a `Tree:` line names is read too: one that
/// cannot be read, or is not a valid design tree, is a `tree` finding, not an error.
///
/// # Errors
/// [`LintError::MissingPath`] or [`LintError::WrongKind`] when the spec or a named glossary or
/// constitution is missing or a directory, [`LintError::TooLarge`] when one is over the read
/// cap, [`LintError::Io`] when one cannot be read.
pub fn lint(args: &SpecArgs) -> Result<Outcome, LintError> {
    let spec = read_whole(&args.file)?;
    let glossary_path = named_or_present(args.glossary.as_deref(), DEFAULT_GLOSSARY);
    let glossary = glossary_path
        .map(|path| read_whole(path).map(|text| Glossary::parse(&text)))
        .transpose()?;
    let constitution_path = named_or_present(args.constitution.as_deref(), DEFAULT_CONSTITUTION);
    let constitution_text = constitution_path.map(read_whole).transpose()?;
    let file = args.file.display().to_string();
    let constitution = constitution_path.map(|path| path.display().to_string());
    let mut report = check(
        Source {
            file: &file,
            text: &spec,
        },
        glossary.as_ref(),
        constitution
            .as_deref()
            .zip(constitution_text.as_deref())
            .map(|(file, text)| Source { file, text }),
    );
    let tree = match &report.tree {
        Some(TreeLink::Named { target, line }) => {
            let path = tree_path(&file, target);
            if let Some(finding) = tree::check_file(&file, *line, &path) {
                report.findings.push(finding);
                sort(&mut report.findings);
            }
            Some(path)
        }
        Some(TreeLink::Unnamed { .. }) | None => None,
    };
    Ok(Outcome {
        file,
        glossary: glossary_path.map(|path| path.display().to_string()),
        constitution,
        tree,
        report,
    })
}

/// The path to read: the one named, else `default` when it exists.
fn named_or_present<'p>(named: Option<&'p Path>, default: &'static str) -> Option<&'p Path> {
    named.or_else(|| {
        let default = Path::new(default);
        default.exists().then_some(default)
    })
}

/// The `[NEEDS CLARIFICATION]` markers in `text`, counted as `lint spec` counts them: outside
/// code fences, HTML comments and code spans. `plan approve` needs none.
#[must_use]
pub fn needs_clarification(text: &str) -> usize {
    let doc = Document::parse(text);
    checks::clarifications("", doc.lines()).0
}

/// The envelope's `data` for `lint spec`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    file: &'a str,
    glossary: Option<&'a str>,
    constitution: Option<ConstitutionData<'a>>,
    criteria: usize,
    needs_clarification: usize,
    /// The design tree the `Tree:` line names; absent without one, so a spec written before
    /// the line existed keeps its envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    tree: Option<&'a Path>,
    findings: Vec<Detail<'a>>,
}

#[derive(Debug, Serialize)]
struct ConstitutionData<'a> {
    file: &'a str,
    version: Option<&'a str>,
    gates: usize,
}

#[derive(Debug, Serialize)]
struct Detail<'a> {
    key: String,
    #[serde(flatten)]
    finding: &'a SpecFinding,
}

/// Writes the envelope for `outcome` and returns its code: 1 when there is any finding, else 0.
///
/// # Errors
/// [`CliError::Output`] when `out` cannot be written.
pub(crate) fn report<O: Write>(outcome: &Outcome, out: &mut O) -> Result<Code, CliError> {
    let report = &outcome.report;
    let code = if report.findings.is_empty() {
        exit_codes::LINT_CLEAN
    } else {
        exit_codes::LINT_FINDINGS
    };
    let data = Data {
        file: &outcome.file,
        glossary: outcome.glossary.as_deref(),
        constitution: outcome
            .constitution
            .as_deref()
            .map(|file| ConstitutionData {
                file,
                version: report.constitution_version.as_deref(),
                gates: report.gates,
            }),
        criteria: report.criteria,
        needs_clarification: report.needs_clarification,
        tree: outcome.tree.as_deref(),
        findings: report
            .findings
            .iter()
            .map(|finding| Detail {
                key: finding.key(),
                finding,
            })
            .collect(),
    };
    let findings: Vec<Finding> = report
        .findings
        .iter()
        .map(SpecFinding::to_envelope)
        .collect();
    Envelope::new(CommandName::Lint, Some("spec"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
