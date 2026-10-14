use super::{Clause, EarsError, Pattern, parse, template};

fn pattern(clauses: &[Clause]) -> Pattern {
    Pattern {
        clauses: clauses.to_vec(),
    }
}

#[test]
fn each_single_pattern_parses_to_its_clauses() {
    let cases = [
        ("THE exporter SHALL write UTF-8.", pattern(&[])),
        (
            "WHEN the user saves, THE editor SHALL store the file.",
            pattern(&[Clause::When]),
        ),
        (
            "WHILE offline, THE app SHALL queue writes.",
            pattern(&[Clause::While]),
        ),
        (
            "IF the disk is full, THEN THE app SHALL show an error.",
            pattern(&[Clause::If]),
        ),
        (
            "WHERE sync is enabled, THE app SHALL upload changes.",
            pattern(&[Clause::Where]),
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(parse(text), Ok(expected), "{text}");
    }
}

#[test]
fn the_pattern_is_named_as_ears_names_it() {
    let names: Vec<&str> = [
        pattern(&[]),
        pattern(&[Clause::When]),
        pattern(&[Clause::While]),
        pattern(&[Clause::If]),
        pattern(&[Clause::Where]),
        pattern(&[Clause::While, Clause::When]),
    ]
    .iter()
    .map(Pattern::name)
    .collect();

    assert_eq!(
        names,
        [
            "ubiquitous",
            "event-driven",
            "state-driven",
            "unwanted-behaviour",
            "optional-feature",
            "complex"
        ],
        "one name per form"
    );
}

#[test]
fn combined_forms_parse_in_ears_order() {
    assert_eq!(
        parse("WHERE sync is on, WHILE offline, WHEN a file changes, THE app SHALL queue it."),
        Ok(pattern(&[Clause::Where, Clause::While, Clause::When])),
        "where, while, when"
    );
    assert_eq!(
        parse("WHILE uploading IF the link drops THEN THE app SHALL retry"),
        Ok(pattern(&[Clause::While, Clause::If])),
        "commas are optional"
    );
}

#[test]
fn lowercase_words_inside_clauses_are_text_not_keywords() {
    assert_eq!(
        parse("WHEN the user picks the file where the data is, THE app SHALL then open it."),
        Ok(pattern(&[Clause::When])),
        "only uppercase words are keywords"
    );
}

#[test]
fn a_criterion_without_shall_is_missing_shall() {
    assert_eq!(
        parse("WHEN the user saves, THE editor stores the file."),
        Err(EarsError::MissingShall),
        "THE but no SHALL"
    );
    assert_eq!(
        parse("WHEN the user saves"),
        Err(EarsError::MissingShall),
        "no main clause at all"
    );
}

#[test]
fn a_criterion_that_starts_with_another_word_names_it() {
    assert_eq!(
        parse("The editor SHALL store the file."),
        Err(EarsError::UnknownStart {
            word: "The".to_owned()
        }),
        "keywords are uppercase"
    );
    assert_eq!(
        parse("Users can export a report."),
        Err(EarsError::UnknownStart {
            word: "Users".to_owned()
        }),
        "prose"
    );
}

#[test]
fn an_empty_criterion_is_empty() {
    assert_eq!(parse("  "), Err(EarsError::Empty), "blank");
}

#[test]
fn after_then_a_lowercase_the_is_reported_as_what_was_expected() {
    assert_eq!(
        parse("IF the disk is full THEN the app SHALL warn."),
        Err(EarsError::Expected {
            after: "THEN",
            expected: "THE <system>",
            found: "the".to_owned(),
        }),
        "THEN was read; the main clause's THE was expected next"
    );
}

#[test]
fn if_needs_then_and_then_needs_if() {
    assert_eq!(
        parse("IF the disk is full, THE app SHALL warn."),
        Err(EarsError::MissingThen),
        "IF without THEN"
    );
    assert_eq!(
        parse("WHEN the disk is full, THEN THE app SHALL warn."),
        Err(EarsError::StrayThen),
        "THEN without IF"
    );
    assert_eq!(
        parse("THEN THE app SHALL warn."),
        Err(EarsError::StrayThen),
        "THEN first"
    );
}

#[test]
fn clauses_out_of_ears_order_or_repeated_are_named() {
    assert_eq!(
        parse("WHEN saving, WHILE offline, THE app SHALL queue."),
        Err(EarsError::OutOfOrder {
            clause: Clause::While,
            after: Clause::When
        }),
        "WHILE comes before WHEN"
    );
    assert_eq!(
        parse("WHEN saving, WHEN offline, THE app SHALL queue."),
        Err(EarsError::Repeated(Clause::When)),
        "one WHEN"
    );
    assert_eq!(
        parse("WHEN saving, IF offline, THEN THE app SHALL queue."),
        Err(EarsError::TwoTriggers),
        "WHEN or IF, not both"
    );
}

#[test]
fn empty_parts_are_named() {
    assert_eq!(
        parse("WHEN, THE app SHALL queue."),
        Err(EarsError::EmptyClause(Clause::When)),
        "no trigger text"
    );
    assert_eq!(
        parse("WHEN THE USER saves, THE app SHALL queue."),
        Err(EarsError::EmptyClause(Clause::When)),
        "THE is reserved, even inside a clause"
    );
    assert_eq!(
        parse("THE SHALL queue."),
        Err(EarsError::MissingSystem),
        "no system"
    );
    assert_eq!(
        parse("WHEN saving, THE app SHALL."),
        Err(EarsError::EmptyResponse),
        "no response"
    );
}

#[test]
fn the_closest_template_follows_the_leading_keywords_in_any_case() {
    let cases = [
        (
            "When the user saves the editor stores it",
            "WHEN <trigger>, THE <system> SHALL <response>.",
        ),
        (
            "while offline, if the link drops the app retries",
            "WHILE <state>, IF <condition>, THEN THE <system> SHALL <response>.",
        ),
        (
            "Users can export a report where needed",
            "THE <system> SHALL <response>.",
        ),
        (
            "where sync is on, when, when a file changes, upload it",
            "WHERE <feature>, WHEN <trigger>, THE <system> SHALL <response>.",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(template(text), expected, "{text}");
    }
}
