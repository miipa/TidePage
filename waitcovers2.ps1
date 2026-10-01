$covers = Join-Path $env:APPDATA 'com.a.tidepage\covers'
$proc = $null
for ($i = 0; $i -lt 30; $i++) {
  $proc = Get-Process tidepage -ErrorAction SilentlyContinue
  if ($proc) { break }
  Start-Sleep -Seconds 3
}
if (-not $proc) { 'NO_PROC'; exit }
"tidepage up, waiting extraction..."
for ($i = 0; $i -lt 60; $i++) {
  $n = (Get-ChildItem $covers -ErrorAction SilentlyContinue).Count
  if ($n -gt 50) { "files=$n"; break }
  Start-Sleep -Seconds 8
}
"final files=" + (Get-ChildItem $covers -ErrorAction SilentlyContinue).Count
