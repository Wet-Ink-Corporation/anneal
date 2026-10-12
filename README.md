# anneal

anneal is a Claude Code plugin that turns prose SDLC gates into enforced checks. It has three layers:

1. **Skills** for each phase of the work (`grill`, `spec`, `plan`, `build`, `diagnose`, `verify`, `ship`) and a few user-invoked ones (`/orchestrate`, `/handoff`, `/correct`, `/write-skill`, `/calibrate`).
2. **A deterministic CLI**, `anneal`, behind hooks on Claude Code events, backed by an append-only event ledger.
3. **Calibrated judgement calls** (Clef decision points) that run in shadow mode until their calibration says they may enforce.

Status: pre-release. The v0.1 plan is issue #1; each release is a Feature issue and each pull request closes one Story.

## Install

Requires Claude Code **2.1.295 or later**.

```
/plugin install anneal --marketplace Wet-Ink-Corporation/anneal
```

The CLI ships inside the plugin as prebuilt binaries under `plugin/libexec/<target>/`, started through `plugin/libexec/anneal` (or `anneal.cmd` on Windows). Nothing is put on your PATH. If no build exists for your platform the launcher falls back to an `anneal` on PATH.

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

The window is `CLAUDE_CODE_AUTO_COMPACT_WINDOW` when set, else the model's context window, else 200,000 tokens. Until a status line saves a reading the monitor stays silent, and the session start shows a one-time setup hint. See `docs/handoff.md`.

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
```

Layout:

- `crates/anneal-cli/`: the `anneal` binary.
- `plugin/`: the plugin as installed (`skills/`, `hooks/`, `playbooks/`, `libexec/`). Only this directory is packaged.
- `evals/`: skill evaluation suites (not packaged).
- `scripts/`: build, release and check scripts.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
