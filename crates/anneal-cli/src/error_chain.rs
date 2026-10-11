//! An error and its sources as one line: what hooks, `floor-guard` and the ledger command show
//! for a failure. Crate-private; `hook::reason` re-exports it.

use std::error::Error;

/// The one-line reason for `error`: its message followed by each source, joined by `": "`,
/// with any line break folded into a space so the result is a single line.
#[must_use]
pub fn reason(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::reason;
    use std::io;

    #[derive(Debug, thiserror::Error)]
    #[error("reading\nthe ledger")]
    struct Outer(#[source] io::Error);

    #[test]
    fn the_reason_is_one_line_with_every_source() {
        let error = Outer(io::Error::other("disk full"));

        assert_eq!(reason(&error), "reading the ledger: disk full");
    }
}
