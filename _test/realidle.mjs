// 真机（真库 ~13.9万 meta_done=0）空闲态验证：全程不滚动、不打开书。
// 30s，每 2s 采样 cover_priority 计数、meta_done=1 计数、后端 enqueue 日志累计条数。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TMP = "C:/Users/A/AppData/Local/Temp/opencode";
const wv = path.join(TMP, "realidle_wv");
const diag = path.join(TMP, "realidle_diag.log");
const REALDB = path.join(process.env.APPDATA, "com.a.flowread", "flowread.db").replace(/\\/g, "/");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);

function dbq(sql) {
  const py = `import sqlite3;c=sqlite3.connect('file:${REALDB}?mode=ro',uri=True,timeout=30);print(c.execute(${JSON.stringify(sql)}).fetchone()[0])`;
  return Number(execFileSync("py", ["-c", py], { encoding: "utf8" }).trim());
}
function enqLines() {
  try { return fs.readFileSync(diag, "utf8").split(/\r?\n/).filter((l) => l.includes("prioritize_covers enqueued")).length; } catch { return 0; }
}

const out = { samples: [] };
let child = null;
try {
  fs.rmSync(wv, { recursive: true, force: true });
  fs.writeFileSync(diag, "");
  // 真库（不设 FLOWREAD_DATA_DIR）；不设 FR_TEST_CDP_PORT（不需要 CDP，正常开窗）。
  const env = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: wv };
  delete env.FLOWREAD_DATA_DIR;
  delete env.FR_TEST_CDP_PORT;
  child = spawn("cmd.exe", ["/d", "/s", "/c", "npm run tauri dev"],
    { cwd: REPO, env, stdio: ["ignore", fs.openSync(diag, "a"), fs.openSync(diag, "a")], windowsHide: true });
  // 等窗口加载（vite ready + 前端挂载 → 首个 enqueue 出现，或最多 40s）
  for (let i = 0; i < 40; i++) { await sleep(1000); if (enqLines() > 0) break; }
  log("library mounted (enqueue seen=" + enqLines() + "); begin 30s idle sampling (NO scroll / NO open)");

  for (let i = 0; i < 15; i++) {
    const t = i * 2;
    const prio = dbq("SELECT COUNT(*) FROM cover_priority");
    const done1 = dbq("SELECT COUNT(*) FROM books WHERE meta_done=1");
    const enq = enqLines();
    out.samples.push({ t, cover_priority: prio, meta1: done1, enqueue_log_lines: enq });
    log(`t=${t}s cover_priority=${prio} meta1=${done1} enqueue_log_lines=${enq}`);
    await sleep(2000);
  }
  out.meta3 = dbq("SELECT COUNT(*) FROM books WHERE meta_done=3");
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (child) { try { execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch {} try { execFileSync("taskkill", ["/IM", "msedgewebview2.exe", "/F"], { stdio: "ignore" }); } catch {} }
  fs.writeFileSync(path.join(TMP, "realidle_out.json"), JSON.stringify(out, null, 1));
  console.log("WROTE realidle_out.json");
}
