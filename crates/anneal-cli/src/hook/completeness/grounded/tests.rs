//! Clef D4 with a fake Clef: the 27B model once per section, a block only in effect enforce on
//! a logged answer below the band, and silence in shadow and on Clef's own misses.

use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use super::{Clef, Judged, Report, STOP_BUDGET, judge};
use crate::decide::clock::Clock;
use crate::decide::point::Point;
use crate::decide::transport::{ClefRequest, Transport, TransportError};
use crate::hook::clef_fake::{Fixed, Stepping, TestResult, World, block_the_log, reply};
use crate::hook::prompt::Budget;

/// A final report with every required section.
const REPORT: &str = "Fixed the parser.\n\nStatus: DONE\nGate: fmt ok | test ok (12 passed)\n\
                      Review: approved\nChanged: src/parse.rs (+4/-1)\nDecisions needed: none\n\
                      Not verified: Windows";

/// Answers each section's question with P(yes) by section, recording the model asked.
struct Endpoint {
    /// P(yes) for the Gate section; every other section gets `others`.
    gate: f64,
    others: f64,
    /// Whether Clef is unreachable instead.
    unreachable: bool,
    models: Mutex<Vec<String>>,
}

impl Endpoint {
    fn answering(gate: f64, others: f64) -> Self {
        Self {
            gate,
            others,
            unreachable: false,
            models: Mutex::new(Vec::new()),
        }
    }

    fn unreachable() -> Self {
        Self {
            unreachable: true,
            ..Self::answering(0.0, 0.0)
        }
    }

