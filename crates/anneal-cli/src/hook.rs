//! `anneal hook <name>`, the commands `plugin/hooks/hooks.json` runs, and the output helpers
//! hooks share: silent on success, exactly one grep-able `ERROR <reason>` line on failure.
//!
//! Hooks fail open: a hook's own failures are reported in its output (or as an `ERROR` line on
//! stderr when stdout cannot be written) and it still exits 0. Only a usage error exits 2. The
//! exception is a guard (`hook guard`, `hook preconditions`, or any hook passed `--fail-closed`),
//! which fails closed: it denies what it cannot judge, and exits 2 when its decision cannot be
//! written.
//! `hooks.metadata.json` describes each hook; its ids are the [`HookName`]s.

pub(crate) mod bootstrap;
pub(crate) mod completeness;
pub(crate) mod fast_check;
pub(crate) mod guard;
pub(crate) mod preconditions;
pub(crate) mod status;
mod stop_output;
mod transcript;

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::Serialize;

/// Arguments of `anneal hook`.
#[derive(Debug, Args)]
pub struct HookArgs {
    /// The hook to run.
    #[command(subcommand)]
    pub hook: HookName,
    /// Fail closed: exit 2 when the hook's output cannot be written. The launcher reads the
    /// same flag to exit 2 when no binary is installed. A guard always fails closed.
    #[arg(long, global = true)]
    pub fail_closed: bool,
}

impl HookArgs {
    /// Whether a failure to deliver the hook's output blocks (exit 2) rather than failing open.
    #[must_use]
    pub const fn fails_closed(&self) -> bool {
        self.fail_closed || self.hook.is_guard()
    }
}

/// The hooks, by their id in `hooks.metadata.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum HookName {
    /// `SessionStart`: inject the bootstrap index, and the live state when it changed
    Bootstrap,
    /// `PreToolUse` (Bash): deny commands on the denylist; fails closed
    Guard,
    /// `PreToolUse` (Edit|Write): in an anneal mode, deny source edits until its preconditions
    /// hold; fails closed
    Preconditions,
    /// `PostToolBatch`: in an anneal mode, run the fast checks once per batch of edits; fails
    /// open, but a lowered floor blocks
    FastCheck,
    /// `Stop`: block a success claim without a fresh green test run, and `[DEBUG-` leftovers;
    /// nudge on the playbook's missing events; fails open
    Completeness,
    /// `SubagentStop`: record the subagent's status block, timing and tokens; fails open
    Status,
}

impl HookName {
    /// Every hook.
    pub const ALL: [Self; 6] = [
        Self::Bootstrap,
        Self::Guard,
        Self::Preconditions,
        Self::FastCheck,
        Self::Completeness,
        Self::Status,
    ];

    /// The hook's id, as typed after `anneal hook` and as listed in `hooks.metadata.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Guard => "guard",
            Self::Preconditions => "preconditions",
            Self::FastCheck => "fast-check",
            Self::Completeness => "completeness",
            Self::Status => "status",
        }
    }

    /// Whether the hook enforces a gate, and so fails closed.
    #[must_use]
    pub const fn is_guard(self) -> bool {
        match self {
            // fast-check blocks on a lowered floor and completeness on a claim, but their own
            // failures never do.
            Self::Bootstrap | Self::FastCheck | Self::Completeness | Self::Status => false,
            Self::Guard | Self::Preconditions => true,
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
/// Only the I/O error from writing `out`; every other failure is reported in the output (a
/// guard denies).
pub(crate) fn run<R, D, L, W>(args: &HookArgs, env: HookEnv<R, D, L>, out: W) -> io::Result<()>
where
    R: Read,
    D: FnOnce() -> io::Result<PathBuf>,
    L: Fn(&str) -> Option<OsString>,
    W: Write,
{
    match args.hook {
        HookName::Bootstrap => bootstrap::run(env, out),
        HookName::Guard => guard::run(env, out),
        HookName::Preconditions => preconditions::run(env, out),
        HookName::FastCheck => fast_check::run(env, out),
        HookName::Completeness => completeness::run(env, out),
        HookName::Status => status::run(env, out),
    }
}

/// Why a hook's stdin could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StdinError {
    /// stdin could not be read.
    #[error("reading stdin")]
    Read(#[source] io::Error),
    /// stdin held more than the hook's cap.
    #[error("the input is larger than {max} bytes")]
    TooLarge {
        /// The cap, in bytes.
        max: u64,
    },
}

/// Reads all of `stdin`, refusing more than `max` bytes (H-13): it reads one byte past the cap,
/// so a larger input is an error, never silently truncated.
pub(crate) fn read_stdin<R: Read>(stdin: R, max: u64) -> Result<Vec<u8>, StdinError> {
    let mut bytes = Vec::new();
    stdin
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(StdinError::Read)?;
    if u64::try_from(bytes.len()).is_ok_and(|len| len <= max) {
        Ok(bytes)
    } else {
        Err(StdinError::TooLarge { max })
    }
}

/// `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", ...}}`
/// (or `"ask"`).
#[derive(Debug, Serialize)]
struct DenyOutput<'a> {
    #[serde(rename = "hookSpecificOutput")]
    specific: PreToolUseOutput<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreToolUseOutput<'a> {
    hook_event_name: &'static str,
    permission_decision: &'static str,
    permission_decision_reason: &'a str,
}

/// Writes Claude Code's `PreToolUse` deny decision, with `line` (an `ERROR <reason>` line) as
/// its reason, any line break folded into a space, then a newline.
///
/// # Errors
/// Returns the I/O error when `out` cannot be written.
pub(crate) fn write_deny<W: Write>(out: W, line: &str) -> io::Result<()> {
    write_permission(out, "deny", line)
}

/// Writes Claude Code's `PreToolUse` ask decision (the human confirms the call), with `line` as
/// its reason, any line break folded into a space, then a newline.
///
/// # Errors
/// Returns the I/O error when `out` cannot be written.
pub(crate) fn write_ask<W: Write>(out: W, line: &str) -> io::Result<()> {
    write_permission(out, "ask", line)
}

/// Writes a `PreToolUse` permission `decision` with `line`, folded to one line, as its reason.
fn write_permission<W: Write>(mut out: W, decision: &'static str, line: &str) -> io::Result<()> {
    let line = line.replace(['\r', '\n'], " ");
    let output = DenyOutput {
        specific: PreToolUseOutput {
            hook_event_name: "PreToolUse",
            permission_decision: decision,
            permission_decision_reason: &line,
        },
    };
    serde_json::to_writer(&mut out, &output)?;
    writeln!(out)
}

pub use crate::error_chain::reason;

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
