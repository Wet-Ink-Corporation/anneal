# anneal

anneal is a Claude Code plugin that turns prose SDLC gates into enforced checks. This repository holds the `anneal` CLI (the deterministic layer the plugin's hooks call) and the plugin itself under `plugin/`. The v0.1 plan is GitHub issue #1; each pull request closes one Story issue.

anneal moves into `Wet-Ink-Corporation/redkiln` as Bench (`rk-bench`) at Move 2. Until then only gate work (#44), security fixes and move prep land here; `docs/redkiln-move.md` is the import manifest, and redkiln's briefing limits apply (no v0.1.0 tag, no feature work the monorepo will redo).

## Crate map

| Crate | Layer | Purpose | Depends on |
|---|---|---|---|
| `crates/anneal-ledger` | adapter (+ its event domain) | the event ledger: registry, envelope, happenstance SQLite store under TRUNCATE | happenstance-core, happenstance-sqlite, tokio, time, tempfile, serde, serde_json, thiserror |
| `crates/anneal-cli` | binary (+ internal lib) | the `anneal` command: argument parsing, JSON output, exit codes | anneal-ledger, clap, serde, serde_json, thiserror |

Dependencies point inward: domain, then application, then adapters, then binaries (ARC-02). Each crate's `AGENTS.md` is its charter (ARC-08).

## Commands

| Purpose | Command |
|---|---|
| Format | `cargo fmt --all` |
| Lint | `node .claude/temper/temper.mjs baseline --check`: clippy, rustdoc and temper's scan. With no baseline it is as strict as `-D warnings`; with `.claude/temper-baseline.json` it fails only on new findings, where plain `cargo clippy -- -D warnings` would fail on every legacy one |
| Test | `cargo nextest run --workspace` (or `cargo test --workspace`), plus `cargo test --doc --workspace` |
| Docs | `cargo doc --workspace --no-deps` (rustdoc lints are part of Lint) |
| Supply chain | `cargo deny check` |
| CI-only checks | `cargo check -p <crate>` for each crate (ARC-16), `cargo hack check --workspace --feature-powerset` (API-16), `cargo machete` (CRF-05) |
| Full gate (Claude Code) | `/temper:gate`, or `node "$TEMPER_CLI" gate` |

Toolchain 1.98.1 (`rust-toolchain.toml`). MSRV 1.98.1 (`rust-version`), kept equal to the pin and to weigh-in's, raised only as an H-16 decision (MOD-01). Profile changes from the template are recorded in the decisions log (PERF-09).

## Engineering standard (rust-craft)

All Rust here follows rust-craft. The full text, with rule IDs, examples, the review rubric and the process, is the `rust-craft` skill shipped in the temper plugin. The non-negotiables for production code (tests may use `unwrap`/`expect`/`assert!`):

- H-01 no `unsafe`; H-02 no `unwrap`/`expect`; H-03 no `panic!`/`todo!`/`unimplemented!`/`unreachable!`, and no std API that panics on input the code does not control; H-04 no panicking indexing or slicing, including `map[&k]` and `deque[i]`, which no lint catches.
- H-05 no `#[allow]`; `#[expect(lint, reason)]` needs human approval; never weaken lint, CI or hook config.
- H-06 no `dbg!`, no print macros (use `tracing` or `writeln!` on a locked handle), no commented-out code, no untracked TODOs.
- H-07 no `Box::leak`/`mem::forget`/`transmute`/`static mut`/`'static` to silence lifetimes; H-08 no clones to appease the borrow checker.
- H-09 typed errors in library APIs (`thiserror`), with `anyhow` only in binaries; H-10 never discard failures; H-11 no lossy `as` casts.
- H-12 never weaken tests or checks, including manifest test switches (`test`, `doctest`, `harness`, `autotests`) and `#[ignore]` or `cfg_attr(.., ignore)` without a reason; H-13 bounded and owned resources.
- H-14 no blocking in async code, and no lock guard, `RefCell` borrow, entered tracing span or `watch::Ref` held across `.await`; H-15 zero warnings, documented public API.
- H-16 new dependencies and major-version bumps, `[patch]` overrides, `rust-version` raises, API breaks, lint exceptions, crate-boundary changes, migrations and test rewrites are human decisions.

Process: explore, frame, plan (`.temper/plans/`), red, green, refactor, self-review, gate, independent review, then report with a Status block (`DONE`, `WIP` or `BLOCKED`, plus the gate result, the review verdict, what changed, decisions needed and what was not verified).

## Decisions log

| Date | Decision | Rule | Decided by |
|---|---|---|---|
| 2026-10-11 | Ship the CLI as prebuilt binaries in `plugin/libexec/<target>/` behind a launcher, not on PATH and no `plugin/bin/` | ARC | Ryan (#9) |
| 2026-10-11 | Toolchain and MSRV pinned to 1.98.1, matching weigh-in | MOD-01 | Ryan (#9) |
| 2026-10-11 | Split the ledger into `crates/anneal-ledger` (dependency profile, reuse by hooks and commands); happenstance 0.3.2, tokio, time added as weigh-in has them; dependabot ignores `happenstance-*` | ARC-07, H-16 | story #11 |
| 2026-10-31 | Move the cockpit mod out of `plugin/` into its own plugin, `anneal-cockpit` (`plugin-cockpit/`), with its own launcher copy; add a human surface check to the release checklist | ARC, H-16 | Ryan (project thread, after Weigh-In's mod-in-chat finding) |
| 2026-11-08 | Move `plugin/playbooks/` into the CLI (`crates/anneal-cli/playbooks/`, served by `anneal playbook show <name>`), so the plugin holds no data; `decide` stops reading the plugin root's thresholds copy and uses the built-in table unless given `--thresholds` | ARC, H-16 | Ryan (redkiln move plan, D2) |
| 2026-11-08 | Move 2 prep: #44 becomes the calibration report only, cut from the labels that exist, with no v0.1.0 tag or release (Bench call E); #27 closes as superseded (temper is frozen); the status hook accepts temper's `Status: WIP` (a test rewrite); anneal freezes once the prep lands | H-12, H-16 | Ryan (project thread "Prepare anneal for the redkiln move") |

## Plugin rules

- **No `plugin/bin/`.** claude.ai rejects hosted plugins that ship one. Binaries live in `plugin/libexec/<target>/` and hooks call `${CLAUDE_PLUGIN_ROOT}/libexec/anneal` by path, in exec form.
- **Hooks fail open, guards fail closed, a missing install warns.** A guard (`--fail-closed`, `onFailure: block`) blocks when its binary fails. With no binary anywhere every hook exits 0, guards included, and `hook bootstrap` warns that anneal is off (Ryan, PR 140).
- **No mod in the `anneal` plugin.** The Claude desktop app's chats turn off a whole plugin whose hooks name a mod (`modules`), skills included, although the docs say chat ignores hooks. Mods ship in `plugin-cockpit/` (the `anneal-cockpit` plugin, same marketplace); `plugin/` keeps skills and command hooks only. The cockpit carries its own copy of the `libexec/` launchers, because a plugin cannot reach another plugin's root (Ryan, 2026-10-31, from Weigh-In's isolation of the bug).
- **Versions move together.** `Cargo.toml`, `plugin/.claude-plugin/plugin.json` and `plugin-cockpit/.claude-plugin/plugin.json` carry one version, and the two plugins' launchers stay identical (`scripts/check-versions.sh`); note user-visible changes under `## Unreleased` in `CHANGELOG.md`.
- **A release is checked on every surface a person uses**, not only where we automate. CI and AFK sessions run in Claude Code and cloud sessions only, so the release checklist (`docs/release-0.1.0.md`) has a human surface check.
- **Line endings are LF** everywhere except `*.cmd` (`.gitattributes`).
