@echo off
setlocal
rem anneal launcher for cmd: runs the prebuilt binary from libexec\TARGET\.
rem The msvc build wins unless the gnu build is newer, as in the sh launcher: a stale native
rem build must not shadow a fresh cross-build. Falls back to an anneal.exe on PATH.
rem The age test is xcopy /D /L: it lists the gnu build only when that is newer than the msvc
rem one, and copies nothing. Both tools are named by path so a Unix find earlier on PATH is
rem never picked up. exit /b names its errorlevel, so the caller sees the binary's exit code.
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
anneal.exe %*
exit /b %ERRORLEVEL%
