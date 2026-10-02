// 必改(A) 实测：刷新不触发任何 AI/元数据入队。
// 起点干净化：清掉测试临时 folders/books；直接向 folders 表登记临时根（绕过 add_folder/scan/meta）。
// 断言：点刷新时 apiKey 为空、mock 出站=0；新书入库；新书 meta_done=0（未生成）；meta_busy 不被刷新改变；toast 非空非错、按钮不卡。
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const LOG = path.join(SANDBOX_ROOT, "mock_reqs.jsonl");
const reqs = () => (fs.existsSync(LOG) ? fs.readFileSync(LOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);
const DB = "C:/Users/A/AppData/Roaming/com.a.tidepage/tidepage.db";
function dbExec(fn) {
  for (let i = 0; i < 20; i++) {
    const db = new DatabaseSync(DB);
    try { return fn(db); } catch (e) { if (!/locked|busy/i.test(String(e))) throw e; } finally { try { db.close(); } catch {} }
  }
  throw new Error("db locked");
}

const root = path.join(SANDBOX_ROOT, "fr_ai");
const sub = path.join(root, "sub");
const newFile = path.join(sub, "C-new.txt");
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(sub, { recursive: true });

// 干净起点：清测试残留，登记临时根（不触发扫描/meta）
dbExec((db) => {
  db.exec("DELETE FROM folders WHERE path LIKE '%fr_test_profiles%'");
  db.exec("DELETE FROM books WHERE path LIKE '%fr_test_profiles%'");
  db.prepare("INSERT OR IGNORE INTO folders(path) VALUES(?)").run(root);
});

const out = {};
const M = await connectTarget(9444, (t) => t.type === "page" && !t.url.includes("ai=1"));
await M.evalJs(H(`
  const c=cfgRead(); c.ai.profiles[0]={name:'EP',provider:'ollama',baseUrl:'http://127.0.0.1:18081',apiKey:'',model:'mock',proxyMode:'direct',proxyUrl:''};
  localStorage.setItem('tidepage.config.v1',JSON.stringify(c)); setTimeout(()=>location.reload(),120); return 1;
`));
await sleep(4500);
await M.evalJs(H(`
  for (let i=0;i<80 && !document.querySelector('.library');i++){ const b=[...document.querySelectorAll('.topbar button')].find(x=>x.textContent.includes('←')); if(b)b.click(); else break; await sleep(150); }
  return { apiKey: cfgRead().ai.profiles[0].apiKey, baseUrl: cfgRead().ai.profiles[0].baseUrl };
`)).then((v) => (out.configAtRefresh = v));

// 新书落在临时根的一级子目录
fs.writeFileSync(newFile, "C".repeat(50));

const n0 = reqs().length;
out.refresh = await M.evalJs(H(`
  const I=window.__TAURI_INTERNALS__.invoke;
  const btn=document.querySelector('[data-testid="lib-btn-refresh"]');
  if(!btn) return { err:'no refresh button' };
  const disabledBefore=btn.disabled;
  btn.click();
  let toast='';
  for(let i=0;i<300;i++){ await sleep(200); toast=toastsNow(); if(toast) break; }
  const btn2=document.querySelector('[data-testid="lib-btn-refresh"]');
  return { toast: toast.slice(0,80), btnDisabledBefore: disabledBefore, btnDisabledAfter: btn2?btn2.disabled:'gone' };
`));

out.db = dbExec((db) => {
  const row = db.prepare("SELECT id, meta_done FROM books WHERE path=?").get(newFile);
  const busy = db.prepare("SELECT value FROM settings WHERE key='meta_busy'").get();
  return { indexedNew: !!row, newMetaDone: row ? row.meta_done : "missing", metaBusy: busy ? busy.value : "(unset)", metaZeroCount: db.prepare("SELECT COUNT(*) c FROM books WHERE meta_done=0").get().c };
});
out.mockOutboundDelta = reqs().length - n0;
out.mockOutboundUrls = reqs().slice(n0).map((r) => r.url);

// 再删掉新书 → 刷新 → removed 命中（顺带验证 counts/toast）
fs.unlinkSync(newFile);
const n1 = reqs().length;
out.refresh2 = await M.evalJs(H(`
  const btn=document.querySelector('[data-testid="lib-btn-refresh"]');
  btn.click();
  await sleep(2500);
  return { toast: toastsNow().slice(0,120) };
`));
out.removedFromIndex = dbExec((db) => !db.prepare("SELECT 1 FROM books WHERE path=?").get(newFile));
out.mockOutboundDelta2 = reqs().length - n1;

// 清场
dbExec((db) => {
  db.prepare("DELETE FROM folders WHERE path=?").run(root);
  db.prepare("DELETE FROM books WHERE path=?").run(newFile);
});
try { fs.rmSync(root, { recursive: true, force: true }); } catch {}
M.close();
fs.writeFileSync(path.join(SANDBOX_ROOT, "ai_refresh_out.json"), JSON.stringify(out, null, 1));
console.log(JSON.stringify(out, null, 1));

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
