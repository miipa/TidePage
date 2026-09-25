// 真机（真库）：滚动条 20% / 50% / 90% 各测 3 次，记录首封面出现秒数的最坏值。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectTarget } from "./cdp.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TMP = "C:/Users/A/AppData/Local/Temp/opencode";
const wv = path.join(TMP, "scroll3_wv");
const diag = path.join(TMP, "scroll3_diag.log");
const REALDB = path.join(process.env.APPDATA, "com.a.flowread", "flowread.db").replace(/\\/g, "/");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
function done1() {
  const py = `import sqlite3;c=sqlite3.connect('file:${REALDB}?mode=ro',uri=True,timeout=30);print(c.execute('SELECT COUNT(*) FROM books WHERE meta_done=1').fetchone()[0])`;
  return Number(execFileSync("py", ["-c", py], { encoding: "utf8" }).trim());
}

const out = { positions: {} };
let child = null;
try {
  fs.rmSync(wv, { recursive: true, force: true });
  fs.writeFileSync(diag, "");
  const env = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: wv, FR_TEST_CDP_PORT: "0" };
  delete env.FLOWREAD_DATA_DIR;
  child = spawn("cmd.exe", ["/d", "/s", "/c", "npm run tauri dev"],
    { cwd: REPO, env, stdio: ["ignore", fs.openSync(diag, "a"), fs.openSync(diag, "a")], windowsHide: true });
  let port = 0;
  const dtp = path.join(wv, "EBWebView", "DevToolsActivePort");
  for (let i = 0; i < 240; i++) { try { port = Number(fs.readFileSync(dtp, "utf8").split(/\r?\n/)[0]); if (port) break; } catch {} await sleep(1000); }
  if (!port) throw new Error("no DTP");
  let M = null;
  for (let i = 0; i < 60; i++) { try { M = await connectTarget(port, isApp); break; } catch { await sleep(1000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch {} await sleep(400); }
  await M.evalJs(`(async()=>{const s=ms=>new Promise(r=>setTimeout(r,ms));for(let i=0;i<400;i++){if(document.querySelector('.card'))return 1;await s(100);}throw new Error('no cards')})()`);
  log("waiting 10s to stabilize");
  await sleep(10000);
  const grid = await M.evalJs(`(()=>{const g=document.querySelector('.grid');return{sh:g.scrollHeight,ch:g.clientHeight}})()`);
  log("grid " + JSON.stringify(grid));

  const setTop = (v) => M.evalJs(`(()=>{const g=document.querySelector('.grid');g.scrollTop=${v};g.dispatchEvent(new Event('scroll'));return g.scrollTop})()`);
  for (const ratio of [0.2, 0.5, 0.9]) {
    const reps = [];
    for (let r = 0; r < 3; r++) {
      // 每次略微不同的偏移，落到该区域内未扫描的新书上
      const top = Math.max(0, Math.floor(grid.sh * ratio) + r * Math.floor(grid.ch * 0.7));
      await setTop(0); await sleep(300);           // 先回顶，避免"已停在此处不再触发"
      const b0 = done1();
      const t0 = Date.now();
      await setTop(top);
      let ms = null;
      for (let i = 0; i < 100; i++) { await sleep(150); if (done1() > b0) { ms = Date.now() - t0; break; } }
      reps.push(ms == null ? null : +(ms / 1000).toFixed(2));
      log(`ratio ${ratio} rep ${r + 1}: ${reps[r]}s`);
      await sleep(800);
    }
    const valid = reps.filter((x) => x != null);
    out.positions[ratio] = { reps, worst: valid.length ? Math.max(...valid) : null };
  }
  log("positions " + JSON.stringify(out.positions));
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (child) { try { execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch {} try { execFileSync("taskkill", ["/IM", "msedgewebview2.exe", "/F"], { stdio: "ignore" }); } catch {} }
  fs.writeFileSync(path.join(TMP, "scroll3_out.json"), JSON.stringify(out, null, 1));
  console.log("WROTE scroll3_out.json");
}
