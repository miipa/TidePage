$covers = Join-Path $env:APPDATA 'com.a.flowread\covers'
for ($i = 0; $i -lt 60; $i++) {
  $n = (Get-ChildItem $covers -ErrorAction SilentlyContinue).Count
  if ($n -gt 100) { break }
  Start-Sleep -Seconds 10
}
"files=$n"
try {
  $r = Invoke-WebRequest -Uri 'http://127.0.0.1:9222/json/version' -UseBasicParsing -TimeoutSec 3
  'CDP_UP'
} catch { 'CDP_DOWN' }
