use super::{
    Change, Direction, Now, RATCHETS_PATH, RatchetError, Ratchets, findings, line_of, loosened,
};
use crate::floor_guard::Category;

const BASE: &str = r#"{
  "version": 1,
  "ratchets": {
    "warnings": { "value": 10, "direction": "down" },
    "coverage": { "value": 80.5, "direction": "up" }
  }
}"#;

fn ratchets(text: &str) -> Ratchets {
    Ratchets::parse(text).expect("the fixture is a valid ratchet file")
}

/// A ratchet file holding `entries` (`(name, value, direction)`).
fn file(entries: &[(&str, f64, &str)]) -> String {
    let body: Vec<String> = entries
        .iter()
        .map(|(name, value, direction)| {
            format!("\"{name}\": {{\"value\": {value}, \"direction\": \"{direction}\"}}")
        })
        .collect();
    format!("{{\"version\": 1, \"ratchets\": {{{}}}}}", body.join(", "))
}

/// `(name, change)` of each loosened ratchet from `base` to `now`.
fn changes(base: &str, now: &str) -> Vec<(String, Change)> {
    let base = ratchets(base);
    let now = ratchets(now);
    loosened(&base, &now)
        .into_iter()
        .map(|loose| (loose.name.to_owned(), loose.change))
        .collect()
}

#[test]
fn a_valid_file_reads_each_ratchet_with_its_value_and_direction() {
    let read = ratchets(BASE);

    assert_eq!(
        read.get("warnings").map(|r| (r.value, r.direction)),
        Some((10.0, Direction::Down)),
        "down: lower is better"
    );
    assert_eq!(
        read.get("coverage").map(|r| (r.value, r.direction)),
        Some((80.5, Direction::Up)),
        "up: higher is better"
    );
    assert_eq!(read.len(), 2, "two ratchets");
}

