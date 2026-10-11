//! Reading `git diff -U0` output: the files it changes, and each hunk's removed and added lines
//! with their line numbers. Pure: no I/O.
//!
//! Hunk bodies are read by the counts in their `@@` header, so a removed line whose text looks
//! like a header (`--- a/x`) stays in its hunk.

use std::borrow::Cow;

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// It is new (or untracked).
    Added,
    /// It was deleted.
    Deleted,
    /// It changed in place.
    Modified,
    /// It was renamed or moved, possibly with changes.
    Renamed,
}

/// One line of a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line<'a> {
    /// Its line number: in the base for a removed line, in the working tree for an added one.
    pub(crate) number: u64,
    /// Its text, without the `+`/`-` marker or line ending.
    pub(crate) text: &'a str,
}

/// One hunk: what it removed and what it added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Hunk<'a> {
    /// Removed lines, in order.
    pub(crate) removed: Vec<Line<'a>>,
    /// Added lines, in order.
    pub(crate) added: Vec<Line<'a>>,
}

/// One file of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileDiff<'a> {
    /// Its path in the base (the same as `path` unless renamed).
    pub(crate) old_path: Cow<'a, str>,
    /// Its path in the working tree (for a deleted file, the path it had).
    pub(crate) path: Cow<'a, str>,
    /// How it changed.
    pub(crate) status: Status,
    /// Its hunks, in order.
    pub(crate) hunks: Vec<Hunk<'a>>,
}

impl<'a> FileDiff<'a> {
    /// An untracked file as a diff: every line of `content` added, numbered from 1.
    pub(crate) fn untracked(path: &'a str, content: &'a str) -> Self {
        let added = (1..)
            .zip(content.lines())
            .map(|(number, text)| Line { number, text })
            .collect();
        Self {
            old_path: Cow::Borrowed(path),
            path: Cow::Borrowed(path),
            status: Status::Added,
            hunks: vec![Hunk {
                removed: Vec::new(),
                added,
            }],
        }
    }
}

/// A file being read: what its headers said so far.
#[derive(Debug, Default)]
struct Pending<'a> {
    header: Option<(Cow<'a, str>, Cow<'a, str>)>,
    old: Option<Cow<'a, str>>,
    new: Option<Cow<'a, str>>,
    /// The status the headers declared, if any: deleted wins over added, added over renamed.
    declared: Option<Status>,
    hunks: Vec<Hunk<'a>>,
}

impl<'a> Pending<'a> {
    /// Records a status a header declares, keeping the strongest seen.
    fn declare(&mut self, status: Status) {
        let rank = |status: Status| match status {
            Status::Deleted => 3,
            Status::Added => 2,
            Status::Renamed => 1,
            Status::Modified => 0,
        };
        if self.declared.is_none_or(|seen| rank(status) > rank(seen)) {
            self.declared = Some(status);
        }
    }

    /// The file, once its paths are known; `None` for a block with no path at all.
    fn finish(self) -> Option<FileDiff<'a>> {
        let (header_old, header_new) = match self.header {
            Some((old, new)) => (Some(old), Some(new)),
            None => (None, None),
        };
        let old_path = self.old.or(header_old);
        let path = self.new.or(header_new);
        let (old_path, path) = match (old_path, path) {
            (Some(old), Some(new)) => (old, new),
            // One path for both sides: genuinely duplicated (a cheap borrow unless unquoted).
            (Some(only), None) | (None, Some(only)) => (only.clone(), only),
            (None, None) => return None,
        };
        let status = match self.declared {
            Some(status) => status,
            None if old_path != path => Status::Renamed,
            None => Status::Modified,
        };
        Some(FileDiff {
            old_path,
            path,
            status,
            hunks: self.hunks,
        })
    }
}

/// Lines still expected in the current hunk, on each side.
#[derive(Debug, Clone, Copy, Default)]
struct Remaining {
    old: u64,
    new: u64,
    old_line: u64,
    new_line: u64,
}

/// Parses `git diff` output (with `a/` and `b/` prefixes) into its files, in order.
pub(crate) fn parse(diff: &str) -> Vec<FileDiff<'_>> {
    let mut files = Vec::new();
    let mut pending: Option<Pending<'_>> = None;
    let mut remaining = Remaining::default();
    for line in diff.lines() {
        if (remaining.old > 0 || remaining.new > 0)
            && let Some(file) = pending.as_mut()
        {
            hunk_line(file, &mut remaining, line);
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            files.extend(pending.take().and_then(Pending::finish));
            pending = Some(Pending {
                header: header_paths(rest),
                ..Pending::default()
            });
            continue;
        }
        let Some(file) = pending.as_mut() else {
            continue;
        };
        header_line(file, &mut remaining, line);
    }
    files.extend(pending.and_then(Pending::finish));
    files
}

/// Reads one extended header, `---`/`+++` or `@@` line of `file`.
fn header_line<'a>(file: &mut Pending<'a>, remaining: &mut Remaining, line: &'a str) {
    if line.starts_with("new file mode") {
        file.declare(Status::Added);
    } else if line.starts_with("deleted file mode") {
        file.declare(Status::Deleted);
    } else if let Some(path) = line.strip_prefix("rename from ") {
        file.declare(Status::Renamed);
        file.old = Some(unquote(path));
    } else if let Some(path) = line.strip_prefix("rename to ") {
        file.declare(Status::Renamed);
        file.new = Some(unquote(path));
    } else if let Some(path) = line.strip_prefix("--- ") {
        match side_path(path, "a/") {
            Some(path) => file.old = Some(path),
            None => file.declare(Status::Added),
        }
    } else if let Some(path) = line.strip_prefix("+++ ") {
        match side_path(path, "b/") {
            Some(path) => file.new = Some(path),
            None => file.declare(Status::Deleted),
        }
    } else if let Some(header) = line.strip_prefix("@@ ")
        && let Some(range) = hunk_range(header)
    {
        *remaining = range;
        file.hunks.push(Hunk::default());
    }
}

