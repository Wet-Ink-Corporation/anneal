#!/bin/sh
# Fail unless Cargo.toml's workspace version and both plugins' plugin.json versions (anneal and
# anneal-cockpit) agree, the cockpit's launchers are byte-for-byte copies of anneal's, and
# CHANGELOG.md has the `## Unreleased` section (0.0.0) or a section for that version.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
plugin=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$root/plugin/.claude-plugin/plugin.json")
cargo=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$plugin" ] || { echo "ERROR plugin.json has no version" >&2; exit 1; }
[ "$plugin" = "$cargo" ] || { echo "ERROR Cargo.toml ($cargo) and plugin.json ($plugin) disagree on the version" >&2; exit 1; }
cockpit=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$root/plugin-cockpit/.claude-plugin/plugin.json")
[ "$cockpit" = "$plugin" ] || { echo "ERROR plugin-cockpit/.claude-plugin/plugin.json ($cockpit) and plugin.json ($plugin) disagree on the version" >&2; exit 1; }
for f in anneal anneal.cmd; do
  cmp -s "$root/plugin/libexec/$f" "$root/plugin-cockpit/libexec/$f" || { echo "ERROR plugin-cockpit/libexec/$f is not a copy of plugin/libexec/$f (cp it across)" >&2; exit 1; }
done
if [ "$plugin" = "0.0.0" ]; then
  grep -q '^## Unreleased' "$root/CHANGELOG.md" || { echo "ERROR CHANGELOG.md has no ## Unreleased section" >&2; exit 1; }
else
  "$root/scripts/changelog-section.sh" "$plugin" >/dev/null
fi
echo "version $plugin"
