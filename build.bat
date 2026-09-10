@echo off
set "PATH=C:\Users\A\.cargo\bin;C:\Program Files\nodejs;C:\Windows\System32;%PATH%"
cd /d C:\Users\A\Documents\a\flowread
npm.cmd run tauri build > build_release.txt 2>&1
echo EXIT=%ERRORLEVEL%
