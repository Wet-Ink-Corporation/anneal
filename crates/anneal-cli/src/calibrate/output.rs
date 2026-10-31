//! What `anneal calibrate` writes: the envelope on stdout, the proposals file, the manual
//! steps that would apply a proposal, and the short summary on stderr.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::baseline;
use crate::calibrate::CalibrateError;
use crate::calibrate::edit::{Change, Edit, EditError};
use crate::calibrate::history::DayScore;
use crate::calibrate::report::{PointReport, QuestionReport, Report, Verdict};
use crate::calibrate::score::{FALSE_ACCEPT_COST, NEEDLESS_COST};
use crate::command::CommandName;
use crate::decide::config::Mode;
use crate::decide::modes::{Modes, ModesError};
use crate::decide::point::{Point, Question};
use crate::envelope::Envelope;
use crate::error_chain::reason;
use crate::exit_codes::{self, Code};

/// The proposals file's `kind`.
pub(crate) const PROPOSALS_KIND: &str = "anneal-threshold-proposals";

/// The proposals file's format version.
pub(crate) const PROPOSALS_VERSION: u32 = 1;

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
    /// The point the step belongs to.
    pub(crate) const fn point(&self) -> Point {
        match self {
            Self::Threshold { point, .. } | Self::Mode { point, .. } => *point,
        }
    }

    /// The text edit a threshold step makes; `None` for a mode step.
    pub(crate) const fn edit(&self) -> Option<Edit<'_>> {
        match self {
            Self::Threshold {
                pointer, from, to, ..
            } => Some(Edit {
                pointer,
                from: *from,
                to: *to,
            }),
            Self::Mode { .. } => None,
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
    /// The selected threshold steps as changed lines of the thresholds text, or why they do
    /// not apply to it.
    pub(crate) diff: &'a Result<Vec<Change>, EditError>,
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
    /// Each point's mode, or why it cannot be read.
    pub(crate) modes: &'a Result<Modes, ModesError>,
    /// Each point's decisions per day.
    pub(crate) history: &'a BTreeMap<Point, Vec<DayScore>>,
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

/// Writes the proposals file atomically, and only when its content would change, so a reader
/// that runs `calibrate` to look (the cockpit's calibration pane) leaves the disk as it was.
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
    baseline::write_snapshot_json_if_changed(calibration.proposals, &file)
        .map_err(CalibrateError::Proposals)
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
    points: Vec<PointOut<'a>>,
    apply: Vec<&'a Step<'a>>,
    /// The selected threshold steps as a line diff of the thresholds file.
    diff: &'a [Change],
    /// Why the steps do not apply to the thresholds text, when they do not.
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_error: Option<String>,
    /// Why no point's mode could be read, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    mode_error: Option<String>,
}

/// One point of the envelope: its report, then what only the envelope carries.
#[derive(Debug, Serialize)]
struct PointOut<'a> {
    #[serde(flatten)]
    report: &'a PointReport<'a>,
    /// The mode in effect, as `anneal status` reports it; `None` when it cannot be read.
    mode: Option<Mode>,
    /// The point's reported questions per day of decisions.
    history: Vec<&'a DayScore>,
}

/// Writes the envelope for the points `selected` lets through, and returns its code.
pub(crate) fn emit<O: Write>(
    calibration: &Calibration<'_>,
    selected: impl Fn(Point) -> bool,
    out: &mut O,
) -> io::Result<Code> {
    let (diff, diff_error) = match calibration.diff {
        Ok(changes) => (changes.as_slice(), None),
        Err(error) => (&[][..], Some(error.to_string())),
    };
    let (modes, mode_error) = match calibration.modes {
        Ok(modes) => (Some(modes), None),
        Err(error) => (None, Some(reason(error))),
    };
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
            .map(|report| PointOut {
                report,
                mode: modes.map(|modes| modes.effective(report.point)),
                history: reported_days(report, calibration.history),
            })
            .collect(),
        apply: calibration
            .steps
            .iter()
            .filter(|step| selected(step.point()))
            .collect(),
        diff,
        diff_error,
        mode_error,
    };
    Envelope::new(CommandName::Calibrate, None, exit_codes::OK, data).emit(out)
}

/// The days of `report`'s point, of the questions it reports (as `points` leaves out a question
/// the thresholds no longer define, so does its history).
fn reported_days<'a>(
    report: &PointReport<'_>,
    history: &'a BTreeMap<Point, Vec<DayScore>>,
) -> Vec<&'a DayScore> {
    history
        .get(&report.point)
        .into_iter()
        .flatten()
        .filter(|day| {
            report
                .questions
                .iter()
                .any(|question| question.question == day.question)
        })
        .collect()
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
    let mut any_threshold_step = false;
    for step in calibration
        .steps
        .iter()
        .filter(|step| selected(step.point()))
    {
        match step {
            Step::Threshold { pointer, to, .. } => {
                any_threshold_step = true;
                writeln!(err, "apply by hand: in {thresholds}, set {pointer} to {to}")?;
            }
            Step::Mode { value, .. } => {
                writeln!(err, "apply by hand: add {value} to {MODE_KEY} ({MODE_VAR})")?;
            }
        }
    }
    if any_threshold_step {
        let file = calibration
            .thresholds
            .map_or_else(|| "<file>".to_owned(), |path| path.display().to_string());
        writeln!(
            err,
            "or, after reviewing data.diff: anneal calibrate --apply --thresholds {file}"
        )?;
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
    // Only a log with labels from a mod gets the count, so other logs' lines stay as they were.
    let via_mod = if question.labelled_via_mod > 0 {
        format!(", via mod {}", question.labelled_via_mod)
    } else {
        String::new()
    };
    writeln!(
        err,
        "  {}: n {}, labelled {}{via_mod}, Brier {brier}, ask rate {ask_rate}{proposal}; {}{reasons}",
        question.question,
        question.n,
        question.labelled,
        question.verdict.as_str(),
    )
}
