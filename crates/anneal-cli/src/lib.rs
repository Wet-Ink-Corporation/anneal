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
//!   `PreToolUse` preconditions hook on file edits), and the one-`ERROR`-line output
//!   hooks share.
//! - [`lint`]: `anneal lint skills|terms|spec|plan` and the lint finding model.
//! - [`plan`]: `anneal plan approve|check`, a plan approval recorded in the ledger by hash.
//! - [`mode`]: `anneal mode build|diagnose|off`, the worktree's anneal mode that the
//!   `PreToolUse` preconditions hook (`hook preconditions`, on file edits) enforces.
//! - [`eval`]: `anneal eval routing`, Tier 1 lexical routing over `evals/routing/`, and with
//!   `--router claude` Tier 2, the same prompts under Claude Code's router with Wilson
//!   intervals; `anneal eval uplift`, Tier 3, each entry skill's behaviour suites with the
//!   skill and without it, with Newcombe intervals.
//! - [`dag`]: `anneal dag check|frontier` over the tickets a tracker holds (markdown, GitHub).
//! - [`decide`]: `anneal decide`, one Clef question per decision point with its thresholds
//!   applied and a row in `~/.anneal/decisions/decisions.jsonl`, and `decide label`.
//! - [`calibrate`]: `anneal calibrate`, each decision point scored against its labelled
//!   outcomes (Brier, bins, ask rate), a proposed threshold and a promotion verdict.
//! - [`grill`]: `anneal grill frontier|classify|confirm` over the `grill` skill's design tree.
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
//! - [`verdict`]: `anneal verdict set|valid`, review verdicts per pull request, fresh for the
//!   head's commit or, after a rebase-only change, its patch id.
//! - [`ship`]: `anneal ship skip-rule`, whether a change may skip full review.
//! - [`verify`]: `anneal verify risk`, whether the working tree's change goes to the `verify`
//!   skill's doubt review (Clef D5 and its deterministic triggers).
//! - [`watch`]: `anneal watch pr|ci|deploy`, a long poll of a pull request, a commit's checks
//!   or a deployment that exits with one code per blocker.
//! - [`pr_diff`]: the bounded git reads of a pull request's diff (commits, base, patch id,
//!   numstat, ancestry, the diff's text) that `verdict`, `ship`, `brief` and the merge gate
//!   share.
//! - [`repo_config`]: `.anneal/config.json`, read within a size cap (`check_fast`,
//!   `review.require_plugin`, `ship.sensitive_globs`, `orchestrate.leads`).
//! - [`brief`]: `anneal brief build`, a subagent's nine-field brief and its review package
//!   under `.anneal/briefs/`.
//! - [`round`]: `anneal round start|status`, a ticket's fix-loop round counter read from the
//!   ledger.
//! - [`orchestrate`]: `anneal orchestrate fan-out|park|rule`, the fan-out gate and one-way
//!   calls parked in weigh-in (through the crate-private `weighin` bridge) or ruled on.
//! - [`glob`]: repository path globs (`**`, `*`, `?`, ASCII case-insensitive).
//! - [`statusline`]: `anneal statusline`, Claude Code's status line, which saves each
//!   session's context usage for the context-monitor hook.
//! - [`rollout`]: `anneal rollout decide`, go, hold or rollback from metrics and band tables.
//! - [`json_file`]: a JSON file named on the command line, read within a size cap.
//! - [`perf`]: `anneal perf sample|noise`, a change kept only when it is faster beyond
//!   run-to-run noise.
//! - [`migrate`]: `anneal migrate start|done|contract`, a migration's waves in the ledger;
//!   contract is refused while a wave is open.

pub mod adr;
pub mod baseline;
pub mod brief;
pub mod calibrate;
pub mod cli;
pub mod command;
mod content_hash;
mod context;
mod cycle;
pub mod dag;
pub mod decide;
pub mod envelope;
pub mod error;
mod error_chain;
pub mod eval;
mod existing_ledger;
pub mod exit_codes;
pub mod floor_guard;
pub mod glob;
pub mod grill;
pub mod hook;
pub mod json_file;
pub mod ledger;
pub mod lint;
pub mod migrate;
pub mod mode;
pub mod orchestrate;
mod path_text;
pub mod paths;
pub mod perf;
pub mod plan;
pub mod pr_diff;
mod process;
#[cfg(test)]
mod release_check;
pub mod repo_config;
pub mod repro;
pub mod rollout;
pub mod round;
pub mod rulings;
pub mod ship;
pub mod statusline;
pub mod task;
#[cfg(test)]
mod test_repo;
pub mod verdict;
pub mod verify;
pub mod watch;
mod weighin;
