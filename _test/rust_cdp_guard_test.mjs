// 直接 spawn target\debug\flowread.exe（不经 _test 启动器），验证 Rust 层端口校验本身。
import { spawn } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const EXE = "C:\\Users\\A\\Documents\\a\\flowread\\src-tauri\\target\\debug\\flowread.exe";
const sandboxDir = path.join(SANDBOX_ROOT, "fr_rustguard");
fs.mkdirSync(sandboxDir, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function listeners(ports) {
  const out = [];
  const ns = fs.existsSync("NUL") ? null : null;
  return out;
}
async function netcheck(ports) {
  const { exec } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const raw = await promisify(exec)("netstat -ano -p tcp", { encoding: "utf8" });
  return raw.stdout.split("\n").filter((l) => ports.some((p) => new RegExp(`:${p}\\s`).test(l) && /LISTENING/.test(l))).map((l) => l.trim());
}
async function webviewCmdlines() {
  const { execFile } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const ps1 = path.join(SANDBOX_ROOT, "wvcmd.ps1");
  fs.writeFileSync(ps1, "$p=Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'fr_rustguard' }; $p | ForEach-Object { $_.CommandLine }");
  const r = await promisify(execFile)("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1], { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 });
  return r.stdout.split("\n").filter((l) => l.trim());
}
async function one(name, portValue, probePorts, probeCdp = false) {
  const child = spawn(EXE, [], {
    env: { ...process.env, FR_TEST_CDP_PORT: portValue, WEBVIEW2_USER_DATA_FOLDER: sandboxDir },
    stdio: "ignore", windowsHide: true,
  });
  await sleep(7000);
  const alive = child.exitCode === null;
  const ls = await netcheck(probePorts);
  const cmds = await webviewCmdlines();
  const injected = cmds.filter((c) => /remote-debugging|disable-web-security/.test(c));
  let version = "not-probed";
  if (probeCdp) {
    version = await (async () => {
      for (let i = 0; i < 5; i++) {
        try { return (await (await fetch("http://127.0.0.1:9444/json/version")).text()).slice(0, 90); }
        catch { await sleep(1000); }
      }
      return "CDP unreachable";
    })();
  }
  const result = { name, portValue, appAlive: alive, listeners: ls, webviewCount: cmds.length, injectedArgs: injected.length ? injected.map((c) => (c.match(/--remote-debugging[^ ]*|--disable-web-security/g) || []).join(" ")) : "NONE", cdpVersion: version };
  killTree(child.pid);
  await sleep(1500);
  sweepSandboxWebviews("fr_rustguard");
  await sleep(1000);
  return result;
}

const results = [];
results.push(await one("reject-9222", "9222", [9222]));
results.push(await one("reject-0", "0", []));
results.push(await one("reject-65536", "65536", []));
results.push(await one("reject-smuggle", "9444 --disable-web-security", [9444]));
// 合法值：9444 仅允许 127.0.0.1:9444 且 flag 精确、CDP 存活期可连
const good = await one("legal-9444", "9444", [9444], true);
console.log(JSON.stringify(results.concat([good]), null, 1));
