#!/bin/sh
# Build the release binaries the plugin launcher looks for, into plugin/libexec/<target>/.
# With no argument it builds for this machine (the host triple). `linux` cross-builds static
# x86_64 musl and `windows` cross-builds x86_64 mingw, both from Linux; on Windows run
#   scripts\build-plugin-bins.ps1
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
build() {
  target=$1; shift
  rustup target add "$target" >/dev/null
  env "$@" cargo build --release --target "$target" -p anneal-cli
  mkdir -p "plugin/libexec/$target"
  for f in anneal anneal.exe; do
    if [ -f "target/$target/release/$f" ]; then
      cp "target/$target/release/$f" "plugin/libexec/$target/"
    fi
  done
  # Hooks start libexec/anneal with no shell, which on Windows resolves to libexec/anneal.exe;
  # the build made last is the one there, as the launchers prefer the newer build.
  if [ -f "target/$target/release/anneal.exe" ]; then
    cp "target/$target/release/anneal.exe" plugin/libexec/anneal.exe
  fi
  echo "built plugin/libexec/$target"
}
case "${1:-host}" in
  host) build "$(rustc -vV | sed -n 's/^host: //p')" ;;
  linux|all) build x86_64-unknown-linux-musl CC_x86_64_unknown_linux_musl=musl-gcc ;;
esac
case "${1:-host}" in
  windows|all) build x86_64-pc-windows-gnu CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc ;;
esac
