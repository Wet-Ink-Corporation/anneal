//! Line heuristics: chat and tool-call markup, tool-call JSON (both outside code fences), and
//! hidden characters.

use super::{Hit, Kind, Scan};

/// Chat-template and tool-call markup, lowercase: a model's own turn and call delimiters have
/// no business in fetched content.
const MARKUP: [&str; 19] = [
    "<|im_start|>",
    "<|im_end|>",
    "<|system|>",
    "<|user|>",
    "<|assistant|>",
    "<|endoftext|>",
    "[inst]",
    "[/inst]",
    "<<sys>>",
    "<system-reminder>",
    "</system-reminder>",
    "<function_calls>",
    "</function_calls>",
    "<invoke name=",
    "<tool_use>",
    "</tool_use>",
    "<tool_call>",
    "</tool_call>",
    "<tool_result>",
];

/// JSON keys and values that open a tool call.
const TOOL_CALL_KEYS: [&str; 4] = [
    "\"tool_use\"",
    "\"tool_calls\"",
    "\"tool_call\"",
    "\"function_call\"",
];
/// What a tool call names besides the tool.
const TOOL_CALL_ARGUMENTS: [&str; 3] = ["\"input\"", "\"arguments\"", "\"parameters\""];
/// Bytes after a [`TOOL_CALL_KEYS`] match looked at for `"name"` and the arguments.
const TOOL_CALL_REACH: usize = 400;

/// Adds the markup and tool-call JSON hits on the lines of `text` outside code fences, at most
/// one of each kind per line. A fence still open when the text ends hides nothing: the lines
/// after its opener are scanned as if it were not there.
pub(super) fn markup(text: &str, found: &mut Scan) {
    if let Some(opener) = markup_lines(text, None, found) {
        markup_lines(text, Some(opener), found);
    }
}

/// Scans the lines of `text` for [`markup`]. With `after: None` it honours code fences and
/// returns the index of a fence's opening line when the text ends inside it; with
/// `after: Some(i)` it scans only the lines after line `i`, ignoring fences, and returns `None`.
fn markup_lines(text: &str, after: Option<usize>, found: &mut Scan) -> Option<usize> {
    let mut fence: Option<(char, usize)> = None;
    let mut start = 0_usize;
    for (index, line) in text.split('\n').enumerate() {
        let number = index.saturating_add(1);
        let line_start = start;
        start = start.saturating_add(line.len()).saturating_add(1);
        match after {
            Some(opener) if index <= opener => continue,
            Some(_) => {}
            None => {
                let marker = starts_with_fence(line.trim_start());
                if let Some((open, _)) = fence {
                    if marker == Some(open) {
                        fence = None;
                    }
                    continue;
                }
                if let Some(open) = marker {
                    fence = Some((open, index));
                    continue;
                }
            }
        }
        if MARKUP
            .iter()
            .any(|needle| contains_ignoring_ascii_case(line, needle))
        {
            found.add(Hit {
                kind: Kind::Markup,
                line: number,
            });
        }
        if tool_call_on_line(text, line_start, line) {
            found.add(Hit {
                kind: Kind::ToolCallJson,
                line: number,
            });
        }
    }
    fence.map(|(_, opener)| opener)
}

