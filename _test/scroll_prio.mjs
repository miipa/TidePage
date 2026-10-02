// 方案 A 真机验证：滚动书库→可见未扫描书插队→按视野顺序生成封面；退出竞态；幂等；20/50/90% 定位。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectTarget } from "./cdp.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TMP = "C:/Users/A/AppData/Local/Temp/opencode";
const sbx = path.join(TMP, "prio_sbx");
const bk = path.join(sbx, "books");
const db = path.join(sbx, "tidepage.db");
const diag = path.join(TMP, "prio_diag.log");
const PY = "py";
const seedpy = path.join(TMP, "seed2.py");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const N = 150;

function sh(cmd, args) { return execFileSync(cmd, args, { encoding: "utf8" }).trim(); }
function counts() { return sh(PY, [seedpy, db, "counts"]); }
function done1() { return Number(sh(PY, [seedpy, db, "done1"])); }
function prioRows() { return Number(sh(PY, [seedpy, db, "prio_rows"])); }

const out = {};
let child = null;
try {
  // 1) sandbox + 60 distinct book files
  fs.rmSync(sbx, { recursive: true, force: true });
  fs.mkdirSync(bk, { recursive: true });
  const src = path.join(process.env.LOCALAPPDATA, "Temp", "fr_test_profiles", "scroll_books", "test_conv.epub");
  const paths = [];
  for (let i = 0; i < N; i++) { const p = path.join(bk, `bk${String(i).padStart(3, "0")}.epub`); fs.copyFileSync(src, p); paths.push(p); }

  // 2) 预建 schema + seed 60 本 meta_done=2（单次启动即可；避免两次 tauri dev 的端口/进程竞态）。
  sh(PY, [seedpy, db, "mkdb"]);
  sh(PY, [seedpy, db, "seed", ...paths]);
  out.seeded = counts();
  log("seeded: " + out.seeded);

  // 3) 单次启动（本次即被测运行）。startup 扫描发现 pending(meta0)=0 → worker 立即退出；
  //    60 本 meta_done=2 不会被自动处理 → 仅「滚动插队」能触发扫描，天然验证退出竞态。
  fs.writeFileSync(diag, "");
  const wv = path.join(sbx, "wv");
  child = spawn("cmd.exe", ["/d", "/s", "/c", "npm run tauri dev"],
    { cwd: REPO, env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: wv, TIDEPAGE_DATA_DIR: sbx, FR_TEST_CDP_PORT: "0" },
      stdio: ["ignore", fs.openSync(diag, "a"), fs.openSync(diag, "a")], windowsHide: true });
  let port = 0;
  const dtp = path.join(wv, "EBWebView", "DevToolsActivePort");
  for (let i = 0; i < 240; i++) { try { port = Number(fs.readFileSync(dtp, "utf8").split(/\r?\n/)[0]); if (port) break; } catch {} await sleep(1000); }
  log("cdp port " + port);
  if (!port) throw new Error("no DevToolsActivePort");
  let M = null;
  for (let i = 0; i < 60; i++) { try { M = await connectTarget(port, isApp); break; } catch { await sleep(1000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch {} await sleep(400); }
  // wait library grid + rows
  await M.evalJs(`(async()=>{const s=ms=>new Promise(r=>setTimeout(r,ms));for(let i=0;i<300;i++){if(document.querySelector('.card'))return 1;await s(100);}throw new Error('no cards')})()`);
  await sleep(1500);

  // #3 baseline: startup finished, pending(meta0)=0, all meta_done=2, worker should have exited.
  out.afterStart = counts();
  out.debug0 = await M.evalJs(`window.__TAURI_INTERNALS__.invoke('meta_debug_stats')`);
  log("afterStart: " + out.afterStart + " debug=" + JSON.stringify(out.debug0));

  // helpers to control the grid scroll
  const gridInfo = `(()=>{const g=document.querySelector('.grid');return g?{sh:g.scrollHeight,ch:g.clientHeight,top:g.scrollTop}:null})()`;
  const setTop = (v) => M.evalJs(`(()=>{const g=document.querySelector('.grid');g.scrollTop=${v};g.dispatchEvent(new Event('scroll'));return g.scrollTop})()`);
  out.grid = await M.evalJs(gridInfo);
  log("grid: " + JSON.stringify(out.grid));

  // #2 + #3: scroll to ~50% region; worker (idle, exited) must respawn via prioritize→ensure_scan_worker and generate covers.
  const done1Before = done1();
  const t0 = Date.now();
  await setTop(Math.floor(out.grid.sh * 0.5));
  // wait until done1 increases (a visible book got scanned 2->1)
  let firstCoverMs = null;
  for (let i = 0; i < 60; i++) { await sleep(250); if (done1() > done1Before) { firstCoverMs = Date.now() - t0; break; } }
  out.scrollToMid = { done1Before, done1After: done1(), firstCoverMs, priorityRows: prioRows() };
  log("scrollToMid: " + JSON.stringify(out.scrollToMid));

  // #5: scrollbar 20/50/90% — measure first-cover seconds at each (reset region each time to a fresh unscanned zone).
  out.positions = {};
  for (const ratio of [0.2, 0.5, 0.9]) {
    const b0 = done1();
    const ts = Date.now();
    await setTop(Math.floor(out.grid.sh * ratio));
    let ms = null;
    for (let i = 0; i < 80; i++) { await sleep(200); if (done1() > b0) { ms = Date.now() - ts; break; } }
    out.positions[ratio] = { seconds: ms == null ? null : +(ms / 1000).toFixed(2), done1: done1() };
    log(`pos ${ratio}: ` + JSON.stringify(out.positions[ratio]));
    await sleep(500);
  }

  // #4 idempotency: jitter the SAME region 10 times rapidly; cover_priority rows must stay bounded (<< 10x window), not linear.
  const region = Math.floor(out.grid.sh * 0.35);
  let maxPrio = 0;
  for (let k = 0; k < 10; k++) {
    await setTop(region + (k % 2 === 0 ? 0 : 40)); // tiny jitter to force viewport-change effect
    await sleep(120);
    const pr = prioRows();
    if (pr > maxPrio) maxPrio = pr;
  }
  await sleep(600);
  out.idempotency = { scrolls: 10, maxPriorityRowsDuringBurst: maxPrio, priorityRowsAfter: prioRows(), note: "ON CONFLICT(book_id) => rows bounded by distinct visible ids, not 10x" };
  log("idempotency: " + JSON.stringify(out.idempotency));

  await sleep(1500);
  out.final = counts();
  // count enqueue log lines from diag (backend prints per prioritize_covers)
  const diagText = fs.readFileSync(diag, "utf8");
  out.enqueueLogLines = diagText.split(/\r?\n/).filter((l) => l.includes("prioritize_covers enqueued"));
  out.startCountFinal = (await M.evalJs(`window.__TAURI_INTERNALS__.invoke('meta_debug_stats')`));
  log("final: " + out.final + " startCount=" + JSON.stringify(out.startCountFinal));
  log("enqueue log sample: " + JSON.stringify(out.enqueueLogLines.slice(0, 8)));
  M.close();
} catch (e) { process.exitCode = 1; /* 失败必须体现在退出码上 */ out.error = String(e); console.error(e); }
finally {
  if (child) { try { execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch {} try { execFileSync("taskkill", ["/IM", "msedgewebview2.exe", "/F"], { stdio: "ignore" }); } catch {} }
  fs.writeFileSync(path.join(TMP, "prio_out.json"), JSON.stringify(out, null, 1));
  console.log("WROTE " + path.join(TMP, "prio_out.json"));
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
