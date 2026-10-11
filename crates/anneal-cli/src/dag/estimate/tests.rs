use std::fs;
use std::path::Path;

use super::{Basis, MAX_FILE_TOKENS, MAX_FILES, UNKNOWN_FILE_TOKENS, estimate, named_files};
use crate::dag::ticket::{Status, Ticket, TicketId};

fn ticket(body: &str, files: &[&str]) -> Ticket {
    Ticket {
        id: TicketId::new("1"),
        title: "t".to_owned(),
        body: body.to_owned(),
        blocked_by: Vec::new(),
        files: files.iter().map(|f| (*f).to_owned()).collect(),
        status: Status::Open,
    }
}

fn nowhere() -> &'static Path {
    Path::new("/nonexistent-anneal-estimate-root")
}

#[test]
fn the_body_costs_its_characters_over_four_rounded_up() {
    let estimate = estimate(&ticket("abcde", &[]), nowhere());

    assert_eq!(estimate.body_tokens, 2, "5 chars -> 2 tokens");
    assert_eq!(estimate.tokens, 2, "no files");
    assert!(estimate.files.is_empty(), "nothing named");
}

#[test]
fn characters_not_bytes_are_counted() {
    let estimate = estimate(&ticket("ééééé", &[]), nowhere());
    assert_eq!(
        estimate.body_tokens, 2,
        "5 chars of 2 bytes each -> 2 tokens"
    );
}

#[test]
fn backticked_paths_in_the_body_and_listed_files_are_named_once_each() {
    let body = "Change `src/dag.rs` and `Cargo.toml:12`, see `src/dag.rs` again. \
                Not `--budget`, `1.5`, `two words`, `https://x.y/z` or `cargo test`.";

    let ticket = ticket(body, &["docs/dag.md", "src/dag.rs"]);

    let named = named_files(&ticket);

    assert_eq!(
        named,
        ["docs/dag.md", "src/dag.rs", "Cargo.toml"],
        "listed files first, then body mentions, line suffix dropped, no repeats"
    );
}

#[test]
fn a_file_that_exists_costs_its_size_over_four_and_others_the_default() -> std::io::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(root.path().join("src/small.rs"), "x".repeat(401))?;

    let estimate = estimate(
        &ticket("", &["src/small.rs", "src/missing.rs"]),
        root.path(),
    );

    let costs: Vec<(&str, u64, Basis)> = estimate
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.tokens, f.basis))
        .collect();
    assert_eq!(
        costs,
        [
            ("src/small.rs", 101, Basis::Size),
            ("src/missing.rs", UNKNOWN_FILE_TOKENS, Basis::Default),
        ],
        "401 bytes -> 101 tokens; a missing file costs the default"
    );
    assert_eq!(estimate.tokens, 101 + UNKNOWN_FILE_TOKENS, "summed");
    Ok(())
}

#[test]
fn a_large_file_is_capped() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let bytes = usize::try_from(MAX_FILE_TOKENS * 8)?;
    fs::write(root.path().join("big.txt"), vec![b'x'; bytes])?;

    let estimate = estimate(&ticket("", &["big.txt"]), root.path());

    assert_eq!(estimate.tokens, MAX_FILE_TOKENS, "capped");
    Ok(())
}

#[test]
fn a_path_outside_the_root_costs_the_default_without_being_read() -> std::io::Result<()> {
    let root = tempfile::tempdir()?;
    let inner = root.path().join("inner");
    fs::create_dir(&inner)?;
    fs::write(root.path().join("outside.txt"), "x".repeat(40_000))?;
    let absolute = root.path().join("outside.txt");

    let estimate = estimate(
        &ticket("", &["../outside.txt", &absolute.to_string_lossy()]),
        &inner,
    );

    assert!(
        estimate.files.iter().all(|f| f.basis == Basis::Default),
        "never sized: {:?}",
        estimate.files
    );
    Ok(())
}

#[test]
fn at_most_max_files_are_counted() {
    let files: Vec<String> = (0..MAX_FILES + 10).map(|n| format!("f{n}.rs")).collect();
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();

    let estimate = estimate(&ticket("", &refs), nowhere());

    assert_eq!(estimate.files.len(), MAX_FILES, "capped list");
    let expected = UNKNOWN_FILE_TOKENS * u64::try_from(MAX_FILES).expect("fits");
    assert_eq!(estimate.tokens, expected, "each at the default");
}
