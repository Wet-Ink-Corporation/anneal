//! `anneal perf sample|noise`: tell a performance change from run-to-run noise.
//!
//! `sample -n <N> --out <file> -- <cmd>` runs the command `N` times and saves the run times
//! (the baseline, measured before the change). `noise -n <N> --baseline <file> -- <cmd>` runs
//! it `N` times again (the candidate, after the change) and compares the two with Welch's 95%
//! interval on the difference of means ([`stats`]): the change is kept (exit 0) only when the
//! whole interval is below zero, so it is faster beyond the noise; a neutral interval (holding
//! zero) or a slower one reverts (exit 1). `N` is 2 to 100 (default 10); each run has
//! `--timeout` seconds (default 300, at most 3600). Runs go through the `runner::Runner`
//! port, so tests never time a real process. See `docs/perf.md`.

pub mod runner;
pub mod stats;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::command::CommandName;
use crate::envelope::{Envelope, Finding};
use crate::error::CliError;
use crate::exit_codes::{self, Code};
use crate::json_file::{self, JsonFileError};
use runner::{RunError, Runner};
use stats::{CONFIDENCE, Comparison, Samples, SamplesError, Verdict};

/// The largest baseline file read, in bytes (H-13).
pub const MAX_SAMPLE_FILE_BYTES: u64 = 1024 * 1024;

/// The sample file format this build writes and reads.
const SAMPLE_FILE_VERSION: u32 = 1;

/// `anneal perf`.
#[derive(Debug, Args)]
pub struct PerfArgs {
    #[command(subcommand)]
    action: PerfAction,
}

/// The `anneal perf` subcommands.
#[derive(Debug, Subcommand)]
pub enum PerfAction {
    /// Run the command after the change and keep it only if it is faster beyond the noise
    Noise(NoiseArgs),
    /// Run the command before the change and save its run times as the baseline
    Sample(SampleArgs),
}

/// `anneal perf noise`.
#[derive(Debug, Args)]
pub struct NoiseArgs {
    /// The baseline's run times, saved by `anneal perf sample`
    #[arg(long, value_name = "FILE")]
    baseline: PathBuf,
    #[command(flatten)]
    run: RunArgs,
}

/// `anneal perf sample`.
#[derive(Debug, Args)]
pub struct SampleArgs {
    /// Where to save the run times
    #[arg(long, value_name = "FILE")]
    out: PathBuf,
    #[command(flatten)]
    run: RunArgs,
}

/// How the command is run.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// How many times to run the command (2 to 100)
    #[arg(
        short = 'n',
        long = "runs",
        value_name = "N",
        default_value_t = 10,
        value_parser = clap::value_parser!(u32).range(2..=100)
    )]
    runs: u32,
    /// Kill a run after this many seconds (1 to 3600)
    #[arg(
        long,
        value_name = "SECS",
        default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(1..=3600)
    )]
    timeout: u64,
    /// The command and its arguments, after `--`; run as given, without a shell
    #[arg(last = true, required = true, value_name = "COMMAND")]
    command: Vec<OsString>,
}

