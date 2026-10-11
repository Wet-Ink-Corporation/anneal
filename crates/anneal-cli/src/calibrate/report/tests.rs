use super::{NoProposal, PointReport, Reason, Report, Verdict, outcome, report, verdict};
use crate::calibrate::read::{Answered, LogRows};
use crate::decide::point::{Point, Question};
use crate::decide::thresholds::Thresholds;

fn thresholds() -> Thresholds {
    Thresholds::built_in().expect("the shipped thresholds are valid")
}

fn point_of<'r, 't>(report: &'r Report<'t>, point: Point) -> &'r PointReport<'t> {
    report
        .points
        .iter()
        .find(|reported| reported.point == point)
        .expect("every point is reported")
}

fn noul(id: &str, point: Point, probability: f64, asked: bool) -> Answered {
    Answered {
        id: id.to_owned(),
        point,
        question: Question::Noul,
        probability,
        top: None,
        options: vec!["no".to_owned(), "yes".to_owned()],
        asked,
    }
}

fn choice(id: &str, point: Point, probability: f64, top: &str) -> Answered {
    Answered {
        id: id.to_owned(),
        point,
        question: Question::Choice,
        probability,
        top: Some(top.to_owned()),
        options: vec!["build".to_owned(), "plan".to_owned()],
        asked: false,
    }
}

/// `n` D7 decisions, each labelled exactly as predicted (Brier 0, no asks).
fn perfect(point: Point, n: u32) -> LogRows {
    let mut rows = LogRows::default();
    for k in 0..n {
        let id = format!("d1-1-{k}");
        let positive = k % 2 == 0;
        rows.answered
            .push(noul(&id, point, if positive { 1.0 } else { 0.0 }, false));
        rows.labels
            .insert(id, if positive { "yes" } else { "no" }.to_owned());
    }
    rows
}

#[test]
fn forty_nine_labelled_decisions_stay_shadow_and_fifty_are_eligible() {
    assert_eq!(
        verdict(49, Some(0.0), Some(0.0)),
        (Verdict::StayShadow, vec![Reason::TooFewLabelled])
    );
    assert_eq!(
        verdict(50, Some(0.0), Some(0.0)),
        (Verdict::EligibleToEnforce, vec![])
    );
}

#[test]
fn brier_over_0_15_or_an_ask_rate_of_20_percent_stays_shadow() {
    assert_eq!(
        verdict(50, Some(0.1501), Some(0.0)),
        (Verdict::StayShadow, vec![Reason::BrierAboveBound])
    );
    assert_eq!(
        verdict(50, Some(0.0), Some(0.2)),
        (Verdict::StayShadow, vec![Reason::AskRateAtOrAboveBound])
    );
    assert_eq!(
        verdict(50, Some(0.15), Some(0.1999)),
        (Verdict::EligibleToEnforce, vec![]),
        "0.15 and just under 20% are within the bounds"
    );
    assert_eq!(
        verdict(0, None, None),
        (Verdict::StayShadow, vec![Reason::TooFewLabelled])
    );
}

#[test]
fn a_point_with_49_perfect_labelled_decisions_is_never_eligible() {
    let thresholds = thresholds();
    let rows = perfect(Point::D7, 49);

    let report = report(&thresholds, &rows);

    let d7 = point_of(&report, Point::D7);
    assert_eq!(d7.verdict, Verdict::StayShadow);
    let question = d7.questions.first().expect("D7 asks noul");
    assert_eq!(
        (question.labelled, question.brier, question.ask_rate),
        (49, Some(0.0), Some(0.0))
    );
    let fifty = perfect(Point::D7, 50);
    let fifty = report_verdict(&thresholds, &fifty, Point::D7);
    assert_eq!(fifty, Verdict::EligibleToEnforce, "the 50th label promotes");
}

fn report_verdict(thresholds: &Thresholds, rows: &LogRows, point: Point) -> Verdict {
    point_of(&report(thresholds, rows), point).verdict
}

