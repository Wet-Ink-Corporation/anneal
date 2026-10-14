//! `anneal lint plan`: checks a plan written from `plugin/playbooks/plan.template.md`.
//!
//! Five rules, each finding with a fix (see `docs/plan.md`): a `Spec:` line in the header
//! names a spec that exists (`spec`), the sections Goal, Tasks and Review focus are present
//! (`sections`), Tasks holds `### <id>: <title>` tasks with unique ids (`tasks`), each task has
//! its files, one test command and its blockers (`task-fields`), and every blocker is another
//! task of the plan, with no cycle (`blockers`). [`check`] is the pure check of the text;
//! [`lint`] the edge that reads the file and looks for the spec.
//!
//! The plan as read ([`Plan`]) is `data.plan` in the envelope, its tasks listed up to
//! [`MAX_LISTED_TASKS`], and `plugin/playbooks/schemas/plan.schema.json` describes it.

mod parse;
mod rules;

use std::fmt::{self, Display, Formatter};
use std::io::Write;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

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
use crate::path_text::slashed;

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
    /// The first [`MAX_FINDINGS`] findings, sorted by line, rule and subject.
    pub findings: Vec<PlanFinding>,
    /// How many findings past [`MAX_FINDINGS`] were dropped.
    pub omitted: usize,
}

/// The most findings a report keeps.
///
/// A plan within the read cap can hold about 100,000 one-line task headings, and a finding
/// for each would make an envelope of about 100 MB. The rest are counted in
/// [`Report::omitted`]; any finding still fails the plan.
pub const MAX_FINDINGS: usize = 1_000;

/// Sorts `findings` and keeps the first [`MAX_FINDINGS`], adding the rest to `omitted`.
fn bound(findings: &mut Vec<PlanFinding>, omitted: &mut usize) {
    sort(findings);
    let over = findings.len().saturating_sub(MAX_FINDINGS);
    findings.truncate(MAX_FINDINGS);
    *omitted = omitted.saturating_add(over);
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
    let mut omitted = 0;
    bound(&mut findings, &mut omitted);
    Report {
        plan: Plan {
            spec: spec.map(|(target, _)| target.to_owned()),
            tasks: tasks.into_iter().map(|read| read.task).collect(),
        },
        spec_line: spec.map_or(0, |(_, line)| line),
        findings,
        omitted,
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
    /// The spec's path: the `Spec:` target joined to the plan's directory, or as written when
    /// it is absolute.
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
    let spec = report
        .plan
        .spec
        .as_deref()
        .map(|target| PathBuf::from(spec_path(&file, target, MAIN_SEPARATOR)));
    if let Some(resolved) = spec.as_deref().filter(|resolved| !resolved.is_file()) {
        report
            .findings
            .push(Rule::Spec.at(&file, report.spec_line).finding(
                format!("the spec {} is not a file", resolved.display()),
                "link the spec relative to this plan's directory, as a markdown link is read.",
                None,
            ));
        bound(&mut report.findings, &mut report.omitted);
    }
    Outcome { file, spec, report }
}

/// The path of the spec a plan at `plan` links as `target`, where `separator` is the
/// platform's path separator: an absolute `target` as is, otherwise `target` joined to the
/// plan's directory. Joined, and the directory's separators replaced, with `/` rather than
/// `Path::join`, so the path shown (and keyed) is the same on every platform; Windows opens
/// `/`-separated paths too.
fn spec_path(plan: &str, target: &str, separator: char) -> String {
    let drive = separator == '\\'
        && matches!(target.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    if drive || target.starts_with(['/', separator]) {
        return target.to_owned();
    }
    match slashed(plan, separator).rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{target}"),
        None => target.to_owned(),
    }
}

/// The most tasks the envelope lists in `data.plan.tasks`.
///
/// A plan within the read cap can hold about 10,000 unique tasks with their fields, about
/// 6 MB of JSON; past this many the rest are counted in `data.omitted_tasks`, and `data.tasks`
/// still counts every task read.
pub const MAX_LISTED_TASKS: usize = 1_000;

/// The envelope's `data` for `lint plan`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    file: &'a str,
    spec: Option<&'a Path>,
    /// Every task read, listed or not.
    tasks: usize,
    plan: Listed<'a>,
    findings: Vec<Detail<'a>>,
    /// How many findings were dropped at [`MAX_FINDINGS`]; present only when some were.
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted: Option<usize>,
    /// How many tasks were left out of `plan.tasks` at [`MAX_LISTED_TASKS`]; present only
    /// when some were.
    #[serde(skip_serializing_if = "Option::is_none")]
    omitted_tasks: Option<usize>,
}

/// `data.plan`: a [`Plan`] with at most [`MAX_LISTED_TASKS`] tasks, the first in file order.
/// Its fields are [`Plan`]'s, as `plan.schema.json` describes them.
#[derive(Debug, Serialize)]
struct Listed<'a> {
    spec: Option<&'a str>,
    tasks: &'a [Task],
}

impl<'a> Listed<'a> {
    fn of(plan: &'a Plan) -> Self {
        // The first chunk is the first `MAX_LISTED_TASKS` tasks, or all of fewer; a plan with
        // no task has no chunk, and lists none.
        let first = plan.tasks.chunks(MAX_LISTED_TASKS).next();
        Self {
            spec: plan.spec.as_deref(),
            tasks: first.unwrap_or(&[]),
        }
    }
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
        plan: Listed::of(&report.plan),
        findings: report
            .findings
            .iter()
            .map(|finding| Detail {
                key: finding.key(),
                finding,
            })
            .collect(),
        omitted: (report.omitted > 0).then_some(report.omitted),
        omitted_tasks: Some(report.plan.tasks.len().saturating_sub(MAX_LISTED_TASKS))
            .filter(|&left_out| left_out > 0),
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
