@echo off
set "PATH=C:\Program Files\nodejs;C:\Windows\System32;%PATH%"
cd /d C:\Users\A\Documents\a\flowread
call npm.cmd run check > check_fe.txt 2>&1
findstr /C:"svelte-check found" check_fe.txt
