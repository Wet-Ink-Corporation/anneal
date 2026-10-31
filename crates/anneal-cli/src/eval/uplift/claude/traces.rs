//! Keeping each run's trace (`--keep-results`). `claude plugin eval --keep-temp` leaves a
//! run's `trace.jsonl` in the run's sandbox root, below the child's `TMPDIR` in the workspace,
//! which goes when the whole run ends; the result names it as the run's `tracePath`.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use super::{Unkept, batch_label};
use crate::eval::real::result::ResultDto;
use crate::eval::uplift::drive::Batch;

/// One run's trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Trace {
    /// Its name below the `--keep-results` directory ([`trace_name`]).
    pub(super) name: String,
    /// Where the result says it was written.
    pub(super) path: String,
}

/// The traces of the runs of `batch`'s cases in `result`, each case's runs numbered from 1 in
/// the result's order; a run naming no trace, or a case the batch did not ask for, has none.
/// They are owned because the result goes on to be judged, which consumes it.
pub(super) fn traces(result: &ResultDto, batch: &Batch<'_>) -> Vec<Trace> {
    let asked: BTreeSet<String> = batch.cases.iter().map(|case| case.name()).collect();
    result
        .runs()
        .filter(|(case, _)| asked.contains(*case))
        .flat_map(|(case, runs)| {
            runs.iter().zip(1_u32..).filter_map(move |(run, number)| {
                run.trace_path().map(|path| Trace {
                    name: trace_name(batch, case, number),
                    path: path.to_owned(),
                })
            })
        })
        .collect()
}

/// Where `--keep-results` keeps run `run` (from 1) of `case` in `batch`:
/// `traces/<batch>-<arm>/<case>-<run>.jsonl`, the arm as in the batch's result's name.
pub(super) fn trace_name(batch: &Batch<'_>, case: &str, run: u32) -> String {
    format!("traces/{}/{case}-{run}.jsonl", batch_label(batch))
}

/// Copies each of `traces` that is inside the workspace at `root` into `dir`, overwriting a
/// file of its name; one the result places outside the workspace is not this run's to copy
/// and is skipped. Each trace that could not be copied comes back with why.
pub(super) fn keep(traces: &[Trace], root: &Path, dir: &Path) -> Vec<Unkept> {
    // `claude` names its paths resolved, so they are compared with the resolved root.
    let root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) => return traces.iter().map(|trace| unkept(trace, &error)).collect(),
    };
    traces
        .iter()
        .filter_map(|trace| copy(trace, &root, dir).err().map(|e| unkept(trace, &e)))
        .collect()
}

/// Copies `trace` into `dir` when its path, as named and once resolved, is inside `root`.
fn copy(trace: &Trace, root: &Path, dir: &Path) -> io::Result<()> {
    let named = root.join(&trace.path);
    if !named.starts_with(root) {
        return Ok(());
    }
    let resolved = fs::canonicalize(&named)?;
    if !resolved.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} resolves outside the workspace", trace.path),
        ));
    }
    let target = dir.join(&trace.name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&resolved, &target).map(drop)
}

/// The [`Unkept`] of `trace`, which could not be copied for `error`.
fn unkept(trace: &Trace, error: &io::Error) -> Unkept {
    Unkept {
        // The finding outlives the batch, so it owns the name.
        name: trace.name.clone(),
        reason: format!("copying {}: {error}", trace.path),
    }
}
