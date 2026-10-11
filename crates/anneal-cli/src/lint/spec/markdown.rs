//! The markdown a spec is read with, by hand: what is visible, level-2 sections, list items and
//! pipe tables. Only what `lint spec` needs; see `docs/spec.md`.
//!
//! Fenced code blocks (```` ``` ```` or `~~~`, closed by a fence of the same character at least
//! as long) and HTML comments (`<!-- -->`, across lines too) are not visible: their lines read
//! as blank, so line numbers stay those of the file.

use std::borrow::Cow;

/// One line of the document as the checks see it.
#[derive(Debug)]
pub(crate) struct Line<'a> {
    /// 1-based line number in the file.
    pub(crate) number: usize,
    /// The visible text: blank inside a code fence, comments removed.
    pub(crate) text: Cow<'a, str>,
}

/// A document read into visible lines.
#[derive(Debug)]
pub(crate) struct Document<'a> {
    lines: Vec<Line<'a>>,
}

/// A `## <title>` heading and the lines up to the next level-1 or level-2 heading.
#[derive(Debug)]
pub(crate) struct Section<'d> {
    /// The heading text.
    pub(crate) title: &'d str,
    /// The heading's line.
    pub(crate) line: usize,
    /// The lines after the heading.
    pub(crate) body: &'d [Line<'d>],
}

/// A list item: its first line and its text, continuation lines joined with a space.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Item {
    /// The line of the item's marker.
    pub(crate) line: usize,
    /// The text after the marker, trimmed.
    pub(crate) text: String,
}

/// A pipe table: its header cells and the rows after the separator row.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Table<'d> {
    /// The header cells, trimmed.
    pub(crate) header: Vec<&'d str>,
    /// The body rows.
    pub(crate) rows: Vec<Row<'d>>,
}

/// One body row of a table.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Row<'d> {
    /// Its line.
    pub(crate) line: usize,
    /// Its cells, trimmed.
    pub(crate) cells: Vec<&'d str>,
}

impl<'a> Document<'a> {
    /// Reads `text` line by line. Never fails: anything unrecognised is plain text.
    pub(crate) fn parse(text: &'a str) -> Self {
        let mut scanner = Scanner::default();
        let lines = text
            .lines()
            .enumerate()
            .map(|(index, raw)| Line {
                number: index.saturating_add(1),
                text: scanner.visible(raw),
            })
            .collect();
        Self { lines }
    }

    /// Every line, in order.
    pub(crate) fn lines(&self) -> &[Line<'a>] {
        &self.lines
    }

    /// The level-2 sections, in order. A section's body ends at the next level-1 or level-2
    /// heading; its bounds are line indices of `self.lines`, so the slices are always in range.
    pub(crate) fn sections(&self) -> Vec<Section<'_>> {
        let mut sections = Vec::new();
        let mut open: Option<(&str, usize, usize)> = None;
        for (index, line) in self.lines.iter().enumerate() {
            let Some((level, title)) = heading(&line.text) else {
                continue;
            };
            if level > 2 {
                continue;
            }
            if let Some((title, heading_line, start)) = open.take() {
                sections.push(Section {
                    title,
                    line: heading_line,
                    body: self.lines.get(start..index).unwrap_or_default(),
                });
            }
            if level == 2 {
                open = Some((title, line.number, index.saturating_add(1)));
            }
        }
        if let Some((title, heading_line, start)) = open {
            sections.push(Section {
                title,
                line: heading_line,
                body: self.lines.get(start..).unwrap_or_default(),
            });
        }
        sections
    }
}

/// Fence and comment state while reading lines.
#[derive(Debug, Default)]
struct Scanner {
    fence: Option<Fence>,
    in_comment: bool,
}

