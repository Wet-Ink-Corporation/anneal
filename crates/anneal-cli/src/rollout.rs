//! `anneal rollout decide --metrics <json> --bands <file>`: go, hold or rollback for a rollout,
//! judged against band tables rather than by feel.
//!
//! `--metrics` is a JSON object of named numbers (`{"error_rate": 0.002, "p99_ms": 240}`);
//! `--bands` is a JSON file giving each metric's direction and thresholds (see [`bands`] and
//! `docs/rollout.md`). Each banded metric is rated, a threshold belonging to the worse band; a
//! banded metric missing from `--metrics` holds; the worst rating is the decision: exit 0 go,
//! 1 hold, 2 rollback. An unreadable or invalid bands file, or metrics that are not an object
//! of numbers, is a usage error (2 with nothing on stdout).

pub mod bands;

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::json_file::{self, JsonFileError};
use bands::{BandError, Bands, BandsDto, Better, Decision, Rated, Rating};

/// The largest bands file read, in bytes (H-13).
pub const MAX_BANDS_BYTES: u64 = 1024 * 1024;

/// `anneal rollout`.
#[derive(Debug, Args)]
pub struct RolloutArgs {
    #[command(subcommand)]
    action: RolloutAction,
}

/// The `anneal rollout` subcommands.
#[derive(Debug, Subcommand)]
pub enum RolloutAction {
    /// Decide go, hold or rollback from metrics and band tables
    Decide(DecideArgs),
}

/// `anneal rollout decide`.
#[derive(Debug, Args)]
pub struct DecideArgs {
    /// The measured metrics, a JSON object of numbers, such as `{"error_rate": 0.002}`
    #[arg(long, value_name = "JSON")]
    metrics: String,
    /// The bands file: each metric's direction, hold and rollback thresholds
    #[arg(long, value_name = "FILE")]
    bands: PathBuf,
}

/// Why `anneal rollout decide` could not judge.
#[derive(Debug, thiserror::Error)]
pub enum RolloutError {
    /// The bands file exists but could not be read.
    #[error("rollout: reading bands file {path}", path = path.display())]
    ReadBands {
        /// The file, as given.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
    /// The bands file is missing, not a regular file, too large or not the JSON expected.
    #[error("rollout: bands file {path}", path = path.display())]
    BandsFile {
        /// The file, as given.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: JsonFileError,
    },
    /// The bands file parsed but a band is invalid.
    #[error("rollout: bands file {path}", path = path.display())]
    Bands {
        /// The file, as given.
        path: PathBuf,
        /// What is wrong with it.
        #[source]
        source: BandError,
    },
    /// `--metrics` is not a JSON object of numbers.
    #[error("rollout: --metrics must be a JSON object of numbers")]
    Metrics(#[source] serde_json::Error),
}

impl RolloutError {
    /// The exit code: a file that exists but cannot be read is internal (70); anything else
    /// is a bad argument (2).
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::ReadBands { .. } => exit_codes::INTERNAL,
            Self::BandsFile { .. } | Self::Bands { .. } | Self::Metrics(_) => exit_codes::USAGE,
        }
    }
}

/// Runs `anneal rollout <action>`.
pub(crate) fn run<O: Write>(args: &RolloutArgs, out: &mut O) -> Result<Code, CliError> {
    let RolloutAction::Decide(args) = &args.action;
    let metrics: BTreeMap<String, f64> =
        serde_json::from_str(&args.metrics).map_err(RolloutError::Metrics)?;
    let bands = read_bands(&args.bands)?;
    let decision = Decision::of(&bands, &metrics);
    emit(&decision, out)
}

/// Reads and validates the bands file at `path`.
fn read_bands(path: &Path) -> Result<Bands, RolloutError> {
    // The error owns the path it names.
    let dto: BandsDto = json_file::read(path, MAX_BANDS_BYTES).map_err(|source| match source {
        JsonFileError::Read(source) if source.kind() != io::ErrorKind::NotFound => {
            RolloutError::ReadBands {
                path: path.to_owned(),
                source,
            }
        }
        source => RolloutError::BandsFile {
            path: path.to_owned(),
            source,
        },
    })?;
    Bands::try_from(dto).map_err(|source| RolloutError::Bands {
        path: path.to_owned(),
        source,
    })
}

/// `data` of `rollout decide`.
#[derive(Debug, Serialize)]
struct DecideData<'a> {
    decision: Rating,
    metrics: Vec<&'a Rated<'a>>,
    unbanded: &'a [&'a str],
}

fn emit<O: Write>(decision: &Decision<'_>, out: &mut O) -> Result<Code, CliError> {
    let code = match decision.rating {
        Rating::Go => exit_codes::ROLLOUT_GO,
        Rating::Hold => exit_codes::ROLLOUT_HOLD,
        Rating::Rollback => exit_codes::ROLLOUT_ROLLBACK,
    };
    let findings: Vec<Finding> = decision.rated().filter_map(finding).collect();
    let data = DecideData {
        decision: decision.rating,
        metrics: decision.rated().collect(),
        unbanded: &decision.unbanded,
    };
    Envelope::new(CommandName::Rollout, Some("decide"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// The finding for a metric that is not go.
fn finding(rated: &Rated<'_>) -> Option<Finding> {
    let metric = rated.metric;
    let (key, message) = match (rated.rating, rated.value) {
        (Rating::Go, _) => return None,
        (_, None) => (
            "missing",
            format!(
                "{metric} has a band but no value in --metrics; an unmeasured metric holds. \
                 fix: measure it and decide again."
            ),
        ),
        (Rating::Hold, Some(value)) => (
            "hold",
            format!(
                "{metric} is {value}, at or past its hold threshold {} ({}). fix: stop \
                 widening the rollout until it recovers.",
                rated.band.hold(),
                direction(rated.band.better()),
            ),
        ),
        (Rating::Rollback, Some(value)) => (
            "rollback",
            format!(
                "{metric} is {value}, at or past its rollback threshold {} ({}). fix: run the \
                 rollback plan.",
                rated.band.rollback(),
                direction(rated.band.better()),
            ),
        ),
    };
    Some(Finding::new(format!("rollout/{key}:{metric}"), message))
}

const fn direction(better: Better) -> &'static str {
    match better {
        Better::Lower => "lower is better",
        Better::Higher => "higher is better",
    }
}
