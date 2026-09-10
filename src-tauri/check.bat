@echo off
set "PATH=C:\Users\A\.cargo\bin;C:\Program Files\nodejs;C:\Windows\System32;%PATH%"
cd /d C:\Users\A\Documents\a\flowread\src-tauri
cargo check > check_output.txt 2>&1
findstr /C:"error" /C:"warning: unused" check_output.txt
