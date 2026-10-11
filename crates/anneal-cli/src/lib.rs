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
//!   `SessionStart` bootstrap, the fail-closed `PreToolUse` Bash guard and the fail-closed
//!   `PreToolUse` preconditions hook on Edit and Write), and the one-`ERROR`-line output
//!   hooks share.
//! - [`lint`]: `anneal lint skills|terms|spec|plan` and the lint finding model.
//! - [`plan`]: `anneal plan approve|check`, a plan approval recorded in the ledger by hash.
//! - [`mode`]: `anneal mode build|diagnose|off`, the worktree's anneal mode that the
//!   `PreToolUse` preconditions hook (`hook preconditions`, on Edit and Write) enforces.
//! - [`eval`]: `anneal eval routing`, Tier 1 lexical routing over `evals/routing/`.
//! - [`dag`]: `anneal dag check|frontier` over the tickets a tracker holds (markdown, GitHub).
//! - [`grill`]: `anneal grill frontier|confirm` over the `grill` skill's design tree.
//! - [`task`]: `anneal task done`, recording `task_done` only after a green test command.
//! - [`floor_guard`]: `anneal floor-guard`, refusing a diff that lowers a quality floor.
//! - [`baseline`]: snapshot findings at session start, report only new ones later.
//! - [`ledger`]: `anneal ledger append|query|tail|journal` over the `anneal-ledger` store.
//! - [`paths`]: the ledger store (normally in the git common dir) and the anneal home
//!   (`~/.anneal`).
//! - [`adr`]: ADR numbering, `docs/adr/NNNN-<slug>.md` with the next free number.
//! - [`repro`]: `anneal repro record|leftovers`, the red reproduction diagnose mode waits for
//!   and the `[DEBUG-<id>]` instrumentation left behind ([`repro::debug_leftovers`]).
//! - [`rulings`]: the out-of-scope ruling store, `.out-of-scope/<slug>.md`, listed by id.

pub mod adr;
pub mod baseline;
pub mod cli;
pub mod command;
mod content_hash;
pub mod dag;
pub mod envelope;
pub mod error;
mod error_chain;
pub mod eval;
mod existing_ledger;
pub mod exit_codes;
pub mod floor_guard;
pub mod grill;
pub mod hook;
pub mod ledger;
pub mod lint;
pub mod mode;
pub mod paths;
pub mod plan;
mod process;
pub mod repro;
pub mod rulings;
pub mod task;
