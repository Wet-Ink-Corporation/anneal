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
//! - [`hook`]: `anneal hook <name>`, the commands `hooks.json` runs (the fail-open
//!   `SessionStart` bootstrap and the fail-closed `PreToolUse` Bash guard), and the
//!   one-`ERROR`-line output hooks share.
//! - [`lint`]: `anneal lint skills`, `anneal lint terms` and the lint finding model.
//! - [`eval`]: `anneal eval routing`, Tier 1 lexical routing over `evals/routing/`.
//! - [`dag`]: `anneal dag check|frontier` over the tickets a tracker holds (markdown, GitHub).
//! - [`grill`]: `anneal grill frontier|confirm` over the `grill` skill's design tree.
//! - [`baseline`]: snapshot findings at session start, report only new ones later.
//! - [`ledger`]: `anneal ledger append|query|tail|journal` over the `anneal-ledger` store.
//! - [`paths`]: the ledger store (normally in the git common dir) and the anneal home
//!   (`~/.anneal`).
//! - [`adr`]: ADR numbering, `docs/adr/NNNN-<slug>.md` with the next free number.
//! - [`rulings`]: the out-of-scope ruling store, `.out-of-scope/<slug>.md`, listed by id.

pub mod adr;
pub mod baseline;
pub mod cli;
pub mod command;
pub mod dag;
pub mod envelope;
pub mod error;
pub mod eval;
pub mod exit_codes;
pub mod grill;
pub mod hook;
pub mod ledger;
pub mod lint;
pub mod paths;
mod process;
pub mod rulings;
