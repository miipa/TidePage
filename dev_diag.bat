@echo off
set "PATH=C:\Users\A\.cargo\bin;C:\Program Files\nodejs;C:\Windows\System32;%PATH%"
cd /d C:\Users\A\Documents\a\TidePage
call npm.cmd run tauri dev > dev_log.txt 2>&1
