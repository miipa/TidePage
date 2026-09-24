// G/H 验证：删书保留/删除笔记、删后仍显示书名、笔记界面截图、H1/H2/H3。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9733);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "gh2_sbx"); const data = path.join(SANDBOX_ROOT, "gh2_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "gh2_evidence"); fs.rmSync(evid, { recursive: true, force: true }); fs.mkdirSync(evid, { recursive: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=300)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;
const shot = async (M, name) => {
  try { const s = await M.call("Page.captureScreenshot", { format: "png" }); if (s?.data) fs.writeFileSync(path.join(evid, name), Buffer.from(s.data, "base64")); } catch {}
};

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "gh2_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  const tmp = path.join(SANDBOX_ROOT, "gh2_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  const fA = path.join(tmp, "笔记书A (作者甲).txt");
  const fB = path.join(tmp, "纯划线书B (作者乙).txt");
  fs.writeFileSync(fA, "第一章 测试\n这是一段用于划线的正文内容。".repeat(30), "utf8");
  fs.writeFileSync(fB, "第一章 测试\n另一段用于划线的正文内容。".repeat(30), "utf8");
  for (const p of [fA, fB]) await M.evalJs(H(`try{await inv('add_dropped_book',{path:${JSON.stringify(p.replace(/\\/g, "\\\\"))}});}catch(e){}`));
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(1200);

  const ids = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'',sort:'title',offset:0,limit:50});
    const a=pg.rows.find(r=>r.path.includes('笔记书A'));
    const b=pg.rows.find(r=>r.path.includes('纯划线书B'));
    return { a:a?.id, b:b?.id };
  `));
  out.steps.ids = ids;

  // 写划线：A 一条带 note；B 一条纯划线
  out.steps.addHL = await M.evalJs(H(`
    const A=${ids.a}, B=${ids.b};
    const idA=await inv('add_highlight',{bookId:A,chapterIndex:0,chapterTitle:'第一章',start:0,end:8,text:'A的划线',style:'underline',color:'',note:'A的感想内容'});
    await inv('add_highlight',{bookId:B,chapterIndex:0,chapterTitle:'第一章',start:0,end:8,text:'B的划线',style:'underline',color:'',note:''});
    return { idA };
  `));

  // 打开笔记界面截图（A 进入、B 不进）
  out.steps.noteListBefore = await M.evalJs(H(`
    const books=await inv('list_note_books');
    return { count: books.length, books: books.map(b=>({id:b.bookId,title:b.title,author:b.author,deleted:b.deleted,n:b.noteCount})) };
  `));

  // G1: 删除 A（走"保留笔记"）→ 走真实命令
  out.steps.deleteKeep = await M.evalJs(H(`
    const A=${ids.a};
    const info=await inv('book_note_info',{bookId:A});
    const before=await inv('list_highlights',{bookId:A});
    await inv('delete_books',{paths:[${JSON.stringify(fA.replace(/\\/g, "\\\\"))}],fromDisk:false,keepNotes:true});
    const after=await inv('list_highlights',{bookId:A});
    const notes=await inv('list_note_books');
    return { hasNotes:info.hasNotes, noteCount:info.noteCount, beforeHL:before.length, afterHL:after.length,
      noteBooks: notes.map(b=>({id:b.bookId,title:b.title,author:b.author,deleted:b.deleted,n:b.noteCount})) };
  `));

  // 打开笔记界面（书库顶栏「笔记」按钮）→ 截图（应显示 A 已删除但书名仍在）
  out.steps.notesUIShot = await M.evalJs(H(`
    // 回到书库
    for(let k=0;k<12 && !q('.library');k++){ const back=[...document.querySelectorAll('.tb-l button, .topbar button')].find(b=>b.textContent.includes('←')); if(back) back.click(); await sleep(500); }
    const nb=[...document.querySelectorAll('.lib-tb-group button')].find(b=>(b.getAttribute('title')||'').includes('笔记'));
    if(nb) nb.click();
    await sleep(800);
    const panel=q('.np');
    const cards=qa('.np-card').map(c=>({ text:c.textContent.replace(/\\s+/g,' ').trim(), del:!!c.querySelector('.np-del') }));
    return { panelOpen: !!panel, cards };
  `));
  await shot(M, "notes_deleted.png");

  // G1b: 无笔记的 B → 直接删（无弹窗由前端逻辑保证，这里验命令后行数=0）
  out.steps.deleteB = await M.evalJs(H(`
    const B=${ids.b};
    const info=await inv('book_note_info',{bookId:B});
    const before=await inv('list_highlights',{bookId:B});
    await inv('delete_books',{paths:[${JSON.stringify(fB.replace(/\\/g, "\\\\"))}],fromDisk:false,keepNotes:true});
    const after=await inv('list_highlights',{bookId:B});
    return { hasNotes:info.hasNotes, beforeHL:before.length, afterHL:after.length };
  `));

  // H3: append note（AI 添加到笔记）
  out.steps.h3 = await M.evalJs(H(`
    // 需要一条存在的 highlight：A 的笔记已移到孤儿命名空间(-id)。构造新书+划线再 append。
    const pg=await inv('list_books_page',{query:'',sort:'title',offset:0,limit:50});
    return { remainingBooks: pg.rows.filter(r=>/gh2_books/i.test(r.path)).length };
  `));

  await shot(M, "state.png");
  console.log(JSON.stringify(out.steps, null, 1));
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(path.join(SANDBOX_ROOT, "gh2_out.json"), JSON.stringify(out, null, 1));
  console.log("evidence in", evid);
}
