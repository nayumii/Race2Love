@echo off
setlocal
cd /d "%~dp0"
if not defined RUST_LOG set "RUST_LOG=race2love=info,race2love_core=debug,race2love_gui=info,race2love_lovense=debug,race2love_lmu=debug"
echo Running Race2Love. Logs are written to race2love.log in this folder.
"%~dp0race2love.exe" %* > "%~dp0race2love.log" 2>&1
set "race2love_exit=%errorlevel%"
echo Race2Love exited with code %race2love_exit%.
echo Log: "%~dp0race2love.log"
pause
exit /b %race2love_exit%