#[test]
fn noul_and_score_labels_are_yes_or_no_words_and_choice_labels_name_an_option() {
    let yes_no = noul("d1-1-0", Point::D1, 0.5, false);
    let picked = choice("d1-1-1", Point::D3, 0.7, "plan");

    let read = |decision: &Answered, labels: &[&str]| -> Vec<Option<bool>> {
        labels
            .iter()
            .map(|label| outcome(decision, label))
            .collect()
    };

    assert_eq!(
        read(
            &yes_no,
            &["yes", "true", "1", "no", "false", "0", "maybe", "Yes"]
        ),
        [
            Some(true),
            Some(true),
            Some(true),
            Some(false),
            Some(false),
            Some(false),
            None,
            None
        ]
    );
    assert_eq!(
        read(&picked, &["plan", "build", "yes", "ship"]),
        [Some(true), Some(false), None, None],
        "the top option, another option, then words that are not options"
    );
}

/// The hand set of `score::tests` as D1 decisions, plus one unlabelled ask, one unusable label
/// and one label for no decision in the log.
fn hand_rows() -> LogRows {
    let mut rows = LogRows::default();
    for (k, (probability, label)) in [
        (0.1, "no"),
        (0.3, "yes"),
        (0.8, "true"),
        (0.95, "0"),
        (1.0, "1"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("d1-1-{k}");
        rows.answered.push(noul(
            &id,
            Point::D1,
            probability,
            (0.4..0.9).contains(&probability),
        ));
        rows.labels.insert(id, label.to_owned());
    }
    rows.answered.push(noul("d1-1-5", Point::D1, 0.5, true));
    rows.answered.push(noul("d1-1-6", Point::D1, 0.2, false));
    rows.labels.insert("d1-1-6".to_owned(), "maybe".to_owned());
    rows.labels.insert("d9-9-9".to_owned(), "yes".to_owned());
    rows.unanswered.insert((Point::D1, Question::Noul), 2);
    rows
}

#[test]
fn the_hand_set_reports_its_counts_brier_ask_rate_and_verdict() {
    let thresholds = thresholds();
    let rows = hand_rows();

    let report = report(&thresholds, &rows);

    let d1 = point_of(&report, Point::D1);
    let noul = d1.questions.first().expect("D1 asks noul");
    assert_eq!(
        (
            noul.n,
            noul.labelled,
            noul.unanswered,
            noul.unusable_labels,
            noul.asks
        ),
        (7, 5, 2, 1, 2),
        "the 0.8 and 0.5 decisions asked"
    );
    assert!(
        (noul.brier.expect("labelled") - 0.2885).abs() < 1e-12,
        "{noul:?}"
    );
    assert!(
        (noul.ask_rate.expect("answered") - 2.0 / 7.0).abs() < 1e-12,
        "{noul:?}"
    );
    assert_eq!(
        (d1.verdict, &noul.reasons),
        (
            Verdict::StayShadow,
            &vec![
                Reason::TooFewLabelled,
                Reason::BrierAboveBound,
                Reason::AskRateAtOrAboveBound
            ]
        )
    );
    assert_eq!(
        report.unmatched_labels, 1,
        "d9-9-9 is no decision in the log"
    );
}

#[test]
fn the_hand_set_proposes_the_cheapest_threshold_under_d1s_deny_band() {
    let thresholds = thresholds();
    let rows = hand_rows();

    let report = report(&thresholds, &rows);

    let noul = point_of(&report, Point::D1)
        .questions
        .first()
        .expect("D1 asks noul");
    let threshold = &noul.threshold;
    assert_eq!(
        (
            threshold.band,
            threshold.current,
            threshold.ceiling,
            threshold.proposed
        ),
        (Some("ask"), Some(0.4), Some(0.9), Some(0.3)),
        "D1's lowest band is ask at 0.4, under deny at 0.9"
    );
    assert_eq!(
        threshold.pointer.as_deref(),
        Some("/points/D1/rules/noul/bands/1/at_least")
    );
    assert!(
        (threshold.current_expected_cost.expect("labelled") - 1.2).abs() < 1e-12,
        "{threshold:?}"
    );
    assert!(
        (threshold.proposed_expected_cost.expect("labelled") - 0.2).abs() < 1e-12,
        "{threshold:?}"
    );
}

#[test]
fn every_point_lists_every_question_its_thresholds_define_even_with_no_decisions() {
    let thresholds = thresholds();

    let report = report(&thresholds, &LogRows::default());

    let listed: Vec<Point> = report.points.iter().map(|point| point.point).collect();
    assert_eq!(listed, Point::ALL, "all seven points, in order");
    let d3 = point_of(&report, Point::D3);
    let d3_questions: Vec<Question> = d3.questions.iter().map(|q| q.question).collect();
    assert_eq!(d3_questions, [Question::Choice, Question::Score]);
    assert_eq!(d3.verdict, Verdict::StayShadow, "no decisions");
    let score = d3.questions.get(1).expect("score");
    assert_eq!(
        (score.threshold.band, score.threshold.no_proposal),
        (None, Some(NoProposal::Bandless)),
        "D3's size score has no bands"
    );
    let choice = d3.questions.first().expect("choice");
    assert_eq!(choice.threshold.no_proposal, Some(NoProposal::Unlabelled));
}

#[test]
fn a_point_is_eligible_only_when_every_question_with_decisions_is() {
    let thresholds = thresholds();
    let mut rows = LogRows::default();
    for k in 0..50 {
        let id = format!("d1-1-{k}");
        rows.answered.push(choice(&id, Point::D3, 1.0, "plan"));
        rows.labels.insert(id, "plan".to_owned());
    }

    let eligible = report_verdict(&thresholds, &rows, Point::D3);
    rows.answered.push(Answered {
        question: Question::Score,
        top: None,
        options: Vec::new(),
        ..choice("d1-1-99", Point::D3, 0.5, "plan")
    });
    let with_an_unlabelled_score = report_verdict(&thresholds, &rows, Point::D3);

    // Since #31 the second value is EligibleToEnforce: D3's size score has no bands, so it only
    // records, and a bandless question no longer holds the point back even with decisions.
    assert_eq!(
        (eligible, with_an_unlabelled_score),
        (Verdict::EligibleToEnforce, Verdict::EligibleToEnforce),
        "a bandless score question does not hold D3 back, with or without decisions"
    );
}

#[test]
fn unlabelled_rows_of_a_bandless_question_do_not_hold_a_passing_point_back() {
    let thresholds = thresholds();
    let mut rows = LogRows::default();
    for k in 0..50 {
        let id = format!("d1-1-{k}");
        rows.answered.push(choice(&id, Point::D3, 1.0, "plan"));
        rows.labels.insert(id, "plan".to_owned());
        // Route logs a size score beside every phase choice; D3's score rule has no bands.
        rows.answered.push(Answered {
            question: Question::Score,
            top: None,
            options: Vec::new(),
            ..choice(&format!("d1-2-{k}"), Point::D3, 0.5, "plan")
        });
    }

    let report = report(&thresholds, &rows);
    let d3 = point_of(&report, Point::D3);

    assert_eq!(d3.verdict, Verdict::EligibleToEnforce);
    let score = d3
        .questions
        .iter()
        .find(|question| question.question == Question::Score)
        .expect("D3 reports its score question");
    assert_eq!(
        (score.n, score.verdict),
        (50, Verdict::StayShadow),
        "the size is still reported, just not counted toward the point"
    );
}

#[test]
fn rows_of_a_question_the_thresholds_do_not_define_are_counted_as_unreported() {
    let thresholds = thresholds();
    let mut rows = LogRows::default();
    rows.answered.push(Answered {
        question: Question::Choice,
        ..noul("d1-1-0", Point::D1, 0.5, false)
    });
    rows.unanswered.insert((Point::D1, Question::Score), 1);

    let report = report(&thresholds, &rows);

    assert_eq!(report.unreported_rows, 2, "D1 asks only noul");
}
