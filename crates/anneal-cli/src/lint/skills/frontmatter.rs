//! A hand parser for the small YAML subset a `SKILL.md` frontmatter uses.
//!
//! Supported: `key: value` scalars (plain, `"double"` or `'single'` quoted), `key: >` folded and
//! `key: |` literal blocks (chomping indicators accepted, trailing newlines always stripped), and
//! `key:` followed by an indented block (a nested map such as `metadata`), kept as raw text.
//! A plain value may not contain `: `, end with `:`, or start with a YAML indicator (strict YAML
//! rejects them; quote such values), except `[`, kept for `argument-hint: [path]`. Anything
//! else is a [`ParseError`] with the line it stopped at.

use std::collections::BTreeSet;

/// One top-level frontmatter key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// The key as written.
    pub key: String,
    /// The value: unquoted scalar, folded or literal block text, or the raw nested block.
    pub value: String,
    /// 1-based line of the key in the file.
    pub line: usize,
}

/// A parsed `SKILL.md`: its frontmatter fields and the body after the closing `---`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document<'a> {
    /// Top-level fields in file order.
    pub fields: Vec<Field>,
    /// The whole text that was parsed.
    pub source: &'a str,
    /// The body text after the closing `---` line.
    pub body: &'a str,
    /// 1-based file line of the first body line.
    pub body_line: usize,
}

/// Why the frontmatter could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseError {
    /// 1-based line where parsing stopped.
    pub line: usize,
    /// What was wrong, one line.
    pub reason: &'static str,
}

impl Document<'_> {
    /// The field named `key`, if present.
    pub(crate) fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// The value of `key`, or `""` when absent.
    pub(crate) fn value(&self, key: &str) -> &str {
        self.field(key).map_or("", |f| f.value.as_str())
    }
}

const FENCE: &str = "---";

/// Splits `text` into frontmatter fields and body.
pub(crate) fn parse(text: &str) -> Result<Document<'_>, ParseError> {
    let opening = text.split_inclusive('\n').next().unwrap_or("");
    if trim_eol(opening) != FENCE {
        return Err(error(
            1,
            "SKILL.md must start with a `---` frontmatter line",
        ));
    }
    let rest = text.get(opening.len()..).unwrap_or("");
    let (block, body, closing_index) =
        split_at_fence(rest).ok_or_else(|| error(1, "frontmatter has no closing `---` line"))?;
    let fields = parse_fields(block)?;
    Ok(Document {
        fields,
        source: text,
        body,
        body_line: closing_index.saturating_add(3),
    })
}

/// Finds the closing fence in `rest` (the text after the opening line). Returns the
/// frontmatter block, the body, and the 0-based index of the closing line within `rest`.
fn split_at_fence(rest: &str) -> Option<(&str, &str, usize)> {
    let mut offset = 0;
    for (index, line) in rest.split_inclusive('\n').enumerate() {
        if trim_eol(line) == FENCE {
            let block = rest.get(..offset)?;
            let body = rest.get(offset.saturating_add(line.len())..)?;
            return Some((block, body, index));
        }
        offset = offset.saturating_add(line.len());
    }
    None
}

fn trim_eol(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

/// Parses the lines between the fences. Line numbers are 1-based file lines, so the block's
/// first line is line 2.
fn parse_fields(block: &str) -> Result<Vec<Field>, ParseError> {
    let lines: Vec<&str> = block.lines().map(trim_eol).collect();
    let mut fields: Vec<Field> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut index = 0;
    while let Some(&line) = lines.get(index) {
        let line_no = index.saturating_add(2);
        index = index.saturating_add(1);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(error(line_no, "unexpected indentation"));
        }
        let (key, rest) = split_key(line).ok_or_else(|| error(line_no, "expected `key: value`"))?;
        if !seen.insert(key) {
            return Err(error(line_no, "duplicate key"));
        }
        let continuation = indented_run(lines.get(index..).unwrap_or(&[]));
        index = index.saturating_add(continuation.len());
        let value = field_value(rest.trim(), continuation, line_no)?;
        fields.push(Field {
            key: key.to_owned(),
            value,
            line: line_no,
        });
    }
    Ok(fields)
}

fn error(line: usize, reason: &'static str) -> ParseError {
    ParseError { line, reason }
}

