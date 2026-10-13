//! `anneal calibrate --apply --thresholds <file>`: the threshold steps of the last run's
//! proposals file written into the thresholds file, number by number.
//!
//! Nothing is written unless every selected step applies: each pointer still holds the step's
//! `from`, the edited text reads as strict thresholds, and each pointer reads back its `to`.
//! Otherwise the run exits [`exit_codes::CALIBRATE_REFUSED`] with one finding. Mode steps are
//! never applied (the mode is the user's plugin setting); they are listed in
//! `data.not_applied`.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::baseline::{self, BaselineError};
use crate::calibrate::edit::{self, Change, Edit, EditError, Edited};
use crate::calibrate::output::{PROPOSALS_KIND, PROPOSALS_VERSION};
use crate::calibrate::{CalibrateArgs, CalibrateError, PROPOSALS_FILE};
use crate::command::CommandName;
use crate::decide::log::DecisionLog;
use crate::decide::point::{Point, Question};
use crate::decide::thresholds::{Thresholds, ThresholdsError};
use crate::decide::{self};
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::error_chain;
use crate::exit_codes::{self, Code};

/// The parts of `proposals.json` that `--apply` reads.
#[derive(Debug, Deserialize)]
struct ProposalsDto {
    kind: String,
    version: u32,
    apply: Vec<StepDto>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StepDto {
    Threshold(ThresholdStep),
    Mode(ModeStep),
}

#[derive(Debug, Deserialize)]
struct ThresholdStep {
    point: Point,
    question: Question,
    pointer: String,
    from: f64,
    to: f64,
}

#[derive(Debug, Deserialize)]
struct ModeStep {
    point: Point,
    value: String,
}

/// One applied step, with its line as the diff shows it.
#[derive(Debug, Serialize)]
struct Applied<'a> {
    point: Point,
    question: Question,
    pointer: &'a str,
    from: f64,
    to: f64,
    line: usize,
    before: &'a str,
    after: &'a str,
}

/// `data` of `calibrate --apply`.
#[derive(Debug, Serialize)]
struct Data<'a> {
    proposals: &'a Path,
    thresholds: &'a Path,
    written: bool,
    applied: Vec<Applied<'a>>,
    not_applied: Vec<&'a str>,
}

/// Why the proposal does not apply to the thresholds file.
#[derive(Debug, thiserror::Error)]
enum Refusal {
    /// A step does not match the text.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// The edited text is not a valid thresholds file.
    #[error("the edited file would not read as thresholds: {reason}")]
    Invalid {
        /// The parse error and its sources, as one line.
        reason: String,
    },
    /// A step's pointer is not under its own point and question.
    #[error("{pointer} is not under /points/{point}/rules/{question}/, the step's own rule")]
    ForeignPointer {
        /// The pointer.
        pointer: String,
        /// The step's point.
        point: Point,
        /// The step's question.
        question: Question,
    },
    /// A pointer does not read back its new value.
    #[error("{pointer} would not read back as {to}")]
    ReadBack {
        /// The pointer.
        pointer: String,
        /// The value written.
        to: f64,
    },
}

impl Refusal {
    fn invalid(error: &ThresholdsError) -> Self {
        Self::Invalid {
            reason: error_chain::reason(error),
        }
    }
}

/// Applies the proposals file's threshold steps for the points `args` selects to its
/// `--thresholds` (relative to `cwd`) and writes the envelope.
///
/// # Errors
/// [`CliError::Usage`] without `--thresholds`; [`CalibrateError`] when the proposals or the
/// thresholds cannot be read or the thresholds cannot be written; [`CliError::Output`] when
/// `out` cannot be written.
pub(crate) fn run<O: Write>(
    args: &CalibrateArgs,
    log: &DecisionLog,
    cwd: &Path,
    out: &mut O,
) -> Result<Code, CliError> {
    let Some(flag) = args.thresholds.as_deref() else {
        return Err(CliError::Usage {
            reason: "--apply needs --thresholds <file>".to_owned(),
        });
    };
    let target = cwd.join(flag);
    let proposals_path = log.dir().join(PROPOSALS_FILE);
    let proposals = read_proposals(&proposals_path)?;
    let (_current, text) =
        decide::read_thresholds_text(Some(&target)).map_err(CalibrateError::Thresholds)?;
    let mut steps = Vec::new();
    let mut not_applied = Vec::new();
    for step in &proposals.apply {
        match step {
            StepDto::Threshold(step) if args.selects(step.point) => steps.push(step),
            StepDto::Mode(step) if args.selects(step.point) => {
                not_applied.push(step.value.as_str());
            }
            StepDto::Threshold(_) | StepDto::Mode(_) => {}
        }
    }
    let edits: Vec<Edit<'_>> = steps
        .iter()
        .map(|step| Edit {
            pointer: &step.pointer,
            from: step.from,
            to: step.to,
        })
        .collect();
    let mut data = Data {
        proposals: &proposals_path,
        thresholds: &target,
        written: false,
        applied: Vec::new(),
        not_applied,
    };
    let edited = match own_rules(&steps)
        .and_then(|()| edit::apply(&text, &edits).map_err(Refusal::from))
        .and_then(|edited| read_back(edited, &edits))
    {
        Ok(edited) => edited,
        Err(refusal) => return refuse(&refusal, &data, out),
    };
    if !edits.is_empty() {
        write_keeping_permissions(&target, &edited.text)?;
        data.written = true;
    }
    data.applied = steps
        .iter()
        .zip(&edited.changes)
        .map(|(step, change)| applied(step, change))
        .collect();
    Envelope::new(CommandName::Calibrate, None, exit_codes::OK, data)
        .emit(out)
        .map_err(CliError::Output)
}