/// Why `anneal perf` could not measure or compare.
#[derive(Debug, thiserror::Error)]
pub enum PerfError {
    /// The baseline file exists but could not be read.
    #[error("perf: reading baseline {path}", path = path.display())]
    ReadBaseline {
        /// The file, as given.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
    /// The baseline file is missing, not a regular file, too large or not a sample file.
    #[error("perf: baseline {path}", path = path.display())]
    BaselineFile {
        /// The file, as given.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: JsonFileError,
    },
    /// The baseline file is from another format version.
    #[error(
        "perf: baseline {path} is sample file version {found}, this anneal reads \
         {SAMPLE_FILE_VERSION}",
        path = path.display()
    )]
    BaselineVersion {
        /// The file, as given.
        path: PathBuf,
        /// The version it holds.
        found: u32,
    },
    /// The baseline's run times are invalid.
    #[error("perf: baseline {path}", path = path.display())]
    BaselineSamples {
        /// The file, as given.
        path: PathBuf,
        /// What is wrong with them.
        #[source]
        source: SamplesError,
    },
    /// The command could not be started.
    #[error("perf")]
    NotStarted(#[source] RunError),
    /// A run failed, timed out or could not be waited on.
    #[error("perf: run {run} of {runs}")]
    Run {
        /// The run, from 1.
        run: u32,
        /// How many were asked for.
        runs: u32,
        /// What went wrong.
        #[source]
        source: RunError,
    },
    /// The measured run times are not a valid sample.
    #[error("perf: the measured runs")]
    Measured(#[source] SamplesError),
    /// The sample file could not be written.
    #[error("perf: writing {path}", path = path.display())]
    Write {
        /// The file, as given.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
}

impl PerfError {
    /// The exit code: a bad argument (a missing or invalid baseline, a command that cannot
    /// start) is 2; a failed run, an unreadable file or a failed write is 70.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            Self::BaselineFile { .. }
            | Self::BaselineVersion { .. }
            | Self::BaselineSamples { .. }
            | Self::NotStarted(_) => exit_codes::USAGE,
            Self::ReadBaseline { .. }
            | Self::Run { .. }
            | Self::Measured(_)
            | Self::Write { .. } => exit_codes::INTERNAL,
        }
    }
}

/// A sample file as read: the run times of one command.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleFile {
    version: u32,
    /// The command, as given (lossy UTF-8).
    command: Vec<String>,
    samples_secs: Vec<f64>,
}

/// A sample file as written, borrowing what it holds.
#[derive(Debug, Serialize)]
struct SampleFileOut<'a> {
    version: u32,
    command: &'a [String],
    samples_secs: &'a [f64],
}

/// Runs `anneal perf <action>`, running the command through `runner`.
pub(crate) fn run<R: Runner, O: Write>(
    args: &PerfArgs,
    runner: &mut R,
    out: &mut O,
) -> Result<Code, CliError> {
    match &args.action {
        PerfAction::Sample(args) => sample(args, runner, out),
        PerfAction::Noise(args) => noise(args, runner, out),
    }
}

/// `perf sample`: measure, save, report.
fn sample<R: Runner, O: Write>(
    args: &SampleArgs,
    runner: &mut R,
    out: &mut O,
) -> Result<Code, CliError> {
    let samples = measure(runner, &args.run)?;
    let command: Vec<String> = args
        .run
        .command
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    let file = SampleFileOut {
        version: SAMPLE_FILE_VERSION,
        command: &command,
        samples_secs: samples.secs(),
    };
    write_sample_file(&args.out, &file)?;
    let data = SampleData {
        out: &args.out,
        runs: samples.len(),
        mean_secs: samples.mean(),
        samples_secs: samples.secs(),
    };
    Envelope::new(
        CommandName::Perf,
        Some("sample"),
        exit_codes::PERF_KEEP,
        data,
    )
    .emit(out)
    .map_err(CliError::Output)
}

/// `perf noise`: read the baseline, measure, compare, report.
fn noise<R: Runner, O: Write>(
    args: &NoiseArgs,
    runner: &mut R,
    out: &mut O,
) -> Result<Code, CliError> {
    let (baseline_command, baseline) = read_baseline(&args.baseline)?;
    let candidate = measure(runner, &args.run)?;
    let comparison = Comparison::of(&baseline, &candidate);
    let keeps = comparison.verdict.keeps();
    let data = NoiseData {
        decision: if keeps { "keep" } else { "revert" },
        verdict: comparison.verdict,
        confidence: CONFIDENCE,
        delta_secs: comparison.delta,
        low_secs: comparison.low,
        high_secs: comparison.high,
        df: comparison.df,
        t: comparison.t,
        baseline: BaselineSummary {
            file: &args.baseline,
            command: &baseline_command,
            runs: baseline.len(),
            mean_secs: baseline.mean(),
        },
        candidate: CandidateSummary {
            runs: candidate.len(),
            mean_secs: candidate.mean(),
            samples_secs: candidate.secs(),
        },
    };
    let (code, findings) = if keeps {
        (exit_codes::PERF_KEEP, Vec::new())
    } else {
        (exit_codes::PERF_REVERT, vec![revert_finding(&comparison)])
    };
    Envelope::new(CommandName::Perf, Some("noise"), code, data)
        .with_findings(&findings)
        .emit(out)
        .map_err(CliError::Output)
}

