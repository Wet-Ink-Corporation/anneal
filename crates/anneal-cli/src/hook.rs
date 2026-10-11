//! Output for hooks: silent on success, exactly one grep-able `ERROR <reason>` line on failure.

use std::error::Error;
use std::io::{self, Write};

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

/// Writes `ERROR <reason>` and a newline to `err`, as one line.
///
/// # Errors
/// Returns the I/O error when `err` cannot be written.
pub fn write_error<W: Write>(mut err: W, error: &dyn Error) -> io::Result<()> {
    writeln!(err, "ERROR {}", reason(error))
}

/// Reports a hook's result: nothing for `Ok`, one `ERROR <reason>` line for `Err`.
///
/// # Errors
/// Returns the I/O error when `err` cannot be written.
pub fn report<W: Write, E: Error>(result: &Result<(), E>, err: W) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => write_error(err, error),
    }
}

#[cfg(test)]
mod tests {
    use super::report;
    use std::io;

    #[derive(Debug, thiserror::Error)]
    #[error("guard failed\non two lines")]
    struct Outer(#[source] io::Error);

    #[test]
    fn success_writes_nothing() {
        let mut err = Vec::new();
        report::<_, io::Error>(&Ok(()), &mut err).expect("a Vec accepts every write");
        assert!(err.is_empty(), "silent on success");
    }

    #[test]
    fn failure_writes_exactly_one_error_line_with_the_source_chain() {
        let mut err = Vec::new();
        let failed: Result<(), Outer> = Err(Outer(io::Error::other("disk full")));

        report(&failed, &mut err).expect("a Vec accepts every write");

        assert_eq!(
            String::from_utf8(err).expect("UTF-8"),
            "ERROR guard failed on two lines: disk full\n",
            "one ERROR line, newlines folded, causes appended"
        );
    }
}
