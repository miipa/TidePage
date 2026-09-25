// 真机（真库 ~13.9 万 meta_done=0）验证方案 A：启动等 10s → 滚到 50% → 首封面秒数；meta3 无残留；cover_priority idle 量级。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectTarget } from "./cdp.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TMP = "C:/Users/A/AppData/Local/Temp/opencode";
const wv = path.join(TMP, "realprio_wv");
const diag = path.join(TMP, "realprio_diag.log");
const REALDB = path.join(process.env.APPDATA, "com.a.flowread", "flowread.db");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

function dbq(sql) {
  const py = `import sqlite3;c=sqlite3.connect('file:${REALDB.replace(/\\/g, "/")}?mode=ro',uri=True,timeout=30);print(c.execute(${JSON.stringify(sql)}).fetchone()[0])`;
  return Number(execFileSync("py", ["-c", py], { encoding: "utf8" }).trim());
}

const out = {};
let child = null;
try {
  fs.rmSync(wv, { recursive: true, force: true });
  fs.writeFileSync(diag, "");
  // 真库：不设 FLOWREAD_DATA_DIR，用默认 %APPDATA%\com.a.flowread。
  const env = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: wv, FR_TEST_CDP_PORT: "0" };
  delete env.FLOWREAD_DATA_DIR;
  child = spawn("cmd.exe", ["/d", "/s", "/c", "npm run tauri dev"],
    { cwd: REPO, env, stdio: ["ignore", fs.openSync(diag, "a"), fs.openSync(diag, "a")], windowsHide: true });
  let port = 0;
  const dtp = path.join(wv, "EBWebView", "DevToolsActivePort");
  for (let i = 0; i < 240; i++) { try { port = Number(fs.readFileSync(dtp, "utf8").split(/\r?\n/)[0]); if (port) break; } catch {} await sleep(1000); }
  if (!port) throw new Error("no DTP");
  log("cdp port " + port);
  let M = null;
  for (let i = 0; i < 60; i++) { try { M = await connectTarget(port, isApp); break; } catch { await sleep(1000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch {} await sleep(400); }
  await M.evalJs(`(async()=>{const s=ms=>new Promise(r=>setTimeout(r,ms));for(let i=0;i<400;i++){if(document.querySelector('.card'))return 1;await s(100);}throw new Error('no cards')})()`);

  out.meta0_start = dbq("SELECT COUNT(*) FROM books WHERE meta_done=0");
  log("meta0 at start ~ " + out.meta0_start);

  // 等 10s 让扫描线程进入稳定状态
  log("waiting 10s for scan to stabilize...");
  await sleep(10000);
  out.debug_after10s = await M.evalJs(`window.__TAURI_INTERNALS__.invoke('meta_debug_stats')`);
  out.meta3_after10s = dbq("SELECT COUNT(*) FROM books WHERE meta_done=3");
  out.priority_after10s = dbq("SELECT COUNT(*) FROM cover_priority");
  log("after10s debug=" + JSON.stringify(out.debug_after10s) + " meta3=" + out.meta3_after10s + " priority=" + out.priority_after10s);

  // 滚到 50%，记录该处可见未扫描书首封面出现秒数
  const grid = await M.evalJs(`(()=>{const g=document.querySelector('.grid');return{sh:g.scrollHeight,ch:g.clientHeight}})()`);
  out.grid = grid;
  // 记录 50% 处可见的未扫描 book id 集合
  await M.evalJs(`(()=>{const g=document.querySelector('.grid');g.scrollTop=Math.floor(${grid.sh}*0.5);g.dispatchEvent(new Event('scroll'));return 1})()`);
  await sleep(400); // 让虚拟列表渲染该区间
  const visible = await M.evalJs(`(()=>{const ids=[];document.querySelectorAll('.card').forEach(c=>{});return window.__frVisibleProbe?window.__frVisibleProbe():null})()`).catch(() => null);
  // 用 DB 直接判定：记录 done1 基线，等待其增长即"该处封面出现"
  const done1_0 = dbq("SELECT COUNT(*) FROM books WHERE meta_done=1");
  const t0 = Date.now();
  // 再次确保 50% 停稳并触发防抖后的 prioritize
  await M.evalJs(`(()=>{const g=document.querySelector('.grid');g.scrollTop=Math.floor(${grid.sh}*0.5);g.dispatchEvent(new Event('scroll'));return 1})()`);
  let firstMs = null;
  for (let i = 0; i < 80; i++) { await sleep(200); if (dbq("SELECT COUNT(*) FROM books WHERE meta_done=1") > done1_0) { firstMs = Date.now() - t0; break; } }
  out.scroll50_firstCoverSeconds = firstMs == null ? null : +(firstMs / 1000).toFixed(2);
  out.priority_afterScroll = dbq("SELECT COUNT(*) FROM cover_priority");
  log("scroll50 firstCover=" + out.scroll50_firstCoverSeconds + "s priorityRows=" + out.priority_afterScroll);

  // cover_priority 随时间是否增长（采 5 次）
  out.prioritySamples = [];
  for (let i = 0; i < 5; i++) { await sleep(1500); out.prioritySamples.push(dbq("SELECT COUNT(*) FROM cover_priority")); }
  out.meta3_final = dbq("SELECT COUNT(*) FROM books WHERE meta_done=3");
  log("prioritySamples=" + JSON.stringify(out.prioritySamples) + " meta3_final=" + out.meta3_final);

  out.enqueueLines = fs.readFileSync(diag, "utf8").split(/\r?\n/).filter((l) => l.includes("prioritize_covers enqueued")).slice(-6);
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (child) { try { execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch {} try { execFileSync("taskkill", ["/IM", "msedgewebview2.exe", "/F"], { stdio: "ignore" }); } catch {} }
  fs.writeFileSync(path.join(TMP, "realprio_out.json"), JSON.stringify(out, null, 1));
  console.log("WROTE realprio_out.json");
}
