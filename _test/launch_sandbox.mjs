// 受守卫沙箱启动器 v2：buildSandboxEnv 通过后才到达 spawnImpl；Node 直接 spawn
// cmd.exe（无 bat、无 PowerShell Start-Process 间接层），child.pid 即真实树根。
// 调用方必须 try/finally，finally 里 killTree(child.pid)。
import path from "node:path";
import fs from "node:fs";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { buildSandboxEnv } from "./isolation_guard.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const REPO = path.resolve(HERE, "..");

export function launchSandbox({
  env = process.env,
  cdpPort,
  logFile = path.join(path.dirname(process.env.LOCALAPPDATA || ""), "Temp", "fr_test_profiles", "sandbox_dev.log"),
  spawnImpl = spawn,
} = {}) {
  // ↓ 校验失败即 throw，spawnImpl 不可达
  const finalEnv = buildSandboxEnv(env, { cdpPort });
  fs.mkdirSync(path.dirname(logFile), { recursive: true });
  const fd = fs.openSync(logFile, "a");
  const child = spawnImpl("cmd.exe", ["/d", "/s", "/c", "npm.cmd run tauri dev"], {
    cwd: REPO,
    env: finalEnv,
    stdio: ["ignore", fd, fd],
    windowsHide: true,
    detached: false,
  });
  return { child, env: finalEnv, cdp: `http://127.0.0.1:${cdpPort}` };
}

// 杀 cmd→npm→tauri CLI→cargo/vite→flowread 全树 + 沙箱内 WebView2 浏览器进程；循环直到归零。
export function killTree(pid) {
  try { execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" }); } catch { /* already gone */ }
}
export function sweepSandboxWebviews(userdataNeedle) {
  const psScript =
    "$p=Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match '" + userdataNeedle + "' };" +
    "foreach($x in $p){ Stop-Process -Id $x.ProcessId -Force -EA SilentlyContinue }; @($p).Count";
  try {
    return Number(execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", psScript], { encoding: "utf8" }).trim()) || 0;
  } catch { return 0; }
}
