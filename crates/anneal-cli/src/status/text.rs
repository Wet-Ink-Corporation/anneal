//! The text form of `anneal status`: one line per section present, in the document's order,
//! then one line for the findings; at most [`MAX_LINES`] lines, each at most
//! [`MAX_LINE_CHARS`] characters.

use std::io::{self, Write};

use anneal_ledger::event::Verdict;

use super::Document;
use super::head_verdict::HeadVerdict;
use super::home::{Clef, Context};
use super::ledger::{Gate, Phase, Plan, TailEvent, Tasks};
use crate::envelope::Finding;
use crate::hook::injection_scan::detect;

/// The most lines the text form writes: eight sections and the findings line fit.
pub(super) const MAX_LINES: usize = 12;

/// The most characters of one line.
const MAX_LINE_CHARS: usize = 200;

/// Writes `document` and `findings` as text to `out`.
///
/// # Errors
/// The I/O error when `out` cannot be written.
pub(super) fn write<O: Write>(
    document: &Document,
    findings: &[Finding],
    mut out: O,
) -> io::Result<()> {
    for line in lines(document, findings).iter().take(MAX_LINES) {
        let cut: String = line.chars().take(MAX_LINE_CHARS).map(shown).collect();
        writeln!(out, "{cut}")?;
    }
    Ok(())
}

/// `c` as the text form shows it: a control character (Cc, CR and LF among them), or one that
/// `injection_scan::detect::is_unprintable` names (default-ignorable, Cf, Zl, Zp), is a space,
/// so recorded text cannot break a line or drive the terminal (as `orchestrate::park` judges
/// one line).
fn shown(c: char) -> char {
    if c.is_control() || detect::is_unprintable(c) {
        ' '
    } else {
        c
    }
}

/// The lines, before they are cut.
fn lines(document: &Document, findings: &[Finding]) -> Vec<String> {
    let problems = (!findings.is_empty()).then(|| {
        let problems: Vec<&str> = findings
            .iter()
            .map(|finding| finding.message.as_str())
            .collect();
        format!("problems: {}", problems.join("; "))
    });
    [
        document.phase.as_ref().map(phase),
        document.plan.as_ref().map(plan),
        document.tasks.as_ref().map(tasks),
        document.gates.as_deref().map(gates),
        document.verdict.as_ref().map(verdict),
        document.clef.as_ref().map(clef),
        document.context.as_ref().map(context),
        document.ledger_tail.as_deref().map(ledger_tail),
        problems,
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn phase(phase: &Phase) -> String {
    match phase.plan() {
        Some(plan) => format!("phase: {} (plan {plan})", phase.mode()),
        None => format!("phase: {}", phase.mode()),
    }
}

fn plan(plan: &Plan) -> String {
    format!("plan: {} {}", plan.status(), plan.id())
}

fn tasks(tasks: &Tasks) -> String {
    let (done, total) = tasks.counts();
    match tasks.next() {
        Some(next) => format!("tasks: {done} of {total} done, next {next}"),
        None => format!("tasks: {done} of {total} done"),
    }
}

fn gates(gates: &[Gate]) -> String {
    match gates.last() {
        None => "gates: none blocked".to_owned(),
        Some(newest) => {
            let (id, reason, event) = newest.parts();
            let count = gates.len();
            let blocks = if count == 1 { "block" } else { "blocks" };
            format!("gates: {count} recent {blocks}, newest {id} (#{event}): {reason}")
        }
    }
}

fn verdict(verdict: &HeadVerdict) -> String {
    let (pr, outcome, level) = verdict.parts();
    let outcome = outcome.as_str();
    match level {
        Some(level) => format!("verdict: pr {pr} {outcome} ({})", verdict_name(level)),
        None => format!("verdict: pr {pr} {outcome}"),
    }
}

fn clef(clef: &Clef) -> String {
    let points: Vec<String> = clef
        .points()
        .map(|(point, answered, mode)| format!("{} {answered} {}", point.as_str(), mode.as_str()))
        .collect();
    format!("clef: {}", points.join(", "))
}

fn context(context: &Context) -> String {
    let (used, window, percent) = context.parts();
    let window = window / 1000;
    match (used, percent) {
        (Some(used), Some(percent)) => {
            format!("context: {:.0}k/{window}k {percent}%", used / 1000.0)
        }
        _ => format!("context: -/{window}k"),
    }
}

fn ledger_tail(events: &[TailEvent]) -> String {
    if events.is_empty() {
        return "ledger: empty".to_owned();
    }
    let shown: Vec<String> = events
        .iter()
        .map(|event| {
            let (position, summary) = event.parts();
            format!("#{position} {summary}")
        })
        .collect();
    format!("ledger: {}", shown.join("; "))
}

/// A verdict as the ledger writes it: serde's `snake_case` name of the variant, kept as a
/// `const` match because the serde name is reachable only through a fallible, allocating
/// `serde_json::to_value`, and the text form has no error path. A test pins the two together.
pub(super) const fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::TestsVerified => "tests_verified",
        Verdict::Reviewed => "reviewed",
        Verdict::Approved => "approved",
        Verdict::ChangesRequested => "changes_requested",
        Verdict::NoiseOnly => "noise_only",
    }
}
