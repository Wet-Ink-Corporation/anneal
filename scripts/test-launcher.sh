#!/bin/sh
# Smoke test for plugin/libexec/anneal. Takes the path of a built anneal binary.
#  1. A build in libexec/<target>/ runs with nothing on PATH.
#  2. With no build, the launcher falls back to an anneal on PATH.
#  3. With neither, a hook exits 0 (fail open), a --fail-closed hook exits 2 with one ERROR line,
#     and a plain call exits 127.
#  4. There is no bin/ directory anywhere in the plugin.
set -eu
built=${1:?usage: test-launcher.sh <path to a built anneal binary>}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64|Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
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
rm -r "$work/plugin/libexec/$target"

# 2. No build: falls back to anneal on PATH.
onpath="$work/on-path"
mkdir -p "$onpath"
cp "$built" "$onpath/anneal"
PATH="$onpath:$empty" "$sh_bin" "$launcher" --version | grep -q '^anneal ' || fail "PATH fallback"

# 3. Neither: hooks fail open, guards fail closed, plain calls exit 127.
PATH="$empty" "$sh_bin" "$launcher" hook bootstrap </dev/null || fail "a hook with no binary must exit 0"
set +e
PATH="$empty" "$sh_bin" "$launcher" hook guard --fail-closed </dev/null 2>"$work/err"
code=$?
PATH="$empty" "$sh_bin" "$launcher" --help >/dev/null 2>&1
plain=$?
set -e
[ "$code" = 2 ] || fail "a --fail-closed hook with no binary must exit 2, got $code"
[ "$(wc -l <"$work/err" | tr -d ' ')" = 1 ] && grep -q '^ERROR ' "$work/err" || fail "one ERROR line expected"
[ "$plain" = 127 ] || fail "a plain call with no binary must exit 127, got $plain"

# 4. No bin/ anywhere in the plugin.
[ -z "$(find "$root/plugin" -type d -name bin)" ] || fail "plugin ships a bin/ directory"
echo "launcher ok"
