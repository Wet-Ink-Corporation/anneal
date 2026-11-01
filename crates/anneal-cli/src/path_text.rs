//! Paths handled as text, so output and ledger keys read the same on every platform. Shared by
//! `lint plan` (the spec's path) and `plan` (the plan's id).

use std::borrow::Cow;

/// `text` with each `separator` replaced by `/`, borrowed when there is none to replace.
pub(crate) fn slashed(text: &str, separator: char) -> Cow<'_, str> {
    if separator == '/' || !text.contains(separator) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.replace(separator, "/"))
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::slashed;

    #[test]
    fn slashed_replaces_only_a_foreign_separator() {
        assert_eq!(slashed(r"a\b\c.md", '\\'), "a/b/c.md", "Windows");
        assert_eq!(
            slashed(r"a\b.md", '/'),
            r"a\b.md",
            "a backslash is a name on unix"
        );
        assert!(
            matches!(slashed("a/b.md", '\\'), Cow::Borrowed(_)),
            "nothing to replace"
        );
    }
}
