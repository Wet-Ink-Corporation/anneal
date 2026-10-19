//! The per-point report: the log's decisions joined to their labels, scored per point and
//! question, with a proposed threshold and a promotion verdict.
//!
//! A question is `stay-shadow` when it has fewer than [`MIN_LABELLED`] labelled decisions, a
//! Brier score over [`MAX_BRIER`], or an ask rate of [`MAX_ASK_RATE`] or more; otherwise it is
//! `eligible-to-enforce`. A point is eligible only when every question it has decisions for is,
//! and it has some. The threshold proposed is the point's lowest band, the boundary between
//! acting and `otherwise`.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::calibrate::read::{Answered, LogRows};
use crate::calibrate::score::{self, Bin, Costs, FalseAccept, Sample};
use crate::decide::point::{Point, Question};
use crate::decide::thresholds::{Band, PointRules, Rule, Thresholds};

/// Fewest labelled decisions a question needs to be eligible.
pub(crate) const MIN_LABELLED: u32 = 50;

/// Highest Brier score an eligible question may have.
pub(crate) const MAX_BRIER: f64 = 0.15;

/// The ask rate at which a question stays in shadow.
pub(crate) const MAX_ASK_RATE: f64 = 0.20;

/// The promotion verdict. Never applied: `calibrate` only reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Verdict {
    /// Keep the point in shadow.
    StayShadow,
    /// The point may be set to `enforce`.
    EligibleToEnforce,
}

/// Why a question stays in shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    /// Fewer than [`MIN_LABELLED`] labelled decisions.
    TooFewLabelled,
    /// A Brier score over [`MAX_BRIER`].
    BrierAboveBound,
    /// An ask rate of [`MAX_ASK_RATE`] or more.
    AskRateAtOrAboveBound,
}

/// Why no threshold is proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum NoProposal {
    /// The rule has no bands (D3's size score only records).
    #[serde(rename = "no_bands")]
    Bandless,
    /// No decision of the question has a usable label.
    #[serde(rename = "no_labels")]
    Unlabelled,
}

impl Verdict {
    /// The name, as serialised.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::StayShadow => "stay-shadow",
            Self::EligibleToEnforce => "eligible-to-enforce",
        }
    }
}

impl Reason {
    /// The name, as serialised.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TooFewLabelled => "too_few_labelled",
            Self::BrierAboveBound => "brier_above_bound",
            Self::AskRateAtOrAboveBound => "ask_rate_at_or_above_bound",
        }
    }
}

impl NoProposal {
    /// The name, as serialised.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Bandless => "no_bands",
            Self::Unlabelled => "no_labels",
        }
    }
}

/// The whole report: every point, in order.
#[derive(Debug, Serialize)]
pub(crate) struct Report<'t> {
    /// Each point's report.
    pub(crate) points: Vec<PointReport<'t>>,
    /// Labels whose id is no answered decision in the log (unanswered, rotated out, mistyped).
    pub(crate) unmatched_labels: u32,
    /// Rows of a question the point's thresholds do not define (the thresholds changed).
    pub(crate) unreported_rows: u32,
}

/// One point's report.
#[derive(Debug, Serialize)]
pub(crate) struct PointReport<'t> {
    pub(crate) point: Point,
    pub(crate) verdict: Verdict,
    /// One per question the point's thresholds define.
    pub(crate) questions: Vec<QuestionReport<'t>>,
}

/// One question of one point.
#[derive(Debug, Serialize)]
pub(crate) struct QuestionReport<'t> {
    pub(crate) question: Question,
    pub(crate) subject: &'t str,
    /// Answered decisions.
    pub(crate) n: u32,
    /// Answered decisions with a usable label.
    pub(crate) labelled: u32,
    /// Of `labelled`, those whose latest label came through a mod (`via: mod`).
    pub(crate) labelled_via_mod: u32,
    /// Decisions without an answer.
    pub(crate) unanswered: u32,
    /// Answered decisions whose label does not fit the question.
    pub(crate) unusable_labels: u32,
    pub(crate) brier: Option<f64>,
    pub(crate) bins: Vec<Bin>,
    /// Answered decisions whose action asked a human.
    pub(crate) asks: u32,
    pub(crate) ask_rate: Option<f64>,
    pub(crate) threshold: ThresholdReport<'t>,
    pub(crate) verdict: Verdict,
    pub(crate) reasons: Vec<Reason>,
}