/// Whether `haystack` holds `needle` (lowercase ASCII) in any ASCII letter case, without
/// lowercasing a copy of the line.
fn contains_ignoring_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The fence character when `trimmed` opens or closes a code fence (three backticks or tildes).
fn starts_with_fence(trimmed: &str) -> Option<char> {
    if trimmed.starts_with("```") {
        Some('`')
    } else if trimmed.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// Whether `line`, which starts at byte `line_start` of `text`, opens a tool call: a
/// [`TOOL_CALL_KEYS`] match followed within [`TOOL_CALL_REACH`] bytes by `"name"` and one of
/// [`TOOL_CALL_ARGUMENTS`].
fn tool_call_on_line(text: &str, line_start: usize, line: &str) -> bool {
    TOOL_CALL_KEYS.iter().any(|key| {
        line.match_indices(key).any(|(offset, _)| {
            let from = line_start.saturating_add(offset);
            let to = text.floor_char_boundary(from.saturating_add(TOOL_CALL_REACH));
            text.get(from..to).is_some_and(|window| {
                window.contains("\"name\"")
                    && TOOL_CALL_ARGUMENTS
                        .iter()
                        .any(|arguments| window.contains(arguments))
            })
        })
    })
}

/// Adds a hit for each line that holds a Unicode tag character (U+E0000..U+E007F) or a bidi
/// embedding, override or isolate control, or two or more other default-ignorable characters
/// ([`super::is_ignorable`]). Some uses in ordinary text are not counted:
///
/// - a supplementary variation selector (U+E0100..U+E01EF) right after a Han ideograph: an
///   ideographic variation sequence, as Japanese names use;
/// - a byte-order mark that starts the text;
/// - a zero-width joiner or non-joiner, or a left-to-right or right-to-left mark, between two
///   visible characters outside ASCII (emoji sequences, Persian, Indic scripts, mixed-direction
///   text), where an emoji presentation selector counts as part of its emoji;
/// - an emoji presentation selector (U+FE0E, U+FE0F) right after a visible character outside
///   ASCII, or right before the keycap mark U+20E3 (`1️⃣`);
/// - a soft hyphen between two letters.
pub(super) fn hidden_characters(text: &str, found: &mut Scan) {
    for (index, line) in text.split('\n').enumerate() {
        let mut counted = 0_usize;
        let mut hard = false;
        let mut previous: Option<char> = None;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            let next = chars.peek().copied();
            match c {
                '\u{E0000}'..='\u{E007F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => {
                    hard = true;
                }
                SUPPLEMENTARY_SELECTORS_START..=SUPPLEMENTARY_SELECTORS_END
                    if previous.is_some_and(han) => {}
                '\u{FEFF}' if index == 0 && previous.is_none() => {}
                '\u{200C}'..='\u{200F}' if joins(previous) && joins(next) => {}
                '\u{FE0E}' | '\u{FE0F}'
                    if previous.is_some_and(visible_outside_ascii) || next == Some(KEYCAP) => {}
                '\u{00AD}' if letter(previous) && letter(next) => {}
                _ if super::is_ignorable(c) => counted = counted.saturating_add(1),
                _ => {}
            }
            previous = Some(c);
        }
        if hard || counted >= 2 {
            found.add(Hit {
                kind: Kind::HiddenCharacters,
                line: index.saturating_add(1),
            });
        }
    }
}

/// The combining enclosing keycap, which follows a digit, `#` or `*` and its presentation
/// selector in a keycap emoji.
const KEYCAP: char = '\u{20E3}';

/// The first supplementary variation selector (VS17).
const SUPPLEMENTARY_SELECTORS_START: char = '\u{E0100}';
/// The last supplementary variation selector (VS256).
const SUPPLEMENTARY_SELECTORS_END: char = '\u{E01EF}';

/// Whether `c` is a Han ideograph, the only base an ideographic variation sequence has: CJK
/// Unified Ideographs and Extension A, the Compatibility Ideographs, and the Supplementary and
/// Tertiary Ideographic Planes (U+20000..U+3FFFF) whole.
fn han(c: char) -> bool {
    matches!(
        c,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{3FFFF}'
    )
}

/// Whether `c` prints something and is outside ASCII.
fn visible_outside_ascii(c: char) -> bool {
    !c.is_ascii() && !super::is_ignorable(c)
}

/// Whether a joiner or directional mark next to `c` sits in text outside ASCII: `c` is a
/// visible character outside ASCII, or an emoji presentation selector that belongs to one.
fn joins(c: Option<char>) -> bool {
    c.is_some_and(|c| matches!(c, '\u{FE0E}' | '\u{FE0F}') || visible_outside_ascii(c))
}

/// Whether `c` is a visible letter.
fn letter(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphabetic() && !super::is_ignorable(c))
}
