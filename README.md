# anneal

anneal is a Claude Code plugin that turns prose SDLC gates into enforced checks. It has three layers:

1. **Skills** for each phase of the work (`grill`, `spec`, `plan`, `build`, `diagnose`, `verify`, `ship`) and a few user-invoked ones (`/anneal:orchestrate`, `/anneal:handoff`, `/anneal:correct`, `/anneal:write-skill`, `/anneal:calibrate`).
2. **A deterministic CLI**, `anneal`, behind hooks on Claude Code events, backed by an append-only event ledger.
3. **Calibrated judgement calls** (Clef decision points) that run in shadow mode until their calibration says they may enforce.

Status: pre-release. The v0.1 plan is issue #1; each release is a Feature issue and each pull request closes one Story.

## Install

Requires Claude Code **2.1.295 or later**.

Add the marketplace, then install the plugin from it (the marketplace and the plugin are both named `anneal`):

```
/plugin marketplace add Wet-Ink-Corporation/anneal#marketplace
/plugin install anneal@anneal
```

That is the whole install. Every option is optional: with none set, anneal runs its deterministic gates and Clef stays off (see Clef below).

### How the CLI is found

The hooks run the `anneal` CLI through `plugin/libexec/anneal` (or `anneal.cmd` on Windows). Nothing is put on your PATH. The launcher takes the first of:

1. a build in the plugin's `libexec/<target>/`. The `marketplace` branch and the release zip carry Linux (x86_64 and aarch64, musl), macOS (Apple silicon) and Windows (x86_64);
2. an `anneal` on PATH (`cargo install --path crates/anneal-cli` from a checkout, or a release binary);
3. a release build it downloaded earlier, in `~/.cache/anneal/<version>/<target>/` (`$XDG_CACHE_HOME/anneal` when set, or `$ANNEAL_CACHE_DIR`);
4. the release build for the plugin's version, downloaded from the GitHub release `v<version>` (`anneal-<target>`) and kept only when it matches the release's `SHA256SUMS`. This needs `curl` or `wget`, `sha256sum` or `shasum`, and access to `github.com` and the host its release downloads redirect to. A failed download is retried after 2 minutes, resuming where it stopped. `ANNEAL_NO_FETCH=1` turns it off. The Windows launcher does not download yet.

So a plugin installed without binaries (synced from claude.ai, or from a marketplace that serves `main`) fetches its CLI at the first session start. If none of the four works, anneal is off for the session: every hook exits quietly, the guards (`guard`, `merge-gate`, `preconditions`, `brief`) included, and the session start warns that nothing anneal would block is checked, and why. Once a binary runs, a guard that fails or times out still blocks.

The binaries are built by the release workflow and are not committed to `main`. Each GitHub release carries `anneal-plugin-<version>.zip` (the plugin with every build in `libexec/`), the raw binaries and `SHA256SUMS`.

### Where it works

| Surface | Skills | Hooks and gates | Clef |
|---|---|---|---|
| Claude Code CLI and desktop (2.1.295 or later) | yes | yes | yes |
| Claude Code on the web (cloud sessions), with the plugin added in claude.ai | yes | yes, once the launcher has downloaded the CLI | no: Clef's client ignores proxy settings, and cloud sessions reach the internet only through a proxy, so every call is unreachable and fails open |
| claude.ai chat and Cowork (upload the release zip) | yes | no | no |

The skills are portable: claude.ai rejects a plugin that ships a `bin/`, and the zip has none, so it can be uploaded there and to Cowork, where the skills load. Hooks run only in Claude Code, so in claude.ai chat and Cowork nothing enforces the gates, and a skill's `anneal` commands work only where the CLI can run.

In a cloud session the plugin arrives without its `libexec/` builds, so the launcher downloads the CLI at session start. The environment's network policy must let it reach `github.com` and GitHub's release-asset host; if it does not, either allow them or put `anneal` on PATH in the environment's setup script (for example, download `anneal-x86_64-unknown-linux-musl` from the release to `/usr/local/bin/anneal`).

### Status line (once, in user settings)

anneal's context monitor warns at 60% and stops at 75% of the auto-compact window, so long sessions hand off before compaction loses their state. Hooks receive no context usage, so the monitor reads what `anneal statusline` saves. Plugins cannot set the main status line, so wire it once in `~/.claude/settings.json`. On macOS and Linux, this runs the newest installed anneal through its launcher, so it survives plugin updates:

```json
{
  "statusLine": {
    "type": "command",
    "command": "sh -c 'exec \"$(ls -td \"$HOME\"/.claude/plugins/cache/*/anneal/*/libexec/anneal | head -n 1)\" statusline'"
  }
}
```

With `anneal` on your PATH (`cargo install --path crates/anneal-cli`, and on Windows), the command is just `anneal statusline`. If you keep another status line, have your script pipe its input to `anneal statusline` as well; it prints one line (`anneal ctx 120k/200k 60%`) and saves the reading to `~/.anneal/context-usage.json`.

