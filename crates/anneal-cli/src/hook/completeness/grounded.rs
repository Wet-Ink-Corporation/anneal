//! Clef D4 in the Stop hook: is each required section of the final report present and grounded?
//!
//! Asked only after the deterministic checks, only in a worktree, and only for a reply holding a
//! status block (its last `Status:` line carries a known status word): one
//! `noul present_and_grounded` question per required section of [`block::SECTIONS`] (an absent
//! section is asked with empty text, so Clef judges presence too), all at once on scoped threads, through `decide`'s own code (`decide::in_process`, then
//! `client::ask_recording`), so the thresholds, model (`clef`), log, modes and shadow count are
//! the command's. Clef gets the section's text and the worktree's newest ledger events, each
//! through `safe_prompt`; the rows keep no input.
//!
//! Only in effect `enforce`, and only for an answer whose row was logged, does a section whose
//! action is `block` (P(yes) below the `pass` band, 0.50 by default) become a hard gate. Shadow,
//! off, a malformed configuration and Clef's own misses (unreachable, timed out, a bad response)
//! leave the hook's output as it would be without D4; D4's other failures are notes only when D4
//! is configured `enforce`. Clef is asked only while its whole timeout fits in [`STOP_BUDGET`].

use std::ffi::OsString;
use std::thread;
use std::time::Duration;

use anneal_ledger::{Filter, LedgerTag, Store, TagKey};
use serde_json::json;

use crate::decide::client::{self, Ask, Asked, Outcome, Recorded};
use crate::decide::clock::Clock;
use crate::decide::config::{ClefConfig, Mode};
use crate::decide::in_process::{self, Prepared, Status};
use crate::decide::point::{Point, Question};
use crate::decide::redact::safe_prompt;
use crate::decide::state::CompactState;
use crate::decide::transport::Transport;
use crate::error_chain::reason;
use crate::hook::handoff::facts::event_word;
use crate::hook::prompt::Budget;
use crate::hook::status::block;

/// How long after the Stop hook's start Clef's whole timeout must still fit for D4 to be asked:
/// the hook's `timeout` is 30 s, and the rest is left for the block count, the ledger record and
/// the output.
pub(super) const STOP_BUDGET: Duration = Duration::from_secs(25);

/// The action of D4's rule that blocks (`clef-thresholds.json`).
const BLOCK: &str = "block";

/// How many of the worktree's newest ledger events Clef gets as evidence (H-13).
const EVIDENCE_EVENTS: usize = 8;

/// What D4 judges: the final reply, and what the deterministic checks saw.
#[derive(Debug, Clone, Copy)]
pub(super) struct Report<'a> {
    /// The final assistant message.
    pub(super) reply: &'a str,
    /// The worktree's ledger, when one exists.
    pub(super) store: Option<&'a Store>,
    /// The worktree key.
    pub(super) worktree: &'a str,
    /// The ids of the hard gates the deterministic checks found.
    pub(super) gates: &'a [&'static str],
}

/// What D4 adds to the hook's findings.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Judged {
    /// The hard gate's line, when a section blocks in effect enforce.
    pub(super) block: Option<String>,
    /// What could not be done, for the hook's `ERROR` line.
    pub(super) notes: Vec<String>,
}

/// The time and the means D4 asks with.
#[derive(Debug)]
pub(super) struct Clef<'h, T, C> {
    /// The port to Workers AI.
    pub(super) transport: &'h T,
    /// The clock the calls and the budget are measured on.
    pub(super) clock: &'h C,
    /// The hook's budget, started when the hook did.
    pub(super) budget: Budget,
}

/// Asks D4 about `report` and returns what the hook adds: see the module documentation.
pub(super) fn judge<T, C>(
    report: &Report<'_>,
    lookup: impl Fn(&str) -> Option<OsString>,
    clef: &Clef<'_, T, C>,
) -> Judged
where
    T: Transport + Sync,
    C: Clock + Sync,
{
    let Some(sections) = block::section_texts(report.reply) else {
        return Judged::default();
    };
    match in_process::status(Point::D4, &lookup) {
        Status::On => {}
        // Off asks and touches nothing; a malformed configuration has no mode to report under.
        Status::Off | Status::Misconfigured(_) => return Judged::default(),
    }
    let enforcing =
        ClefConfig::from_env(&lookup).is_ok_and(|config| config.mode(Point::D4) == Mode::Enforce);
    let mut judged = ask(report, &sections, &lookup, clef);
    if !enforcing {
        judged.notes.clear();
    }
    judged
}