/// Runs the command `args.runs` times and returns the run times.
fn measure<R: Runner>(runner: &mut R, args: &RunArgs) -> Result<Samples, PerfError> {
    let timeout = Duration::from_secs(args.timeout);
    let secs = (1..=args.runs)
        .map(|run| {
            runner
                .run(&args.command, timeout)
                .map(|elapsed| elapsed.as_secs_f64())
                .map_err(|source| match source {
                    RunError::NotStarted { .. } => PerfError::NotStarted(source),
                    source => PerfError::Run {
                        run,
                        runs: args.runs,
                        source,
                    },
                })
        })
        .collect::<Result<Vec<f64>, PerfError>>()?;
    Samples::new(secs).map_err(PerfError::Measured)
}

/// Reads the baseline at `path`: its command and its validated run times.
fn read_baseline(path: &Path) -> Result<(Vec<String>, Samples), PerfError> {
    // Each error owns the path it names.
    let file: SampleFile =
        json_file::read(path, MAX_SAMPLE_FILE_BYTES).map_err(|source| match source {
            JsonFileError::Read(source) if source.kind() != io::ErrorKind::NotFound => {
                PerfError::ReadBaseline {
                    path: path.to_owned(),
                    source,
                }
            }
            source => PerfError::BaselineFile {
                path: path.to_owned(),
                source,
            },
        })?;
    let SampleFile {
        version,
        command,
        samples_secs,
    } = file;
    if version != SAMPLE_FILE_VERSION {
        return Err(PerfError::BaselineVersion {
            path: path.to_owned(),
            found: version,
        });
    }
    let samples = Samples::new(samples_secs).map_err(|source| PerfError::BaselineSamples {
        path: path.to_owned(),
        source,
    })?;
    Ok((command, samples))
}

/// Writes `file` to `path` as one line of JSON.
fn write_sample_file(path: &Path, file: &SampleFileOut<'_>) -> Result<(), PerfError> {
    let mut bytes = serde_json::to_vec(file).map_err(|source| PerfError::Write {
        path: path.to_owned(),
        source: io::Error::other(source),
    })?;
    bytes.push(b'\n');
    std::fs::write(path, bytes).map_err(|source| PerfError::Write {
        path: path.to_owned(),
        source,
    })
}

/// The finding of a reverted change.
fn revert_finding(comparison: &Comparison) -> Finding {
    let Comparison {
        delta, low, high, ..
    } = *comparison;
    let (key, why) = match comparison.verdict {
        Verdict::Slower => ("slower", "slower beyond the noise"),
        Verdict::Neutral | Verdict::Faster => ("neutral", "not distinguishable from noise"),
    };
    Finding::new(
        format!("perf/{key}"),
        format!(
            "the change is {why}: mean delta {delta:.6} s, 95% interval [{low:.6}, {high:.6}] \
             s. fix: revert it, or measure with more runs."
        ),
    )
}

/// `data` of `perf sample`.
#[derive(Debug, Serialize)]
struct SampleData<'a> {
    out: &'a Path,
    runs: usize,
    mean_secs: f64,
    samples_secs: &'a [f64],
}

/// `data` of `perf noise`.
#[derive(Debug, Serialize)]
struct NoiseData<'a> {
    decision: &'static str,
    verdict: Verdict,
    confidence: f64,
    delta_secs: f64,
    low_secs: f64,
    high_secs: f64,
    df: f64,
    t: f64,
    baseline: BaselineSummary<'a>,
    candidate: CandidateSummary<'a>,
}

/// The baseline as shown in `data`.
#[derive(Debug, Serialize)]
struct BaselineSummary<'a> {
    file: &'a Path,
    command: &'a [String],
    runs: usize,
    mean_secs: f64,
}

/// The candidate as shown in `data`.
#[derive(Debug, Serialize)]
struct CandidateSummary<'a> {
    runs: usize,
    mean_secs: f64,
    samples_secs: &'a [f64],
}

#[cfg(test)]
mod tests;
