//! The `anneal` CLI as a library: the binary in `main.rs` only hands it the process's
//! arguments and streams.
//!
//! This library is internal to the `anneal` binary and its tests (`publish = false`); its
//! items are not a stable API.
//!
//! - [`cli`]: the command tree and [`cli::run`], the entry point.
//! - [`command`]: the closed set of commands and what a green result from each proves.
//! - [`exit_codes`]: the exit-code table, rendered into `--help` and `docs/exit-codes.md`.
//! - [`error`]: errors that stop a command, each with a global exit code.
//! - [`envelope`]: the JSON envelope on stdout and its findings.
//! - [`hook`]: silent-on-success, one-`ERROR`-line-on-failure output for hooks.
//! - [`lint`]: `anneal lint skills` and the lint finding model.
//! - [`baseline`]: snapshot findings at session start, report only new ones later.
//! - [`paths`]: the git common dir (ledger home) and the anneal home (`~/.anneal`).

pub mod baseline;
pub mod cli;
pub mod command;
pub mod envelope;
pub mod error;
pub mod exit_codes;
pub mod hook;
pub mod lint;
pub mod paths;
