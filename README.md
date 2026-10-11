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