#[test]
fn a_file_without_ratchets_is_valid_and_empty() {
    assert_eq!(ratchets(r#"{"version": 1, "ratchets": {}}"#).len(), 0);
}

#[test]
fn another_version_is_rejected() {
    assert!(
        matches!(
            Ratchets::parse(r#"{"version": 2, "ratchets": {}}"#),
            Err(RatchetError::Version { found: 2 })
        ),
        "version 2 is not read as version 1"
    );
}

#[test]
fn malformed_files_are_rejected_as_json_errors() {
    let cases = [
        (r#"{"ratchets": {}}"#, "no version"),
        (r#"{"version": 1}"#, "no ratchets"),
        (
            r#"{"version": 1, "ratchets": {}, "extra": 0}"#,
            "unknown top-level field",
        ),
        (
            r#"{"version": 1, "ratchets": {"w": {"value": 1, "direction": "down", "x": 0}}}"#,
            "unknown ratchet field",
        ),
        (
            r#"{"version": 1, "ratchets": {"w": {"value": 1, "direction": "sideways"}}}"#,
            "unknown direction",
        ),
        (
            r#"{"version": 1, "ratchets": {"w": {"value": "1", "direction": "down"}}}"#,
            "a string value",
        ),
        (
            r#"{"version": 1, "ratchets": {"w": {"direction": "down"}}}"#,
            "no value",
        ),
        ("not json", "not JSON"),
    ];
    for (text, why) in cases {
        assert!(
            matches!(Ratchets::parse(text), Err(RatchetError::Json(_))),
            "{why}: {text}"
        );
    }
}

#[test]
fn a_name_outside_the_allowed_characters_or_length_is_rejected() {
    let long = "w".repeat(65);
    for name in ["", "has space", "quote\\\"d", long.as_str()] {
        let text = format!(
            "{{\"version\": 1, \"ratchets\": {{\"{name}\": {{\"value\": 1, \"direction\": \"up\"}}}}}}"
        );
        assert!(
            matches!(Ratchets::parse(&text), Err(RatchetError::Name { .. })),
            "{name:?} is not a ratchet name"
        );
    }
    let longest = "w".repeat(64);
    assert_eq!(
        ratchets(&file(&[(&longest, 1.0, "up")])).len(),
        1,
        "64 is allowed"
    );
    assert_eq!(
        ratchets(&file(&[("a-b_c.d9", 1.0, "up")])).len(),
        1,
        "- _ . allowed"
    );
}

#[test]
fn more_than_the_most_ratchets_is_rejected() {
    let names: Vec<String> = (0..=super::MAX_RATCHETS).map(|n| format!("r{n}")).collect();
    let entries: Vec<(&str, f64, &str)> = names.iter().map(|n| (n.as_str(), 1.0, "up")).collect();

    assert!(matches!(
        Ratchets::parse(&file(&entries)),
        Err(RatchetError::TooMany { .. })
    ));
}

#[test]
fn a_down_ratchet_that_rose_and_an_up_ratchet_that_fell_are_loosened() {
    let now = file(&[("warnings", 12.0, "down"), ("coverage", 79.0, "up")]);

    assert_eq!(
        changes(BASE, &now),
        vec![
            (
                "coverage".to_owned(),
                Change::Moved {
                    from: 80.5,
                    to: 79.0,
                    direction: Direction::Up
                }
            ),
            (
                "warnings".to_owned(),
                Change::Moved {
                    from: 10.0,
                    to: 12.0,
                    direction: Direction::Down
                }
            ),
        ]
    );
}

#[test]
fn tightened_and_unchanged_ratchets_are_not_loosened() {
    let tightened = file(&[("warnings", 9.0, "down"), ("coverage", 81.0, "up")]);
    let unchanged = file(&[("warnings", 10.0, "down"), ("coverage", 80.5, "up")]);

    assert_eq!(changes(BASE, &tightened), Vec::new(), "tightened");
    assert_eq!(changes(BASE, &unchanged), Vec::new(), "unchanged");
}

#[test]
fn a_removed_ratchet_and_a_flipped_direction_are_loosened() {
    let now = file(&[("warnings", 10.0, "up")]);

    assert_eq!(
        changes(BASE, &now),
        vec![
            ("coverage".to_owned(), Change::Removed),
            (
                "warnings".to_owned(),
                Change::Redirected {
                    from: Direction::Down,
                    to: Direction::Up
                }
            ),
        ]
    );
}

#[test]
fn a_new_ratchet_is_not_judged() {
    let now = file(&[
        ("warnings", 10.0, "down"),
        ("coverage", 80.5, "up"),
        ("clippy", 400.0, "down"),
    ]);

    assert_eq!(changes(BASE, &now), Vec::new());
}

#[test]
fn each_change_reads_as_one_line_naming_the_ratchet() {
    let now = ratchets(&file(&[("warnings", 12.0, "down")]));
    let base = ratchets(BASE);
    let messages: Vec<String> = loosened(&base, &now)
        .iter()
        .map(super::Loosened::message)
        .collect();

    assert_eq!(
        messages,
        vec![
            "removes ratchet coverage (80.5, up)".to_owned(),
            "raises ratchet warnings from 10 to 12 (down: lower is better)".to_owned(),
        ]
    );
}

#[test]
fn a_working_tree_file_that_cannot_be_read_whole_is_one_whole_file_finding() {
    let found = findings(&ratchets(BASE), Now::TooLarge);

    assert_eq!(
        found
            .iter()
            .map(|f| (f.category, f.file.as_str(), f.line))
            .collect::<Vec<_>>(),
        vec![(Category::RatchetLoosened, RATCHETS_PATH, None)],
        "an oversized file cannot hide a lowered floor"
    );
}

#[test]
fn findings_carry_the_line_of_each_loosened_ratchet() {
    let now = file(&[("warnings", 11.0, "down"), ("coverage", 80.5, "up")]);

    let found = findings(&ratchets(BASE), Now::Text(&now));

    assert_eq!(
        found
            .iter()
            .map(|f| (f.line, f.message.as_str()))
            .collect::<Vec<_>>(),
        vec![(
            Some(1),
            "raises ratchet warnings from 10 to 11 (down: lower is better)"
        )]
    );
}

#[test]
fn the_line_of_a_ratchet_is_where_its_name_is_a_key() {
    assert_eq!(line_of(BASE, "warnings"), Some(4));
    assert_eq!(line_of(BASE, "coverage"), Some(5));
    assert_eq!(line_of(BASE, "missing"), None);
    assert_eq!(
        line_of("{\"note\": \"warnings\",\n\"warnings\" : {}}", "warnings"),
        Some(2),
        "a value that names it is not the key"
    );
}
