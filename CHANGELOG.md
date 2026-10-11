# Changelog

All notable changes to anneal are recorded here. A release renames `## Unreleased` to `## <version> — <date>`; that section is the release notes.

## Unreleased

- Plugin skeleton: marketplace and plugin manifests with `userConfig` (`cf_account_id`, `cf_api_token` as a secret, `clef_mode`), empty hooks manifest, and the `libexec` launchers (`anneal`, `anneal.cmd`) that pick the build for the platform and fall back to PATH.
- Rust workspace (edition 2024, toolchain 1.98.1) with temper's rust-craft lints, and a scaffold `anneal` binary that answers `--help` and `--version`.
- `anneal` CLI core: a command tree with a stub (exit 64) for every v0.1 command, a JSON result envelope (`ok`, `command`, `data`, `findings`, `proves`, `does_not_prove`), per-command exit codes shown in `--help` and generated into `docs/exit-codes.md`, and one `ERROR <reason>` line on stderr for every error.
- `anneal lint skills [paths]`: checks skill directories against the 21 skill design rules (`docs/skill-rules.md`). Every finding has a rule, a checker id, `file:line` and a fix string; severities are error (exit 1), warning and review (listed, exit 0). Frontmatter is hand-parsed, `--cache` replays unchanged skills by content hash, and CI lints `plugin/skills`.
- CI on Linux, macOS and Windows; a tag-triggered release workflow that publishes per-target binaries.
