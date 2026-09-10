@echo off
set "PATH=C:\Users\A\.cargo\bin;C:\Windows\System32;%PATH%"
cd /d C:\Users\A\Documents\a\flowread\src-tauri
cargo check > check_out.txt 2>&1
echo EXIT=%ERRORLEVEL%
