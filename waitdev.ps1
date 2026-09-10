$log = 'C:\Users\A\Documents\a\flowread\dev_log.txt'
for ($i = 0; $i -lt 40; $i++) {
  if (Test-Path $log) {
    $t = Get-Content $log -Raw
    if ($t -match 'Running `target') { break }
    if ($t -match 'error\[E') { break }
  }
  Start-Sleep -Seconds 8
}
Get-Content $log -Tail 5 | Out-String