The window is `CLAUDE_CODE_AUTO_COMPACT_WINDOW` when set, else the model's context window, else 200,000 tokens. Until a status line saves a reading the monitor stays silent, and the session start shows a one-time setup hint. The status line is the fallback for anneal's TypeScript mod (#57), which saves the same reading through `anneal snapshot write --json` (to `~/.anneal/context-usage-mod.json`); when both save one, the monitor reads the fresher, and while the mod has saved one in the last 10 minutes the setup hint stays away. See `docs/handoff.md`.

`anneal status` shows the rest of the state anneal holds for a worktree in at most 12 lines: its mode, the plan's approval, tasks done, gates that blocked, the verdict for `HEAD`, Clef's points, the context reading and the newest ledger events. With `--json` it is one versioned document a UI can render without logic of its own. See `docs/status.md`.

### Clef (optional)

Clef answers anneal's judgement calls: seven decision points (D1 to D7), such as "is this Bash command destructive?" (the guard) or "is this question a decision for the human?" (`grill`). Each call goes to a model on Cloudflare Workers AI, its probability is checked against the point's thresholds (`plugin/playbooks/clef-thresholds.json`), and the decision is logged under `~/.anneal/decisions/`. Set it in the plugin's options (`userConfig`):

| Option | Meaning |
|---|---|
| `cf_account_id` | Cloudflare account that runs Clef. Empty (the default): Clef is off and every point uses its deterministic layer alone. |
| `cf_api_token` | Workers AI token, stored as a secret. It is never written to a log, an error or a decision row. |
| `clef_mode` | `off`, `shadow` (the default) or `enforce`, optionally followed by per-point overrides: `shadow,D1=off,D3=enforce`. |

**Shadow** asks Clef and logs what it would decide, but no caller acts on it. **Enforce** acts on Clef's answer, and only after the point has 50 answered decisions; until then it stays in shadow. Promote a point only when `anneal calibrate` (or `/anneal:calibrate`) shows it meets the bar: at least 50 labelled decisions, a Brier score of 0.15 or less and an ask rate under 20%. Then set it per point (`shadow,D1=enforce`). See `docs/decide.md` and `docs/calibrate.md`.

## Skills and evals

Model-invoked skills (`grill`, `spec`, `plan`, `build`, `diagnose`, `verify`, `ship`) load when the request fits. User-invoked skills run only when you type them: `/anneal:handoff` (at a phase boundary or when the context monitor warns), `/anneal:orchestrate` (fan an approved plan out across subagents), `/anneal:correct` (turn a mistake into a check), `/anneal:write-skill` (write a skill test-first) and `/anneal:calibrate` (score the Clef points and propose thresholds). See `plugin/skills/README.md`.

Skills are tested in four tiers (`evals/README.md`):

| Tier | What | Where it runs |
|---|---|---|
| 0 | Static checks: `anneal lint skills`, `claude plugin validate --strict` | CI; `cargo test` runs the skill lint |
| 1 | Lexical routing: 100% rank-1 on each skill's fixtures (`anneal eval routing`) | CI, through `cargo test` |
| 2 | Real routing under Claude Code's router (`anneal eval routing --router claude`) | locally before a release, with `claude` signed in |
| 3 | Behaviour uplift with and without each skill (`anneal eval uplift`) | locally before a release, with `claude` signed in; `cargo test` checks the suites (`--check`) |

Tiers 2 and 3 run `claude plugin eval`, so they spend money or plan usage (capped by `--max-cost-usd`, 25 US dollars by default) and run the plugin's hooks on the host: run them in a container.

## Develop

```sh
cargo test --workspace
cargo fmt --all --check
node .claude/temper/temper.mjs baseline --check   # clippy, rustdoc and temper's scan
cargo deny check
cargo install --path crates/anneal-cli            # the developer path: anneal on PATH
./scripts/build-plugin-bins.sh                    # release binaries into plugin/libexec/<target>/
./scripts/test-launcher.sh target/debug/anneal    # launcher smoke test
claude plugin validate --strict .                 # marketplace and plugin manifests
tsc -p plugin && claude plugin test plugin         # the mod: type check and tests (no model call)
```

Layout:

- `crates/anneal-cli/`: the `anneal` binary.
- `plugin/`: the plugin as installed (`skills/`, `hooks/` with the settings hooks and the cockpit mod, `types/` and `tests/` for the mod, `playbooks/`, `libexec/`). Only this directory is packaged. See `docs/mods.md`.
- `mod-types/`: a minimal hand-written declaration of the mods API the cockpit uses, for `tsc -p plugin` (not packaged).
- `evals/`: skill evaluation suites (not packaged).
- `scripts/`: build, release and check scripts.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
