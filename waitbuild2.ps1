$log = 'C:\Users\A\Documents\a\flowread\dev_log.txt'
for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Seconds 10
  $tail = Get-Content $log -Tail 200 | Select-String 'error\[|Finished|Running `target'
  if ($tail -match 'Running `target' -and -not ($tail -match 'error\[')) { break }
  if ($tail -match 'error\[') { 'COMPILE_ERROR'; break }
}
Get-Content $log -Tail 3 | Out-String
