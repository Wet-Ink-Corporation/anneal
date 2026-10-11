//! What `anneal calibrate` writes: the envelope on stdout, the proposals file, the manual
//! steps that would apply a proposal, and the short summary on stderr.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::baseline;
use crate::calibrate::CalibrateError;
use crate::calibrate::report::{PointReport, QuestionReport, Report, Verdict};
use crate::calibrate::score::{FALSE_ACCEPT_COST, NEEDLESS_COST};
use crate::command::CommandName;
use crate::decide::point::{Point, Question};
use crate::envelope::Envelope;
use crate::exit_codes::{self, Code};

/// The proposals file's `kind`.
const PROPOSALS_KIND: &str = "anneal-threshold-proposals";

/// The proposals file's format version.
const PROPOSALS_VERSION: u32 = 1;

/// The `userConfig` key holding the Clef mode.
const MODE_KEY: &str = "clef_mode";

/// The variable Claude Code passes the mode in.
const MODE_VAR: &str = "CLAUDE_PLUGIN_OPTION_CLEF_MODE";

/// One manual step that would apply a proposal. `calibrate` never takes it.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step<'a> {
    /// Set a band's `at_least` in the thresholds file.
    Threshold {
        point: Point,
        question: Question,
        /// The thresholds file read; `None` for the copy built in, which cannot be edited.
        file: Option<&'a Path>,
        pointer: &'a str,
        from: f64,
        to: f64,
    },
    /// Add a per-point `enforce` override to the Clef mode.
    Mode {
        point: Point,
        user_config: &'static str,
        env: &'static str,
        value: String,
    },
}

impl Step<'_> {
    const fn point(&self) -> Point {
        match self {
            Self::Threshold { point, .. } | Self::Mode { point, .. } => *point,
        }
    }
}

/// The cost model, as written in the envelope and the proposals file.
#[derive(Debug, Clone, Copy, Serialize)]
struct CostModel {
    false_accept: u32,
    needless: u32,
}

const COSTS: CostModel = CostModel {
    false_accept: FALSE_ACCEPT_COST,
    needless: NEEDLESS_COST,
};

/// One calibration run, as the outputs need it.
#[derive(Debug)]
pub(crate) struct Calibration<'a> {
    /// The report of every point.
    pub(crate) report: &'a Report<'a>,
    /// The manual steps for every point.
    pub(crate) steps: &'a [Step<'a>],
    /// The live log file.
    pub(crate) log: &'a Path,
    /// The log files read, oldest first.
    pub(crate) files: &'a [PathBuf],
    /// Lines of the log that are not a row this build reads.
    pub(crate) skipped_rows: u32,
    /// The proposals file.
    pub(crate) proposals: &'a Path,
    /// The thresholds file read; `None` for the copy built in.
    pub(crate) thresholds: Option<&'a Path>,
}

/// The steps that would apply `report`: a threshold edit wherever the proposal differs from
/// the current value, and an `enforce` override for each eligible point.
pub(crate) fn steps<'a>(report: &'a Report<'_>, file: Option<&'a Path>) -> Vec<Step<'a>> {
    let mut steps = Vec::new();
    for point in &report.points {
        for question in &point.questions {
            let threshold = &question.threshold;
            if let (Some(pointer), Some(from), Some(to)) =
                (&threshold.pointer, threshold.current, threshold.proposed)
                && from.total_cmp(&to).is_ne()
            {
                steps.push(Step::Threshold {
                    point: point.point,
                    question: question.question,
                    file,
                    pointer,
                    from,
                    to,
                });
            }
        }
        if point.verdict == Verdict::EligibleToEnforce {
            steps.push(Step::Mode {
                point: point.point,
                user_config: MODE_KEY,
                env: MODE_VAR,
                value: format!("{}=enforce", point.point),
            });
        }
    }
    steps
}

/// `proposals.json`: every point's report and every step, whatever `--point` selected.
#[derive(Debug, Serialize)]
struct ProposalsFile<'a> {
    kind: &'static str,
    version: u32,
    thresholds: Option<&'a Path>,
    costs: CostModel,
    points: &'a [PointReport<'a>],
    apply: &'a [Step<'a>],
}

