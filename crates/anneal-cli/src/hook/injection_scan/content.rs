//! What a `Read` or `WebFetch` call returned, as text to scan, and where it came from.
//!
//! Claude Code 2.1.x's `tool_response` for `Read` is `{"type": "text", "file": {"filePath",
//! "content", "startLine", ...}}` and for `WebFetch` `{"code", "result", "url", ...}`; neither is
//! a documented contract, so a plain string, a `result` or `content` string, and finally every
//! string in the response (with no line numbers) are read too.

use std::borrow::Cow;

use serde_json::Value;

/// The tools whose results are scanned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    /// `Read`: a local file.
    Read,
    /// `WebFetch`: a fetched page, as the fetch model summarised it.
    WebFetch,
}

impl Tool {
    /// The scanned tool named `name`, if it is one.
    pub(super) fn named(name: &str) -> Option<Self> {
        match name {
            "Read" => Some(Self::Read),
            "WebFetch" => Some(Self::WebFetch),
            _ => None,
        }
    }
}

/// The text of one call's result, cut to the scan cap, and where it came from.
#[derive(Debug)]
pub(super) struct Content<'a> {
    /// Which tool returned it.
    pub(super) tool: Tool,
    /// The file path or URL the call named; `None` when the input names none.
    pub(super) place: Option<&'a str>,
    /// The line of the source the text's first line is; `None` when the text was gathered
    /// from a response of an unknown shape, so its lines are not the source's.
    pub(super) first_line: Option<usize>,
    /// The text, at most the cap.
    pub(super) text: Cow<'a, str>,
}

/// The content of a `tool` call with `input` and `response`, cut to `max` bytes; `None` for an
/// image or a response with no text.
pub(super) fn extract<'a>(
    tool: Tool,
    input: &'a Value,
    response: &'a Value,
    max: usize,
) -> Option<Content<'a>> {
    let place_key = match tool {
        Tool::Read => "file_path",
        Tool::WebFetch => "url",
    };
    let place = input.get(place_key).and_then(Value::as_str);
    if response.get("type").and_then(Value::as_str) == Some("image") {
        return None;
    }
    let (text, first_line) = if let Some(text) = response.as_str() {
        let offset = match tool {
            Tool::Read => line_number(input.get("offset")),
            Tool::WebFetch => 1,
        };
        (Cow::Borrowed(cut(text, max)), Some(offset))
    } else if let Some((text, file)) = file_content(response) {
        (
            Cow::Borrowed(cut(text, max)),
            Some(line_number(file.get("startLine"))),
        )
    } else if let Some(text) = ["result", "content"]
        .iter()
        .find_map(|key| response.get(key).and_then(Value::as_str))
    {
        (Cow::Borrowed(cut(text, max)), Some(1))
    } else {
        (Cow::Owned(gather(response, max)), None)
    };
    (!text.is_empty()).then_some(Content {
        tool,
        place,
        first_line,
        text,
    })
}

/// `Read`'s `file.content` and the `file` object holding it.
fn file_content(response: &Value) -> Option<(&str, &Value)> {
    let file = response.get("file")?;
    Some((file.get("content")?.as_str()?, file))
}

/// A 1-based line number from JSON: 1 when absent, zero or not a number that fits.
fn line_number(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .and_then(|line| usize::try_from(line).ok())
        .filter(|line| *line > 0)
        .unwrap_or(1)
}

/// `text` cut to at most `max` bytes, at a character boundary.
fn cut(text: &str, max: usize) -> &str {
    let end = text.floor_char_boundary(max);
    text.get(..end).unwrap_or_default()
}

/// Every string in `value`, depth first (array items in order, object fields in the map's
/// order), one per line, until `max` bytes.
fn gather(value: &Value, max: usize) -> String {
    let mut text = String::new();
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        if text.len() >= max {
            break;
        }
        match value {
            Value::String(string) => {
                text.push_str(cut(string, max.saturating_sub(text.len())));
                text.push('\n');
            }
            Value::Array(items) => pending.extend(items.iter().rev()),
            Value::Object(fields) => pending.extend(fields.values().rev()),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    text.truncate(text.floor_char_boundary(max));
    text
}
