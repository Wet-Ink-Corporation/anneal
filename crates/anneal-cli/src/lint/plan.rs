//! `anneal lint plan`: checks a plan written from `plugin/playbooks/plan.template.md`.
//!
//! Five rules, each finding with a fix (see `docs/plan.md`): a `Spec:` line in the header
//! names a spec that exists (`spec`), the sections Goal, Tasks and Review focus are present
//! (`sections`), Tasks holds `### <id>: <title>` tasks with unique ids (`tasks`), each task has
//! its files, one test command and its blockers (`task-fields`), and every blocker is another
//! task of the plan, with no cycle (`blockers`). [`check`] is the pure check of the text;
//! [`lint`] the edge that reads the file and looks for the spec.
//!
//! The plan as read ([`Plan`]) is `data.plan` in the envelope, and
//! `plugin/playbooks/schemas/plan.schema.json` describes it.

mod parse;
mod rules;

use std::fmt::{self, Display, Formatter};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::lint::read_whole_bytes;
use crate::lint::skills::LintError;
use crate::lint::spec::Source;
use crate::lint::spec::markdown::Document;

/// Arguments of `anneal lint plan`.
#[derive(Debug, Clone, Args)]
pub struct PlanArgs {
    /// The plan to check
    #[arg(value_name = "FILE")]
    pub file: PathBuf,
}

/// The rule a plan finding breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// The `Spec:` line is missing, names nothing, is repeated, or its file does not exist.
    Spec,
    /// A required section is missing, repeated or empty.
    Sections,
    /// No task, a heading that is not a task, or a repeated task id.
    Tasks,
    /// A task's `Files:`, `Test:` or `Blocked by:` line is missing, repeated or malformed.
    TaskFields,
    /// A blocker that is the task itself, not a task of the plan, or part of a cycle.
    Blockers,
}

impl Rule {
    /// The rule's id in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::Sections => "sections",
            Self::Tasks => "tasks",
            Self::TaskFields => "task-fields",
            Self::Blockers => "blockers",
        }
    }

    /// A finding of this rule at `line` of `file` (0 for the whole file), still to be worded.
    const fn at(self, file: &str, line: usize) -> At<'_> {
        At {
            rule: self,
            file,
            line,
        }
    }
}

impl Display for Rule {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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
    /// The finding, with what is wrong, what to do about it and what it is about.
    fn finding(
        self,
        message: impl Into<String>,
        fix: impl Into<String>,
        subject: Option<&str>,
    ) -> PlanFinding {
        PlanFinding {
            rule: self.rule,
            file: self.file.to_owned(),
            line: self.line,
            message: message.into(),
            fix: fix.into(),
            subject: subject.map(str::to_owned),
        }
    }
}

/// One plan finding. Every finding is an error (exit 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanFinding {
    /// The broken rule.
    pub rule: Rule,
    /// The plan file.
    pub file: String,
    /// 1-based line, or 0 for the file as a whole.
    pub line: usize,
    /// What is wrong, one line.
    pub message: String,
    /// What to do about it, one line.
    pub fix: String,
    /// What the finding is about (a section, a field, a blocker) when one line can hold
    /// several.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl PlanFinding {
    /// Stable identity for baselines: `lint/plan/<rule>:<file>:<line>[:<subject>]`.
    #[must_use]
    pub fn key(&self) -> String {
        match &self.subject {
            Some(subject) => format!(
                "lint/plan/{}:{}:{}:{subject}",
                self.rule, self.file, self.line
            ),
            None => format!("lint/plan/{}:{}:{}", self.rule, self.file, self.line),
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

/// A plan as read: `data.plan`, described by `plugin/playbooks/schemas/plan.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    /// The spec the `Spec:` line names, as written (relative to the plan's directory).
    pub spec: Option<String>,
    /// The tasks with well-formed headings, in file order, each id once.
    pub tasks: Vec<Task>,
}

/// One task of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    /// The id from its heading (`T1`).
    pub id: String,
    /// The title from its heading.
    pub title: String,
    /// The heading's line.
    pub line: usize,
    /// The files its `Files:` line names in backticks.
    pub files: Vec<String>,
    /// The command its `Test:` line names in backticks, if any.
    pub test: Option<String>,
    /// The ids its `Blocked by:` line lists (empty for `none`).
    pub blocked_by: Vec<String>,
}

