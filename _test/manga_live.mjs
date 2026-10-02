// Live verification of the manga-mode changes (isolated sandbox + real WebView2 + debug CDP).
// Covers: first-run guide once globally + live preview, topbar manga button per-book persist,
//         dedicated Manga settings tab, double-page horizontal (direction/gap/no-shrink),
//         drag panning, real topbar drag reorder (no misclick), image bump beyond global cap.
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9577);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const sbx = path.join(SANDBOX_ROOT, "manga_sbx");
const data = path.join(SANDBOX_ROOT, "manga_data");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
for (const dir of [sbx, data]) {
  for (const f of fs.readdirSync(dir)) fs.rmSync(path.join(dir, f), { recursive: true, force: true });
}
const outFile = path.join(SANDBOX_ROOT, "manga_out.json");
const diagLog = path.join(SANDBOX_ROOT, "manga_diag.log");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=200)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const cards=(rounds=200)=>wait(()=>{const c=qa('.card,.row');return c.length?c:null;},'cards',rounds);
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const rect=(el)=>{ if(!el) return null; const r=el.getBoundingClientRect(); return {x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}; };
const cfgRead=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}');
const bcRead=()=>JSON.parse(localStorage.getItem('tidepage.bookcfg.v1')||'{}');
const rdVar=()=>getComputedStyle(q('.reader')).getPropertyValue('--rdr-img-scale').trim();
const field=(t)=>[...document.querySelectorAll('.field')].find(f=>f.textContent.includes(t));
const setSelect=async(label,val)=>{ const s=field(label)?.querySelector('select'); if(!s) throw new Error('no select '+label); s.value=val; s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(180); };
const setInput=async(label,val)=>{ const i=field(label)?.querySelector('input'); if(!i) throw new Error('no input '+label); i.value=String(val); i.dispatchEvent(new Event('input',{bubbles:true})); i.dispatchEvent(new Event('change',{bubbles:true})); await sleep(180); };
const openSettings=async(tab)=>{ q('[data-testid="lib-btn-settings"]')?.click(); await wait(()=>q('dialog[open]'),'dialog'); document.querySelector('[data-testid="set-tab-'+tab+'"]')?.click(); await sleep(300); };
const closeSettings=async()=>{ document.querySelector('[data-testid=\"set-done\"]')?.click(); await sleep(250); };
const tbBtn=(id)=>document.querySelector('[data-testid="tb-'+id+'"]');
const ptr=(el,type,x,y,id)=>{ el.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,clientX:Math.round(x),clientY:Math.round(y),button:0,buttons:type==='pointerup'?0:1,pointerId:id||11,pointerType:'mouse',isPrimary:true})); };
const dragTo=async(from,to,place,id)=>{ const a=rect(from), b=rect(to); const sx=a.x+a.w/2, sy=a.y+a.h/2; ptr(from,'pointerdown',sx,sy,id); await sleep(40); window.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,cancelable:true,clientX:Math.round(sx+9),clientY:Math.round(sy+2),button:0,buttons:1,pointerId:id||11})); await sleep(40); const tx = place==='after' ? b.x+b.w+4 : b.x-4; window.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,cancelable:true,clientX:Math.round(tx),clientY:Math.round(b.y+b.h/2),button:0,buttons:1,pointerId:id||11})); await sleep(60); window.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,cancelable:true,clientX:Math.round(tx),clientY:Math.round(b.y+b.h/2),button:0,buttons:0,pointerId:id||11})); await sleep(200); };
`;
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

async function evalSafe(M, expr, tries = 8) {
  let last;
  for (let i = 0; i < tries; i++) {
    try { return await M.evalJs(expr); }
    catch (e) {
      last = e;
      if (/context was destroyed|Target closed|Cannot find context|Execution context/i.test(String(e))) { await sleep(1000); continue; }
      throw e;
    }
  }
  throw last;
}

async function ensureReady(M, label) {
  for (let i = 0; i < 300; i++) {
    try { const r = await M.evalJs(`({'tauri':!!window.__TAURI_INTERNALS__,url:location.href})`); if (r && r.tauri) return r; } catch { /* wait */ }
    await sleep(400);
  }
  throw new Error("not ready: " + label);
}

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: diagLog });
  log("launched pid " + app.child.pid);
  let M = null;
  for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  await ensureReady(M, "boot");

  const addBook = async (name) => {
    const p = path.join(BOOKS, name).replace(/\\/g, "\\\\");
    return M.evalJs(H(`try { return await inv('add_dropped_book',{path:"${p}"}); } catch(e){ return {err:String(e)}; }`));
  };
  out.steps.zhen = await addBook("zhen_dian.epub");
  log("added zhen_dian");
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close();
  M = await connectTarget(PORT, isApp);
  await ensureReady(M, "after add");

  const openZhen = await M.evalJs(H(`
    const c=await cards();
    const b=c.find(x=>x.textContent.includes('zhen_dian'))||c[0];
    b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',300);
    await sleep(1500);
    const id=${out.steps.zhen?.id ?? 0};
    const o=await inv('open_book',{id});
    return { id, isManga:o.is_manga, title:o.book?.title, toc:o.toc?.length, images:o.images?.length };
  `));
  out.steps.zhenOpen = openZhen;
  log("zhen open: " + JSON.stringify(openZhen));

  let bookId = openZhen.id;
  let isManga = openZhen.isManga;
  let activeTitle = openZhen.title || "zhen_dian";

  if (!isManga) {
    out.steps.heidao = await addBook("heidao_qianjin.epub");
    log("added heidao");
    await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
    await sleep(3500);
    M.close();
    M = await connectTarget(PORT, isApp);
    await ensureReady(M, "after add2");
    const openH = await M.evalJs(H(`
      const c=await cards(400);
      const b=c.find(x=>x.textContent.includes('heidao'))||c[0];
      b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
      await wait(()=>q('#chapter-content'),'chapter',600);
      await sleep(2500);
      const id=${out.steps.heidao?.id ?? 0};
      const o=await inv('open_book',{id});
      return { id, isManga:o.is_manga, title:o.book?.title, toc:o.toc?.length, images:o.images?.length };
    `));
    out.steps.heidaoOpen = openH;
    log("heidao open: " + JSON.stringify(openH));
    if (openH.isManga) { bookId = openH.id; isManga = true; activeTitle = openH.title || activeTitle; }
  }

  // ---- 1) first-run guide: global once + live preview ----
  if (isManga) {
    out.steps.guide = await M.evalJs(H(`
      const g=await wait(()=>q('.manga-guide'),'guide',80).catch(()=>null);
      const doneBefore=!!cfgRead().reader?.mangaGuideDone;
      const before=rdVar();
      const slider=q('.manga-guide input[type=range]');
      if(slider){ slider.value='1.7'; slider.dispatchEvent(new Event('input',{bubbles:true})); await sleep(500); }
      const after=rdVar();
      const liveCfg=cfgRead().reader?.manga?.imgScale;
      qa('.manga-guide .tb-menu-item').find(b=>b.textContent.includes('完成'))?.click();
      await sleep(500);
      return { appeared:!!g, doneBefore, scaleBefore:before, scaleAfter:after, liveCfg, doneAfter:!!cfgRead().reader?.mangaGuideDone, guideGone:!q('.manga-guide') };
    `));
    log("guide: " + JSON.stringify(out.steps.guide));
    out.steps.guideReopen = await M.evalJs(H(`
      for(let k=0;k<12 && !q('.library');k++){ q('.tb-l button.icon')?.click(); await sleep(700); }
      if(!q('.library')) throw new Error('cannot return to library');
      await sleep(700);
      const c=await cards(300);
      const b=c.find(x=>x.textContent.includes("${activeTitle.slice(0, 8)}"))||c[0];
      b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
      await wait(()=>q('#chapter-content'),'chapter',700); await sleep(1500);
      return { guideAgain: !!q('.manga-guide') };
    `));
    log("guideReopen: " + JSON.stringify(out.steps.guideReopen));
  } else {
    out.steps.guide = { skipped: true, reason: "no auto-detected manga book in sandbox" };
  }

  // ---- 2) topbar manga button toggle + per-book persistence ----
  out.steps.mangaBtn = await M.evalJs(H(`
    const wasManga=q('.reader').classList.contains('manga');
    let b=tbBtn('manga');
    if(!b) return { err:'no manga button', btns: qa('.tb-r button').map(x=>x.getAttribute('title')) };
    b.click(); await sleep(700);
    const nowManga=q('.reader').classList.contains('manga');
    const bc=bcRead(); const id=Object.keys(bc).find(k=>bc[k].mangaMode!==undefined);
    const persisted=id?bc[id].mangaMode:undefined;
    b=tbBtn('manga'); b.click(); await sleep(700);
    const backManga=q('.reader').classList.contains('manga');
    return { wasManga, toggled:nowManga, persisted, backManga };
  `));
  log("mangaBtn: " + JSON.stringify(out.steps.mangaBtn));

  // ---- 3) dedicated Manga settings tab + double-page horizontal ----
  out.steps.settings = await M.evalJs(H(`
    if(!q('.reader')?.classList.contains('manga')){ tbBtn('manga')?.click(); await sleep(700); }
    const flowDom = { viewportBefore: !!q('.manga-h-viewport'), scrollBefore: !!q('.scroll-area') };
    await openSettings('manga');
    const labels=[...document.querySelectorAll('.field label')].map(l=>l.textContent.trim());
    const has=(t)=>labels.some(x=>x.includes(t));
    const dirSel0=field('阅读方向')?.querySelector('select');
    const res={ fields:{ display:has('显示方式'), dir:has('阅读方向'), scale:has('图片大小'), marginH:has('左右边距'), marginV:has('上下边距'), gap:has('图片间距') },
      flowDom, dirDisabledOnFlow: !!dirSel0?.disabled };
    await setSelect('显示方式','horizontal');
    res.dirEnabledOnHorizontal = !field('阅读方向')?.querySelector('select')?.disabled;
    await setSelect('阅读方向','rtl');
    await setInput('图片间距','0');
    await sleep(700);
    res.saved={ layout:cfgRead().reader?.manga?.layout, dir:cfgRead().reader?.manga?.direction, gap:cfgRead().reader?.manga?.imgGap };
    await closeSettings(); await sleep(1200);
    const vp=q('.manga-h-viewport'); const cc=q('.manga-h-canvas #chapter-content');
    const cs=cc?getComputedStyle(cc):null;
    res.dom={ viewport:!!vp, flex:cs?.display, dir:cs?.display==='flex'?cs.flexDirection:null, gap:cs?.columnGap, rtlClass:cc?.classList.contains('rtl')||false,
      directImgs: cc ? [...cc.children].filter(x=>x.tagName==='IMG').length : 0 };
    res.geometry = vp&&cc ? { stripW:Math.round(cc.getBoundingClientRect().width), vpW:Math.round(vp.getBoundingClientRect().width), overflowX: Math.round(cc.getBoundingClientRect().width) > Math.round(vp.getBoundingClientRect().width) } : null;
    return res;
  `));
  log("settings: " + JSON.stringify(out.steps.settings));

  // ---- 4) drag panning on a multi-image chapter (screen fixed) ----
  out.steps.pan = await M.evalJs(H(`
    const id=${bookId};
    let jumped=null;
    for(let i=1;i<40;i++){ try{ const c=await inv('get_chapter',{bookId:id,chapterIndex:i,keepStyles:false}); const n=(c.html.match(/<img/gi)||[]).length; if(n>=2){ jumped=i; break; } }catch(e){} }
    if(jumped!=null){ window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:jumped})); await sleep(3000); }
    const vp=q('.manga-h-viewport'); const cv=q('.manga-h-canvas');
    if(!vp||!cv) return { err:'no viewport', jumped };
    const cc=q('.manga-h-canvas #chapter-content');
    const overflow = cc ? Math.round(cc.getBoundingClientRect().width) > Math.round(vp.getBoundingClientRect().width) : null;
    const tf0=cv.style.transform||'none';
    const r=vp.getBoundingClientRect(); const cx=r.left+r.width/2, cy=r.top+r.height/2;
    ptr(vp,'pointerdown',cx,cy,21); await sleep(40);
    ptr(vp,'pointermove',cx+8,cy,21); await sleep(40);
    ptr(vp,'pointermove',cx-200,cy,21); await sleep(150);
    const tfDrag=cv.style.transform;
    ptr(vp,'pointerup',cx-200,cy,21);
    await sleep(250);
    const viewerOpened=!!q('.iv-backdrop');
    const tfDragEnd=cv.style.transform;
    vp.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,deltaY:500,deltaMode:0}));
    await sleep(250);
    const tfWheel=cv.style.transform;
    return { jumped, imgs: qa('.manga-h-canvas #chapter-content img').length, overflow, tf0, tfDrag, moved: tfDrag!==tf0, viewerOpened, wheelMoved: tfWheel!==tfDragEnd };
  `));
  log("pan: " + JSON.stringify(out.steps.pan));

  // ---- 5) real topbar drag reorder + no misclick + persisted ----
  out.steps.drag = await M.evalJs(H(`
    const before=cfgRead().reader.topbarOrder.slice();
    const toc=tbBtn('toc'); const set=tbBtn('settings');
    if(!toc||!set) return { err:'missing buttons' };
    await dragTo(toc,set,'after',31);
    await sleep(800);
    const after=cfgRead().reader.topbarOrder.slice();
    return { before, after, changed: JSON.stringify(before)!==JSON.stringify(after),
      tocAfterSettings: after.indexOf('toc')>after.indexOf('settings'),
      settingsOpened: !!q('dialog[open]'), dragClasses: qa('.fr-drag-src,.fr-drop-before,.fr-drop-after').length };
  `));
  log("drag: " + JSON.stringify(out.steps.drag));

  out.steps.dragReload = await M.evalJs(`(()=>{ const o=JSON.parse(localStorage.getItem('tidepage.config.v1')).reader.topbarOrder; localStorage.setItem('fr.test.order', JSON.stringify(o)); setTimeout(()=>location.reload(),80); return o; })()`);
  await sleep(3500);
  M.close();
  M = await connectTarget(PORT, isApp);
  await ensureReady(M, "after reload");
  out.steps.dragPersisted = await M.evalJs(`(()=>{ const o=JSON.parse(localStorage.getItem('tidepage.config.v1')).reader.topbarOrder; const want=JSON.parse(localStorage.getItem('fr.test.order')||'[]'); return { equal: JSON.stringify(o)===JSON.stringify(want) }; })()`);
  log("dragPersisted: " + JSON.stringify(out.steps.dragPersisted));

  // ---- 6) right-bottom image bump goes beyond global manga cap ----
  await sleep(800);
  out.steps.bump = await evalSafe(M, H(`
    const c=await cards(200);
    const b=c.find(x=>x.textContent.includes("${activeTitle.slice(0, 8)}"))||c[0];
    b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',400); await sleep(1500);
    // 通过设置把全局漫画倍率设为上限 3.0（真实 UI 路径，写入响应式 config）
    if(!q('.reader')?.classList.contains('manga')){ tbBtn('manga')?.click(); await sleep(700); }
    await openSettings('manga');
    await setInput('图片大小','3');
    await sleep(500);
    await closeSettings(); await sleep(700);
    const g=cfgRead().reader?.manga?.imgScale;
    const body=q('.body'); const r=body.getBoundingClientRect();
    body.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:r.right-20,clientY:r.bottom-20}));
    await sleep(500);
    const plus=qa('.font-ctl .icon').find(x=>(x.getAttribute('title')||'').includes('放大图片'));
    if(!plus) return { err:'no plus', ctl:!!q('.font-ctl'), titles:qa('.font-ctl .icon').map(x=>x.getAttribute('title')) };
    const pr=plus.getBoundingClientRect();
    ptr(plus,'pointerdown',pr.left+pr.width/2,pr.top+pr.height/2,41); await sleep(120);
    ptr(plus,'pointerup',pr.left+pr.width/2,pr.top+pr.height/2,41); await sleep(600);
    const bc=bcRead();
    const entry=Object.entries(bc).find(([k,v])=>v.imgScale!==undefined);
    return { setGlobalViaUI: g, globalScale: cfgRead().reader.manga.imgScale, overrideScale: entry?entry[1].imgScale:undefined, bookId: entry?entry[0]:null, scaleVar: rdVar() };
  `));
  log("bump: " + JSON.stringify(out.steps.bump));

  // ---- 7) library topbar real drag reorder + no misclick + persisted ----
  out.steps.libDrag = await evalSafe(M, H(`
    for(let k=0;k<12 && !q('.library');k++){ q('.tb-l button.icon')?.click(); await sleep(700); }
    if(!q('.library')) return { err:'not in library' };
    await sleep(600);
    const grp=q('.lib-tb-group'); const btns=grp?[...grp.querySelectorAll('button[data-drag-id]')]:[];
    if(btns.length<2) return { err:'no lib buttons', n:btns.length };
    const before=cfgRead().library.topbarOrder.slice();
    const from=btns[0], to=btns[btns.length-1];
    await dragTo(from,to,'after',51);
    await sleep(800);
    const after=cfgRead().library.topbarOrder.slice();
    const firstId=from.getAttribute('data-drag-id'), lastId=to.getAttribute('data-drag-id');
    return { before, after, changed: JSON.stringify(before)!==JSON.stringify(after),
      firstMovedAfterLast: after.indexOf(firstId)>after.indexOf(lastId),
      drawerOpened: !!q('.wd-drawer'), dragClasses: qa('.fr-drag-src,.fr-drop-before,.fr-drop-after').length };
  `));
  log("libDrag: " + JSON.stringify(out.steps.libDrag));

  M.close();
} catch (e) {
    process.exitCode = 1; /* 失败必须体现在退出码上 */
  out.error = String(e);
  console.error(e);
} finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
