//! `Key: value` header lines that name a file, as a plan's `Spec:` line and a spec's `Tree:`
//! line are written: the line's key and value ([`key_value`]), the path its value names
//! ([`spec_target`]) and where that path lies ([`linked_path`]). `pub(crate)` beside
//! `spec::markdown`, so `lint plan` reads its `Spec:` line as `lint spec` reads its `Tree:`
//! line, and `lint spec` never reaches into `lint plan`.

use crate::path_text::slashed;

/// The key and value of a `Key: value` line, both trimmed, whatever the key: optionally a list
/// item, the key optionally bold (`**Key:**` or `**Key**:`). A plan's field lines and a spec's
/// `Tree:` line are read with it.
pub(crate) fn key_value(text: &str) -> Option<(&str, &str)> {
    let mut rest = text.trim_start();
    if let Some(after) = rest
        .strip_prefix(['-', '*', '+'])
        .filter(|after| after.starts_with([' ', '\t']))
    {
        rest = after.trim_start();
    }
    let bold = rest.strip_prefix("**");
    let (key, value) = bold.unwrap_or(rest).split_once(':')?;
    let key = key.strip_suffix("**").unwrap_or(key);
    let value = match bold {
        Some(_) => value.strip_prefix("**").unwrap_or(value),
        None => value,
    };
    Some((key.trim(), value.trim()))
}

/// The closed backtick spans of `text`, trimmed, without blank ones.
pub(crate) fn code_spans(text: &str) -> Vec<&str> {
    let pieces: Vec<&str> = text.split('`').collect();
    // Spans are the odd pieces; the last piece follows the last backtick, so it is never
    // a closed span.
    let closed = pieces.len().saturating_sub(1);
    pieces
        .iter()
        .take(closed)
        .skip(1)
        .step_by(2)
        .map(|span| span.trim())
        .filter(|span| !span.is_empty())
        .collect()
}

/// The path a `Spec:` value names: a markdown link's destination, else the first backticked
/// span, else the value itself. `None` when it names nothing. A spec's `Tree:` value is read
/// the same way.
///
/// A destination written in angle brackets (`[s](<my spec.md>)`, `CommonMark`'s form for one
/// holding spaces or parentheses) is what lies inside them; a bare `<...>` value is no link
/// and is returned whole, so a template placeholder still reads as one.
pub(crate) fn spec_target(value: &str) -> Option<&str> {
    let target = if let Some((_, after)) = value.split_once("](") {
        link_destination(after)
    } else if let Some(span) = code_spans(value).first() {
        span
    } else {
        value
    };
    Some(target.trim()).filter(|target| !target.is_empty())
}

/// The destination of a link whose text `after` follows `](`: inside `<` and the first `>`
/// when it opens with `<` and that `>` is there, else up to the first `)`, else all of it.
fn link_destination(after: &str) -> &str {
    let pointy = after
        .strip_prefix('<')
        .and_then(|inside| inside.split_once('>'))
        .map(|(inside, _)| inside);
    pointy.unwrap_or_else(|| after.split_once(')').map_or(after, |(target, _)| target))
}

/// The path of the file a header line of the file `from` names as `target`, where `separator`
/// is the platform's path separator: an absolute `target` as is, otherwise `target` joined to
/// `from`'s directory. Joined, and the directory's separators replaced, with `/` rather than
/// `Path::join`, so the path shown (and keyed) is the same on every platform; Windows opens
/// `/`-separated paths too. A plan's spec and a spec's design tree are resolved with it.
pub(crate) fn linked_path(from: &str, target: &str, separator: char) -> String {
    let drive = separator == '\\'
        && matches!(target.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    if drive || target.starts_with(['/', separator]) {
        return target.to_owned();
    }
    match slashed(from, separator).rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{target}"),
        None => target.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::spec_target;

    #[test]
    fn a_link_destination_in_angle_brackets_is_what_lies_inside_them() {
        assert_eq!(
            (
                spec_target("[t](<../trees/t.json>)"),
                spec_target("[t](<a (b).json>) here"),
                spec_target("[t](<>)"),
            ),
            (Some("../trees/t.json"), Some("a (b).json"), None),
            "inside `<` and `>`; an empty one names nothing"
        );
    }

    #[test]
    fn an_unclosed_angle_bracket_and_a_bare_placeholder_are_read_as_before() {
        assert_eq!(
            (
                spec_target("[t](<a.json)"),
                spec_target("<the design tree, relative to this spec>"),
            ),
            (
                Some("<a.json"),
                Some("<the design tree, relative to this spec>")
            ),
            "no closing `>` is a plain destination; a value that is no link is whole"
        );
    }
}
