// 回归：横向漫画路径 / 普通文字书路径（确认本轮纵向改动没有外溢）。
// 布局切换走「localStorage 配置 + 重载」这条真实代码路径，避免依赖设置弹窗的 DOM 结构。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9593);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "mflow_sbx");
const data = path.join(SANDBOX_ROOT, "mflow_data");
const BOOKS = path.join(SANDBOX_ROOT, "mflow_books");
const FIXTURE = path.join(BOOKS, "manga_flow_fixture.epub");
const TEXTBOOK = path.join(BOOKS, "text_book.epub");
if (!fs.existsSync(TEXTBOOK)) fs.copyFileSync(path.join(SANDBOX_ROOT, "scroll_books", "zhen_dian.epub"), TEXTBOOK);
const diagLog = path.join(SANDBOX_ROOT, "mflow_regress_diag.log");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const raf=()=>new Promise(r=>requestAnimationFrame(()=>r()));
const wait=async(fn,name,rounds=300)=>{for(let i=0;i<rounds;i++){const v=await fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const cfg=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}');
const setLayout=(l)=>{ const c=cfg(); c.reader=c.reader||{}; c.reader.manga=Object.assign({},c.reader.manga||{},{layout:l}); localStorage.setItem('tidepage.config.v1',JSON.stringify(c)); };
const cards=(rounds=400)=>wait(()=>{const c=qa('.card,.row');return c.length?c:null;},'cards',rounds);
const openBook=async(name)=>{ const c=await cards(); const b=c.find(x=>x.textContent.includes(name)); if(!b) throw new Error('card not found: '+name);
  b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true})); await wait(()=>q('.reader'),'reader',400); await sleep(3000);
  if(!q('.reader').classList.contains('manga')){ const btn=qa('.tb-r button').find(x=>(x.getAttribute('title')||'').includes('漫画模式')); btn&&btn.click(); await sleep(2000);} await sleep(2500); };
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;

const out = { steps: {} };
let app = null;
const reload = async () => {
  await M.evalJs(`setTimeout(()=>location.reload(),80);1`);
  await sleep(4500);
  M.close();
  M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { const r = await M.evalJs(`({'t':!!window.__TAURI_INTERNALS__})`); if (r?.t) return; } catch { /* wait */ } await sleep(400); }
  throw new Error("app not ready after reload");
};

