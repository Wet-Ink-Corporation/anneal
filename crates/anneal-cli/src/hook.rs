//! `anneal hook <name>`, the commands `plugin/hooks/hooks.json` runs, and the output helpers
//! hooks share: silent on success, exactly one grep-able `ERROR <reason>` line on failure.
//!
//! Hooks fail open: a hook's own failures are reported in its output (or as an `ERROR` line on
//! stderr when stdout cannot be written) and it still exits 0. Only a usage error exits 2.
//! `hooks.metadata.json` describes each hook; its ids are the [`HookName`]s.

pub(crate) mod bootstrap;

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand};

/// Arguments of `anneal hook`.
#[derive(Debug, Args)]
pub struct HookArgs {
    /// The hook to run.
    #[command(subcommand)]
    pub hook: HookName,
}

/// The hooks, by their id in `hooks.metadata.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum HookName {
    /// `SessionStart`: inject the bootstrap index, and the live state when it changed
    Bootstrap,
}

impl HookName {
    /// Every hook.
    pub const ALL: [Self; 1] = [Self::Bootstrap];

    /// The hook's id, as typed after `anneal hook` and as listed in `hooks.metadata.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
        }
    }
}

/// What a hook reads from its process, read at the edge (`cli`) and passed in so tests can
/// supply their own.
pub(crate) struct HookEnv<R, D, L> {
    /// The hook input (Claude Code writes one JSON object).
    pub(crate) stdin: R,
    /// Reads the working directory, called only when the input names none.
    pub(crate) current_dir: D,
    /// Reads one environment variable.
    pub(crate) lookup: L,
}

/// Runs `anneal hook <name>` with what it reads from `env`, and writes the hook's output to
/// `out`. Pass `&mut out` to keep the writer.
///
/// # Errors
/// Only the I/O error from writing `out`; every other failure is reported in the output.
pub(crate) fn run<R, D, L, W>(args: &HookArgs, env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    match args.hook {
        HookName::Bootstrap => bootstrap::run(env, out),
    }
}

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
