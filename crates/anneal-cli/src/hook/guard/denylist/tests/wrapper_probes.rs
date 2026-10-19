//! The probe table of `tests/fixtures/wrappers/probes.txt`: what each wrapper ran on the real
//! binaries (`probe.sh` replaces each word `W1`..`W9` with a recorder), against what the walk
//! reads the same words to run.

use crate::hook::guard::denylist::{Layout, layout};
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

/// Whether the walk reads `command` to run the word `observed`: as its program, or inside the
/// shell text it runs.
fn walk_runs(command: &str, observed: &str) -> Result<(), String> {
    let commands = shell::parse(command).map_err(|error| format!("{error:?}"))?;
    let [simple] = commands.as_slice() else {
        return Err(format!("{} simple commands", commands.len()));
    };
    let (layout, _) = layout(&simple.words);
    match &layout {
        Layout::Program(at) => match simple.words.get(*at) {
            Some(word) if word.text == observed => Ok(()),
            word => Err(format!("the program is {word:?}")),
        },
        Layout::Script(text) => text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == observed)
            .then_some(())
            .ok_or_else(|| format!("the script is {text:?}")),
        other => Err(format!("the layout is {other:?}")),
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
