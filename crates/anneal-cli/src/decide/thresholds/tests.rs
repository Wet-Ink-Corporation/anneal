use serde_json::{Value, json};

use super::{Applied, Thresholds, ThresholdsError};
use crate::decide::point::{Point, Question};

fn built_in() -> Thresholds {
    Thresholds::built_in().expect("the shipped thresholds parse")
}

/// The shipped file as JSON, to edit one field per test.
fn shipped_json() -> Value {
    serde_json::from_str(super::BUILT_IN).expect("the shipped file is JSON")
}

fn parse(value: &Value) -> Result<Thresholds, ThresholdsError> {
    Thresholds::parse(value.to_string().as_bytes())
}

#[test]
fn the_shipped_file_routes_d4_to_clef_and_every_other_point_to_clef_flash() {
    let thresholds = built_in();

    for point in Point::ALL {
        let rules = thresholds.point(point);
        let expected = if point == Point::D4 {
            "clef"
        } else {
            "clef-flash"
        };
        assert_eq!(rules.model_alias(), expected, "{point}");
    }
}

#[test]
fn a_model_alias_resolves_to_the_id_in_the_models_table() -> Result<(), ThresholdsError> {
    let mut value = shipped_json();
    value["models"]["clef"] = json!("@cf/example/clef-v2");

    let thresholds = parse(&value)?;

    let d4 = thresholds.point(Point::D4);
    assert_eq!(d4.model(), "@cf/example/clef-v2", "changed without code");
    Ok(())
}

#[test]
fn d1_bands_deny_from_090_ask_from_040_and_defer_below() {
    let thresholds = built_in();
    let rule = thresholds
        .point(Point::D1)
        .rule(Question::Noul)
        .expect("D1 asks noul");

    let applied: Vec<Applied<'_>> = [0.95, 0.9, 0.5, 0.4, 0.39]
        .into_iter()
        .map(|p| rule.apply(p))
        .collect();

    assert_eq!(
        applied,
        [
            Applied::new("deny", Some(0.9)),
            Applied::new("deny", Some(0.9)),
            Applied::new("ask", Some(0.4)),
            Applied::new("ask", Some(0.4)),
            Applied::new("defer", None),
        ]
    );
}

#[test]
fn d1_falls_back_to_ask_and_the_others_fail_open() {
    let thresholds = built_in();

    for point in Point::ALL {
        let fallback = thresholds.point(point).fallback();
        let expected = if point == Point::D1 {
            "ask"
        } else {
            "fail_open"
        };
        assert_eq!(fallback, expected, "{point}");
    }
}

#[test]
fn a_question_a_point_does_not_ask_has_no_rule() {
    let thresholds = built_in();

    let d1 = thresholds.point(Point::D1);

    assert_eq!(d1.rule(Question::Choice), None, "D1 asks only noul");
}

#[test]
fn another_version_is_refused() {
    let mut value = shipped_json();
    value["version"] = json!(2);

    assert!(matches!(
        parse(&value),
        Err(ThresholdsError::Version { found: 2 })
    ));
}

#[test]
fn a_missing_point_is_refused() {
    let mut value = shipped_json();
    value["points"]
        .as_object_mut()
        .expect("points is an object")
        .remove("D5");

    assert!(matches!(
        parse(&value),
        Err(ThresholdsError::MissingPoint(Point::D5))
    ));
}

#[test]
fn an_unknown_model_alias_is_refused() {
    let mut value = shipped_json();
    value["points"]["D2"]["model"] = json!("clef-huge");

    assert!(matches!(
        parse(&value),
        Err(ThresholdsError::UnknownModel {
            point: Point::D2,
            ..
        })
    ));
}

#[test]
fn bands_must_descend_strictly_within_zero_to_one() {
    for bands in [
        json!([{"at_least": 0.4, "action": "ask"}, {"at_least": 0.9, "action": "deny"}]),
        json!([{"at_least": 0.4, "action": "ask"}, {"at_least": 0.4, "action": "deny"}]),
        json!([{"at_least": 1.5, "action": "deny"}]),
        json!([{"at_least": -0.1, "action": "deny"}]),
    ] {
        let mut value = shipped_json();
        value["points"]["D1"]["rules"]["noul"]["bands"] = bands.clone();

        assert!(
            matches!(
                parse(&value),
                Err(ThresholdsError::Bands {
                    point: Point::D1,
                    question: Question::Noul,
                    ..
                })
            ),
            "{bands} is refused"
        );
    }
}

#[test]
fn an_action_name_outside_the_word_alphabet_is_refused() {
    let mut value = shipped_json();
    value["points"]["D7"]["rules"]["noul"]["otherwise"] = json!("find fact");

    assert!(matches!(parse(&value), Err(ThresholdsError::Name { .. })));
}

#[test]
fn a_model_id_with_a_space_or_query_is_refused() {
    for id in ["@cf/a b", "@cf/a?x=1", ""] {
        let mut value = shipped_json();
        value["models"]["clef"] = json!(id);

        assert!(
            matches!(parse(&value), Err(ThresholdsError::ModelId { .. })),
            "{id:?} is refused"
        );
    }
}

#[test]
fn a_model_id_with_an_empty_dot_or_dot_dot_segment_is_refused() {
    for id in [
        "@cf//clef",
        "/@cf/clef",
        "@cf/clef/",
        "@cf/./clef",
        "@cf/../clef",
        "..",
        ".",
    ] {
        let mut value = shipped_json();
        value["models"]["clef"] = json!(id);

        assert!(
            matches!(parse(&value), Err(ThresholdsError::ModelId { .. })),
            "{id:?} is refused"
        );
    }
}

#[test]
fn a_model_id_with_dots_inside_a_segment_is_kept() -> Result<(), ThresholdsError> {
    let mut value = shipped_json();
    value["models"]["clef"] = json!("@cf/wet-ink/clef-1.5..b");

    let thresholds = parse(&value)?;
    assert_eq!(
        thresholds.point(Point::D4).model(),
        "@cf/wet-ink/clef-1.5..b"
    );
    Ok(())
}

#[test]
fn unknown_fields_are_refused() {
    let mut value = shipped_json();
    value["points"]["D1"]["confidence_threshold"] = json!(0.5);

    assert!(matches!(parse(&value), Err(ThresholdsError::Json(_))));
}