/// Adds one body line to the current hunk of `file`.
fn hunk_line<'a>(file: &mut Pending<'a>, remaining: &mut Remaining, line: &'a str) {
    let Some(hunk) = file.hunks.last_mut() else {
        return;
    };
    if let Some(text) = line.strip_prefix('-') {
        hunk.removed.push(Line {
            number: remaining.old_line,
            text,
        });
        remaining.old = remaining.old.saturating_sub(1);
        remaining.old_line = remaining.old_line.saturating_add(1);
    } else if let Some(text) = line.strip_prefix('+') {
        hunk.added.push(Line {
            number: remaining.new_line,
            text,
        });
        remaining.new = remaining.new.saturating_sub(1);
        remaining.new_line = remaining.new_line.saturating_add(1);
    } else if line.starts_with('\\') {
        // "\ No newline at end of file" belongs to the line before it.
    } else {
        // Context (not produced with -U0, but read correctly if present).
        remaining.old = remaining.old.saturating_sub(1);
        remaining.new = remaining.new.saturating_sub(1);
        remaining.old_line = remaining.old_line.saturating_add(1);
        remaining.new_line = remaining.new_line.saturating_add(1);
    }
}

/// `-a[,b] +c[,d] @@...`: the start and count of each side. A count left out is 1.
fn hunk_range(header: &str) -> Option<Remaining> {
    let mut parts = header.split(' ');
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (old_line, old) = start_and_count(old)?;
    let (new_line, new) = start_and_count(new)?;
    Some(Remaining {
        old,
        new,
        old_line,
        new_line,
    })
}

fn start_and_count(range: &str) -> Option<(u64, u64)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

/// The path of a `---`/`+++` line without its `prefix`; `None` for `/dev/null`.
fn side_path<'a>(path: &'a str, prefix: &str) -> Option<Cow<'a, str>> {
    (path != "/dev/null").then(|| strip_owned(unquote(path), prefix))
}

/// The two paths of a `diff --git a/<old> b/<new>` header. Unquoted paths with spaces are
/// ambiguous; they are split where both halves are the same path (the usual case, and the
/// only one where a later `---`/`+++` or `rename` line may be missing).
fn header_paths(rest: &str) -> Option<(Cow<'_, str>, Cow<'_, str>)> {
    if rest.starts_with('"') {
        let (old, after) = quoted_token(rest)?;
        let after = after.strip_prefix(' ')?;
        let new = if after.starts_with('"') {
            quoted_token(after)?.0
        } else {
            Cow::Borrowed(after)
        };
        return Some((strip_owned(old, "a/"), strip_owned(new, "b/")));
    }
    let both = rest.strip_prefix("a/")?;
    // `<p> b/<p>`: the separator sits at (len - 3) / 2 when both halves are equal.
    let half = both.len().checked_sub(3)?.checked_div(2)?;
    if let (Some(old), Some(new)) = (both.get(..half), both.get(half..))
        && new.strip_prefix(" b/") == Some(old)
    {
        return Some((Cow::Borrowed(old), Cow::Borrowed(old)));
    }
    let (old, new) = both.split_once(" b/")?;
    Some((Cow::Borrowed(old), Cow::Borrowed(new)))
}

fn strip_owned<'a>(path: Cow<'a, str>, prefix: &str) -> Cow<'a, str> {
    match path {
        Cow::Borrowed(path) => Cow::Borrowed(path.strip_prefix(prefix).unwrap_or(path)),
        Cow::Owned(path) => Cow::Owned(path.strip_prefix(prefix).unwrap_or(&path).to_owned()),
    }
}

/// A path as git prints it: as is, or in C-style quotes when it holds special characters.
fn unquote(path: &str) -> Cow<'_, str> {
    match quoted_token(path) {
        Some((unquoted, "")) => unquoted,
        _ => Cow::Borrowed(path),
    }
}

/// Reads one C-style quoted string at the start of `text` (git's `quote_c_style`: `\\`, `\"`,
/// `\a\b\t\n\v\f\r` and octal bytes), returning it and the text after its closing quote.
fn quoted_token(text: &str) -> Option<(Cow<'_, str>, &str)> {
    let body = text.strip_prefix('"')?;
    let mut bytes = Vec::new();
    let mut chars = body.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => {
                let rest = body.get(at.saturating_add(1)..)?;
                let unquoted = String::from_utf8_lossy(&bytes).into_owned();
                return Some((Cow::Owned(unquoted), rest));
            }
            '\\' => {
                let (_, escaped) = chars.next()?;
                let byte = match escaped {
                    'a' => 0x07,
                    'b' => 0x08,
                    't' => b'\t',
                    'n' => b'\n',
                    'v' => 0x0b,
                    'f' => 0x0c,
                    'r' => b'\r',
                    '0'..='7' => {
                        let (_, second) = chars.next()?;
                        let (_, third) = chars.next()?;
                        octal([escaped, second, third])?
                    }
                    other => u8::try_from(other).ok()?,
                };
                bytes.push(byte);
            }
            other => {
                let mut buffer = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    None
}

/// Three octal digits as one byte.
fn octal(digits: [char; 3]) -> Option<u8> {
    let mut value: u32 = 0;
    for digit in digits {
        value = value.checked_mul(8)?.checked_add(digit.to_digit(8)?)?;
    }
    u8::try_from(value).ok()
}

#[cfg(test)]
mod tests;
