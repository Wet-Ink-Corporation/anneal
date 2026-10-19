//! The probe table of `tests/fixtures/wrappers/probes.txt`: what each wrapper ran on the real
//! binaries (`probe.sh` replaces each word `W1`..`W9` with a recorder), against what the walk
//! reads the same words to run.

use crate::hook::guard::denylist::wrappers::{Program, Run, unwrap};
use crate::hook::guard::shell;

const PROBES: &str = include_str!("../../../../../tests/fixtures/wrappers/probes.txt");

/// Each row the binaries ran: the word that ran, and the command.
fn ran() -> Vec<(&'static str, &'static str)> {
    PROBES
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_once('\t').expect("a tab after the observation"))
        .filter(|(observed, _)| *observed != "-")
        .collect()
}

/// Whether the walk reads `command` to run the word `observed`: as the program of one of its
/// simple commands, or of one in the shell text it runs.
fn walk_runs(command: &str, observed: &str) -> Result<(), String> {
    let commands = shell::parse(command).map_err(|error| format!("{error:?}"))?;
    let [simple] = commands.as_slice() else {
        return Err(format!("{} simple commands", commands.len()));
    };
    let run = unwrap(&simple.words);
    runs(&run, observed)
        .then_some(())
        .ok_or_else(|| format!("the walk runs {run:?}"))
}

/// Whether `run` has `observed` as a program, in itself or in the shell text it runs.
fn runs(run: &Run<'_>, observed: &str) -> bool {
    match run {
        Run::Command {
            program: Program::Named(name),
            ..
        } => *name == observed,
        Run::Script(text) | Run::Split(text) => shell::parse(text).is_ok_and(|commands| {
            commands
                .iter()
                .any(|simple| runs(&unwrap(&simple.words), observed))
        }),
        Run::Either(readings) => readings.iter().any(|reading| runs(reading, observed)),
        _ => false,
    }
}

#[test]
fn every_probed_wrapper_runs_what_the_binary_ran() {
    let rows = ran();
    assert!(rows.len() > 100, "only {} rows ran", rows.len());
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|(observed, command)| {
            walk_runs(command, observed)
                .err()
                .map(|why| format!("{command:?} ran {observed}, but {why}"))
        })
        .collect();
    assert!(wrong.is_empty(), "misread:\n{}", wrong.join("\n"));
}
