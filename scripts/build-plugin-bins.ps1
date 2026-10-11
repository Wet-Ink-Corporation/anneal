# Build the Windows binary the plugin launcher looks for (run from the repo root in PowerShell).
# "Stop" does not cover native commands: each one's exit code is checked, so a failed build never
# copies a stale exe.
$ErrorActionPreference = "Stop"
$target = "x86_64-pc-windows-msvc"
cargo build --release -p anneal-cli
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
New-Item -ItemType Directory -Force -Path "plugin/libexec/$target" | Out-Null
Copy-Item "target/release/anneal.exe" "plugin/libexec/$target/anneal.exe" -Force
# Hooks start libexec/anneal with no shell, which resolves to libexec/anneal.exe; the
# build made last is the one there, as the launchers prefer the newer build.
Copy-Item "target/release/anneal.exe" "plugin/libexec/anneal.exe" -Force
Write-Host "built plugin/libexec/$target/anneal.exe and plugin/libexec/anneal.exe"
