//! Lint and test suppressions on a line: `#[allow(`, `#![allow(`, `#[ignore`, `// eslint-disable`,
//! `/* eslint-disable`, `# noqa`, `<receiver>.skip(` and `xit(`. `#[expect(` is temper's to judge.

/// The receivers whose `.skip(` skips a test (`it.skip(`, `describe.skip(`, `pytest.skip(`,
/// `@pytest.mark.skip(`), so an iterator's `.skip(1)` is not one.
const SKIP_RECEIVERS: [&str; 8] = [
    "it", "test", "describe", "context", "suite", "pytest", "unittest", "mark",
];

/// File extensions of prose, where a suppression is only mentioned, never in effect.
const PROSE: [&str; 5] = ["md", "markdown", "rst", "txt", "adoc"];

/// True for a prose file (by extension, in any case).
pub(super) fn is_prose(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| PROSE.iter().any(|prose| prose.eq_ignore_ascii_case(ext)))
}

/// The suppression `text` holds, as the pattern that matched, if any.
pub(super) fn suppression(text: &str) -> Option<&'static str> {
    if text.contains("#[allow(") || text.contains("#![allow(") {
        return Some("#[allow(...)]");
    }
    if ["#[ignore]", "#[ignore =", "#[ignore="]
        .iter()
        .any(|form| text.contains(form))
    {
        return Some("#[ignore]");
    }
    if text.contains("// eslint-disable") || text.contains("/* eslint-disable") {
        return Some("eslint-disable");
    }
    if has_noqa(text) {
        return Some("# noqa");
    }
    if has_test_skip(text) {
        return Some(".skip(");
    }
    if has_xit(text) {
        return Some("xit(");
    }
    None
}

/// `#` then optional spaces then `noqa`, in any letter case.
fn has_noqa(text: &str) -> bool {
    text.match_indices('#').any(|(at, _)| {
        text.get(at.saturating_add(1)..)
            .map(str::trim_start)
            .and_then(|rest| rest.get(..4))
            .is_some_and(|word| word.eq_ignore_ascii_case("noqa"))
    })
}

/// `.skip(` called on one of [`SKIP_RECEIVERS`].
fn has_test_skip(text: &str) -> bool {
    text.match_indices(".skip(").any(|(at, _)| {
        text.get(..at)
            .is_some_and(|before| SKIP_RECEIVERS.contains(&trailing_identifier(before)))
    })
}

/// `xit(` as a whole word (so `exit(` is not one).
fn has_xit(text: &str) -> bool {
    text.match_indices("xit(").any(|(at, _)| {
        text.get(..at)
            .is_none_or(|before| !before.ends_with(|c: char| is_identifier(c) || c == '.'))
    })
}

/// The identifier `text` ends with (possibly empty).
fn trailing_identifier(text: &str) -> &str {
    let start = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_identifier(*c))
        .last()
        .map_or(text.len(), |(at, _)| at);
    text.get(start..).unwrap_or("")
}

fn is_identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}
