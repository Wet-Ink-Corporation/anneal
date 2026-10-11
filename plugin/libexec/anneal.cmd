@echo off
setlocal
rem anneal launcher for cmd: runs the prebuilt binary from libexec\TARGET\.
rem The msvc build wins unless the gnu build is newer, as in the sh launcher: a stale native
rem build must not shadow a fresh cross-build. Falls back to an anneal.exe on PATH.
rem The age test is xcopy /D /L: it lists the gnu build only when that is newer than the msvc
rem one, and copies nothing. Both tools are named by path so a Unix find earlier on PATH is
rem never picked up. exit /b names its errorlevel, so the caller sees the binary's exit code.
rem With no binary anywhere, a hook call exits 0 (fail open) unless it carries --fail-closed,
rem which exits 2 with one ERROR line, as in the sh launcher. hook bootstrap (SessionStart) also
rem prints the install hint as SessionStart output. The hint has no quotes, backslashes or cmd
rem metacharacters, so the echoed line is valid JSON as written.
set "D=%~dp0"
set "M=%D%x86_64-pc-windows-msvc\anneal.exe"
set "G=%D%x86_64-pc-windows-gnu\anneal.exe"
set "P="
if exist "%M%" set "P=%M%"
if not exist "%G%" goto run
if not defined P set "P=%G%"
if "%P%"=="%G%" goto run
"%SystemRoot%\System32\xcopy.exe" /D /L /Y "%G%" "%D%x86_64-pc-windows-msvc" <nul 2>nul | "%SystemRoot%\System32\find.exe" /i "anneal.exe" >nul && set "P=%G%"
:run
if not defined P goto path
"%P%" %*
exit /b %ERRORLEVEL%
:path
"%SystemRoot%\System32\where.exe" anneal.exe >nul 2>nul || goto nobinary
anneal.exe %*
exit /b %ERRORLEVEL%
:nobinary
if /i not "%~1"=="hook" goto missing
for %%A in (%*) do if /i "%%~A"=="--fail-closed" goto blocked
if /i "%~2"=="bootstrap" goto hint
exit /b 0
:hint
echo {"systemMessage":"anneal is not installed: no anneal binary in the plugin's libexec and none on PATH, so anneal's hooks and gates are off this session. Install a release, or build one with scripts/build-plugin-bins.ps1 or cargo install --path crates/anneal-cli.","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"anneal is not installed: no anneal binary in the plugin's libexec and none on PATH, so anneal's hooks and gates are off this session. Install a release, or build one with scripts/build-plugin-bins.ps1 or cargo install --path crates/anneal-cli."}}
exit /b 0
:blocked
>&2 echo ERROR anneal: no anneal binary under %D% and none on PATH
exit /b 2
:missing
>&2 echo anneal: no anneal binary under %D% and none on PATH; build one with scripts\build-plugin-bins.ps1 or install a release
exit /b 127
