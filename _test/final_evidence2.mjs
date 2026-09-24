// 打回2（删书弹窗截图 + 删除笔记路径计数）+ 同路径删两次 + C/D 数据。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9755);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "fin2_sbx"); const data = path.join(SANDBOX_ROOT, "fin2_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "fin2_evidence"); fs.rmSync(evid, { recursive: true, force: true }); fs.mkdirSync(evid, { recursive: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=300)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;
const SHOT = async (M, n) => { try { const s = await M.call("Page.captureScreenshot", { format: "png" }); if (s?.data) fs.writeFileSync(path.join(evid, n), Buffer.from(s.data, "base64")); } catch {} };

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "fin2_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  const tmp = path.join(SANDBOX_ROOT, "fin2_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  const fDel = path.join(tmp, "删除笔记书 (作者丙).txt");
  const fKeep = path.join(tmp, "保留笔记书 (作者丁).txt");
  const fTwice = path.join(tmp, "同路径删两次 (作者戊).txt");
  fs.writeFileSync(fDel, "第一章 测试\n正文内容。".repeat(30), "utf8");
  fs.writeFileSync(fKeep, "第一章 测试\n正文内容。".repeat(30), "utf8");
  fs.writeFileSync(fTwice, "第一章 测试\n正文内容。".repeat(30), "utf8");
  for (const p of [fDel, fKeep, fTwice]) await M.evalJs(H(`try{await inv('add_dropped_book',{path:${JSON.stringify(p.replace(/\\/g, "\\\\"))}});}catch(e){}`));
  await sleep(1500); await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(1000);

  const ids = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'',sort:'title',offset:0,limit:50});
    const g=(s)=>pg.rows.find(r=>r.path.includes(s))?.id;
    return { del:g('删除笔记书'), keep:g('保留笔记书'), twice:g('同路径删两次') };
  `));
  out.steps.ids = ids;
  // 给三本书各加带 note 划线
  out.steps.setup = await M.evalJs(H(`
    const mk=async(id,n)=>{ await inv('add_highlight',{bookId:id,chapterIndex:0,chapterTitle:'第一章',start:0,end:6,text:'划线'+n,style:'underline',color:'',note:'感想'+n});
      // bookmarks 各加一条
      await inv('add_bookmark',{bookId:id,chapterIndex:0,title:'B'+n,excerpt:'e'}); };
    await mk(${ids.del},'del'); await mk(${ids.keep},'keep'); await mk(${ids.twice},'twice');
    return { ok:true };
  `));

  // 打开书库并触发删除（有笔记）→ 截图弹窗
  await M.evalJs(H(`
    for(let i=0;i<20;i++){ if(q('.lib-tb-group')) break; await sleep(300); }
    const ok=window.__frLibDebug?.deleteByPath(${JSON.stringify(fDel.replace(/\\/g, "\\\\"))},false);
    await sleep(600); return ok;
  `));
  out.steps.dialog = await M.evalJs(H(`
    const d=q('dialog[open]');
    return { hasDialog: !!d, msg: q('.confirm-msg')?.textContent?.replace(/\\s+/g,' ').trim(), buttons: qa('.confirm-row button').map(b=>b.textContent.trim()) };
  `));
  await SHOT(M, "delete_dialog.png");

  // 走"删除笔记"：第一次 confirm 点是（保留）→ 第二次 confirm 点确定（删除笔记）
  out.steps.delNotes = await M.evalJs(H(`
    const clickConfirm=(label)=>{ const b=qa('.confirm-row button').find(x=>x.textContent.includes(label)); b?.click(); };
    // 第一次弹窗：确定=保留笔记
    clickConfirm('确定'); await sleep(500);
    // 第二次弹窗：确定=连同笔记删除
    const second={ msg:q('.confirm-msg')?.textContent?.replace(/\\s+/g,' ').trim(), btns:qa('.confirm-row button').map(b=>b.textContent.trim()) };
    clickConfirm('确定'); await sleep(800);
    const id=${ids.del};
    const hl=await inv('list_highlights',{bookId:id});
    const bm=await inv('list_bookmarks',{bookId:id});
    const notes=await inv('list_note_books');
    return { secondMsg:second.msg, secondBtns:second.btns, afterHL:hl.length, afterBookmarks:bm.length,
      noteBooks:notes.map(n=>n.title) };
  `));

  // 走"保留笔记"：保留书
  out.steps.keepNotes = await M.evalJs(H(`
    const clickConfirm=(label)=>{ const b=qa('.confirm-row button').find(x=>x.textContent.includes(label)); b?.click(); };
    window.__frLibDebug.deleteByPath(${JSON.stringify(fKeep.replace(/\\/g, "\\\\"))},false);
    await sleep(600);
    clickConfirm('确定'); await sleep(500);   // 保留
    clickConfirm('取消'); await sleep(700);   // 取消"一起删笔记" = 保留笔记
    const id=${ids.keep};
    const hl=await inv('list_highlights',{bookId:id});
    const bm=await inv('list_bookmarks',{bookId:id});
    const notes=await inv('list_note_books');
    return { afterHL:hl.length, afterBookmarks:bm.length, noteTitles:notes.map(n=>n.title) };
  `));

  // 同路径删两次（必验5）
  out.steps.twice = await M.evalJs(H(`
    const p=${JSON.stringify(fTwice.replace(/\\/g, "\\\\"))};
    const id=${ids.twice};
    // 第一次删（保留笔记）
    await inv('delete_books',{paths:[p],fromDisk:false,keepNotes:true});
    // 重新导入同路径并加第二批笔记
    const b2=await inv('add_dropped_book',{path:p});
    await inv('add_highlight',{bookId:b2.id,chapterIndex:0,chapterTitle:'第一章',start:0,end:6,text:'第二批划线',style:'underline',color:'',note:'第二批感想'});
    // 第二次删（保留笔记）
    await inv('delete_books',{paths:[p],fromDisk:false,keepNotes:true});
    const notes=await inv('list_note_books');
    const titles=notes.filter(n=>n.title.includes('同路径删两次')).map(n=>({title:n.title,n:n.noteCount,id:n.bookId}));
    return { groups: titles, totalNoteBooks: notes.length };
  `));

  // C/D：storage_stats + 清理前后
  out.steps.storage = await M.evalJs(H(`
    const before=await inv('storage_stats');
    const freed=await inv('clear_cache',{what:'covers'});
    const after=await inv('storage_stats');
    return { before, freed, after };
  `));

  // C：文案检查（三处 AI 味解释应已不在 DOM）
  out.steps.copy = await M.evalJs(H(`
    const openSettings=async(t)=>{ q('button[title="设置"]')?.click(); await wait(()=>q('dialog[open]'),'dlg'); [...document.querySelectorAll('.set-nav-item')].find(x=>x.textContent.includes(t))?.click(); await sleep(350); };
    await openSettings('界面外观');
    const html=document.body.innerHTML;
    const bad=['与阅读正文的字号/字体/配色完全独立','浅色/深色各自保存所选皮肤'];
    const foundAppearance=bad.filter(t=>html.includes(t));
    [...document.querySelectorAll('.set-nav-item')].find(x=>x.textContent.includes('阅读排版'))?.click(); await sleep(350);
    const html2=document.body.innerHTML;
    const foundReader=['存档点','点击缩略图即套用该方案'].filter(t=>html2.includes(t));
    return { foundAppearance, foundReader };
  `));

  console.log(JSON.stringify(out.steps, null, 1));
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(path.join(SANDBOX_ROOT, "fin2_out.json"), JSON.stringify(out, null, 1));
  console.log("evidence:", evid);
}
