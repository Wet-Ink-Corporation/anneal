#!/bin/sh
# Fail unless Cargo.toml's workspace version and plugin.json's version agree, and CHANGELOG.md
# has the `## Unreleased` section (0.0.0) or a section for that version.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
plugin=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$root/plugin/.claude-plugin/plugin.json")
cargo=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$plugin" ] || { echo "ERROR plugin.json has no version" >&2; exit 1; }
[ "$plugin" = "$cargo" ] || { echo "ERROR Cargo.toml ($cargo) and plugin.json ($plugin) disagree on the version" >&2; exit 1; }
if [ "$plugin" = "0.0.0" ]; then
  grep -q '^## Unreleased' "$root/CHANGELOG.md" || { echo "ERROR CHANGELOG.md has no ## Unreleased section" >&2; exit 1; }
else
  "$root/scripts/changelog-section.sh" "$plugin" >/dev/null
fi
echo "version $plugin"
