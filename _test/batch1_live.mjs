// 批1 验证：B 刷新长按（点=只读书籍/长按=刷新封面）、A 封面进度条、D 存储+清缓存、C 文案、E 搜索框可拖出。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9666);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "b1_sbx"); const data = path.join(SANDBOX_ROOT, "b1_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "batch1_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=250)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a); const rect=(el)=>{if(!el)return null;const r=el.getBoundingClientRect();return{x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}};\n${b} })()`;

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "b1_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  // 加书
  const tmp = path.join(SANDBOX_ROOT, "b1_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  for (let i = 0; i < 8; i++) { const d = path.join(tmp, `b_${i}.epub`); fs.copyFileSync(path.join(BOOKS, "test_conv.epub"), d); await M.evalJs(H(`try{await inv('add_dropped_book',{path:${JSON.stringify(d.replace(/\\/g,"\\\\"))}});}catch(e){}`)); }
  await sleep(1200);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(1500);

  // D. 存储统计
  out.steps.storage = await M.evalJs(H(`
    return await inv('storage_stats');
  `));
  log("storage: " + JSON.stringify(out.steps.storage));

  // D. 清缓存（covers）
  out.steps.clear = await M.evalJs(H(`
    const before=await inv('storage_stats');
    const freed=await inv('clear_cache',{what:'covers'});
    const after=await inv('storage_stats');
    return { freedBytes: freed, coversBefore: before.covers_bytes, coversAfter: after.covers_bytes };
  `));
  log("clear: " + JSON.stringify(out.steps.clear));

  // B. 长按刷新 → 菜单出现；普通点击不出现菜单
  out.steps.longPress = await M.evalJs(H(`
    const btn=[...document.querySelectorAll('.toolbar button')].find(b=>/刷新/.test(b.textContent));
    if(!btn) return { err:'no refresh btn' };
    // 普通点击
    btn.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerId:1}));
    btn.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,pointerId:1}));
    btn.click(); await sleep(300);
    const menuAfterClick=!!q('.refresh-menu');
    await sleep(1500);
    // 长按 700ms
    btn.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerId:2}));
    await sleep(700);
    const menuAfterHold=!!q('.refresh-menu');
    const items=qa('.refresh-menu .rm-item').map(x=>x.textContent.trim());
    btn.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,pointerId:2}));
    q('.rm-backdrop')?.click(); await sleep(200);
    return { menuAfterClick, menuAfterHold, items, hasCoversItem: items.some(t=>t.includes('刷新封面')) };
  `));
  log("longPress: " + JSON.stringify(out.steps.longPress));

  // A. 刷新封面 → 底栏出现进度条（观察 DOM；不依赖模块导入）
  out.steps.progress = await M.evalJs(H(`
    // 先让 books 有 cover（此时 covers 已被清空、meta_done 已复位）
    await inv('refresh_covers');
    let seen=null;
    for(let i=0;i<60;i++){
      const bar=q('.meta-progress');
      if(bar){ seen={ text: bar.querySelector('.scan-text')?.textContent||'', w: bar.querySelector('.scan-fill')?.style.width||'' }; break; }
      await sleep(200);
    }
    return { barAppeared: !!seen, sample: seen };
  `));
  log("progress: " + JSON.stringify(out.steps.progress));

  // C. 文案：配色方案下方长解释应已删除
  out.steps.copy = await M.evalJs(H(`
    const raw=document.body.innerHTML;
    const bad=['存档点','与阅读正文的字号/字体/配色完全独立','浅色/深色各自保存所选皮肤'];
    const found=bad.filter(t=>raw.includes(t));
    return { removedExplanations: found.length===0, stillPresent: found };
  `));
  log("copy: " + JSON.stringify(out.steps.copy));

  // E. 搜索框：结构上在 .body 之外 + 用真实鼠标选中文字后打开搜索框并拖出阅读区
  await M.evalJs(H(`
    const c=await wait(()=>{const x=qa('.card,.row');return x.length?x:null;},'cards',300);
    c[0].dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',400); await sleep(1500); return 1;
  `));
  // 用 CDP 真实鼠标拖选一段文字（产生真实 selection）
  const selPt = await M.evalJs(H(`
    const root=q('#chapter-content');
    const w=document.createTreeWalker(root,NodeFilter.SHOW_TEXT); let n;
    while(n=w.nextNode()){ if(n.nodeValue && n.nodeValue.trim().length>12){ break; } }
    if(!n) return null;
    const r=document.createRange(); r.selectNodeContents(n);
    const rects=r.getClientRects(); const rc=rects[0]||r.getBoundingClientRect();
    return { x1:Math.round(rc.left+2), y1:Math.round(rc.top+rc.height/2), x2:Math.round(Math.min(rc.right-2, rc.left+120)), y2:Math.round(rc.top+rc.height/2) };
  `));
  if (selPt) {
    await M.call("Input.dispatchMouseEvent", { type: "mousePressed", x: selPt.x1, y: selPt.y1, button: "left", clickCount: 1 });
    await M.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: selPt.x2, y: selPt.y2, button: "left", buttons: 1 });
    await M.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: selPt.x2, y: selPt.y2, button: "left", clickCount: 1 });
    await sleep(600);
  }
  out.steps.searchPopup = await M.evalJs(H(`
    const menu=[...document.querySelectorAll('button')].find(b=>(b.getAttribute('title')||'').includes('在本书内搜索'));
    if(menu){ menu.click(); await sleep(700); }
    const pop=q('.sb-pop');
    if(!pop) return { err:'no popup', menuFound: !!menu };
    const inBody = !!q('.body .sb-pop');
    const isReaderChild = !!q('.reader > .sb-pop');
    const zidx = getComputedStyle(pop).zIndex;
    const beforeE=rect(pop);
    const head=q('.sb-head');
    head.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerId:11,clientX:beforeE.x+40,clientY:beforeE.y+16}));
    window.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,pointerId:11,clientX:beforeE.x+40-260,clientY:beforeE.y+16-140}));
    window.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,pointerId:11}));
    await sleep(300);
    const afterE=rect(pop);
    return { popupRendered:true, inBody, isReaderChild, zidx, before:beforeE, after:afterE, moved: beforeE.x!==afterE.x || beforeE.y!==afterE.y };
  `));
  log("searchPopup: " + JSON.stringify(out.steps.searchPopup));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