/// `key: rest` with a key of ASCII letters, digits, `-` or `_`.
fn split_key(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    let valid = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let separated = rest.is_empty() || rest.starts_with([' ', '\t']);
    (valid && separated).then_some((key, rest))
}

/// The lines after a key that belong to it: indented lines, and blank lines followed by more
/// indented lines.
fn indented_run<'l>(lines: &'l [&'l str]) -> &'l [&'l str] {
    let mut end = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with([' ', '\t']) && !line.trim().is_empty() {
            end = index.saturating_add(1);
        } else if !line.trim().is_empty() {
            break;
        }
    }
    lines.get(..end).unwrap_or(&[])
}

fn field_value(rest: &str, continuation: &[&str], line: usize) -> Result<String, ParseError> {
    match rest {
        ">" | ">-" | ">+" => Ok(fold(continuation)),
        "|" | "|-" | "|+" => Ok(literal(continuation)),
        "" => Ok(continuation
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n")),
        scalar if continuation.is_empty() => unquote(scalar, line),
        _ => Err(error(
            line,
            "a scalar value cannot continue on indented lines",
        )),
    }
}

/// Folded block: lines joined by spaces, blank lines kept as line breaks.
fn fold(lines: &[&str]) -> String {
    let mut text = String::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            text.push('\n');
        } else {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push(' ');
            }
            text.push_str(trimmed);
        }
    }
    text.trim_end().to_owned()
}

/// Literal block: lines kept, common indentation removed.
fn literal(lines: &[&str]) -> String {
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.bytes().take_while(|b| matches!(b, b' ' | b'\t')).count())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| l.get(indent..).unwrap_or("").trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// One scalar value on `line`: quoted (quotes and escapes removed) or plain (trailing comment
/// dropped). Shared with the routing fixture reader, so both accept the same scalars.
pub(crate) fn unquote(scalar: &str, line: usize) -> Result<String, ParseError> {
    let Some(quote @ ('"' | '\'')) = scalar.chars().next() else {
        return plain(scalar, line);
    };
    let quoted = read_quoted(scalar.get(1..).unwrap_or(""), quote);
    let (value, after) = quoted.ok_or_else(|| error(line, "unterminated quoted value"))?;
    let after = after.trim_start();
    if after.is_empty() || after.starts_with('#') {
        Ok(value)
    } else {
        Err(error(line, "text after a quoted value"))
    }
}

/// A plain (unquoted) scalar, minus any trailing ` # comment`. Strict YAML rejects `: ` inside
/// one, a trailing `:`, and a leading indicator (`` ` `` `@` `%` `*` `&` `!` `|` `>` `#`, or `-`
/// or `?` followed by a space), so they must be quoted. A leading `[` is accepted on purpose:
/// Claude Code's `argument-hint: [path]` is written unquoted, and it is kept as text.
fn plain(scalar: &str, line: usize) -> Result<String, ParseError> {
    let value = scalar
        .split_once(" #")
        .map_or(scalar, |(value, _)| value)
        .trim_end();
    if value.contains(": ") || value.ends_with(':') {
        return Err(error(line, "plain value contains `: `; quote it"));
    }
    let indicator = value.starts_with(['`', '@', '%', '*', '&', '!', '|', '>', '#'])
        || value.starts_with("- ")
        || value.starts_with("? ");
    if indicator {
        return Err(error(
            line,
            "plain value starts with a YAML indicator; quote it",
        ));
    }
    Ok(value.to_owned())
}

