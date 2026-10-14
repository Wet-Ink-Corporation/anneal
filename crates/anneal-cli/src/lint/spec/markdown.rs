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
        Cow::Owned(self.strip_comments(raw))
    }

    /// `raw` without its HTML comments, read left to right as `CommonMark` does: whichever of a
    /// code span and a comment opens first wins, and the other's markers inside it are text.
    /// One pass: every search starts where the last one stopped (PERF-10).
    fn strip_comments(&mut self, raw: &str) -> String {
        const OPEN: &str = "<!--";
        const CLOSE: &str = "-->";
        let runs = backtick_runs(raw);
        let mut shown = String::new();
        let mut pos = 0;
        let mut run = 0;
        let mut opener: Option<usize> = None;
        loop {
            if self.in_comment {
                let Some(end) = raw.get(pos..).and_then(|rest| rest.find(CLOSE)) else {
                    return shown;
                };
                pos = pos.saturating_add(end).saturating_add(CLOSE.len());
                self.in_comment = false;
                continue;
            }
            let at = match opener {
                Some(at) if at >= pos => at,
                _ => {
                    let Some(found) = raw.get(pos..).and_then(|rest| rest.find(OPEN)) else {
                        shown.push_str(raw.get(pos..).unwrap_or_default());
                        return shown;
                    };
                    pos.saturating_add(found)
                }
            };
            opener = Some(at);
            while runs.get(run).is_some_and(|&(start, _)| start < pos) {
                run = run.saturating_add(1);
            }
            match runs.get(run) {
                Some(&(start, len)) if start < at => {
                    let partner = partner(&runs, run);
                    // Without a partner the run is literal backticks.
                    let literal_end = start.saturating_add(len);
                    let end = partner
                        .and_then(|index| runs.get(index))
                        .map_or(literal_end, |&(close, _)| close.saturating_add(len));
                    shown.push_str(raw.get(pos..end).unwrap_or_default());
                    pos = end;
                    run = partner.unwrap_or(run).saturating_add(1);
                }
                _ => {
                    shown.push_str(raw.get(pos..at).unwrap_or_default());
                    pos = at.saturating_add(OPEN.len());
                    self.in_comment = true;
                }
            }
        }
    }
}

/// The index of the run that closes the code span `runs[index]` opens: the next run of the
/// same length, if any.
fn partner(runs: &[(usize, usize)], index: usize) -> Option<usize> {
    let &(_, len) = runs.get(index)?;
    let after = index.saturating_add(1);
    runs.get(after..)?
        .iter()
        .position(|&(_, other)| other == len)
        .map(|offset| after.saturating_add(offset))
}

/// An open code fence: its character and length.
#[derive(Debug, Clone, Copy)]
struct Fence {
    mark: char,
    len: usize,
}

impl Fence {
    /// The fence `raw` opens: after any indentation (a fence inside a list item is indented
    /// further than `CommonMark`'s three spaces), three or more backticks or tildes; a backtick
    /// fence's info string holds no backtick, so a line that is an inline span, such as
    /// ```` ```x``` ````, is text.
    fn opened_by(raw: &str) -> Option<Self> {
        let trimmed = raw.trim_start();
        let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let len = trimmed.chars().take_while(|c| *c == mark).count();
        let info = trimmed.get(len..)?;
        (len >= 3 && !(mark == '`' && info.contains('`'))).then_some(Self { mark, len })
    }

    /// Whether `raw` closes this fence: after any indentation, at least as many of the fence
    /// character, then only whitespace.
    fn is_closed_by(self, raw: &str) -> bool {
        let trimmed = raw.trim_start();
        let run = trimmed.chars().take_while(|c| *c == self.mark).count();
        run >= self.len
            && trimmed
                .get(run..)
                .is_some_and(|rest| rest.trim().is_empty())
    }
}

/// `text` with every inline code span, backticks included, replaced by as many spaces as it has
/// bytes, so offsets in the result are offsets in `text`.
///
/// As in `CommonMark`, a run of N backticks opens a span that the next run of exactly N backticks
/// closes; a run with no such partner is plain text. Spans are read within one line. Each run
/// without a partner is the last run of its length, so the partner searches stay linear in
/// practice (PERF-10).
pub(crate) fn without_code_spans(text: &str) -> Cow<'_, str> {
    if !text.contains('`') {
        return Cow::Borrowed(text);
    }
    let runs = backtick_runs(text);
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut next = 0;
    while let Some(&(start, len)) = runs.get(next) {
        let Some((close_index, &(close, _))) =
            partner(&runs, next).and_then(|index| runs.get(index).map(|run| (index, run)))
        else {
            next = next.saturating_add(1);
            continue;
        };
        let end = close.saturating_add(len);
        out.push_str(text.get(copied..start).unwrap_or_default());
        out.extend(std::iter::repeat_n(' ', end.saturating_sub(start)));
        copied = end;
        next = close_index.saturating_add(1);
    }
    out.push_str(text.get(copied..).unwrap_or_default());
    Cow::Owned(out)
}

/// The runs of backticks in `text`: byte offset and length of each.
fn backtick_runs(text: &str) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut open: Option<usize> = None;
    for (at, c) in text.char_indices() {
        match (c == '`', open) {
            (true, None) => open = Some(at),
            (false, Some(start)) => {
                runs.push((start, at.saturating_sub(start)));
                open = None;
            }
            (true, Some(_)) | (false, None) => {}
        }
    }
    if let Some(start) = open {
        runs.push((start, text.len().saturating_sub(start)));
    }
    runs
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
