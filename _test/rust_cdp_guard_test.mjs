// 直接 spawn target\debug\tidepage.exe（不经 JS 守卫），验证 Rust 层 ASCII 纯数字校验。
// 监听判定按“本轮 spawn 进程树 + fr_rustguard webview”归属，外部旁观者（如 adb）单列不误杀。
import { spawn } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const EXE = "C:\\Users\\A\\Documents\\a\\TidePage\\src-tauri\\target\\debug\\tidepage.exe";
const sandboxDir = path.join(SANDBOX_ROOT, "fr_rustguard");
fs.mkdirSync(sandboxDir, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const { execFile } = await import("node:child_process");
const { promisify } = await import("node:util");
const pexec = promisify(execFile);

async function ps(script) {
  const ps1 = path.join(SANDBOX_ROOT, "guardq.ps1");
  fs.writeFileSync(ps1, script);
  const r = await pexec("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1], { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 });
  return r.stdout.split(/\r?\n/).filter((l) => l.trim());
}
async function listeners() {
  const raw = (await pexec("netstat", ["-ano", "-p", "tcp"], { encoding: "utf8" })).stdout;
  return raw.split("\n").filter((l) => /LISTENING/.test(l) && /:(9222|9444)\s/.test(l)).map((l) => l.trim());
}
async function webviewCmdlines() {
  return ps("$p=Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'fr_rustguard' }; $p | ForEach-Object { $_.ProcessId.ToString() + '|' + $_.CommandLine }");
}
async function one(name, portValue, { probeCdp = false } = {}) {
  const child = spawn(EXE, [], {
    env: { ...process.env, FR_TEST_CDP_PORT: portValue, WEBVIEW2_USER_DATA_FOLDER: sandboxDir },
    stdio: "ignore", windowsHide: true,
  });
  await sleep(7000);
  const alive = child.exitCode === null;
  // 本轮 spawn 树 pid 集合：child + descendants + fr_rustguard webviews
  const treeLines = await ps(`$root=${child.pid}; $all=@($root); do { $before=$all.Count; $all += Get-CimInstance Win32_Process | Where-Object { $all -contains $_.ParentProcessId -and -not ($all -contains $_.ProcessId) } | ForEach-Object { $_.ProcessId } } while ($all.Count -gt $before); $wv = Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'fr_rustguard' } | ForEach-Object { $_.ProcessId }; ($all + $wv) | Select-Object -Unique`);
  const treePids = new Set(treeLines.map((l) => l.trim()).filter(Boolean));
  const allListeners = await listeners();
  const mine = allListeners.filter((l) => treePids.has(l.split(/\s+/).pop()));
  const bystanders = allListeners.filter((l) => !treePids.has(l.split(/\s+/).pop()));
  const cmds = await webviewCmdlines();
  const injected = [];
  for (const line of cmds) {
    const c = line.split("|").slice(1).join("|");
    for (const m of c.match(/--remote-debugging[^\s"]*|--disable-web-security/g) || []) if (!injected.includes(m)) injected.push(m);
  }
  let version = "not-probed";
  if (probeCdp) {
    version = await (async () => {
      for (let i = 0; i < 5; i++) {
        try { return (await (await fetch("http://127.0.0.1:9444/json/version")).text()).slice(0, 50); }
        catch { await sleep(1000); }
      }
      return "CDP unreachable";
    })();
  }
  const r = { name, envValue: JSON.stringify(portValue), appAlive: alive, treeListeners: mine.length ? mine : "NONE", injectedArgs: injected.length ? injected : "NONE", cdp: version };
  killTree(child.pid);
  await sleep(1500);
  sweepSandboxWebviews("fr_rustguard");
  await sleep(1000);
  if (bystanders.length) r.bystandersUnrelated = bystanders;
  return r;
}

const results = [];
for (const [n, v] of [
  ["lead-space", " 9444"], ["trail-space", "9444 "], ["tab", "\t9444"], ["plus", "+9444"],
  ["smuggle", "9444 --disable-web-security"], ["banned-9222", "9222"], ["zero", "0"], ["overflow", "65536"],
]) results.push(await one(n, v));
results.push(await one("legal-9444", "9444", { probeCdp: true }));
const post = await listeners();
results.push({ name: "post-kill-audit(all 9222/9444 sockets)", sockets: post.length ? post : "NONE", sandboxWebview: (await webviewCmdlines()).length });
console.log(JSON.stringify(results, null, 1));
const malicious = results.filter((r) => ["lead-space","trail-space","tab","plus","smuggle","banned-9222","zero","overflow"].includes(r.name));
const bad = malicious.filter((r) => r.treeListeners !== "NONE" || r.injectedArgs !== "NONE");
const legal = results.find((r) => r.name === "legal-9444");
console.log(bad.length === 0 ? "GUARD: ALL 8 MALICIOUS VALUES REJECTED (zero tree listeners, zero injected args)" : "GUARD FAIL: " + JSON.stringify(bad));
console.log("legal:", JSON.stringify({ treeListeners: legal.treeListeners, injectedArgs: legal.injectedArgs, cdp: legal.cdp }));
