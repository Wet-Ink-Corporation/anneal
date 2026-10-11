use std::fs;
use std::path::Path;

use super::{Markdown, MarkdownError, parse_ticket};
use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::dag::tracker::{Tracker, TrackerError};

fn parsed(text: &str) -> Ticket {
    parse_ticket("7", text).expect("valid ticket")
}

fn blockers(ticket: &Ticket) -> Vec<&str> {
    ticket.blocked_by.iter().map(TicketId::as_str).collect()
}

#[test]
fn a_flow_list_of_blockers_is_read_with_quotes_removed() {
    let ticket = parsed("---\ntitle: Add auth\nblocked_by: [001, \"002-db\", 'x']\n---\nBody\n");

    assert_eq!(blockers(&ticket), ["001", "002-db", "x"], "three refs");
    assert_eq!(ticket.title, "Add auth", "title field");
    assert_eq!(ticket.body, "Body\n", "body after the fence");
    assert_eq!(ticket.status, Status::Open, "no status is open");
    assert_eq!(ticket.id.as_str(), "7", "the id it was given");
}

#[test]
fn a_block_list_and_a_comma_list_are_read_too() {
    let block = parsed("---\nblocked_by:\n  - 1\n  - \"2\"\n---\n");
    let comma = parsed("---\nblocked_by: 1, 2\n---\n");
    let empty = parsed("---\nblocked_by: []\n---\n");

    assert_eq!(blockers(&block), ["1", "2"], "block list");
    assert_eq!(blockers(&comma), ["1", "2"], "comma list");
    assert!(blockers(&empty).is_empty(), "empty flow list");
}

#[test]
fn done_and_closed_in_any_case_are_done() {
    for status in ["done", "Closed", " DONE "] {
        let ticket = parsed(&format!("---\nstatus: \"{status}\"\n---\n"));
        assert_eq!(ticket.status, Status::Done, "{status:?}");
    }
    let open = parsed("---\nstatus: in-progress\n---\n");
    assert_eq!(open.status, Status::Open, "anything else is open");
}

#[test]
fn without_frontmatter_a_ticket_is_open_unblocked_and_titled_by_its_heading() {
    let ticket = parsed("intro\n# The heading\ntext\n");

    assert!(ticket.blocked_by.is_empty(), "no blockers");
    assert_eq!(ticket.title, "The heading", "first level-1 heading");
    assert_eq!(
        ticket.body, "intro\n# The heading\ntext\n",
        "all of it is body"
    );
}

#[test]
fn with_no_title_or_heading_the_id_is_the_title() {
    assert_eq!(parsed("plain text\n").title, "7", "falls back to the id");
}

#[test]
fn listed_files_are_kept() {
    let ticket = parsed("---\nfiles: [src/a.rs, docs/b.md]\n---\n");
    assert_eq!(ticket.files, ["src/a.rs", "docs/b.md"], "files list");
}

#[test]
fn malformed_frontmatter_or_lists_report_the_line() {
    let cases = [
        ("---\nblocked_by: [1, 2\n---\n", 2, "unclosed"),
        ("---\nblocked_by:\n  - 1\n  two\n---\n", 2, "`- item`"),
        ("---\njust words\n---\n", 2, "key: value"),
        ("---\ntitle: x\n", 1, "closing"),
    ];
    for (text, line, reason) in cases {
        let error = parse_ticket("7", text).expect_err(text);
        assert_eq!(error.0, line, "{text:?}");
        assert!(error.1.contains(reason), "{text:?}: {}", error.1);
    }
}

fn write(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).expect("write ticket");
}

#[test]
fn the_reader_takes_each_md_file_by_stem_and_skips_the_rest()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    write(dir.path(), "002.md", "---\nblocked_by: [001]\n---\n");
    write(dir.path(), "001.md", "# First\n");
    write(dir.path(), "README.md", "# About tickets\n");
    write(dir.path(), "notes.txt", "not a ticket\n");
    // A directory named like a ticket is not one.
    fs::create_dir(dir.path().join("sub.md"))?;

    let tickets = Markdown::new(dir.path()).read()?;

    let ids: Vec<&str> = tickets.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        ["001", "002"],
        "file stems, sorted; README and non-files skipped"
    );
    Ok(())
}

#[test]
fn a_bad_ticket_is_an_error_naming_its_file_and_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "1.md", "---\nblocked_by: [2\n---\n");

    let result = Markdown::new(dir.path()).read();

    assert!(
        matches!(
            &result,
            Err(TrackerError::Markdown(MarkdownError::Ticket { path, line: 2, .. }))
                if path.ends_with("1.md")
        ),
        "got {result:?}"
    );
}

#[test]
fn a_missing_directory_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");

    let result = Markdown::new(dir.path().join("absent")).read();

    assert!(
        matches!(
            &result,
            Err(TrackerError::Markdown(MarkdownError::ReadDir { .. }))
        ),
        "got {result:?}"
    );
}

#[test]
fn an_oversized_ticket_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let size = usize::try_from(super::MAX_TICKET_BYTES + 1).expect("fits");
    write(dir.path(), "1.md", &"x".repeat(size));

    let result = Markdown::new(dir.path()).read();

    assert!(
        matches!(
            &result,
            Err(TrackerError::Markdown(MarkdownError::TooLarge { .. }))
        ),
        "got {result:?}"
    );
}
