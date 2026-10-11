//! Heredoc bodies: data, whose substitutions still run unless the delimiter is quoted.

use std::mem;

use super::{Heredoc, Lexer, ParseError, Word};

impl Lexer {
    /// Reads the bodies of the heredocs opened on the line just ended.
    pub(super) fn heredoc_bodies(&mut self, depth: usize) -> Result<(), ParseError> {
        for heredoc in mem::take(&mut self.heredocs) {
            let body = self.heredoc_body(&heredoc);
            if heredoc.expands {
                let mut lexer = Self::new(&body);
                lexer.quoted_body(&mut Word::new(), None, depth.saturating_add(1))?;
                self.commands.append(&mut lexer.commands);
            }
        }
        Ok(())
    }

    /// Consumes lines up to the delimiter line (or the end) and returns them.
    fn heredoc_body(&mut self, heredoc: &Heredoc) -> String {
        let mut body = String::new();
        while self.peek().is_some() {
            let line = self.line();
            let check = if heredoc.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                &line
            };
            if check == heredoc.delimiter {
                break;
            }
            body.push_str(&line);
            body.push('\n');
        }
        body
    }

    /// Consumes one line and its newline, returning the line.
    fn line(&mut self) -> String {
        let mut line = String::new();
        while let Some(c) = self.bump() {
            if c == '\n' {
                break;
            }
            line.push(c);
        }
        line
    }
}