/// Reads a quoted scalar body up to its closing `quote`. Double quotes take `\"` and `\\`
/// escapes; single quotes take `''`. Returns the value and the text after the closing quote.
fn read_quoted(text: &str, quote: char) -> Option<(String, &str)> {
    let mut value = String::new();
    let mut chars = text.char_indices().peekable();
    let escape = if quote == '"' { '\\' } else { '\'' };
    while let Some((index, c)) = chars.next() {
        if c == escape
            && let Some((_, literal)) =
                chars.next_if(|&(_, n)| n == quote || (quote == '"' && n == '\\'))
        {
            value.push(literal);
        } else if c == quote {
            return Some((value, text.get(index.saturating_add(c.len_utf8())..)?));
        } else {
            value.push(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Field, ParseError, parse};

    fn field(key: &str, value: &str, line: usize) -> Field {
        Field {
            key: key.to_owned(),
            value: value.to_owned(),
            line,
        }
    }

    #[test]
    fn plain_and_quoted_scalars_are_read_with_their_lines() {
        let doc = parse("---\nname: plan\ndescription: \"Use when: a plan\"\nlicense: 'it''s MIT' # c\n---\n# Body\n")
            .expect("valid frontmatter");

        assert_eq!(
            doc.fields,
            [
                field("name", "plan", 2),
                field("description", "Use when: a plan", 3),
                field("license", "it's MIT", 4),
            ],
            "quotes and escapes removed, trailing comment dropped"
        );
        assert_eq!(doc.body, "# Body\n", "body follows the closing fence");
        assert_eq!(doc.body_line, 6, "body starts on line 6");
    }

    #[test]
    fn folded_and_literal_blocks_join_their_lines() {
        let text = "---\ndescription: >\n  Use when\n  planning.\nnotes: |\n  a\n    b\n---\n";
        let doc = parse(text).expect("valid frontmatter");

        assert_eq!(doc.value("description"), "Use when planning.", "folded");
        assert_eq!(
            doc.value("notes"),
            "a\n  b",
            "literal keeps relative indent"
        );
    }

    #[test]
    fn literal_block_indent_counts_only_spaces_and_tabs() {
        let text = "---\nnotes: |\n \u{a0}a\n \u{a0}b\n---\n";
        let doc = parse(text).expect("valid frontmatter");

        assert_eq!(
            doc.value("notes"),
            "\u{a0}a\n\u{a0}b",
            "a no-break space is content, not indentation"
        );
    }

    #[test]
    fn a_plain_value_may_hold_a_colon_without_a_following_space() {
        let doc = parse("---\ndescription: Use at 10:30 for https://x.example\n---\n")
            .expect("`:` without a space is plain YAML");
        assert_eq!(
            doc.value("description"),
            "Use at 10:30 for https://x.example",
            "kept whole"
        );
    }

    #[test]
    fn a_bracketed_argument_hint_and_inner_dashes_stay_plain() {
        let doc = parse("---\nargument-hint: [path] [--fix]\ndescription: -x or ?y\n---\n")
            .expect("`[` and `-`/`?` without a space are accepted");
        assert_eq!(doc.value("argument-hint"), "[path] [--fix]", "kept as text");
        assert_eq!(doc.value("description"), "-x or ?y", "kept as text");
    }

    #[test]
    fn a_nested_map_is_kept_as_raw_lines() {
        let doc = parse("---\nmetadata:\n  owner: anneal\n  tier: 1\nname: x\n---\n")
            .expect("valid frontmatter");

        assert_eq!(doc.value("metadata"), "owner: anneal\ntier: 1", "raw block");
        assert_eq!(doc.field("name").map(|f| f.line), Some(5), "next key line");
    }

    #[test]
    fn malformed_frontmatter_reports_the_line() {
        let cases = [
            ("# no fence\n", 1),
            ("---\nname: x\n", 1),
            ("---\nname: x\njust words\n---\n", 3),
            ("---\nname: x\nname: y\n---\n", 3),
            ("---\ndescription: \"open\n---\n", 2),
            ("---\n  indented: x\n---\n", 2),
            ("---\nname: x\ndescription: Use when: a spec\n---\n", 3),
            ("---\nname: plan:\n---\n", 2),
            ("---\ndescription: `code` first\n---\n", 2),
            ("---\ndescription: @handle\n---\n", 2),
            ("---\ndescription: %directive\n---\n", 2),
            ("---\ndescription: *alias\n---\n", 2),
            ("---\ndescription: &anchor x\n---\n", 2),
            ("---\ndescription: !tag x\n---\n", 2),
            ("---\ndescription: | x\n---\n", 2),
            ("---\ndescription: >x\n---\n", 2),
            ("---\ndescription: #x\n---\n", 2),
            ("---\ndescription: - item\n---\n", 2),
            ("---\ndescription: ? key\n---\n", 2),
        ];
        for (text, line) in cases {
            let result = parse(text).map(|d| d.fields);
            assert!(
                matches!(result, Err(ParseError { line: l, .. }) if l == line),
                "{text:?} should fail at line {line}: {result:?}"
            );
        }
    }
}