/// What the checks found in one plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The plan as read.
    pub plan: Plan,
    /// The `Spec:` line's number, or 0 when there is none.
    pub spec_line: usize,
    /// Every finding, sorted by line, rule and subject.
    pub findings: Vec<PlanFinding>,
}

/// Checks the text of `plan`. Never fails: every problem is a finding. Whether the spec exists
/// is left to [`lint`], which reads the file system.
#[must_use]
pub fn check(plan: Source<'_>) -> Report {
    let doc = Document::parse(plan.text);
    let sections = doc.sections();
    let header_end = sections.first().map_or(usize::MAX, |section| section.line);
    let header = doc
        .lines()
        .iter()
        .take_while(|line| line.number < header_end);
    let (spec, mut findings) = rules::spec(plan.file, header);
    findings.extend(rules::sections(plan.file, &sections));
    let (tasks, task_findings) = rules::tasks(plan.file, rules::find(&sections, rules::TASKS));
    findings.extend(task_findings);
    findings.extend(rules::blockers(plan.file, &tasks));
    sort(&mut findings);
    Report {
        plan: Plan {
            spec: spec.map(|(target, _)| target.to_owned()),
            tasks: tasks.into_iter().map(|read| read.task).collect(),
        },
        spec_line: spec.map_or(0, |(_, line)| line),
        findings,
    }
}

fn sort(findings: &mut [PlanFinding]) {
    findings.sort_by(|a, b| (a.line, a.rule, &a.subject).cmp(&(b.line, b.rule, &b.subject)));
}

/// What a `lint plan` run read and found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The plan, as named.
    pub file: String,
    /// The spec's path: the `Spec:` target joined to the plan's directory.
    pub spec: Option<PathBuf>,
    /// What the checks found, including a missing spec file.
    pub report: Report,
}

/// Reads and checks `args.file`, and checks that the spec it links is a file.
///
/// # Errors
/// [`LintError::MissingPath`] or [`LintError::WrongKind`] when the plan is missing or a
/// directory, [`LintError::TooLarge`] when it is over the read cap, [`LintError::Io`] when it
/// cannot be read.
pub fn lint(args: &PlanArgs) -> Result<Outcome, LintError> {
    let bytes = read_whole_bytes(&args.file)?;
    Ok(examine(&args.file, &bytes))
}

/// Checks the plan `path` whose content is `bytes` (read lossily), and looks for its spec.
#[must_use]
pub fn examine(path: &Path, bytes: &[u8]) -> Outcome {
    let file = path.display().to_string();
    let text = String::from_utf8_lossy(bytes);
    let mut report = check(Source {
        file: &file,
        text: &text,
    });
    let spec = report.plan.spec.as_deref().map(|target| {
        path.parent()
            .map_or_else(|| PathBuf::from(target), |dir| dir.join(target))
    });
    if let Some(resolved) = spec.as_deref().filter(|resolved| !resolved.is_file()) {
        report
            .findings
            .push(Rule::Spec.at(&file, report.spec_line).finding(
                format!("the spec {} is not a file", resolved.display()),
                "link the spec relative to this plan's directory, as a markdown link is read.",
                None,
            ));
        sort(&mut report.findings);
    }
    Outcome { file, spec, report }
}

/// The envelope's `data` for `lint plan`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    file: &'a str,
    spec: Option<&'a Path>,
    tasks: usize,
    plan: &'a Plan,
    findings: Vec<Detail<'a>>,
}

#[derive(Debug, Serialize)]
struct Detail<'a> {
    key: String,
    #[serde(flatten)]
    finding: &'a PlanFinding,
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
        spec: outcome.spec.as_deref(),
        tasks: report.plan.tasks.len(),
        plan: &report.plan,
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
        .map(PlanFinding::to_envelope)
        .collect();
    Envelope::new(CommandName::Lint, Some("plan"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

#[cfg(test)]
mod tests;
