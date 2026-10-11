#!/bin/sh
# Print the CHANGELOG.md section for a version (the lines under `## <version>`, up to the next
# `## `). Exits 1 when the section is missing or empty: the release workflow uses it for the
# release notes, and CI uses it to check that a version bump came with its changelog entry.
set -eu
v=${1:?usage: changelog-section.sh <version>}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out=$(awk -v v="$v" '
  /^## / { if (found) exit; split($0, a, " "); if (a[2] == v) { found = 1; next } }
  found { print }
' "$root/CHANGELOG.md" | sed -e '/./,$!d')
[ -n "$out" ] || { echo "CHANGELOG.md has no entry for $v" >&2; exit 1; }
printf '%s\n' "$out"