/// The lowest band's threshold now and as proposed.
#[derive(Debug, Serialize)]
pub(crate) struct ThresholdReport<'t> {
    /// The lowest band's action; `None` without bands.
    pub(crate) band: Option<&'t str>,
    /// Its `at_least` in the thresholds file, as a JSON pointer.
    pub(crate) pointer: Option<String>,
    pub(crate) current: Option<f64>,
    /// The next band's `at_least`, which a proposal stays below.
    pub(crate) ceiling: Option<f64>,
    pub(crate) false_accept: FalseAccept,
    pub(crate) current_expected_cost: Option<f64>,
    pub(crate) proposed: Option<f64>,
    pub(crate) proposed_expected_cost: Option<f64>,
    pub(crate) no_proposal: Option<NoProposal>,
}

/// One question's decisions while they are counted.
#[derive(Debug, Default)]
struct Group {
    n: u32,
    asks: u32,
    unusable: u32,
    /// Samples whose label came through a mod.
    via_mod: u32,
    samples: Vec<Sample>,
}

/// Which error is the false accept at each point (see [`FalseAccept`]).
const fn false_accept(point: Point) -> FalseAccept {
    match point {
        Point::D1 | Point::D2 | Point::D5 | Point::D7 => FalseAccept::BelowThreshold,
        Point::D3 | Point::D4 | Point::D6 => FalseAccept::AtOrAboveThreshold,
    }
}

/// Whether `label` says the predicted event happened: noul and score take `yes`, `true` or
/// `1` and `no`, `false` or `0`; a choice takes the name of an option, positive when it is the
/// top one. `None` for a label that does not fit.
pub(crate) fn outcome(decision: &Answered, label: &str) -> Option<bool> {
    match decision.question {
        Question::Noul | Question::Score => match label {
            "yes" | "true" | "1" => Some(true),
            "no" | "false" | "0" => Some(false),
            _ => None,
        },
        Question::Choice => {
            if decision.top.as_deref() == Some(label) {
                Some(true)
            } else {
                decision
                    .options
                    .iter()
                    .any(|option| option == label)
                    .then_some(false)
            }
        }
    }
}

/// The verdict for a question, and every reason it stays in shadow.
pub(crate) fn verdict(
    labelled: u32,
    brier: Option<f64>,
    ask_rate: Option<f64>,
) -> (Verdict, Vec<Reason>) {
    let reasons: Vec<Reason> = [
        (labelled < MIN_LABELLED).then_some(Reason::TooFewLabelled),
        brier
            .is_some_and(|brier| brier > MAX_BRIER)
            .then_some(Reason::BrierAboveBound),
        ask_rate
            .is_some_and(|rate| rate >= MAX_ASK_RATE)
            .then_some(Reason::AskRateAtOrAboveBound),
    ]
    .into_iter()
    .flatten()
    .collect();
    let verdict = if reasons.is_empty() {
        Verdict::EligibleToEnforce
    } else {
        Verdict::StayShadow
    };
    (verdict, reasons)
}

/// Reports every point of `thresholds` over `rows`.
pub(crate) fn report<'t>(thresholds: &'t Thresholds, rows: &LogRows) -> Report<'t> {
    let mut groups: BTreeMap<(Point, Question), Group> = BTreeMap::new();
    let mut answered_ids: HashSet<&str> = HashSet::with_capacity(rows.answered.len());
    for decision in &rows.answered {
        answered_ids.insert(&decision.id);
        let group = groups
            .entry((decision.point, decision.question))
            .or_default();
        group.n = group.n.saturating_add(1);
        group.asks = group.asks.saturating_add(u32::from(decision.asked));
        match rows
            .labels
            .get(&decision.id)
            .map(|label| outcome(decision, label))
        {
            Some(Some(positive)) => {
                group
                    .samples
                    .push(Sample::new(decision.probability, positive));
                let via_mod = rows.via_mod.contains(&decision.id);
                group.via_mod = group.via_mod.saturating_add(u32::from(via_mod));
            }
            Some(None) => group.unusable = group.unusable.saturating_add(1),
            None => {}
        }
    }
    let undefined =
        |(point, question): &(Point, Question)| thresholds.point(*point).rule(*question).is_none();
    let unreported_rows = groups
        .iter()
        .filter(|(key, _)| undefined(key))
        .map(|(_, group)| group.n)
        .chain(
            rows.unanswered
                .iter()
                .filter(|(key, _)| undefined(key))
                .map(|(_, n)| *n),
        )
        .fold(0, u32::saturating_add);
    let unmatched_labels = score::count(
        rows.labels
            .keys()
            .filter(|id| !answered_ids.contains(id.as_str()))
            .count(),
    );
    let points = Point::ALL
        .into_iter()
        .map(|point| point_report(point, thresholds.point(point), &groups, &rows.unanswered))
        .collect();
    Report {
        points,
        unmatched_labels,
        unreported_rows,
    }
}