impl Scanner {
    /// The visible text of `raw`, updating the fence and comment state.
    fn visible<'a>(&mut self, raw: &'a str) -> Cow<'a, str> {
        if let Some(fence) = self.fence {
            if fence.is_closed_by(raw) {
                self.fence = None;
            }
            return Cow::Borrowed("");
        }
        if !self.in_comment
            && let Some(fence) = Fence::opened_by(raw)
        {
            self.fence = Some(fence);
            return Cow::Borrowed("");
        }
        if !self.in_comment && !raw.contains("<!--") {
            return Cow::Borrowed(raw);
        }
        let mut shown = String::new();
        let mut rest = raw;
        loop {
            if self.in_comment {
                let Some((_, after)) = rest.split_once("-->") else {
                    break;
                };
                self.in_comment = false;
                rest = after;
            } else if let Some((before, after)) = rest.split_once("<!--") {
                shown.push_str(before);
                self.in_comment = true;
                rest = after;
            } else {
                shown.push_str(rest);
                break;
            }
        }
        Cow::Owned(shown)
    }
}

/// An open code fence: its character and length.
#[derive(Debug, Clone, Copy)]
struct Fence {
    mark: char,
    len: usize,
}

impl Fence {
    /// The fence `raw` opens: three or more backticks or tildes after any indentation.
    fn opened_by(raw: &str) -> Option<Self> {
        let trimmed = raw.trim_start();
        let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let len = trimmed.chars().take_while(|c| *c == mark).count();
        (len >= 3).then_some(Self { mark, len })
    }

    /// Whether `raw` closes this fence: only the fence character, at least as many.
    fn is_closed_by(self, raw: &str) -> bool {
        let trimmed = raw.trim();
        trimmed.chars().count() >= self.len && trimmed.chars().all(|c| c == self.mark)
    }
}

/// The level and title of an ATX heading (`## Title`, optionally closed by `#`s).
pub(crate) fn heading(text: &str) -> Option<(usize, &str)> {
    let trimmed = text.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = trimmed.get(level..)?;
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let title = rest.trim();
    let unclosed = title.trim_end_matches('#');
    if unclosed.is_empty() || unclosed.ends_with([' ', '\t']) {
        Some((level, unclosed.trim_end()))
    } else {
        Some((level, title))
    }
}

/// The list items of `lines`: a line starting with `-`, `*` or `+`, or with digits and `.` or
/// `)`, then a space or the line end. Indented lines that follow join the item; a blank line
/// or any other line ends it.
pub(crate) fn items(lines: &[Line<'_>]) -> Vec<Item> {
    let mut found: Vec<Item> = Vec::new();
    let mut open = false;
    for line in lines {
        let text = line.text.as_ref();
        if let Some(content) = item_start(text) {
            found.push(Item {
                line: line.number,
                text: content.trim().to_owned(),
            });
            open = true;
        } else if open && text.starts_with([' ', '\t']) && !text.trim().is_empty() {
            if let Some(last) = found.last_mut() {
                if !last.text.is_empty() {
                    last.text.push(' ');
                }
                last.text.push_str(text.trim());
            }
        } else {
            open = false;
        }
    }
    found
}

/// The text after a list marker, when `text` starts a list item.
fn item_start(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let rest = if let Some(rest) = trimmed.strip_prefix(['-', '*', '+']) {
        rest
    } else {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        trimmed.get(digits..)?.strip_prefix(['.', ')'])?
    };
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then_some(rest)
}

/// The pipe tables of `lines`: runs of lines starting with `|`, whose second line is a
/// separator row (`|---|:-:|`).
pub(crate) fn tables<'d>(lines: &'d [Line<'d>]) -> Vec<Table<'d>> {
    lines
        .chunk_by(|a, b| is_row(a) == is_row(b))
        .filter_map(|run| match run {
            [header, separator, rows @ ..] if is_row(header) && is_separator(separator) => {
                Some(Table {
                    header: cells(header),
                    rows: rows
                        .iter()
                        .map(|row| Row {
                            line: row.number,
                            cells: cells(row),
                        })
                        .collect(),
                })
            }
            _ => None,
        })
        .collect()
}

fn is_row(line: &Line<'_>) -> bool {
    line.text.trim_start().starts_with('|')
}

fn is_separator(line: &Line<'_>) -> bool {
    cells(line)
        .iter()
        .all(|cell| cell.contains('-') && cell.chars().all(|c| matches!(c, '-' | ':' | ' ')))
}

/// The trimmed cells of a table row, without its outer pipes.
fn cells<'d>(line: &'d Line<'d>) -> Vec<&'d str> {
    let trimmed = line.text.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(str::trim).collect()
}

#[cfg(test)]
mod tests;
