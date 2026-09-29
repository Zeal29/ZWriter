@echo off
rem ZWriter release build (NSIS installer + portable exe in target/release).
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
rem A running instance locks the exe - stop it first.
taskkill /f /im zwriter.exe >nul 2>&1
timeout /t 2 /nobreak >nul
cd /d "%~dp0.."
pnpm tauri build -b nsis
