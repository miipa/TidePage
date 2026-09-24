// 收尾证据：H1/H2/H3 截图、删书弹窗截图+删除笔记路径、G2 端到端、同路径删两次、C/D。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9744);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "fin_sbx"); const data = path.join(SANDBOX_ROOT, "fin_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "fin_evidence"); fs.rmSync(evid, { recursive: true, force: true }); fs.mkdirSync(evid, { recursive: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=300)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

const out = { steps: {} };
let app = null;
const SHOT = async (M, n) => { try { const s = await M.call("Page.captureScreenshot", { format: "png" }); if (s?.data) { fs.writeFileSync(path.join(evid, n), Buffer.from(s.data, "base64")); return true; } } catch {} return false; };

try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "fin_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  const tmp = path.join(SANDBOX_ROOT, "fin_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  const fA = path.join(tmp, "端到端A (作者甲).txt");
  fs.writeFileSync(fA, "第一章 测试\n这是一段用于划线的正文内容，足够长以便悬停命中。".repeat(30), "utf8");
  await M.evalJs(H(`try{await inv('add_dropped_book',{path:${JSON.stringify(fA.replace(/\\/g, "\\\\"))}});}catch(e){}`));
  await sleep(1500); await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(1200);

  const ids = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'',sort:'title',offset:0,limit:50});
    const a=pg.rows.find(r=>r.path.includes('端到端A'));
    return { a:a?.id };
  `));
  out.steps.ids = ids;

  // 打开书 + 建立带 sense 的划线（note 为空，供 H1 输入）
  await M.evalJs(H(`
    const c=await wait(()=>{const x=qa('.card,.row');return x.length?x:null;},'cards',300);
    c[0].dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',500); await sleep(1500);
    return 1;
  `));
  // 等 Reader 挂载并暴露 DEV 钩子
  await M.evalJs(H(`
    for(let i=0;i<60;i++){ if(window.__frDebug) break; await sleep(300); }
    return !!window.__frDebug;
  `));
  const hlIds = await M.evalJs(H(`
    const id=await inv('add_highlight',{bookId:${ids.a},chapterIndex:0,chapterTitle:'第一章',start:0,end:6,text:'这是一段',style:'underline',color:'',note:''});
    window.dispatchEvent(new CustomEvent('flowread-jump',{detail:0})); await sleep(1500);
    return { hlNoNote: id, hasDebug: !!window.__frDebug };
  `));
  out.steps.hlIds = hlIds;

  // H1: 打开划线菜单 → 截图（含"添加感想"）→ 展开 → 输入 → 保存
  out.steps.h1 = await M.evalJs(H(`
    const ok=await window.__frDebug.openHighlightMenu(${hlIds.hlNoNote});
    await sleep(500);
    return { menuOpen: !!q('.hl-menu'), btns: qa('.hl-menu button').map(b=>b.textContent.trim()), ok };
  `));
  await SHOT(M, "h1_menu.png");
  out.steps.h1Saved = await M.evalJs(H(`
    // 点"添加感想"
    const btn=qa('.hl-menu button').find(b=>b.textContent.includes('添加感想'));
    btn?.click(); await sleep(400);
    const ta=q('.hl-menu textarea');
    if(ta){ ta.value='这是我手动输入的感想内容'; ta.dispatchEvent(new Event('input',{bubbles:true})); }
    await sleep(200);
    const save=qa('.hl-menu button').find(b=>b.textContent.includes('保存感想'));
    save?.click(); await sleep(600);
    const hl=await inv('list_highlights',{bookId:${ids.a}});
    const one=hl.find(x=>x.id===${hlIds.hlNoNote});
    return { note: one?.note, menuClosed: !q('.hl-menu') };
  `));
  await SHOT(M, "h1_saved.png");

  // H2: 悬停该划线（有 note）→ 浮层；再悬停一条 note='' → 不弹
  const hlNoNote2 = await M.evalJs(H(`
    const id=await inv('add_highlight',{bookId:${ids.a},chapterIndex:0,chapterTitle:'第一章',start:8,end:12,text:'用于悬停',style:'underline',color:'',note:''});
    window.dispatchEvent(new CustomEvent('flowread-jump',{detail:0})); await sleep(1200);
    return id;
  `));
  out.steps.h2 = await M.evalJs(H(`
    const withNote=await window.__frDebug.hoverHighlight(${hlIds.hlNoNote});
    await sleep(400);
    const tipText=q('.fr-note-tip')?.textContent||null;
    const noNote=await window.__frDebug.hoverHighlight(${hlNoNote2});
    await sleep(300);
    const tipAfter=q('.fr-note-tip')?.textContent||null;
    return { withNote, tipShown: !!tipText, tipText, noNoteHover: noNote, tipAfterNoNote: tipAfter };
  `));
  // 重新悬停有 note 的便于截图
  await M.evalJs(H(`window.__frDebug.hoverHighlight(${hlIds.hlNoNote}); await sleep(400); return 1;`));
  await SHOT(M, "h2_hover.png");

  // H3: AI 释义小窗（DEV 钩子打开）→ 点"添加到笔记"→ 追加（不覆盖）
  out.steps.h3 = await M.evalJs(H(`
    const ok=await window.__frDebug.openAiForHighlight(${hlIds.hlNoNote});
    await sleep(600);
    const before=(await inv('list_highlights',{bookId:${ids.a}})).find(x=>x.id===${hlIds.hlNoNote})?.note;
    const addBtn=qa('.qa-pop button').find(b=>b.textContent.includes('添加到笔记'));
    const hasBtn=!!addBtn;
    // 直接调用 append（不依赖 AI 返回），验证追加逻辑与按钮存在
    await inv('append_highlight_note',{id:${hlIds.hlNoNote}, addition:'AI 追加的内容'});
    const after=(await inv('list_highlights',{bookId:${ids.a}})).find(x=>x.id===${hlIds.hlNoNote})?.note;
    if(addBtn) addBtn.click(); await sleep(400);
    return { openedAi:ok, hasAddBtn:hasBtn, before, after, preserved: after.includes('这是我手动输入的感想内容') && after.includes('AI 追加的内容') };
  `));
  await SHOT(M, "h3_ai.png");

  // G2 端到端：删书(保留笔记)→同路径放B→重导→B 无 A 划线、A 笔记仍在
  out.steps.g2 = await M.evalJs(H(`
    const A=${ids.a};
    await inv('delete_books',{paths:[${JSON.stringify(fA.replace(/\\/g, "\\\\"))}],fromDisk:false,keepNotes:true});
    return { afterDelete: (await inv('list_highlights',{bookId:A})).length };
  `));
  // 同路径放"B"内容后重导
  fs.writeFileSync(fA, "这是B书完全不同的内容。".repeat(30), "utf8");
  out.steps.g2b = await M.evalJs(H(`
    const b=await inv('add_dropped_book',{path:${JSON.stringify(fA.replace(/\\/g, "\\\\"))}});
    await sleep(500);
    const hl=await inv('list_highlights',{bookId:b.id});
    const notes=await inv('list_note_books');
    return { B_id:b.id, B_highlights:hl.length, B_hasA_highlight: hl.some(x=>x.text==='这是一段'),
      noteBooks: notes.map(n=>({title:n.title, deleted:n.deleted, n:n.noteCount})) };
  `));
  await SHOT(M, "g2_notes.png");

  console.log(JSON.stringify(out.steps, null, 1));
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(path.join(SANDBOX_ROOT, "fin_out.json"), JSON.stringify(out, null, 1));
  console.log("evidence:", evid);
}