/// Writes the proposals file atomically.
///
/// # Errors
/// [`CalibrateError::Proposals`] when it cannot be written.
pub(crate) fn write_proposals(calibration: &Calibration<'_>) -> Result<(), CalibrateError> {
    let file = ProposalsFile {
        kind: PROPOSALS_KIND,
        version: PROPOSALS_VERSION,
        thresholds: calibration.thresholds,
        costs: COSTS,
        points: &calibration.report.points,
        apply: calibration.steps,
    };
    baseline::write_snapshot_json(calibration.proposals, &file).map_err(CalibrateError::Proposals)
}

/// `data` of `anneal calibrate`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    log: &'a Path,
    files: &'a [PathBuf],
    proposals: &'a Path,
    thresholds: Option<&'a Path>,
    costs: CostModel,
    skipped_rows: u32,
    unmatched_labels: u32,
    unreported_rows: u32,
    points: Vec<&'a PointReport<'a>>,
    apply: Vec<&'a Step<'a>>,
}

/// Writes the envelope for the points `selected` lets through, and returns its code.
pub(crate) fn emit<O: Write>(
    calibration: &Calibration<'_>,
    selected: impl Fn(Point) -> bool,
    out: &mut O,
) -> io::Result<Code> {
    let data = Data {
        log: calibration.log,
        files: calibration.files,
        proposals: calibration.proposals,
        thresholds: calibration.thresholds,
        costs: COSTS,
        skipped_rows: calibration.skipped_rows,
        unmatched_labels: calibration.report.unmatched_labels,
        unreported_rows: calibration.report.unreported_rows,
        points: calibration
            .report
            .points
            .iter()
            .filter(|point| selected(point.point))
            .collect(),
        apply: calibration
            .steps
            .iter()
            .filter(|step| selected(step.point()))
            .collect(),
    };
    Envelope::new(CommandName::Calibrate, None, exit_codes::OK, data).emit(out)
}

/// Writes the short human summary: a line per point and per question, where the proposals
/// are, and each manual step.
pub(crate) fn write_summary<E: Write>(
    calibration: &Calibration<'_>,
    selected: impl Fn(Point) -> bool,
    err: &mut E,
) -> io::Result<()> {
    for point in calibration
        .report
        .points
        .iter()
        .filter(|point| selected(point.point))
    {
        writeln!(err, "{}: {}", point.point, point.verdict.as_str())?;
        for question in &point.questions {
            write_question(question, err)?;
        }
    }
    writeln!(
        err,
        "proposals: {} (nothing was applied)",
        calibration.proposals.display()
    )?;
    let thresholds = calibration.thresholds.map_or_else(
        || "a copy of plugin/playbooks/clef-thresholds.json passed with --thresholds".to_owned(),
        |path| path.display().to_string(),
    );
    for step in calibration
        .steps
        .iter()
        .filter(|step| selected(step.point()))
    {
        match step {
            Step::Threshold { pointer, to, .. } => {
                writeln!(err, "apply by hand: in {thresholds}, set {pointer} to {to}")?;
            }
            Step::Mode { value, .. } => {
                writeln!(err, "apply by hand: add {value} to {MODE_KEY} ({MODE_VAR})")?;
            }
        }
    }
    Ok(())
}

fn write_question<E: Write>(question: &QuestionReport<'_>, err: &mut E) -> io::Result<()> {
    let brier = question
        .brier
        .map_or_else(|| "-".to_owned(), |brier| format!("{brier:.4}"));
    let ask_rate = question
        .ask_rate
        .map_or_else(|| "-".to_owned(), |rate| format!("{:.0}%", rate * 100.0));
    let threshold = &question.threshold;
    let proposal = match (threshold.current, threshold.proposed, threshold.no_proposal) {
        (Some(current), Some(proposed), _) if current.total_cmp(&proposed).is_eq() => {
            format!("; threshold {current} kept (nothing cheaper)")
        }
        (Some(current), Some(proposed), _) => format!("; threshold {current} -> {proposed}"),
        (Some(current), None, Some(why)) => {
            format!("; threshold {current}, no proposal ({})", why.as_str())
        }
        _ => String::new(),
    };
    let reasons: Vec<&str> = question
        .reasons
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    let reasons = if reasons.is_empty() {
        String::new()
    } else {
        format!(" ({})", reasons.join(", "))
    };
    writeln!(
        err,
        "  {}: n {}, labelled {}, Brier {brier}, ask rate {ask_rate}{proposal}; {}{reasons}",
        question.question,
        question.n,
        question.labelled,
        question.verdict.as_str(),
    )
}
