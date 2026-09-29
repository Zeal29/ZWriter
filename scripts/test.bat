@echo off
rem ZWriter test runner (cargo test) — needs the MSVC environment.
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0..\src-tauri"
cargo test %*