    fn models(&self) -> Vec<String> {
        // The list is copied out of the lock for the assertion.
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Endpoint {
    fn send(&self, request: &ClefRequest<'_>) -> Result<Vec<u8>, TransportError> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.to_owned());
        if self.unreachable {
            return Err(TransportError::Unreachable {
                reason: "connection refused".to_owned(),
            });
        }
        let body = String::from_utf8_lossy(&request.body);
        let yes = if body.contains(r#"\"section\":\"Gate\""#) {
            self.gate
        } else {
            self.others
        };
        reply(Ok(json!({"probabilities": {"yes": yes, "no": 1.0 - yes}})))
    }
}

/// Runs D4 on `reply` in `world` with `clock`.
fn judged_at(
    world: &World,
    reply: &str,
    endpoint: &Endpoint,
    clock: &(impl Clock + Sync),
) -> Judged {
    let report = Report {
        reply,
        store: None,
        worktree: "wt",
        gates: &[],
    };
    let clef = Clef {
        transport: endpoint,
        clock,
        budget: Budget::start_for(clock, STOP_BUDGET),
    };
    judge(&report, |name: &str| world.lookup(name), &clef)
}

fn judged(world: &World, reply: &str, endpoint: &Endpoint) -> Judged {
    judged_at(world, reply, endpoint, &Fixed)
}

fn d4_rows(world: &World) -> TestResult<Vec<Value>> {
    Ok(world
        .rows()?
        .into_iter()
        .filter(|row| row["point"] == json!("D4"))
        .collect())
}

#[test]
fn d4_asks_the_27b_model_once_per_required_section_and_keeps_no_input() -> TestResult {
    let world = World::configured()?;
    let endpoint = Endpoint::answering(0.9, 0.9);

    judged(&world, REPORT, &endpoint);

    assert_eq!(
        endpoint.models(),
        vec!["@cf/wet-ink/clef".to_owned(); 5],
        "one call per section, each to the `clef` model, not `clef-flash`"
    );
    let rows = d4_rows(&world)?;
    assert_eq!(rows.len(), 5, "one row per section");
    assert_eq!(
        rows.iter().filter(|row| row.get("input").is_some()).count(),
        0,
        "no row keeps the report's text: {rows:?}"
    );
    Ok(())
}

#[test]
fn a_reply_without_a_status_line_asks_nothing() -> TestResult {
    let world = World::enforcing(Point::D4)?;
    let endpoint = Endpoint::answering(0.0, 0.0);

    let judged = judged(&world, "Done: the parser is fixed.", &endpoint);

    assert_eq!(judged, Judged::default());
    assert!(endpoint.models().is_empty(), "not a report: nothing asked");
    Ok(())
}

#[test]
fn in_enforce_a_section_below_the_band_blocks_naming_only_it() -> TestResult {
    let world = World::enforcing(Point::D4)?;

    let judged = judged(&world, REPORT, &Endpoint::answering(0.2, 0.9));

    let line = judged.block.expect("the Gate section blocks");
    assert!(line.contains("Gate (0.20)"), "{line}");
    assert!(!line.contains("Review"), "a section at 0.90 passes: {line}");
    assert!(judged.notes.is_empty(), "{:?}", judged.notes);
    Ok(())
}

#[test]
fn in_shadow_no_section_blocks_and_nothing_is_noted_though_every_answer_is_logged() -> TestResult {
    let world = World::configured()?;

    let judged = judged(&world, REPORT, &Endpoint::answering(0.0, 0.0));

    assert_eq!(judged, Judged::default(), "shadow changes nothing");
    let rows = d4_rows(&world)?;
    assert_eq!(rows.len(), 5, "every section was asked and logged");
    assert!(
        rows.iter().all(|row| row["action"] == json!("block")),
        "the action is recorded for calibration: {rows:?}"
    );
    Ok(())
}

#[test]
fn configured_enforce_inside_the_shadow_period_never_blocks() -> TestResult {
    let world = World::enforce_configured_in_shadow_period()?;

    let judged = judged(&world, REPORT, &Endpoint::answering(0.0, 0.0));

    assert_eq!(judged.block, None, "fewer than 50 answers: still shadow");
    let rows = d4_rows(&world)?;
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert_eq!(
        rows.iter()
            .filter(|row| row["mode"] == json!("shadow"))
            .count(),
        5,
        "each row records the shadow mode in effect: {rows:?}"
    );
    Ok(())
}

#[test]
fn with_clef_off_nothing_is_asked_or_created() -> TestResult {
    let world = World::new(&[])?;
    let endpoint = Endpoint::answering(0.0, 0.0);

    let judged = judged(&world, REPORT, &endpoint);

    assert_eq!(judged, Judged::default());
    assert!(endpoint.models().is_empty(), "off asks nothing");
    assert_eq!(
        std::fs::read_dir(world.home())?.count(),
        0,
        "nothing created in the home"
    );
    Ok(())
}

#[test]
fn in_enforce_an_unreachable_clef_neither_blocks_nor_notes() -> TestResult {
    let world = World::enforcing(Point::D4)?;

    let judged = judged(&world, REPORT, &Endpoint::unreachable());

    assert_eq!(judged, Judged::default(), "D4 fails open, silently");
    Ok(())
}

#[test]
fn in_enforce_an_answer_that_cannot_be_logged_is_noted_and_never_blocks() -> TestResult {
    let world = World::enforcing(Point::D4)?;
    block_the_log(world.home())?;

    let judged = judged(&world, REPORT, &Endpoint::answering(0.0, 0.0));

    assert_eq!(judged.block, None, "an unlogged answer is not acted on");
    assert_eq!(judged.notes.len(), 5, "{:?}", judged.notes);
    assert!(
        judged
            .notes
            .iter()
            .all(|note| note.contains("was not logged")),
        "{:?}",
        judged.notes
    );
    Ok(())
}

#[test]
fn a_spent_budget_asks_nothing_and_is_noted_only_when_d4_is_configured_to_enforce() -> TestResult {
    let endpoint = Endpoint::answering(0.0, 0.0);
    let enforcing = World::enforcing(Point::D4)?;
    let shadow = World::configured()?;

    let noted = judged_at(&enforcing, REPORT, &endpoint, &Stepping::by_ms(24_000));
    let silent = judged_at(&shadow, REPORT, &endpoint, &Stepping::by_ms(24_000));

    assert!(
        endpoint.models().is_empty(),
        "no time for Clef: nothing asked"
    );
    assert_eq!(noted.block, None);
    assert_eq!(noted.notes.len(), 1, "{:?}", noted.notes);
    assert!(
        noted
            .notes
            .iter()
            .all(|note| note.contains("Clef was not asked")),
        "{:?}",
        noted.notes
    );
    assert_eq!(silent, Judged::default(), "shadow stays silent");
    Ok(())
}
