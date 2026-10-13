//! A proposal applied to the thresholds file's text: each number at a JSON pointer replaced in
//! place, so the file keeps its layout and the diff is exactly the changed numbers.
//!
//! There is no JSON library that keeps byte offsets, so a small scanner walks the text to the
//! value at the pointer. Object keys are compared as written between their quotes (the
//! thresholds file has no escaped keys); a key or index it cannot find is
//! [`EditError::NotFound`]. The caller re-reads the edited text strictly and checks each
//! pointer reads back its new value before anything is written.

use std::ops::Range;

use serde::Serialize;

/// One threshold change: the number at `pointer` goes from `from` to `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Edit<'a> {
    /// A JSON pointer (`/points/D1/rules/noul/bands/1/at_least`).
    pub(crate) pointer: &'a str,
    /// The value the file must hold now.
    pub(crate) from: f64,
    /// The value to write.
    pub(crate) to: f64,
}

/// One changed line, as a diff shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Change {
    /// The pointer whose number changed.
    pub(crate) pointer: String,
    /// The 1-based line it is on.
    pub(crate) line: usize,
    /// The line before the change.
    pub(crate) before: String,
    /// The line after it.
    pub(crate) after: String,
}

/// The edited text and its changes, in edit order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edited {
    /// The whole text after every edit.
    pub(crate) text: String,
    /// One change per edit.
    pub(crate) changes: Vec<Change>,
}

/// Why an edit cannot be applied to the text.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum EditError {
    /// No value at the pointer.
    #[error("{pointer} names no value in the thresholds file")]
    NotFound {
        /// The pointer.
        pointer: String,
    },
    /// The value at the pointer is not a number.
    #[error("{pointer} is not a number in the thresholds file")]
    NotANumber {
        /// The pointer.
        pointer: String,
    },
    /// The file no longer holds the proposal's `from`.
    #[error("{pointer} is {found} in the thresholds file, not the proposal's {expected}")]
    Stale {
        /// The pointer.
        pointer: String,
        /// What the proposal expected.
        expected: f64,
        /// What the file holds.
        found: f64,
    },
    /// The new value has no JSON form (not finite).
    #[error("{pointer} cannot be set to {to}")]
    NotFinite {
        /// The pointer.
        pointer: String,
        /// The value.
        to: f64,
    },
}

impl EditError {
    /// The pointer of the edit that failed.
    pub(crate) fn pointer(&self) -> &str {
        match self {
            Self::NotFound { pointer }
            | Self::NotANumber { pointer }
            | Self::Stale { pointer, .. }
            | Self::NotFinite { pointer, .. } => pointer,
        }
    }
}

