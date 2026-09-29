@echo off
rem ZWriter dev launcher — sets up the MSVC environment (needed because
rem Git Bash's /usr/bin/link.exe otherwise shadows the MSVC linker), then
rem runs the Tauri dev server with hot reload.
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0.."
pnpm tauri dev