let M = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: diagLog });
  for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  for (let i = 0; i < 300; i++) { try { const r = await M.evalJs(`({'t':!!window.__TAURI_INTERNALS__})`); if (r?.t) break; } catch { /* wait */ } await sleep(400); }
  const add = await M.evalJs(H(`return await inv('add_dropped_book', { path: ${JSON.stringify(FIXTURE)} });`));
  const addTxt = await M.evalJs(H(`try { return await inv('add_dropped_book', { path: ${JSON.stringify(TEXTBOOK)} }); } catch(e){ return {err:String(e)}; }`));
  log("added manga=" + add.id + " text=" + (addTxt.id ?? addTxt.err));
  await reload();

  // ---- 1) 横向漫画路径：滚轮 + 拖拽 + 窗口裁剪 ----
  out.steps.setHorizontal = await M.evalJs(H(`setLayout('horizontal'); return cfg().reader.manga.layout;`));
  await reload();
  out.steps.horizontal = await M.evalJs(H(`
    await openBook('manga_flow_fixture');
    // 回到第 5 话，保证不在横向窗口尽头（否则滚轮/拖拽都无从移动）
    window.dispatchEvent(new CustomEvent('tidepage-jump', { detail: 5 }));
    await sleep(3500);
    const vp = q('.manga-h-viewport'), cv = q('.manga-h-canvas');
    if (!vp || !cv) return { err: 'no horizontal viewport', layout: cfg().reader?.manga?.layout };
    const w0 = cv.offsetWidth, tf0 = cv.style.transform || 'none';
    vp.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: 900, deltaMode: 0 }));
    await sleep(600);
    const tfWheel = cv.style.transform, w1 = cv.offsetWidth;
    const r = vp.getBoundingClientRect();
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    vp.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true, clientX: cx, clientY: cy, button: 0, buttons: 1, pointerId: 71, pointerType: 'mouse', isPrimary: true }));
    await sleep(60);
    vp.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, cancelable: true, clientX: cx - 240, clientY: cy, button: 0, buttons: 1, pointerId: 71, pointerType: 'mouse', isPrimary: true }));
    await sleep(250);
    const tfDrag = cv.style.transform;
    vp.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, cancelable: true, clientX: cx - 240, clientY: cy, button: 0, buttons: 0, pointerId: 71, pointerType: 'mouse', isPrimary: true }));
    await sleep(500);
    const imgs = qa('.manga-h-canvas .manga-block img').length;
    const loaded = qa('.manga-h-canvas .manga-block img').filter(x => x.complete && x.naturalWidth > 0).length;
    return { layout: cfg().reader?.manga?.layout, canvasW: w0, canvasWAfterWheel: w1, tf0, tfWheel, tfDrag,
             wheelMoved: tfWheel !== tf0, dragMoved: tfDrag !== tfWheel,
             blocks: qa('.manga-h-canvas .manga-block').length, imgs, loaded,
             transformSet: !!cv.style.transform };
  `));
  log("horizontal: " + JSON.stringify(out.steps.horizontal));

  // ---- 2) 纵向（flow）仍可用，并确认与横向互不干扰 ----
  await M.evalJs(H(`setLayout('flow'); return 1;`));
  await reload();
  out.steps.flowAgain = await M.evalJs(H(`
    await openBook('manga_flow_fixture');
    const vp = q('.manga-flow-viewport');
    if (!vp) return { err: 'no flow viewport' };
    const reader = q('.reader');
    for (let i = 0; i < 12; i++) { reader.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: 600, deltaMode: 0 })); await raf(); await sleep(50); }
    const bs = qa('.manga-flow-viewport .manga-block').map(b => Number(b.dataset.index));
    return { blocks: bs.length, first: bs[0], last: bs[bs.length-1], top: vp.scrollTop, sh: vp.scrollHeight,
             topChapter: (() => { for (const b of qa('.manga-flow-viewport .manga-block')) if (b.offsetTop + b.offsetHeight > vp.scrollTop) return Number(b.dataset.index); return -1; })() };
  `));
  log("flowAgain: " + JSON.stringify(out.steps.flowAgain));

  // ---- 3) 普通文字书：滚动路径未被改动 ----
  out.steps.text = await M.evalJs(H(`
    for (let k = 0; k < 16 && !q('.library'); k++) { q('.tb-l button.icon')?.click(); await sleep(700); }
    await sleep(900);
    const c = await cards();
    const b = c.find(x => x.textContent.includes('text_book'));
    if (!b) return { err: 'text book card not found', some: c.slice(0, 4).map(x => x.textContent.slice(0, 20)) };
    b.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await wait(() => q('#chapter-content'), 'chapter', 800);
    await sleep(3000);
    const reader = q('.reader');
    const wasManga = reader.classList.contains('manga');
    if (wasManga) { const btn = qa('.tb-r button').find(x => (x.getAttribute('title')||'').includes('漫画模式')); btn && btn.click(); await sleep(2000); }
    const area = q('.scroll-area');
    const before = { mode: cfg().reader?.mode, wasManga, nowManga: reader.classList.contains('manga'),
                     chapterHtml: !!q('#chapter-content'), overflow: area ? getComputedStyle(area).overflow : null,
                     scrollTop: area ? area.scrollTop : -1, scrollHeight: area ? area.scrollHeight : -1,
                     clientHeight: area ? area.clientHeight : -1, transform: area ? (area.style.transform||'') : '' };
    // 第 1 话内容太短（几乎不满屏）→ 跳到第 20 话再测滚动
    window.dispatchEvent(new CustomEvent('tidepage-jump', { detail: 20 }));
    await sleep(3000);
    for (let i = 0; i < 14; i++) { reader.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: 500, deltaMode: 0 })); await raf(); await sleep(55); }
    const mid = area.scrollTop;
    return { before, scrollHeight2: area.scrollHeight, mid, afterTop: area.scrollTop, afterTransform: area.style.transform || '',
             scrolled: area.scrollTop > 0, movedTwice: area.scrollTop > mid };
  `));
  log("text: " + JSON.stringify(out.steps.text));

  M.close();
} catch (e) { process.exitCode = 1; /* 失败必须体现在退出码上 */ out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("mflow_sbx"); }
  fs.writeFileSync(path.join(SANDBOX_ROOT, "mflow_regress.json"), JSON.stringify(out, null, 1));
  console.log("=== REGRESS ===");
  console.log(JSON.stringify(out, null, 1));
}
// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
