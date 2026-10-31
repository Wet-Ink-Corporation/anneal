//! `anneal calibrate`: each Clef decision point scored against its labelled outcomes.
//!
//! It reports how well the probabilities match the outcomes labelled with
//! `anneal decide label`, the threshold that would have cost least, and whether the point may
//! leave shadow mode. `calibrate [--point <D1..D7>] [--thresholds <file>]` reads every file of the decision log
//! (`decisions.4.jsonl` .. `decisions.1.jsonl`, then `decisions.jsonl`), joins each answered
//! decision to its latest label, and reports per point and question: n, the labelled n, the
//! Brier score, 10 reliability bins, the ask rate, the lowest band's threshold now and as
//! proposed, and a verdict (`stay-shadow` or `eligible-to-enforce`). It writes every point's
//! report and the manual steps that would apply it to `<anneal home>/decisions/proposals.json`
//! (only when those bytes would change), writes the envelope (for `--point`'s point only, when
//! given; each point with its mode in effect and its per-day history) on stdout and a short
//! summary on stderr, and exits 0. Its `data.diff` shows each proposed threshold as the changed
//! line of the thresholds file. It never edits the thresholds or the mode: only
//! `calibrate --apply --thresholds <file>` writes a thresholds file, from the last run's
//! proposals, and never the mode. See `docs/calibrate.md`.
//!
//! Private modules:
//! - `read`: the log's files into rows.
//! - `score`: Brier score, bins, expected cost and the proposed threshold.
//! - `report`: the join, the per-question report and the verdict.
//! - `history`: each question's decisions per UTC day, for a trend.
//! - `output`: envelope, proposals file, manual steps and summary.
//! - `edit`: a threshold step applied to the file's text, number by number (pure).
//! - `apply`: `--apply`, the proposals file's threshold steps written into `--thresholds`.

mod apply;
mod edit;
mod history;
mod output;
mod read;
mod report;
mod score;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;

use crate::baseline::BaselineError;
use crate::decide::modes::Modes;
use crate::decide::point::Point;
use crate::decide::{self, DecideError, log::DecisionLog};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::paths;
use output::Calibration;

/// The proposals file inside the decision log's directory.
pub(crate) const PROPOSALS_FILE: &str = "proposals.json";

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
    /// Write the last run's proposed thresholds (proposals.json, its `data.diff`) into
    /// --thresholds, which must be given; the only way calibrate writes a thresholds file
    #[arg(long, requires = "thresholds")]
    apply: bool,
}

impl CalibrateArgs {
    /// Whether `point` is reported (and applied): every point without `--point`.
    fn selects(&self, point: Point) -> bool {
        self.point.is_none_or(|only| only == point)
    }
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
    /// A decision log file is larger than any `decide` writes.
    #[error("{path} is larger than {cap} bytes", path = path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// The most read.
        cap: u64,
    },
    /// The thresholds file could not be read.
    #[error("reading the thresholds")]
    Thresholds(#[source] DecideError),
    /// The proposals file could not be written.
    #[error("writing the proposals")]
    Proposals(#[source] BaselineError),
    /// `--apply` found no proposals file: no run has proposed anything yet.
    #[error("no proposals at {path}: run `anneal calibrate` first", path = path.display())]
    NoProposals {
        /// Where the proposals file should be.
        path: PathBuf,
    },
    /// `--apply` could not read the proposals file.
    #[error("reading the proposals")]
    ProposalsUnreadable(#[source] BaselineError),
    /// The proposals file is not one `calibrate` wrote in this format.
    #[error("{path} is not an anneal threshold proposals file of version 1", path = path.display())]
    NotProposals {
        /// The file.
        path: PathBuf,
        /// Why it does not parse, when it does not.
        #[source]
        source: Option<serde_json::Error>,
    },
    /// `--apply` could not write the thresholds file.
    #[error("writing the thresholds")]
    WriteThresholds(#[source] BaselineError),
}

impl CalibrateError {
    /// The exit code: a missing proposals file is a usage error (run `calibrate` first); the
    /// rest are 70, as the log, thresholds and proposals are internal state.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::NoProposals { .. } => exit_codes::USAGE,
            Self::Read(_)
            | Self::TooLarge { .. }
            | Self::Thresholds(_)
            | Self::Proposals(_)
            | Self::ProposalsUnreadable(_)
            | Self::NotProposals { .. }
            | Self::WriteThresholds(_) => exit_codes::INTERNAL,
        }
    }
}

/// Runs `anneal calibrate` with `env`: the proposals file, then the envelope on `out` (flushed),
/// then the summary on `err` (flushed), so the summary is written only for a run that
/// succeeded. The summary is best effort: a summary stderr refuses leaves the exit code 0.
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
    let selected = |point: Point| args.selects(point);
    if args.apply {
        return apply::run(args, &log, env.cwd, out);
    }
    let thresholds_file = decide::thresholds_path(args.thresholds.as_deref(), env.cwd, &env.lookup);
    let (thresholds, text) = decide::read_thresholds_text(thresholds_file.as_deref())
        .map_err(CalibrateError::Thresholds)?;
    let rows = read::read_log(&log)?;
    let report = report::report(&thresholds, &rows);
    // A mode that cannot be read is reported in the envelope, never fatal: the scores stand.
    let modes = Modes::read(&log, &env.lookup);
    let history = history::history(&rows);
    let steps = output::steps(&report, thresholds_file.as_deref());
    let edits: Vec<_> = steps
        .iter()
        .filter(|step| selected(step.point()))
        .filter_map(output::Step::edit)
        .collect();
    let diff = edit::apply(&text, &edits).map(|edited| edited.changes);
    let live = log.path();
    let proposals = log.dir().join(PROPOSALS_FILE);
    let calibration = Calibration {
        report: &report,
        steps: &steps,
        diff: &diff,
        log: &live,
        files: &rows.files,
        skipped_rows: rows.skipped_rows,
        proposals: &proposals,
        thresholds: thresholds_file.as_deref(),
        modes: &modes,
        history: &history,
    };
    output::write_proposals(&calibration)?;
    let code = output::emit(&calibration, selected, out).map_err(CliError::Output)?;
    out.flush().map_err(CliError::Output)?;
    let summary =
        output::write_summary(&calibration, selected, &mut *err).and_then(|()| err.flush());
    match summary {
        Ok(()) => Ok(code),
        // Best effort, by design (docs/calibrate.md): the summary repeats the envelope, which
        // is already on stdout with `ok: true`, so the exit code stays the envelope's. Nothing
        // is left to carry this failure: stderr is the stream that refused it, and stdout
        // holds exactly one envelope.
        Err(_unreportable) => Ok(code),
    }
}

#[cfg(test)]
mod tests;
