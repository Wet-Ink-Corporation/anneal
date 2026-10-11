use std::error::Error;
use std::fs;

use super::{Adr, AdrError, Placement, next, ordered, place, slug};

type TestResult = Result<(), Box<dyn Error>>;

/// An ADR with the canonical file name, as a new one gets.
fn adr(number: u32, slug: &str) -> Adr {
    Adr::numbered(number, slug.to_owned())
}

#[test]
fn a_parsed_adr_keeps_its_file_name_as_written() -> TestResult {
    for name in [
        "0007-use-postgres.md",
        "00012-extra-padding.md",
        "12345-big.md",
        "0003-Use-Postgres.md",
    ] {
        let parsed = Adr::parse(name)?.map(|adr| adr.file_name().to_owned());

        assert_eq!(
            parsed.as_deref(),
            Some(name),
            "parse then file_name round-trips"
        );
    }
    Ok(())
}

#[test]
fn a_title_becomes_a_lowercase_ascii_kebab_slug() -> TestResult {
    let cases = [
        ("Use Postgres", "use-postgres"),
        ("  Store events, not state!  ", "store-events-not-state"),
        ("ADR #12: v2 API", "adr-12-v2-api"),
        ("Café au lait", "caf-au-lait"),
        ("snake_case-and--dashes", "snake-case-and-dashes"),
    ];
    for (title, expected) in cases {
        assert_eq!(slug(title)?, expected, "slug of {title:?}");
    }
    Ok(())
}

#[test]
fn a_title_with_no_ascii_letter_or_digit_has_no_slug() {
    let result = slug(" -- ! ");

    assert!(
        matches!(&result, Err(AdrError::EmptySlug { title }) if title == " -- ! "),
        "the error names the title: {result:?}"
    );
}

#[test]
fn an_adr_file_name_is_four_or_more_digits_a_dash_a_slug_and_md() -> TestResult {
    assert_eq!(
        Adr::parse("0007-use-postgres.md")?,
        Some(adr(7, "use-postgres")),
        "padded"
    );
    assert_eq!(
        Adr::parse("12345-big.md")?,
        Some(adr(12345, "big")),
        "wider"
    );
    for other in [
        "README.md",
        "007-short.md",
        "0007-.md",
        "0007-x.txt",
        "x0007-y.md",
        "0007.md",
    ] {
        assert_eq!(Adr::parse(other)?, None, "{other} is not an ADR");
    }
    Ok(())
}

#[test]
fn an_adr_number_too_large_to_count_is_an_error_naming_the_file() {
    let result = Adr::parse("99999999999-huge.md");

    assert!(
        matches!(&result, Err(AdrError::Number { name, .. }) if name == "99999999999-huge.md"),
        "{result:?}"
    );
}

#[test]
fn adrs_are_ordered_by_number_then_slug_whatever_the_listing_order() -> TestResult {
    // Two branches both took 0002; after the merge both files exist.
    let names = [
        "0002-zebra.md",
        "README.md",
        "0010-later.md",
        "0002-alpha.md",
        "0001-first.md",
    ];
    let mut reversed = names;
    reversed.reverse();

    let forward = ordered(names)?;
    let backward = ordered(reversed)?;

    assert_eq!(
        forward,
        [
            adr(1, "first"),
            adr(2, "alpha"),
            adr(2, "zebra"),
            adr(10, "later")
        ],
        "number first, then slug; other files ignored"
    );
    assert_eq!(backward, forward, "the input order does not matter");
    Ok(())
}

#[test]
fn a_new_title_takes_the_next_number_after_the_highest() -> TestResult {
    let existing = [
        adr(1, "first"),
        adr(2, "alpha"),
        adr(2, "zebra"),
        adr(7, "gap-before"),
    ];

    let placement = next(&existing, "Use Postgres")?;

    assert_eq!(
        placement,
        Placement::New(adr(8, "use-postgres")),
        "max + 1; gaps are never reused"
    );
    assert_eq!(
        placement.adr().file_name(),
        "0008-use-postgres.md",
        "zero-padded to four"
    );
    assert_eq!(
        next(&[], "First")?,
        Placement::New(adr(1, "first")),
        "0001 when none"
    );
    Ok(())
}

#[test]
fn a_title_already_recorded_returns_its_existing_adr() -> TestResult {
    let existing = [adr(1, "Use-Postgres"), adr(2, "other")];

    let placement = next(&existing, "use postgres")?;

    assert_eq!(
        placement,
        Placement::Existing(adr(1, "Use-Postgres")),
        "idempotent"
    );
    Ok(())
}

#[test]
fn the_highest_possible_number_cannot_be_followed() {
    let result = next(&[adr(u32::MAX, "last")], "one more");

    assert!(matches!(result, Err(AdrError::Exhausted)), "{result:?}");
}

#[test]
fn placing_reads_the_directory_and_is_idempotent() -> TestResult {
    let dir = tempfile::tempdir()?;
    let adrs = dir.path().join("docs/adr");
    fs::create_dir_all(&adrs)?;
    fs::write(adrs.join("0001-first.md"), "# First\n")?;
    fs::write(adrs.join("0003-third.md"), "# Third\n")?;

    let first = place(&adrs, "Use Postgres")?;
    let again = place(&adrs, "Use Postgres")?;
    fs::write(adrs.join(first.adr().file_name()), "# Use Postgres\n")?;
    let recorded = place(&adrs, "Use Postgres")?;

    assert_eq!(first, Placement::New(adr(4, "use-postgres")), "after 0003");
    assert_eq!(again, first, "nothing written, same answer");
    assert_eq!(
        recorded,
        Placement::Existing(adr(4, "use-postgres")),
        "written, found"
    );
    Ok(())
}

#[test]
fn a_missing_directory_has_no_adrs_yet() -> TestResult {
    let dir = tempfile::tempdir()?;

    let placement = place(&dir.path().join("docs/adr"), "First")?;

    assert_eq!(placement, Placement::New(adr(1, "first")), "0001");
    Ok(())
}