/// Asks every section: the block line, if any section blocks, and what failed.
fn ask<T, C>(
    report: &Report<'_>,
    sections: &[(&'static str, String)],
    lookup: impl Fn(&str) -> Option<OsString>,
    clef: &Clef<'_, T, C>,
) -> Judged
where
    T: Transport + Sync,
    C: Clock + Sync,
{
    let mut notes = Vec::new();
    let prepared = match Prepared::load(&lookup) {
        Ok(prepared) => prepared,
        Err(error) => {
            notes.push(format!("D4: {}", reason(&error)));
            return Judged { block: None, notes };
        }
    };
    let evidence = match evidence(report.store, report.worktree) {
        Ok(evidence) => evidence,
        Err(why) => {
            notes.push(format!(
                "D4: the ledger could not be read for evidence: {why}"
            ));
            "unreadable".to_owned()
        }
    };
    let mut states = Vec::with_capacity(sections.len());
    for (section, text) in sections {
        let state = json!({
            "tool": "stop",
            "input": safe_prompt(text),
            "section": section,
            "present": !text.is_empty(),
            "evidence": safe_prompt(&evidence),
            "gates": report.gates,
        });
        match CompactState::parse(&state.to_string()) {
            Ok(state) => states.push((*section, state)),
            Err(error) => notes.push(format!("D4: the {section} state: {}", reason(&error))),
        }
    }
    // Checked last, right before the calls, so the ledger read cannot eat into Clef's time.
    if let Err(note) = clef.budget.fits_clef(clef.clock) {
        notes.push(format!("D4: {note}"));
        return Judged { block: None, notes };
    }
    let below = ask_all(&prepared, &states, clef)
        .into_iter()
        .filter_map(|(section, asked)| blocking(section, asked, &mut notes))
        .collect::<Vec<_>>();
    Judged {
        block: block_line(&below),
        notes,
    }
}

/// Asks D4 about each of `states` at once, one scoped thread per section, all joined before
/// this returns (ASY-10, ASY-15); a failure to ask is its note's text.
fn ask_all<'c, T, C>(
    prepared: &'c Prepared,
    states: &[(&'static str, CompactState)],
    clef: &Clef<'_, T, C>,
) -> Vec<(&'static str, Result<Asked<'c>, String>)>
where
    T: Transport + Sync,
    C: Clock + Sync,
{
    let setup = prepared.setup();
    let &Clef {
        transport, clock, ..
    } = clef;
    thread::scope(|scope| {
        // Every thread starts before any is joined, so the calls share one timeout's time.
        let mut started = Vec::with_capacity(states.len());
        for (index, (section, state)) in states.iter().enumerate() {
            let recorded = Recorded {
                ask: Ask {
                    point: Point::D4,
                    question: Question::Noul,
                    options: &[],
                    state,
                },
                // The rows keep no input: the section's text goes to Clef, only its hash to
                // the log.
                input: None,
            };
            let spawned = thread::Builder::new()
                .name(format!("anneal-d4-{index}"))
                .spawn_scoped(scope, move || {
                    client::ask_recording(&recorded, &setup, transport, clock)
                });
            started.push((*section, spawned));
        }
        started
            .into_iter()
            .map(|(section, spawned)| {
                let asked = match spawned {
                    Ok(handle) => match handle.join() {
                        Ok(asked) => asked.map_err(|error| format!("D4: {}", reason(&error))),
                        // The panic payload has no diagnostic value here (ERR-04).
                        Err(_panic) => Err(format!("D4: the {section} question panicked")),
                    },
                    Err(error) => Err(format!(
                        "D4: the {section} question could not start: {error}"
                    )),
                };
                (section, asked)
            })
            .collect()
    })
}

/// `section` with Clef's P(yes), when its answer blocks: in effect enforce, logged, action
/// [`BLOCK`]. A failure to ask or to log is noted and never acted on.
fn blocking(
    section: &'static str,
    asked: Result<Asked<'_>, String>,
    notes: &mut Vec<String>,
) -> Option<String> {
    match asked {
        Ok(Asked {
            outcome: Outcome::Decided(decision),
            unrecorded: None,
        }) => (decision.stamp.mode == Mode::Enforce && decision.action == BLOCK)
            .then(|| format!("{section} ({:.2})", decision.answer.probability())),
        // Clef's own misses and an off point are silent: D4 fails open.
        Ok(Asked {
            outcome: Outcome::Off(_) | Outcome::NoAnswer(_) | Outcome::Misconfigured(_),
            unrecorded: None,
        }) => None,
        Ok(Asked {
            unrecorded: Some(failure),
            ..
        }) => {
            notes.push(format!(
                "D4: the {section} decision was not logged, so it is not acted on: {}",
                reason(&failure)
            ));
            None
        }
        Err(why) => {
            notes.push(why);
            None
        }
    }
}

/// The hard gate's line naming the `below` sections; `None` when there are none.
fn block_line(below: &[String]) -> Option<String> {
    let (verb, noun) = match below {
        [] => return None,
        [_] => ("is", "section"),
        _ => ("are", "sections"),
    };
    Some(format!(
        "Clef D4 finds the report's {} {noun} {verb} not present and grounded in evidence (P(yes) \
         in brackets, below 0.50); give each the evidence it rests on (the green `test_run`, the \
         gate result, the review verdict) or say plainly what was not verified",
        below.join(", ")
    ))
}

/// The worktree's newest [`EVIDENCE_EVENTS`] ledger events, oldest first: `<type> <subject>`
/// each.
fn evidence(store: Option<&Store>, worktree: &str) -> Result<String, String> {
    let Some(store) = store else {
        return Ok("no ledger".to_owned());
    };
    // A key the ledger cannot tag has no events: `task done` and `mode` refuse such a worktree.
    let Ok(tag) = LedgerTag::new(TagKey::Worktree, worktree) else {
        return Ok("no events".to_owned());
    };
    let filter = Filter {
        types: Vec::new(),
        tags: vec![tag],
    };
    let events = store
        .tail(&filter, EVIDENCE_EVENTS)
        .map_err(|error| reason(&error))?;
    if events.is_empty() {
        return Ok("no events".to_owned());
    }
    Ok(events.iter().map(event_word).collect::<Vec<_>>().join("; "))
}

#[cfg(test)]
mod tests;
