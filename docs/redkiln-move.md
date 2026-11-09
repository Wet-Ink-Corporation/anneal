# Moving anneal into redkiln (Move 2)

anneal moves into `Wet-Ink-Corporation/redkiln` as Bench (`rk-bench`), the suite's in-chat front end. This page is anneal's side of that move: where each path lands, what is not imported, and how anneal differs today from the SPEC and the charter it will live under. redkiln's Move 2 runbook (`docs/plan/` there) is written from it, the way the Move 1 runbook was written from Weigh-In.

Facts checked on 2026-11-08 against anneal `main` at `f80ec44` and redkiln `main` at `3a8295a`. The end state is redkiln's `references/docs/bench.md`; the formats are its `spec/`; the rules are its `AGENTS.md`. Where this page and those disagree, those win.

## Gate and state

| Item | State |
| --- | --- |
| Gate: the calibration report (#44) | Open. Ryan runs `anneal calibrate --json` on his anneal home; the report is cut from the labels that exist (Ryan, 2026-11-08) into `docs/calibration-2026-11.md`. No v0.1.0 tag or release (Bench call E) |
| Tier 3 uplift evals | Closed clean at r21 (2026-11-08): all 7 skills beat baseline, every guard held. redkiln's gate row still lists the "seed a workspace" fix as outstanding; it is done |
| Open PRs | None after the Move 2 triage (dependabot #222 and #199 merged 2026-11-08) |
| Open issues | Each one that transfers carries the `move:transfer` label and a comment naming its destination; release containers and finished releases are closed |
| Stale branches | `blacksmith-migration-fa6b1c1` and four squash-merged `claude/*` branches, for Ryan to delete (cloud sessions are refused branch deletes) |
| Lint baseline | None: anneal has no `.claude/temper-baseline.json`, so the re-baseline at the new paths adds nothing |
| Tags | `v0.0.1`, `v0.0.5`; not imported |

## Where the files land

Rows follow the migration plan's placement rule (crates to `crates/<pkg>/`, the plugin to `plugins/<pkg>/`, docs to `docs/<pkg>/`, evals to `evals/<pkg>/`, scripts to `tools/<pkg>/`).

| From (anneal) | To (redkiln) | Notes |
| --- | --- | --- |
| `crates/anneal-cli/` | `crates/bench/anneal-cli/` | Renamed at the door. Splits later (below), each once, in the monorepo |
| `crates/anneal-ledger/` | `crates/bench/anneal-ledger/` | Dissolves into `redkiln-core` plus `bench.*` types (below) |
| `plugin/` | `plugins/bench/` | Install id `rk-bench` at the door. `playbooks/` and `hooks/hooks.metadata*.json` failed `cargo xtask plugins`; both moved into the CLI (below) |
| `plugin-cockpit/` | `plugins/bench-cockpit/` | A mod plugin with its own launcher copy. `hooks/cockpit.tsx` moves to `hooks/mod/cockpit.tsx` (the mod path `xtask` allows); `types/index.d.ts` stays (`types/` is allowed in a mod plugin); `tsconfig.json` and `tests/` to `tools/bench/cockpit-check/` |
| `mod-types/` | `tools/bench/cockpit-check/` | Type stubs for the cockpit's type check |
| `docs/` | `docs/bench/` | Records keep their numbers |
| `evals/` | `evals/bench/` | Not shipped |
| `scripts/test-launcher.sh` | `tools/launcher/` | Beside the shared launcher's tests |
| `scripts/build-plugin-bins.*`, `changelog-section.sh`, `check-versions.sh` | Not placed | Replaced by `cargo xtask` packaging, version and changelog tasks |
| `.kb/_intake/decisions/` (20 atoms) | `.kb/_intake/decisions/` | Weigh-In 2.0 moves atoms to `.kb/decision/`; the D6 reader (`hook/ruled_before/precedents.rs`, `ATOMS_DIR`) must read both at Move 2 |
| `CHANGELOG.md`, `README.md` | `plugins/bench/` | Per-package files |
| Root `AGENTS.md`, `CLAUDE.md` | `docs/bench/charter.md`, `docs/bench/working-rules.md` | The decisions log moves to `docs/bench/decisions.md` |
| `.github/workflows/*` | Not placed | Rewritten into redkiln's `ci.yml` under the `ok` check |
| `.claude/` (temper copy, rules, settings), `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`, `deny.toml`, `.cargo/`, `.githooks/`, licences, `.claude-plugin/marketplace.json` | Not placed | The root's copies govern. Diff each in the import PR; anything anneal needs that the root lacks is a decision for Ryan |

## Plugin layout against `cargo xtask plugins`

Run on 2026-11-08 over copies of both plugins placed in a redkiln checkout. 13 files fail; everything else passes. The 7 playbook files and the 2 hook metadata files are resolved (below), leaving 4.

| File | Resolution |
| --- | --- |
| `plugin/playbooks/` (7 files: two templates, `route.json`, `clef-thresholds.json`, two schemas, `README.md`) | **Done.** Moved to `crates/anneal-cli/playbooks/`, built into the binary and served by `anneal playbook show <name>` (`docs/playbook.md`); the skills ask the binary, and `decide` uses the built-in thresholds unless given `--thresholds` |
| `plugin/hooks/hooks.metadata.json`, `hooks.metadata.schema.json` | **Done (wave 2c, 2026-11-09).** A code table keyed by hook id (`HookName::metadata` in `crates/anneal-cli/src/hook/metadata.rs`), checked against `hooks.json` by `tests/it/hooks_manifest.rs`; `lint skills` and `lint rules` use the built-in hook ids. `plugin/hooks/` holds only `hooks.json` |
| `plugin/skills/README.md` | Moves to `docs/bench/skills.md` at the import |
| `plugin-cockpit/hooks/cockpit.tsx`, `tsconfig.json`, `tests/cockpit.test.ts` | Relocated at the import (table above); needs a human surface check of the mod |

## The ledger against the SPEC

anneal's ledger is `crates/anneal-ledger/` (about 3.0k non-test lines), described in `docs/ledger.md`, registry version 10.

| Topic | anneal today | SPEC |
| --- | --- | --- |
| File | `<git-common-dir>/anneal/ledger.sqlite3`, `$ANNEAL_STORE`, `--store` | `<git-common-dir>/redkiln/events.sqlite3`, `$REDKILN_STORE`, `suite.toml` `store` (ST-01); the old variable may be read for one major |
| happenstance | 0.3.2 | `=0.4.5`, `happenstance-sqlite` with `sync` (ST-04) |
| Journal mode | TRUNCATE; a file in WAL is refused | TRUNCATE; WAL converted on open, a stale `-wal` renamed aside (ST-05) |
| Schema stamp | `user_version` 1 | `major * 1000 + minor`, 1.0 = `1000` (ST-06) |
| Envelope | Flat: `type`, payload fields, `actor`, `session_id`, `worktree`, `branch`, `head`, `ts` | Nested `data`, plus `version`, `trust`, `writer`, optional `origin` and `cites` (EV-01) |
| Type names | Unprefixed (`plan_approved`, `migrate.wave.started`) | `<namespace>.<name>`, here `bench.*` (EV-03) |
| Reads | Strict: the first unknown type fails a query | A reader skips unknown types (EV-08) and counts unusable ones (EV-10) |
| Worktree | Normalised top-level path | A hashed id, `wt-…` (ID-06) |
| Plan approvals | `plan_approved` with a `sha256` | `core.approval_recorded`, imported with `asserted` trust (AP-05) |
| Old ledgers | n/a | Imported once on first run and never deleted (ST-15, ST-16) |

### Event types (17)

| Today | In the suite (Bench plan) |
| --- | --- |
| `grill_confirmed`, `test_run`, `task_done`, `repro_red`, `mode_set`, `round_started`, `gate_blocked`, `stop_blocked`, `subagent_status` | Same names under `bench.` |
| `plan_approved` | `bench.plan_approved`, citing Core's `core.approval_recorded` |
| `verdict_set` | `review.verdict`, owned by Review; Bench only reads it |
| `orchestrate.ruled` | A two-way Weigh-In item through `weighin-contract` (call F) |
| `weighin_parked`, `bridge.unavailable` | Dropped: `weighin-contract` replaces the text bridge |
| `migrate.wave.started`, `migrate.wave.done`, `migrate.contract` | Frozen with `migrate` |

## The CLI against the SPEC

| Topic | anneal today | SPEC |
| --- | --- | --- |
| JSON output | Commands write the envelope by default, with no flag; hooks write Claude Code's hook response instead, `status` prints text unless given `--json`, and `snapshot write --json` names its stdin format, not its output | One JSON object when run with `--json` (CLI-01) |
| Envelope keys | `ok`, `command`, `data`, `findings`, `proves`, `does_not_prove` | The same plus `schema`, `<namespace>.<command>/<n>` (CLI-02) |
| Exit 2 | Clap's usage error, and a command-specific meaning in 11 commands, plus the hook guard's failed-closed exit 2 that CLI-10 allows (`docs/exit-codes.md`) | Never a command's own meaning, except a guard that cannot write its deny (CLI-06, CLI-10) |
| Broken pipe | Exit 74 | Exit 0, nothing on stderr (CLI-07) |
| Harness protocol | Hook payloads are handled in the `hook` module; `CLAUDE_*` variables are also read in `decide/`, `weighin/locate.rs`, `context/usage.rs` and `status/home.rs` | One adapter module per binary (CLI-13); prep moves the reads into one module |
| Commands named by skills | All exist; `tests/it/plugin_commands.rs` checks every `anneal …` span and hook entry | `cargo xtask plugins` checks named binaries |

## Seams: code that leaves Bench

Done once, in the monorepo, not in anneal. Sizes are non-test lines in `crates/anneal-cli/src/`, measured 2026-11-08.

| Code | Size | Goes to |
| --- | --- | --- |
| `hook/guard/` (guard and redaction) | about 10.6k | `redkiln-guard` kit; Bench keeps its rule pack |
| `dag`, `pr_diff`, `watch` | about 4.5k | `redkiln-github` kit |
| `verdict` | about 1.8k | Review owns `review.verdict`; `merge-gate` stays as a reader |
| `lint/skills`, `eval` | about 14.5k | `tools/skillcheck`, never shipped |
| `calibrate` | about 2.4k | Scoring to `redkiln-calibrate` |
| `rollout`, `perf`, `migrate` | about 1.7k | Frozen |
| `decide` (Clef client) | about 5.0k | Stays in Bench, behind its `Transport` port (call D) |

## Interfaces with other products

| Interface | Today | At Move 2 |
| --- | --- | --- |
| Weigh-In | `weighin batch` (park) and `weighin open --all --json` (D6), with a text receipt parse in `src/weighin/` | `weighin-contract` |
| Weigh-In decision atoms | D6 reads `.kb/_intake/decisions/` | Reads `.kb/decision/` too |
| assay / Review | `merge-gate` reads `verdict_set` and posts `anneal/verdict` | Reads `review.verdict`; the required check is Review's |
| temper / Rust | Bench's status hook reads temper's Status grammar | Core's one Status grammar |

## Open items carried into the move

- The transfer-labelled issues, including #232 (eval runs can read the checkout) and #233 (correlated judge votes), both for `tools/skillcheck`.
- Bench calls A to F (`references/docs/bench.md` in redkiln).
- Older optional calls from anneal's handoffs, not taken: a nextest slow-timeout, a typed `DoneTasks` error, and an injectable git ledger-lookup budget for the Windows `preconditions` flake.