/// Applies `edits` to `text` in order.
///
/// # Errors
/// [`EditError`] for the first edit that cannot be applied; nothing is returned then.
pub(crate) fn apply(text: &str, edits: &[Edit<'_>]) -> Result<Edited, EditError> {
    let mut current = text.to_owned();
    let mut changes = Vec::with_capacity(edits.len());
    for edit in edits {
        let (next, change) = apply_one(&current, edit)?;
        current = next;
        changes.push(change);
    }
    Ok(Edited {
        text: current,
        changes,
    })
}

fn apply_one(text: &str, edit: &Edit<'_>) -> Result<(String, Change), EditError> {
    let pointer = || edit.pointer.to_owned();
    let span =
        value_span(text, edit.pointer).ok_or_else(|| EditError::NotFound { pointer: pointer() })?;
    let (Some(head), Some(literal), Some(tail)) = (
        text.get(..span.start),
        text.get(span.start..span.end),
        text.get(span.end..),
    ) else {
        return Err(EditError::NotFound { pointer: pointer() });
    };
    let found: f64 = literal
        .parse()
        .map_err(|_not_a_number| EditError::NotANumber { pointer: pointer() })?;
    if found.total_cmp(&edit.from).is_ne() {
        return Err(EditError::Stale {
            pointer: pointer(),
            expected: edit.from,
            found,
        });
    }
    if !edit.to.is_finite() {
        return Err(EditError::NotFinite {
            pointer: pointer(),
            to: edit.to,
        });
    }
    let replacement = serde_json::Number::from_f64(edit.to)
        .ok_or_else(|| EditError::NotFinite {
            pointer: pointer(),
            to: edit.to,
        })?
        .to_string();
    // The line holding the number: what precedes it on its line, and what follows it there,
    // without a CRLF file's `\r`.
    let line_start = head.rsplit_once('\n').map_or(head, |(_, rest)| rest);
    let line_end = tail.split_once('\n').map_or(tail, |(rest, _)| rest);
    let line_end = line_end.strip_suffix('\r').unwrap_or(line_end);
    let change = Change {
        pointer: pointer(),
        line: head.matches('\n').count().saturating_add(1),
        before: format!("{line_start}{literal}{line_end}"),
        after: format!("{line_start}{replacement}{line_end}"),
    };
    Ok((format!("{head}{replacement}{tail}"), change))
}

/// The byte range of the value at `pointer`, or `None` when the text has no value there.
fn value_span(text: &str, pointer: &str) -> Option<Range<usize>> {
    let tokens = pointer.strip_prefix('/')?.split('/');
    let mut cursor = Cursor {
        bytes: text.as_bytes(),
        at: 0,
    };
    for token in tokens {
        let token = token.replace("~1", "/").replace("~0", "~");
        cursor.skip_whitespace();
        match cursor.peek()? {
            b'{' => cursor.enter_member(&token)?,
            b'[' => cursor.enter_element(token.parse().ok()?)?,
            _ => return None,
        }
    }
    cursor.skip_whitespace();
    let start = cursor.at;
    cursor.skip_value()?;
    Some(start..cursor.at)
}

/// A position in JSON text.
#[derive(Debug)]
struct Cursor<'t> {
    bytes: &'t [u8],
    at: usize,
}

