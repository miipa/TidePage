// 打回1：三按钮删书弹窗 + 两路径计数；打回2：D 清理前后；待验3：删本地文件入口弹窗。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9777);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// 预检：端口 1420 若被占（上次 vite 残留）会令 tauri dev 的 beforeDevCommand 失败。
import { execFileSync } from "node:child_process";
try {
  const out = execFileSync("powershell.exe", ["-NoProfile", "-Command",
    "@(Get-NetTCPConnection -State Listen -EA SilentlyContinue | Where-Object { $_.LocalPort -eq 1420 }).Count"], { encoding: "utf8" }).trim();
  if (Number(out) > 0) { console.log("[warn] port 1420 busy; releasing stale vite"); execFileSync("powershell.exe", ["-NoProfile", "-Command",
    "Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match 'vite' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -EA SilentlyContinue }"], { stdio: "ignore" }); }
} catch { /* ignore */ }
const sbx = path.join(SANDBOX_ROOT, "f3_sbx"); const data = path.join(SANDBOX_ROOT, "f3_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "f3_evidence"); fs.rmSync(evid, { recursive: true, force: true }); fs.mkdirSync(evid, { recursive: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;
const SHOT = async (M, n) => { try { const s = await M.call("Page.captureScreenshot", { format: "png" }); if (s?.data) fs.writeFileSync(path.join(evid, n), Buffer.from(s.data, "base64")); } catch {} };
const step = (n, v) => { console.log("[step]", n, JSON.stringify(v)); return v; };

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "f3_diag.log") });
  console.log("[step] launched pid", app.child.pid);
  let M = null; for (let i = 0; i < 150; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  console.log("[step] cdp connected");
  for (let i = 0; i < 200; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  const tmp = path.join(SANDBOX_ROOT, "f3_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  const fK = path.join(tmp, "保留路径书 (作者甲).txt");
  const fD = path.join(tmp, "删除路径书 (作者乙).txt");
  const fDisk = path.join(tmp, "删文件书 (作者丙).txt");
  for (const p of [fK, fD, fDisk]) fs.writeFileSync(p, "第一章 测试\n正文内容。".repeat(30), "utf8");
  for (const p of [fK, fD, fDisk]) await M.evalJs(H(`try{await inv('add_dropped_book',{path:${JSON.stringify(p.replace(/\\/g, "\\\\"))}});}catch(e){}`));
  await sleep(1500); await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 200; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(1000);

  const ids = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'',sort:'title',offset:0,limit:50});
    const g=(s)=>pg.rows.find(r=>r.path.includes(s))?.id;
    return { keep:g('保留路径书'), del:g('删除路径书'), disk:g('删文件书') };
  `));
  step("ids", ids); out.steps.ids = ids;
  await M.evalJs(H(`
    const mk=async(id,n)=>{ await inv('add_highlight',{bookId:id,chapterIndex:0,chapterTitle:'第一章',start:0,end:6,text:'划线'+n,style:'underline',color:'',note:'感想'+n});
      await inv('add_bookmark',{bookId:id,chapterIndex:0,title:'B'+n,excerpt:'e'}); };
    await mk(${ids.keep},'K'); await mk(${ids.del},'D'); await mk(${ids.disk},'F'); return 1;
  `));

  // ===== 打回1-A：保留笔记并删除 =====
  await M.evalJs(H(`for(let i=0;i<30;i++){ if(window.__frLibDebug) break; await sleep(300); } await window.__frLibDebug.deleteByPath(${JSON.stringify(fK.replace(/\\/g, "\\\\"))},false); return 1;`));
  await M.evalJs(H(`await wait(()=>q('.choice-row button'),'choice',60); return 1;`));
  out.steps.dialogKeep = step("dialogKeep", await M.evalJs(H(`
    return { msg:q('.confirm-msg')?.textContent?.replace(/\\s+/g,' ').trim(), buttons:qa('.choice-row button').map(b=>b.textContent.trim()) };
  `)));
  await SHOT(M, "dialog_3buttons.png");
  out.steps.pathKeep = step("pathKeep", await M.evalJs(H(`
    [...document.querySelectorAll('.choice-row button')].find(b=>b.textContent.includes('保留笔记并删除'))?.click();
    await sleep(1000);
    const id=${ids.keep};
    const hl=await inv('list_highlights',{bookId:id});
    const bm=await inv('list_bookmarks',{bookId:id});
    const notes=await inv('list_note_books');
    return { afterHL:hl.length, afterBookmarks:bm.length, noteBooksHas: notes.some(n=>n.title.includes('保留路径书')) };
  `)));

  // ===== 打回1-B：删除笔记并删除 =====
  await M.evalJs(H(`await window.__frLibDebug.deleteByPath(${JSON.stringify(fD.replace(/\\/g, "\\\\"))},false); await wait(()=>q('.choice-row button'),'choice2',60); return 1;`));
  out.steps.pathDel = step("pathDel", await M.evalJs(H(`
    [...document.querySelectorAll('.choice-row button')].find(b=>b.textContent.includes('删除笔记并删除'))?.click();
    await sleep(1000);
    const id=${ids.del};
    const hl=await inv('list_highlights',{bookId:id});
    const bm=await inv('list_bookmarks',{bookId:id});
    const notes=await inv('list_note_books');
    return { afterHL:hl.length, afterBookmarks:bm.length, noteBooksHas: notes.some(n=>n.title.includes('删除路径书')) };
  `)));

  // ===== 待验3：删本地文件入口的三按钮弹窗 =====
  await M.evalJs(H(`await window.__frLibDebug.deleteByPath(${JSON.stringify(fDisk.replace(/\\/g, "\\\\"))},true); await wait(()=>q('.choice-row button'),'choice3',60); return 1;`));
  out.steps.diskDialog = step("diskDialog", await M.evalJs(H(`
    return { msg:q('.confirm-msg')?.textContent?.replace(/\\s+/g,' ').trim(), buttons:qa('.choice-row button').map(b=>b.textContent.trim()) };
  `)));
  await SHOT(M, "dialog_disk_3buttons.png");
  await M.evalJs(H(`[...document.querySelectorAll('.choice-row button')].find(b=>b.textContent.trim()==='取消')?.click(); await sleep(400); return 1;`));

  // ===== 打回2：D 存储清理前后 =====
  const covDir = path.join(data, "covers"); fs.mkdirSync(covDir, { recursive: true });
  for (let i = 0; i < 20; i++) fs.writeFileSync(path.join(covDir, `fake${i}.png`), Buffer.alloc(10000));
  const resDir = path.join(data, "res", "1"); fs.mkdirSync(resDir, { recursive: true });
  for (let i = 0; i < 10; i++) fs.writeFileSync(path.join(resDir, `r${i}.bin`), Buffer.alloc(5000));
  out.steps.storage = step("storage", await M.evalJs(H(`
    const before=await inv('storage_stats');
    const freedCovers=await inv('clear_cache',{what:'covers'});
    const afterCovers=await inv('storage_stats');
    const freedAll=await inv('clear_cache',{what:'all'});
    const afterAll=await inv('storage_stats');
    return { beforeCovers:before.covers_bytes, freedCovers, afterCoversCovers:afterCovers.covers_bytes,
      beforeRes:afterCovers.res_bytes, freedAll, afterAllCache:afterAll.cache_bytes };
  `)));

  console.log(JSON.stringify(out.steps, null, 1));
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(path.join(SANDBOX_ROOT, "f3_out.json"), JSON.stringify(out, null, 1));
  console.log("evidence:", evid);
}