fn applied<'a>(step: &'a ThresholdStep, change: &'a Change) -> Applied<'a> {
    Applied {
        point: step.point,
        question: step.question,
        pointer: &step.pointer,
        from: step.from,
        to: step.to,
        line: change.line,
        before: &change.before,
        after: &change.after,
    }
}

/// Writes `text` over `target` atomically (staged beside it, synced, renamed), giving the new
/// file `target`'s permission bits so a deliberately restricted thresholds file stays so.
fn write_keeping_permissions(target: &Path, text: &str) -> Result<(), CalibrateError> {
    let write = || -> io::Result<()> {
        let permissions = fs::metadata(target)?.permissions();
        let staged = baseline::stage(target, text.as_bytes())?;
        fs::set_permissions(&staged, permissions)
            .and_then(|()| fs::rename(&staged, target))
            .map_err(|error| baseline::discard(&staged, error))
    };
    write().map_err(|source| {
        CalibrateError::WriteThresholds(BaselineError::Io {
            op: "writing",
            // The error owns the path it names.
            path: target.to_path_buf(),
            source,
        })
    })
}

/// The proposals file, which must exist and be one `calibrate` wrote in this format.
fn read_proposals(path: &Path) -> Result<ProposalsDto, CalibrateError> {
    let Some(bytes) =
        baseline::read_state_bytes(path).map_err(CalibrateError::ProposalsUnreadable)?
    else {
        return Err(CalibrateError::NoProposals {
            // The error owns the path it names.
            path: path.to_path_buf(),
        });
    };
    let not_proposals = |source| CalibrateError::NotProposals {
        path: path.to_path_buf(),
        source,
    };
    let file: ProposalsDto =
        serde_json::from_slice(&bytes).map_err(|error| not_proposals(Some(error)))?;
    if file.kind == PROPOSALS_KIND && file.version == PROPOSALS_VERSION {
        Ok(file)
    } else {
        Err(not_proposals(None))
    }
}

/// Every step's pointer lies under its own rule, `/points/<point>/rules/<question>/` (the
/// layout `calibrate::report` writes), since a step is selected by its point and must edit
/// only that point's numbers.
fn own_rules(steps: &[&ThresholdStep]) -> Result<(), Refusal> {
    let foreign = steps.iter().find(|step| {
        let prefix = format!("/points/{}/rules/{}/", step.point, step.question);
        !step.pointer.starts_with(&prefix)
    });
    match foreign {
        Some(step) => Err(Refusal::ForeignPointer {
            // The refusal owns the pointer it names, as `ReadBack` does.
            pointer: step.pointer.clone(),
            point: step.point,
            question: step.question,
        }),
        None => Ok(()),
    }
}

/// The edited text, once it reads as strict thresholds and every pointer holds its `to`.
fn read_back(edited: Edited, edits: &[Edit<'_>]) -> Result<Edited, Refusal> {
    Thresholds::parse(edited.text.as_bytes()).map_err(|error| Refusal::invalid(&error))?;
    let value: Value = serde_json::from_str(&edited.text)
        .map_err(|error| Refusal::invalid(&ThresholdsError::Json(error)))?;
    for edit in edits {
        let holds = value
            .pointer(edit.pointer)
            .and_then(Value::as_f64)
            .is_some_and(|read| read.total_cmp(&edit.to).is_eq());
        if !holds {
            return Err(Refusal::ReadBack {
                pointer: edit.pointer.to_owned(),
                to: edit.to,
            });
        }
    }
    Ok(edited)
}

/// The refusal envelope: exit 3, one finding, nothing written.
fn refuse<O: Write>(refusal: &Refusal, data: &Data<'_>, out: &mut O) -> Result<Code, CliError> {
    let subject = match refusal {
        Refusal::Edit(error) => error.pointer(),
        Refusal::ReadBack { pointer, .. } | Refusal::ForeignPointer { pointer, .. } => pointer,
        Refusal::Invalid { .. } => "thresholds",
    };
    let findings = [Finding::new(
        format!("calibrate/refused:{subject}"),
        format!(
            "{refusal}; nothing was written. Run `anneal calibrate --thresholds {}` again and \
             review its data.diff.",
            data.thresholds.display()
        ),
    )];
    Envelope::new(
        CommandName::Calibrate,
        None,
        exit_codes::CALIBRATE_REFUSED,
        data,
    )
    .with_findings(&findings)
    .emit(out)
    .map_err(CliError::Output)
}
