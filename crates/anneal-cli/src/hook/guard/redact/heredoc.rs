//! Redirections and heredocs: a heredoc's delimiter is read on its line, and its body, from
//! the next line to its closing line, becomes [`REDACTED`].

use super::spans::{after, char_at, word_end};
use super::{Quoting, REDACTED, Scan, Scanner};

/// A heredoc whose body starts after the current line.
#[derive(Debug)]
pub(super) struct Heredoc {
    delimiter: String,
    /// `<<-`: leading tabs are stripped from the closing line.
    strip_tabs: bool,
}

impl Scanner<'_, '_> {
    /// A run of `<` or `>`; exactly `<<` (or `<<-`) outside double quotes opens a heredoc.
    pub(super) fn redirection(&mut self, c: char) -> Scan {
        let run = self
            .text
            .get(self.pos..)
            .map_or(0, |rest| rest.chars().take_while(|&d| d == c).count());
        let end = self.pos.saturating_add(run);
        if c == '<' && run == 2 && self.quoting == Quoting::Bare {
            self.copy_to(end);
            self.heredoc()
        } else {
            self.copy_to(end)
        }
    }

    /// After `<<`: an optional `-`, blanks and the delimiter word, queued for the next line.
    fn heredoc(&mut self) -> Scan {
        let strip_tabs = char_at(self.text, self.pos) == Some('-');
        if strip_tabs {
            self.copy_to(after(self.pos, '-'));
        }
        while let Some(blank @ (' ' | '\t')) = char_at(self.text, self.pos) {
            self.copy_to(after(self.pos, blank));
        }
        let Some(end) = word_end(self.text, self.pos, self.quoting, self.depth) else {
            return self.redact_rest();
        };
        let raw = self.text.get(self.pos..end).unwrap_or_default();
        let delimiter = raw
            .chars()
            .filter(|c| !matches!(c, '\'' | '"' | '\\'))
            .collect();
        self.heredocs.push(Heredoc {
            delimiter,
            strip_tabs,
        });
        self.copy_to(end)
    }

    /// At the start of a line: each queued heredoc's body becomes [`REDACTED`], its closing
    /// line is kept.
    pub(super) fn heredoc_bodies(&mut self) -> Scan {
        for heredoc in std::mem::take(&mut self.heredocs) {
            let mut line_start = self.pos;
            loop {
                let rest = self.text.get(line_start..).unwrap_or_default();
                let line_len = rest.find('\n').unwrap_or(rest.len());
                let line = rest.get(..line_len).unwrap_or_default();
                let closing = if heredoc.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if closing == heredoc.delimiter {
                    if line_start > self.pos {
                        self.out.push_str(REDACTED);
                        self.out.push('\n');
                    }
                    self.pos = line_start;
                    let newline = usize::from(line_len < rest.len());
                    self.copy_to(line_start.saturating_add(line_len).saturating_add(newline));
                    break;
                }
                if line_len == rest.len() {
                    return self.redact_rest();
                }
                line_start = line_start.saturating_add(line_len).saturating_add(1);
            }
        }
        Scan::Closed
    }
}