fn point_report<'t>(
    point: Point,
    rules: &'t PointRules,
    groups: &BTreeMap<(Point, Question), Group>,
    unanswered: &BTreeMap<(Point, Question), u32>,
) -> PointReport<'t> {
    let empty = Group::default();
    let questions: Vec<QuestionReport<'t>> = Question::ALL
        .into_iter()
        .filter_map(|question| {
            let rule = rules.rule(question)?;
            let key = (point, question);
            // A question with no decisions, answered or not, reports zeros.
            let group = groups.get(&key).unwrap_or(&empty);
            let unanswered = unanswered.get(&key).copied().unwrap_or(0);
            Some(question_report(key, rule, group, unanswered))
        })
        .collect();
    // A bandless question is record-only: it never acts, so its rows do not hold the point back.
    let mut active = questions
        .iter()
        .filter(|question| question.n > 0 && question.threshold.band.is_some())
        .peekable();
    let eligible = active.peek().is_some()
        && active.all(|question| question.verdict == Verdict::EligibleToEnforce);
    PointReport {
        point,
        verdict: if eligible {
            Verdict::EligibleToEnforce
        } else {
            Verdict::StayShadow
        },
        questions,
    }
}

/// The report of the question `key` from its rule and decisions.
fn question_report<'t>(
    key: (Point, Question),
    rule: &'t Rule,
    group: &Group,
    unanswered: u32,
) -> QuestionReport<'t> {
    let labelled = score::count(group.samples.len());
    let brier = score::brier(&group.samples);
    let ask_rate = (group.n > 0).then(|| f64::from(group.asks) / f64::from(group.n));
    let (verdict, reasons) = verdict(labelled, brier, ask_rate);
    QuestionReport {
        question: key.1,
        subject: rule.subject(),
        n: group.n,
        labelled,
        labelled_via_mod: group.via_mod,
        unanswered,
        unusable_labels: group.unusable,
        brier,
        bins: score::bins(&group.samples),
        asks: group.asks,
        ask_rate,
        threshold: threshold_report(key, rule, &group.samples),
        verdict,
        reasons,
    }
}

fn threshold_report<'t>(
    (point, question): (Point, Question),
    rule: &'t Rule,
    samples: &[Sample],
) -> ThresholdReport<'t> {
    let costs = Costs::new(false_accept(point));
    let Some((lowest, higher)) = rule.bands().split_last() else {
        return ThresholdReport {
            band: None,
            pointer: None,
            current: None,
            ceiling: None,
            false_accept: false_accept(point),
            current_expected_cost: None,
            proposed: None,
            proposed_expected_cost: None,
            no_proposal: Some(NoProposal::Bandless),
        };
    };
    let ceiling = higher.last().map(Band::at_least);
    let current = lowest.at_least();
    // `propose` is `None` only without labelled samples; otherwise it is the current value
    // unless a candidate is strictly cheaper.
    let proposal = score::propose(samples, costs, current, ceiling);
    let no_proposal = proposal.is_none().then_some(NoProposal::Unlabelled);
    ThresholdReport {
        band: Some(lowest.action()),
        pointer: Some(format!(
            "/points/{point}/rules/{question}/bands/{}/at_least",
            higher.len()
        )),
        current: Some(current),
        ceiling,
        false_accept: false_accept(point),
        current_expected_cost: score::expected_cost(samples, costs, current),
        proposed: proposal.map(|proposal| proposal.threshold),
        proposed_expected_cost: proposal
            .and_then(|proposal| score::expected_cost(samples, costs, proposal.threshold)),
        no_proposal,
    }
}

#[cfg(test)]
mod tests;
