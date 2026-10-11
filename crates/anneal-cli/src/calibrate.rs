//! `anneal calibrate`: each Clef decision point scored against its labelled outcomes.
//!
//! It reports how well the probabilities match the outcomes labelled with
//! `anneal decide label`, the threshold that would have cost least, and whether the point may
//! leave shadow mode. `calibrate [--point <D1..D7>] [--thresholds <file>]` reads every file of the decision log
//! (`decisions.4.jsonl` .. `decisions.1.jsonl`, then `decisions.jsonl`), joins each answered
//! decision to its latest label, and reports per point and question: n, the labelled n, the
//! Brier score, 10 reliability bins, the ask rate, the lowest band's threshold now and as
//! proposed, and a verdict (`stay-shadow` or `eligible-to-enforce`). It writes every point's
//! report and the manual steps that would apply it to `<anneal home>/decisions/proposals.json`,
//! writes the envelope (for `--point`'s point only, when given) on stdout and a short summary
//! on stderr, and exits 0. It never edits the thresholds or the mode.
//! See `docs/calibrate.md`.
//!
//! Private modules:
//! - `read`: the log's files into rows.
//! - `score`: Brier score, bins, expected cost and the proposed threshold.
//! - `report`: the join, the per-question report and the verdict.
//! - `output`: envelope, proposals file, manual steps and summary.

mod output;
mod read;
mod report;
mod score;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;

use crate::baseline::BaselineError;
use crate::decide::point::Point;
use crate::decide::{self, DecideError, log::DecisionLog};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths;
use output::Calibration;

/// The proposals file inside the decision log's directory.
pub const PROPOSALS_FILE: &str = "proposals.json";

/// `anneal calibrate`.
#[derive(Debug, Args)]
pub struct CalibrateArgs {
    /// Report only this decision point (proposals.json still holds every point)
    #[arg(long, value_enum)]
    point: Option<Point>,
    /// The thresholds file [default: $CLAUDE_PLUGIN_ROOT/playbooks/clef-thresholds.json, else
    /// the copy built in]
    #[arg(long, value_name = "FILE")]
    thresholds: Option<PathBuf>,
}

/// What `anneal calibrate` reads from outside: the working directory and the environment.
#[derive(Debug)]
pub(crate) struct CalibrateEnv<'a, L> {
    /// The working directory, for a relative `--thresholds`.
    pub(crate) cwd: &'a Path,
    /// Reads one environment variable.
    pub(crate) lookup: L,
}

/// Why `anneal calibrate` could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum CalibrateError {
    /// A decision log file could not be read.
    #[error("reading the decision log")]
    Read(#[source] BaselineError),
    /// The thresholds file could not be read.
    #[error("reading the thresholds")]
    Thresholds(#[source] DecideError),
    /// The proposals file could not be written.
    #[error("writing the proposals")]
    Proposals(#[source] BaselineError),
}

impl CalibrateError {
    /// The exit code: 70 for each, as the log, thresholds and proposals are internal state.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::Read(_) | Self::Thresholds(_) | Self::Proposals(_) => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal calibrate` with `env`: the proposals file, then the envelope on `out` (flushed),
/// then the summary on `err`, so the summary is written only for a run that succeeded.
pub(crate) fn run<O, E, L>(
    args: &CalibrateArgs,
    env: &CalibrateEnv<'_, L>,
    out: &mut O,
    err: &mut E,
) -> Result<Code, CliError>
where
    O: Write,
    E: Write,
    L: Fn(&str) -> Option<OsString>,
{
    let home = paths::anneal_home_from(&env.lookup)?;
    let log = DecisionLog::in_home(&home);
    let thresholds_file = decide::thresholds_path(args.thresholds.as_deref(), env.cwd, &env.lookup);
    let thresholds =
        decide::read_thresholds(thresholds_file.as_deref()).map_err(CalibrateError::Thresholds)?;
    let rows = read::read_log(&log)?;
    let report = report::report(&thresholds, &rows);
    let steps = output::steps(&report, thresholds_file.as_deref());
    let live = log.path();
    let proposals = log.dir().join(PROPOSALS_FILE);
    let calibration = Calibration {
        report: &report,
        steps: &steps,
        log: &live,
        files: &rows.files,
        skipped_rows: rows.skipped_rows,
        proposals: &proposals,
        thresholds: thresholds_file.as_deref(),
    };
    output::write_proposals(&calibration)?;
    let selected = |point: Point| args.point.is_none_or(|only| only == point);
    let code = output::emit(&calibration, selected, out).map_err(CliError::Output)?;
    out.flush().map_err(CliError::Output)?;
    output::write_summary(&calibration, selected, err).map_err(CliError::Output)?;
    Ok(code)
}

#[cfg(test)]
mod tests;
