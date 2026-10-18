#!/bin/sh
# Smoke test for plugin/libexec/anneal. Takes the path of a built anneal binary.
#  1. A build in libexec/<target>/ runs with nothing on PATH, and `hook guard --fail-closed`
#     through it denies a force push with a PreToolUse deny decision.
#  2. With no build, the launcher falls back to an anneal on PATH.
#  3. With neither and nothing to download with, a hook exits 0 (fail open), `hook bootstrap`
#     also prints the install hint as SessionStart output, a --fail-closed hook exits 2 with one
#     ERROR line, and a plain call exits 127.
#  4. There is no bin/ directory anywhere in the plugin, and the launcher is executable (hooks
#     start it by path with no shell).
#  5. With neither but a release to download (a file:// one), the launcher fetches this
#     platform's build, keeps it only when it matches SHA256SUMS, caches and runs it, and runs
#     the cached build with no release in reach; a build that does not match is refused.
set -eu
built=${1:?usage: test-launcher.sh <path to a built anneal binary>}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
# Never read or fill the real download cache.
ANNEAL_CACHE_DIR="$work/cache"
export ANNEAL_CACHE_DIR

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu; published=x86_64-unknown-linux-musl ;;
  Linux-aarch64|Linux-arm64) target=aarch64-unknown-linux-gnu; published=aarch64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin; published=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin; published=x86_64-apple-darwin ;;
  *) echo "skip: the sh launcher is not used on $(uname -s)"; exit 0 ;;
esac
sh_bin=$(command -v sh)
cp -R "$root/plugin" "$work/plugin"
launcher="$work/plugin/libexec/anneal"
empty="$work/empty-path"
mkdir -p "$empty"
# The launcher needs a few POSIX tools; give it only those, never an anneal.
for tool in uname dirname chmod; do
  ln -s "$(command -v "$tool")" "$empty/$tool"
done

# 1. A built binary beside the launcher runs with nothing else on PATH.
mkdir -p "$work/plugin/libexec/$target"
cp "$built" "$work/plugin/libexec/$target/anneal"
PATH="$empty" "$sh_bin" "$launcher" --help | grep -q '^anneal:' || fail "--help through libexec/$target"
# The guard as hooks.json runs it; the ledger is named so no git is needed.
printf '%s' '{"tool_name":"Bash","tool_input":{"command":"git push -f origin main"}}' \
  | PATH="$empty" ANNEAL_STORE="$work/ledger.sqlite3" "$sh_bin" "$launcher" hook guard --fail-closed >"$work/deny" \
  || fail "hook guard through libexec/$target must exit 0 with its decision"
grep -q '"permissionDecision":"deny","permissionDecisionReason":"ERROR guard force-push: ' "$work/deny" \
  || fail "hook guard through libexec/$target must deny a force push to main"
rm -r "$work/plugin/libexec/$target"

# 2. No build: falls back to anneal on PATH.
onpath="$work/on-path"
mkdir -p "$onpath"
cp "$built" "$onpath/anneal"
PATH="$onpath:$empty" "$sh_bin" "$launcher" --version | grep -q '^anneal ' || fail "PATH fallback"

# 3. Neither: hooks fail open, guards fail closed, plain calls exit 127.
PATH="$empty" "$sh_bin" "$launcher" hook bootstrap </dev/null >"$work/hint" || fail "a hook with no binary must exit 0"
grep -q '"hookEventName":"SessionStart","additionalContext":"anneal is not installed' "$work/hint" \
  || fail "hook bootstrap with no binary must print the install hint as SessionStart context"
if command -v python3 >/dev/null 2>&1; then
  python3 -c 'import json, sys; json.load(sys.stdin)' <"$work/hint" || fail "the hint is not valid JSON"
fi
PATH="$empty" "$sh_bin" "$launcher" hook other </dev/null >"$work/other" || fail "a hook with no binary must exit 0"
[ ! -s "$work/other" ] || fail "other hooks with no binary must stay silent"
set +e
PATH="$empty" "$sh_bin" "$launcher" hook guard --fail-closed </dev/null 2>"$work/err"
code=$?
PATH="$empty" "$sh_bin" "$launcher" --help >/dev/null 2>&1
plain=$?
set -e
[ "$code" = 2 ] || fail "a --fail-closed hook with no binary must exit 2, got $code"
[ "$(wc -l <"$work/err" | tr -d ' ')" = 1 ] && grep -q '^ERROR ' "$work/err" || fail "one ERROR line expected"
[ "$plain" = 127 ] || fail "a plain call with no binary must exit 127, got $plain"

# 4. No bin/ anywhere in the plugin, and the launcher keeps its executable bit.
[ -z "$(find "$root/plugin" -type d -name bin)" ] || fail "plugin ships a bin/ directory"
[ -x "$root/plugin/libexec/anneal" ] || fail "plugin/libexec/anneal is not executable"

# 5. Neither, but a release to download.
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}
if command -v curl >/dev/null 2>&1 && { command -v sha256sum || command -v shasum; } >/dev/null 2>&1; then
  version=$(sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$root/plugin/.claude-plugin/plugin.json")
  release="$work/release"
  tools="$work/tools"
  fetchcache="$work/fetch-cache"
  mkdir -p "$release" "$tools"
  for tool in uname dirname chmod curl awk sed mkdir mv rm rmdir find sha256sum shasum; do
    if p=$(command -v "$tool" 2>/dev/null); then
      ln -s "$p" "$tools/$tool"
    fi
  done
  cp "$built" "$release/anneal-$published"
  (cd "$release" && sha256 "anneal-$published") >"$release/SHA256SUMS"
  printf '%s  %s\n' 0000000000000000000000000000000000000000000000000000000000000000 anneal-other >>"$release/SHA256SUMS"
  PATH="$tools" ANNEAL_CACHE_DIR="$fetchcache" ANNEAL_RELEASE_URL="file://$release" "$sh_bin" "$launcher" --version \
    | grep -q '^anneal ' || fail "the launcher must download, check and run the release build"
  [ -x "$fetchcache/$version/$published/anneal" ] || fail "the downloaded build must be kept in the cache"
  PATH="$tools" ANNEAL_CACHE_DIR="$fetchcache" ANNEAL_RELEASE_URL="file://$work/nowhere" "$sh_bin" "$launcher" --version \
    | grep -q '^anneal ' || fail "the cached build must run with no release in reach"
  rm -r "$fetchcache"
  printf '%s  %s\n' 0000000000000000000000000000000000000000000000000000000000000000 "anneal-$published" >"$release/SHA256SUMS"
  set +e
  PATH="$tools" ANNEAL_CACHE_DIR="$fetchcache" ANNEAL_RELEASE_URL="file://$release" "$sh_bin" "$launcher" hook guard --fail-closed </dev/null 2>"$work/mismatch"
  code=$?
  set -e
  [ "$code" = 2 ] || fail "a build that does not match SHA256SUMS must not run, got $code"
  grep -q '^ERROR .*SHA256SUMS' "$work/mismatch" || fail "the refusal must name SHA256SUMS"
  [ ! -e "$fetchcache/$version/$published/anneal" ] || fail "a build that does not match SHA256SUMS must not be kept"
else
  echo "skip: no curl, or no sha256sum or shasum, for the download case"
fi
echo "launcher ok"
