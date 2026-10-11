# Changelog

All notable changes to anneal are recorded here. A release renames `## Unreleased` to `## <version> — <date>`; that section is the release notes.

## Unreleased

- Plugin skeleton: marketplace and plugin manifests with `userConfig` (`cf_account_id`, `cf_api_token` as a secret, `clef_mode`), empty hooks manifest, and the `libexec` launchers (`anneal`, `anneal.cmd`) that pick the build for the platform and fall back to PATH.
- Rust workspace (edition 2024, toolchain 1.98.1) with temper's rust-craft lints, and a scaffold `anneal` binary that answers `--help` and `--version`.
- CI on Linux, macOS and Windows; a tag-triggered release workflow that publishes per-target binaries.