impl<'t> Cursor<'t> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn advance(&mut self) {
        self.at = self.at.saturating_add(1);
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.advance();
        }
    }

    /// Consumes `byte`, after whitespace.
    fn eat(&mut self, byte: u8) -> Option<()> {
        self.skip_whitespace();
        (self.peek()? == byte).then(|| self.advance())
    }

    /// At an opening quote: consumes the string and returns its raw contents.
    fn string(&mut self) -> Option<&'t [u8]> {
        let bytes = self.bytes;
        self.eat(b'"')?;
        let start = self.at;
        loop {
            match self.peek()? {
                b'"' => break,
                b'\\' => {
                    self.advance();
                    self.advance();
                }
                _ => self.advance(),
            }
        }
        let raw = bytes.get(start..self.at)?;
        self.advance();
        Some(raw)
    }

    /// Consumes one value of any kind. Containers are skipped by counting brackets outside
    /// strings, so the walk needs no stack.
    fn skip_value(&mut self) -> Option<()> {
        self.skip_whitespace();
        match self.peek()? {
            b'"' => self.string().map(|_raw| ()),
            b'{' | b'[' => {
                let mut depth = 0_usize;
                loop {
                    match self.peek()? {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth = depth.saturating_add(1),
                        b'}' | b']' => {
                            depth = depth.checked_sub(1)?;
                            if depth == 0 {
                                self.advance();
                                return Some(());
                            }
                        }
                        _ => {}
                    }
                    self.advance();
                }
            }
            _ => {
                let start = self.at;
                while self
                    .peek()
                    .is_some_and(|b| !b.is_ascii_whitespace() && !b",]}".contains(&b))
                {
                    self.advance();
                }
                (self.at > start).then_some(())
            }
        }
    }

    /// At `{`: moves to the value of the member named `key`.
    fn enter_member(&mut self, key: &str) -> Option<()> {
        self.eat(b'{')?;
        loop {
            let name = self.string()?;
            self.eat(b':')?;
            if name == key.as_bytes() {
                return Some(());
            }
            self.skip_value()?;
            self.eat(b',')?;
        }
    }

    /// At `[`: moves to the element at `index`.
    fn enter_element(&mut self, index: usize) -> Option<()> {
        self.eat(b'[')?;
        for _ in 0..index {
            self.skip_value()?;
            self.eat(b',')?;
        }
        self.skip_whitespace();
        (self.peek()? != b']').then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, Edit, EditError, apply};

    const TEXT: &str = "{\n  \"points\": {\n    \"D1\": {\n      \"name\": \"a]{\\\"\",\n      \
                        \"bands\": [\n        { \"at_least\": 0.9, \"action\": \"deny\" },\n        \
                        { \"at_least\": 0.4, \"action\": \"ask\" }\n      ]\n    }\n  }\n}\n";

    const POINTER: &str = "/points/D1/bands/1/at_least";

    #[test]
    fn only_the_number_at_the_pointer_changes() {
        let edited = apply(
            TEXT,
            &[Edit {
                pointer: POINTER,
                from: 0.4,
                to: 0.3,
            }],
        )
        .expect("the pointer holds 0.4");

        assert_eq!(edited.text, TEXT.replacen("0.4", "0.3", 1));
        assert_eq!(
            edited.changes,
            [Change {
                pointer: POINTER.to_owned(),
                line: 7,
                before: "        { \"at_least\": 0.4, \"action\": \"ask\" }".to_owned(),
                after: "        { \"at_least\": 0.3, \"action\": \"ask\" }".to_owned(),
            }]
        );
    }

    #[test]
    fn a_file_that_no_longer_holds_from_is_stale() {
        let result = apply(
            TEXT,
            &[Edit {
                pointer: POINTER,
                from: 0.5,
                to: 0.3,
            }],
        );

        assert_eq!(
            result,
            Err(EditError::Stale {
                pointer: POINTER.to_owned(),
                expected: 0.5,
                found: 0.4
            })
        );
    }

    #[test]
    fn a_missing_key_or_an_index_past_the_end_is_not_found() {
        for pointer in [
            "/points/D2/bands/0/at_least",
            "/points/D1/bands/2/at_least",
            "/points/D1/bands/x",
            "points",
        ] {
            let result = apply(
                TEXT,
                &[Edit {
                    pointer,
                    from: 0.4,
                    to: 0.3,
                }],
            );

            assert_eq!(
                result,
                Err(EditError::NotFound {
                    pointer: pointer.to_owned()
                }),
                "{pointer}"
            );
        }
    }

    #[test]
    fn a_string_value_is_not_a_number() {
        let pointer = "/points/D1/bands/0/action";

        let result = apply(
            TEXT,
            &[Edit {
                pointer,
                from: 0.4,
                to: 0.3,
            }],
        );

        assert_eq!(
            result,
            Err(EditError::NotANumber {
                pointer: pointer.to_owned()
            })
        );
    }

    #[test]
    fn a_crlf_line_is_shown_without_its_carriage_return() {
        let crlf = TEXT.replace('\n', "\r\n");

        let edited = apply(
            &crlf,
            &[Edit {
                pointer: POINTER,
                from: 0.4,
                to: 0.3,
            }],
        )
        .expect("the pointer holds 0.4");

        assert_eq!(edited.text, crlf.replacen("0.4", "0.3", 1));
        assert_eq!(
            edited.changes,
            [Change {
                pointer: POINTER.to_owned(),
                line: 7,
                before: "        { \"at_least\": 0.4, \"action\": \"ask\" }".to_owned(),
                after: "        { \"at_least\": 0.3, \"action\": \"ask\" }".to_owned(),
            }]
        );
    }

    #[test]
    fn edits_apply_in_order_each_to_the_text_before_it() {
        let edits = [
            Edit {
                pointer: POINTER,
                from: 0.4,
                to: 0.3,
            },
            Edit {
                pointer: "/points/D1/bands/0/at_least",
                from: 0.9,
                to: 0.85,
            },
        ];

        let edited = apply(TEXT, &edits).expect("both pointers hold their from");

        assert_eq!(
            edited.text,
            TEXT.replacen("0.4", "0.3", 1).replacen("0.9", "0.85", 1)
        );
        let lines: Vec<usize> = edited.changes.iter().map(|c| c.line).collect();
        assert_eq!(lines, [7, 6]);
    }
}
