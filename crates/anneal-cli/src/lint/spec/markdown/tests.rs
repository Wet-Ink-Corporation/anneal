use super::{Document, Item, Row, Table, heading, items, tables};

fn item(line: usize, text: &str) -> Item {
    Item {
        line,
        text: text.to_owned(),
    }
}

#[test]
fn fenced_code_and_html_comments_are_not_visible() {
    let doc = Document::parse(
        "a <!-- hidden --> b\n<!-- start\nstill hidden\nend --> c\n```md\n## Not a heading\n```\n~~~\nx\n~~~\nd\n",
    );

    let visible: Vec<&str> = doc.lines().iter().map(|line| line.text.as_ref()).collect();

    assert_eq!(
        visible,
        ["a  b", "", "", " c", "", "", "", "", "", "", "d"],
        "comments and fences blanked, line numbers kept"
    );
}

#[test]
fn a_longer_fence_needs_a_closing_fence_as_long() {
    let doc = Document::parse("````\n```\nhidden\n````\nshown\n");

    let visible: Vec<&str> = doc.lines().iter().map(|line| line.text.as_ref()).collect();

    assert_eq!(
        visible,
        ["", "", "", "", "shown"],
        "``` does not close ````"
    );
}

#[test]
fn headings_have_a_level_and_a_title() {
    assert_eq!(heading("## Out of Scope"), Some((2, "Out of Scope")), "h2");
    assert_eq!(
        heading("### Complexity ###"),
        Some((3, "Complexity")),
        "closing #s"
    );
    assert_eq!(heading("#hashtag"), None, "no space");
    assert_eq!(heading("####### seven"), None, "too deep");
    assert_eq!(
        heading("## C#"),
        Some((2, "C#")),
        "a # inside the title stays"
    );
}

#[test]
fn sections_are_level_2_headings_with_their_bodies() {
    let doc = Document::parse("# Spec\nintro\n## Problem\np\n### Sub\ns\n## Users\nu\n");

    let sections: Vec<(&str, usize, usize)> = doc
        .sections()
        .iter()
        .map(|section| (section.title, section.line, section.body.len()))
        .collect();

    assert_eq!(
        sections,
        [("Problem", 3, 3), ("Users", 7, 1)],
        "title, heading line, body lines up to the next level-2 heading"
    );
}

#[test]
fn list_items_join_their_indented_continuation_lines() {
    let doc = Document::parse(
        "- WHEN a user saves,\n  THE app SHALL store it.\n* second\n1. third\n2) fourth\n-\nplain\n- fifth\n\n  not joined\n",
    );

    let found = items(doc.lines());

    assert_eq!(
        found,
        [
            item(1, "WHEN a user saves, THE app SHALL store it."),
            item(3, "second"),
            item(4, "third"),
            item(5, "fourth"),
            item(6, ""),
            item(8, "fifth"),
        ],
        "markers -, *, 1., 2); blank lines end an item"
    );
}

#[test]
fn bold_text_and_rules_are_not_list_items() {
    let doc = Document::parse("**AC1**: x\n---\n*emphasis*\n");

    assert_eq!(items(doc.lines()), [], "no item");
}

#[test]
fn tables_have_a_header_and_rows_with_their_lines() {
    let doc = Document::parse(
        "text\n| Gate | Violation | Justification |\n|---|:-:|---|\n| G1 | two stores | needed |\n|G2||\nafter\n",
    );

    let found = tables(doc.lines());

    assert_eq!(
        found,
        [Table {
            header: vec!["Gate", "Violation", "Justification"],
            rows: vec![
                Row {
                    line: 4,
                    cells: vec!["G1", "two stores", "needed"],
                },
                Row {
                    line: 5,
                    cells: vec!["G2", ""],
                },
            ],
        }],
        "separator row skipped, short rows kept"
    );
}
