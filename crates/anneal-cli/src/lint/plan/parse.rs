//! The plan's own line syntax: `Key: value` field lines, task headings, backticked spans and
//! blocker lists. The markdown around them (sections, fences, comments) is
//! `lint::spec::markdown`'s. Reading a `Key: value` line, its backticked spans and the path a
//! `Spec:` value names is `lint::spec::field`'s, shared with a spec's `Tree:` line.

use crate::lint::spec::field::key_value;

/// The longest task id, in bytes.
pub(super) const MAX_ID_BYTES: usize = 64;

/// A field a plan line can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    /// `Spec:`, in the plan's header.
    Spec,
    /// `Files:`, in a task.
    Files,
    /// `Test:`, in a task.
    Test,
    /// `Blocked by:`, in a task.
    BlockedBy,
}

impl Key {
    /// Every task field, in the order findings name them.
    pub(super) const TASK: [Self; 3] = [Self::Files, Self::Test, Self::BlockedBy];

    /// The key as written in a plan.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "Spec",
            Self::Files => "Files",
            Self::Test => "Test",
            Self::BlockedBy => "Blocked by",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Spec, Self::Files, Self::Test, Self::BlockedBy]
            .into_iter()
            .find(|key| key.as_str().eq_ignore_ascii_case(text))
    }
}

/// The field `text` sets and its value, trimmed: `Key: value`, optionally a list item, with
/// the key optionally bold (`**Key:**` or `**Key**:`). Lines with any other key are not fields.
pub(super) fn field(text: &str) -> Option<(Key, &str)> {
    let (key, value) = key_value(text)?;
    Key::parse(key).map(|key| (key, value))
}

/// A task heading's id and title: `<id>: <title>` or `Task <id>: <title>`.
pub(super) fn task_heading(heading: &str) -> Option<(&str, &str)> {
    let heading = heading
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("task "))
        .and_then(|_| heading.get(5..))
        .unwrap_or(heading);
    let (id, title) = heading.split_once(':')?;
    let (id, title) = (id.trim(), title.trim());
    (is_id(id) && !title.is_empty()).then_some((id, title))
}

/// A task id: one word of ASCII letters, digits, `-`, `_` or `.`, at most [`MAX_ID_BYTES`].
pub(super) fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ID_BYTES
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The ids a `Blocked by:` value lists: `none` (any case) is no blockers; otherwise ids
/// separated by commas, each optionally in backticks. A trailing `.` is ignored. `Err` holds
/// the first piece that is not an id.
pub(super) fn blockers(value: &str) -> Result<Vec<&str>, &str> {
    let value = value.strip_suffix('.').unwrap_or(value).trim();
    if value.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|piece| {
            let id = piece.trim().trim_matches('`').trim();
            if is_id(id) { Ok(id) } else { Err(piece.trim()) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Key, blockers, field, task_heading};
    use crate::lint::spec::field::{code_spans, spec_target};

    #[test]
    fn fields_read_plain_bold_and_listed_forms() {
        for line in [
            "Files: `a.rs`",
            "- Files: `a.rs`",
            "* **Files:** `a.rs`",
            "**files**: `a.rs`",
        ] {
            assert_eq!(field(line), Some((Key::Files, "`a.rs`")), "{line}");
        }
        assert_eq!(
            field("- Blocked by: T1"),
            Some((Key::BlockedBy, "T1")),
            "two words"
        );
        assert_eq!(field("Note: a.rs"), None, "an unknown key");
        assert_eq!(field("**Files `a.rs`"), None, "no colon");
    }

    #[test]
    fn only_closed_non_blank_code_spans_count() {
        assert_eq!(
            code_spans("`a` and ` b ` and `` and `open"),
            vec!["a", "b"],
            "spans"
        );
    }

    #[test]
    fn a_spec_target_is_a_link_a_span_or_the_value() {
        assert_eq!(spec_target("[x](a/b.md) here"), Some("a/b.md"), "link");
        assert_eq!(spec_target("see `a.md`"), Some("a.md"), "span");
        assert_eq!(spec_target("a.md"), Some("a.md"), "plain");
        assert_eq!(spec_target("[x]()"), None, "empty link");
    }

    #[test]
    fn task_headings_are_an_id_and_a_title() {
        assert_eq!(task_heading("T1: Writer"), Some(("T1", "Writer")), "plain");
        assert_eq!(
            task_heading("task 2: Writer"),
            Some(("2", "Writer")),
            "Task prefix"
        );
        assert_eq!(task_heading("Writer"), None, "no id");
        assert_eq!(task_heading("T 1: Writer"), None, "an id is one word");
        assert_eq!(task_heading("T1:"), None, "no title");
    }

    #[test]
    fn blockers_are_none_or_a_list_of_ids() {
        assert_eq!(blockers("None."), Ok(vec![]), "none");
        assert_eq!(blockers("T1, `T2`"), Ok(vec!["T1", "T2"]), "ids");
        assert_eq!(blockers("T1,, T2"), Err(""), "an empty piece");
        assert_eq!(blockers("after T1"), Err("after T1"), "prose");
    }
}
