//! A parsed `SKILL.md`: frontmatter, body lines with their section and fence state, and helpers
//! the checks share.

use crate::lint::finding::Location;
use crate::lint::skills::discover::SkillDir;
use crate::lint::skills::frontmatter::Document;

/// One body line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line<'a> {
    /// 1-based file line.
    pub no: usize,
    /// The text, without its line break.
    pub text: &'a str,
    /// True inside a fenced code block (fence lines included).
    pub in_fence: bool,
    /// The title of the level-2 section the line is in, if any.
    pub section: Option<&'a str>,
}

/// A level-2 heading (`## Title`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Section<'a> {
    /// The heading text after `## `.
    pub title: &'a str,
    /// 1-based file line.
    pub line: usize,
}

/// A skill whose `SKILL.md` parsed.
#[derive(Debug)]
pub(crate) struct Skill<'a> {
    /// Its directory.
    pub dir: &'a SkillDir,
    /// The `SKILL.md` path as printed in findings.
    pub file: &'a str,
    /// The file's size in bytes.
    pub size: u64,
    /// Frontmatter and body.
    pub doc: Document<'a>,
    /// Body lines in order.
    pub lines: Vec<Line<'a>>,
    /// Level-2 headings in order, outside code fences.
    pub sections: Vec<Section<'a>>,
}

impl<'a> Skill<'a> {
    /// Indexes the body of `doc`.
    pub(crate) fn new(dir: &'a SkillDir, file: &'a str, size: u64, doc: Document<'a>) -> Self {
        let (lines, sections) = index_body(doc.body, doc.body_line);
        Self {
            dir,
            file,
            size,
            doc,
            lines,
            sections,
        }
    }

    /// A location in this skill's `SKILL.md`.
    pub(crate) fn at(&self, line: usize) -> Location {
        Location::new(self.file, line)
    }

    /// The line of frontmatter key `key`, or 1 when it is absent.
    pub(crate) fn field_line(&self, key: &str) -> usize {
        self.doc.field(key).map_or(1, |f| f.line)
    }

    /// Body lines outside code fences.
    pub(crate) fn prose(&self) -> impl Iterator<Item = &Line<'a>> + '_ {
        self.lines.iter().filter(|l| !l.in_fence)
    }

    /// True when `disable-model-invocation` is `true`.
    pub(crate) fn model_invocation_disabled(&self) -> bool {
        self.doc.value("disable-model-invocation") == "true"
    }

    /// The part of the description before the "Do NOT use for" clause: its trigger.
    pub(crate) fn trigger(&self) -> &str {
        let description = self.doc.value("description");
        description
            .split_once(super::checker::NOT_FOR_CLAUSE)
            .map_or(description, |(trigger, _)| trigger)
    }
}

fn index_body(body: &str, first_line: usize) -> (Vec<Line<'_>>, Vec<Section<'_>>) {
    let mut lines = Vec::new();
    let mut sections = Vec::new();
    let mut open_fence = false;
    let mut section = None;
    for (offset, text) in body.lines().enumerate() {
        let no = first_line.saturating_add(offset);
        let marker = text.trim_start().starts_with("```") || text.trim_start().starts_with("~~~");
        if marker {
            open_fence = !open_fence;
        }
        let fenced = open_fence || marker;
        if !fenced && let Some(title) = text.strip_prefix("## ") {
            let title = title.trim();
            sections.push(Section { title, line: no });
            section = Some(title);
        }
        lines.push(Line {
            no,
            text,
            in_fence: fenced,
            section,
        });
    }
    (lines, sections)
}

/// Words of `text`: lowercase ASCII-alphanumeric runs (apostrophes kept inside words).
pub(crate) fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\'').to_lowercase())
        .filter(|w| !w.is_empty())
}

/// True when `text` holds `word` as a whole, case-sensitive token.
pub(crate) fn has_token(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|t| t == word)
}

/// The spans between backticks on one line.
pub(crate) fn code_spans(text: &str) -> impl Iterator<Item = &str> {
    text.split('`').skip(1).step_by(2)
}

#[cfg(test)]
mod tests {
    use super::{code_spans, has_token, index_body, words};

    #[test]
    fn body_lines_know_their_section_and_fence() {
        let body = "## Overview\ntext\n```\n## not a heading\n```\n## Process\n1. step\n";
        let (lines, sections) = index_body(body, 10);

        let titles: Vec<(&str, usize)> = sections.iter().map(|s| (s.title, s.line)).collect();
        assert_eq!(
            titles,
            [("Overview", 10), ("Process", 15)],
            "fenced ## skipped"
        );
        let fenced: Vec<usize> = lines.iter().filter(|l| l.in_fence).map(|l| l.no).collect();
        assert_eq!(fenced, [12, 13, 14], "fence lines and content");
        assert_eq!(
            lines.last().and_then(|l| l.section),
            Some("Process"),
            "last line is in Process"
        );
    }

    #[test]
    fn word_and_token_helpers_split_as_documented() {
        let found: Vec<String> = words("It's a Small-change, MUST!").collect();
        assert_eq!(
            found,
            ["it's", "a", "small", "change", "must"],
            "lowercased words"
        );
        assert!(has_token("you MUST run", "MUST"), "whole token");
        assert!(!has_token("MUSTARD", "MUST"), "not a substring");
        let spans: Vec<&str> = code_spans("run `anneal lint` then `guard`").collect();
        assert_eq!(spans, ["anneal lint", "guard"], "backtick spans");
    }
}
