// Real-app UI reproduction/verification harness (isolated sandbox, real WebView2).
// Usage: node _test/ui_repro.mjs <scenario> [bookMatch]
// Scenarios: viewer | topbar | library | manga | fonthold
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const isApp = (t) => t.type === "page" && (t.url.includes("localhost:1420") || t.url.includes("tauri.localhost"));
const PORT = Number(process.env.FR_PORT || 9566);
const SCENARIO = process.argv[2] || "viewer";
const BOOK_MATCH = process.argv[3] || "zhen_dian";
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const sbx = path.join(SANDBOX_ROOT, "ui_sbx");
const data = path.join(SANDBOX_ROOT, "ui_data");
const booksDir = path.join(SANDBOX_ROOT, "scroll_books");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
if (process.env.FR_KEEP_DATA !== "1") {
  for (const f of fs.readdirSync(data)) fs.rmSync(path.join(data, f), { recursive: true, force: true });
  for (const f of fs.readdirSync(sbx)) fs.rmSync(path.join(sbx, f), { recursive: true, force: true });
}
const outFile = path.join(SANDBOX_ROOT, `ui_out_${SCENARIO}.json`);
const diagLog = path.join(SANDBOX_ROOT, `ui_diag_${SCENARIO}.log`);

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=200)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const rect=(el)=>{ if(!el) return null; const r=el.getBoundingClientRect(); return {x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}; };
`;
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;

async function main() {
  const out = { scenario: SCENARIO, book: BOOK_MATCH };
  let app = null;
  try {
    if (process.env.FR_EXE) {
      const fd = fs.openSync(diagLog, "a");
      const child = spawn(process.env.FR_EXE, [], {
        env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data, FR_TEST_CDP_PORT: String(PORT), TIDEPAGE_OPEN_DIAG: "1" },
        stdio: ["ignore", fd, fd], windowsHide: true,
      });
      app = { child };
    } else {
      app = launchSandbox({
        env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data, TIDEPAGE_OPEN_DIAG: "1" },
        cdpPort: PORT, logFile: diagLog,
      });
    }
    log("launched pid " + app.child.pid);
    let main = null;
    for (let i = 0; i < 240; i++) { try { main = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
    if (!main) throw new Error("no cdp");
    for (let i = 0; i < 300; i++) {
      try { const r = await main.evalJs(`({'url':location.href,'tauri':!!window.__TAURI_INTERNALS__})`); if (r && r.tauri) { log("ready " + JSON.stringify(r)); break; } } catch { /* wait */ }
      await sleep(500);
    }

    const names = fs.readdirSync(booksDir).filter((f) => f.endsWith(".epub") && f.includes(BOOK_MATCH));
    const added = [];
    for (const n of names) {
      const p = path.join(booksDir, n).replace(/\\/g, "\\\\");
      try { const r = await main.evalJs(H(`return await inv('add_dropped_book',{path:"${p}"});`)); added.push({ n, id: r?.id }); }
      catch (e) { added.push({ n, err: String(e) }); }
    }
    out.added = added;
    log("added " + JSON.stringify(added));

    // seed config
    await main.evalJs(H(`
      const raw=localStorage.getItem('tidepage.config.v1');
      const cfg=raw?JSON.parse(raw):{};
      cfg.reader=Object.assign({},cfg.reader||{},{mode:'${process.env.FR_MODE || "scroll"}'});
      localStorage.setItem('tidepage.config.v1',JSON.stringify(cfg));
      return 1;`));
    await main.evalJs(`setTimeout(()=>location.reload(),100);1`);
    await sleep(4000);
    main.close();
    main = await connectTarget(PORT, isApp);
    for (let i = 0; i < 300; i++) { try { const r = await main.evalJs(`({'tauri':!!window.__TAURI_INTERNALS__})`); if (r && r.tauri) break; } catch { /* retry */ } await sleep(400); }

    if (SCENARIO !== "libprogress") {
      await main.evalJs(H(`
        const cards=await wait(()=>{const c=qa('.card,.row');return c.length?c:null;},'cards');
        const b=cards.find(x=>x.textContent.includes(${JSON.stringify(BOOK_MATCH)}))||cards[0];
        b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
        await wait(()=>q('#chapter-content'),'chapter');
        await sleep(1000); return 1;`));
      out.opened = await main.evalJs(H(`return {title:q('.btitle')?.textContent||'', hasTopbar:!!q('.topbar'), mangaGuide:!!q('.manga-guide')};`));
    }

    if (SCENARIO === "viewer") {
      // jump to a chapter with an image
      await main.evalJs(H(`
        const id=${(added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id};
        for(let i=0;i<80;i++){ try{ const c=await inv('get_chapter',{bookId:id,chapterIndex:i,keepStyles:false}); if(/<img/i.test(c.html)){ window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:i})); return {i}; } }catch(e){} }
        return null;`));
      await sleep(2500);
      // open viewer by pointerdown+pointerup on first img
      out.openRes = await main.evalJs(H(`
        const img=q('#chapter-content img'); if(!img) return {err:'no img'};
        const r=img.getBoundingClientRect(); const x=r.left+r.width/2, y=r.top+r.height/2;
        img.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:x,clientY:y,button:0,pointerId:1}));
        img.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:x,clientY:y,button:0,pointerId:1}));
        await sleep(600);
        return { viewer: !!q('.iv-backdrop'), img: rect(img) };`));
      out.navBefore = await main.evalJs(H(`return { left: rect(q('.iv-nav.left')), right: rect(q('.iv-nav.right')), stage: rect(q('.iv-stage')), img: rect(q('.iv-stage img')), idx: q('.iv-meta')?.textContent };`));
      // trusted press on right nav (to trigger :active), measure while pressed
      const nr = await main.evalJs(`(()=>{const r=document.querySelector('.iv-nav.right').getBoundingClientRect();return {x:Math.round(r.left+r.width/2),y:Math.round(r.top+r.height/2)};})()`);
      await main.call("Input.dispatchMouseEvent", { type: "mousePressed", x: nr.x, y: nr.y, button: "left", clickCount: 1 });
      await sleep(120);
      out.navDuringPress = await main.evalJs(H(`return { left: rect(q('.iv-nav.left')), right: rect(q('.iv-nav.right')) };`));
      await main.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: nr.x, y: nr.y, button: "left", clickCount: 1 });
      await sleep(700);
      out.navAfterRight = await main.evalJs(H(`return { left: rect(q('.iv-nav.left')), right: rect(q('.iv-nav.right')), stage: rect(q('.iv-stage')), img: rect(q('.iv-stage img')), idx: q('.iv-meta')?.textContent };`));
      await main.evalJs(H(`q('.iv-nav.left')?.dispatchEvent(new MouseEvent('click',{bubbles:true})); await sleep(700); return 1;`));
      out.navAfterLeft = await main.evalJs(H(`return { left: rect(q('.iv-nav.left')), right: rect(q('.iv-nav.right')), idx: q('.iv-meta')?.textContent };`));
      // click above the image inside stage
      out.clickAbove = await main.evalJs(H(`
        const st=q('.iv-stage'); const img=q('.iv-stage img');
        const sr=st.getBoundingClientRect(), ir=img?img.getBoundingClientRect():null;
        const x=sr.left+sr.width/2; const y=ir? Math.max(sr.top+2, ir.top-12) : sr.top+10;
        const target=document.elementFromPoint(x,y);
        (target||st).dispatchEvent(new MouseEvent('click',{bubbles:true,clientX:x,clientY:y}));
        await sleep(400);
        return { y:Math.round(y), target: target? target.className||target.tagName : 'none', stillOpen: !!q('.iv-backdrop') };`));
      // close via x
      out.closeXhit = await main.evalJs(H(`
        const b=q('.iv-stage');
        if(!b) return {found:false};
        const r=b.getBoundingClientRect();
        const el=document.elementFromPoint(r.left+r.width/2, r.top+r.height/2);
        return { found:true, rect:{x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}, hit: el? (el===b || b.contains(el)) : false, hitCls: el?el.className:'' };`));
      out.closeX = await main.evalJs(H(`const b=q('.iv-stage'); b?.dispatchEvent(new MouseEvent('click',{bubbles:true})); await sleep(300); return { closed: !q('.iv-backdrop') };`));
      // reopen viewer then press Esc: should close viewer only, stay in reader (not return to library)
      out.reopen = await main.evalJs(H(`
        const img=q('#chapter-content img'); const r=img.getBoundingClientRect(); const x=r.left+r.width/2,y=r.top+r.height/2;
        img.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:x,clientY:y,button:0,pointerId:21}));
        img.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:x,clientY:y,button:0,pointerId:21}));
        await sleep(500); return { open:!!q('.iv-backdrop') };`));
      out.esc = await main.evalJs(H(`
        window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
        await sleep(400);
        return { viewerClosed: !q('.iv-backdrop'), inReader: !!q('#chapter-content'), inLibrary: !!q('.library') };`));
    } else if (SCENARIO === "topbar") {
      out.topbar = await main.evalJs(H(`
        return { buttons: qa('.tb-r .icon').map(b=>({title:b.getAttribute('title'), text:b.textContent.trim()})),
          hasManager: !!q('[data-tb="manager"]'), hasImg: !!q('[data-tb="images"]'), hasCss: !!q('[data-tb="css"]') };`));
    } else if (SCENARIO === "library") {
      out.lib = await main.evalJs(H(`
        return { toolbar: qa('.toolbar button').map(b=>({title:b.getAttribute('title'), text:b.textContent.trim()})),
          statusbar: qa('.statusbar button').map(b=>({title:b.getAttribute('title')})) };`));
    } else if (SCENARIO === "flashline") {
      await main.evalJs(H(`window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:20})); return 1;`));
      await sleep(2000);
      // in-page strip analyzer: returns per-row average brightness
      await main.evalJs(H(`
        window.__analyze=async(b64)=>{ const img=new Image(); img.src='data:image/png;base64,'+b64; await img.decode();
          const c=document.createElement('canvas'); c.width=img.width; c.height=img.height; const ctx=c.getContext('2d'); ctx.drawImage(img,0,0);
          const rows=[]; for(let y=0;y<img.height;y++){ const d=ctx.getImageData(0,y,img.width,1).data; let s=0; for(let i=0;i<d.length;i+=4) s+=(d[i]+d[i+1]+d[i+2])/3; rows.push(+(s/(d.length/4)).toFixed(2)); }
          return rows; };
        return 1;`));
      out.rects = await main.evalJs(H(`
        const g=(s)=>{const e=q(s); if(!e) return null; const r=e.getBoundingClientRect(); return {top:+r.top.toFixed(2),bottom:+r.bottom.toFixed(2),h:+r.height.toFixed(2)};};
        return { topbar:g('.topbar'), body:g('.body'), content:g('.content'), scrollArea:g('.scroll-area'),
          border:getComputedStyle(q('.topbar')).borderBottomWidth, dpr:window.devicePixelRatio };`));
      const tb = await main.evalJs(H(`const r=q('.topbar').getBoundingClientRect(); return {bottom:Math.round(r.bottom), vw:Math.round(window.innerWidth), vh:Math.round(window.innerHeight)};`));
      out.topbar = tb;
      const strips = [];
      for (let i = 0; i < 70; i++) {
        await main.call("Input.dispatchMouseEvent", { type: "mouseWheel", x: Math.round(tb.vw / 2), y: Math.round(tb.vh / 2), deltaX: 0, deltaY: Number(process.env.FR_DY || 260) });
        await sleep(12);
        try {
          const r = await main.call("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width: tb.vw, height: 90, scale: 1 } });
          const rows = await main.evalJs(`window.__analyze(${JSON.stringify(r.data)})`);
          strips.push(rows);
        } catch (e) { strips.push(["err:" + String(e).slice(0, 40)]); }
      }
      out.strips = strips;
    } else if (SCENARIO === "features") {
      // jump to a chapter with an image
      await main.evalJs(H(`
        const id=${(added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id};
        for(let i=0;i<80;i++){ try{ const c=await inv('get_chapter',{bookId:id,chapterIndex:i,keepStyles:false}); if(/<img/i.test(c.html)){ window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:i})); return {i}; } }catch(e){} }
        return null;`));
      await sleep(2500);
      out.f = await main.evalJs(H(`
        const out={};
        const btnById=(id)=>document.querySelector('[data-testid="tb-'+id+'"]');
        const cfg=()=>JSON.parse(localStorage.getItem('tidepage.config.v1'));
        const bcfg=()=>JSON.parse(localStorage.getItem('tidepage.bookcfg.v1')||'{}');
        const bid=String(${(added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id});
        // --- manager ---
        btnById('manager')?.click(); await sleep(250);
        out.managerOpen=!!q('.tb-panel'); out.managerRows=qa('.tb-panel .tb-row').length;
        const aiRow=qa('.tb-panel .tb-row').find(r=>r.textContent.includes('AI 助手'));
        const cb=aiRow?.querySelector('input[type=checkbox]');
        cb.checked=false; cb.dispatchEvent(new Event('change',{bubbles:true})); await sleep(250);
        out.aiHidden=!qa('.tb-r .icon').some(b=>b.textContent.trim()==='AI');
        await sleep(500);
        out.hiddenPersist=cfg().reader.topbarHidden;
        cb.checked=true; cb.dispatchEvent(new Event('change',{bubbles:true})); await sleep(250);
        out.aiBack=qa('.tb-r .icon').some(b=>b.textContent.trim()==='AI');
        q('.tb-panel-head button.icon')?.click(); await sleep(150);
        // --- image menu: 全部关闭 ---
        btnById('images')?.click(); await sleep(250);
        out.imgMenuItems=qa('.tb-panel .tb-menu-item').map(x=>x.textContent.trim());
        qa('.tb-panel .tb-menu-item').find(x=>x.textContent.includes('全部关闭'))?.click(); await sleep(400);
        out.noImages=q('.reader')?.classList.contains('no-images');
        const im=q('#chapter-content img'); out.imgDisplay=im?getComputedStyle(im).display:'noimg';
        out.showImagesOv=bcfg()[bid]?.showImages;
        // 菜单仍开着，文案已变为「全部开启图片」，直接点它
        qa('.tb-panel .tb-menu-item').find(x=>x.textContent.includes('全部开启'))?.click(); await sleep(300);
        out.noImages2=q('.reader')?.classList.contains('no-images');
        await sleep(400); out.hiddenPersist2=cfg().reader.topbarHidden;
        // --- block pick (menu still open after 全部开启) ---
        qa('.tb-panel .tb-menu-item').find(x=>x.textContent.includes('屏蔽选中图片'))?.click(); await sleep(250);
        const im2=q('#chapter-content img');
        if(im2){ const rr=im2.getBoundingClientRect(); const x=rr.left+rr.width/2, y=rr.top+rr.height/2;
          im2.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:x,clientY:y,button:0,pointerId:9}));
          im2.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:x,clientY:y,button:0,pointerId:9}));
          await sleep(400);
        }
        out.blockedCount=(bcfg()[bid]?.blockedImages||[]).length;
        out.blockedHidden=im2?getComputedStyle(im2).display:'noimg';
        btnById('images')?.click(); await sleep(200);
        qa('.tb-panel .tb-menu-item').find(x=>x.textContent.includes('已屏蔽图片'))?.click(); await sleep(300);
        out.blockedListRows=qa('.tb-panel .tb-block-row').length;
        qa('.tb-panel .tb-block-row .tb-menu-item')[0]?.click(); await sleep(300);
        out.blockedAfterUnblock=(bcfg()[bid]?.blockedImages||[]).length;
        q('.tb-panel-head button.icon')?.click(); await sleep(150);
        // --- css toggle ---
        const ovr0=q('.reader').classList.contains('ovr');
        btnById('css')?.click(); await sleep(700);
        out.ovr=[ovr0, q('.reader').classList.contains('ovr')];
        out.ovrOv=bcfg()[bid]?.overrideBookCss;
        // --- drag reorder: AI before 添加书签 ---
        try{
          const from=qa('.tb-r .icon').find(b=>b.textContent.trim()==='AI');
          const to=btnById('bookmarkAdd');
          const dt=new DataTransfer();
          from.dispatchEvent(new DragEvent('dragstart',{bubbles:true,dataTransfer:dt}));
          to.dispatchEvent(new DragEvent('dragover',{bubbles:true,cancelable:true,dataTransfer:dt}));
          to.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:dt}));
          from.dispatchEvent(new DragEvent('dragend',{bubbles:true,dataTransfer:dt}));
          await sleep(300);
        }catch(e){ out.dragErr=String(e); }
        out.order=cfg().reader.topbarOrder.slice(0,6);
        out.orderPersist=!!localStorage.getItem('tidepage.config.v1');
        // --- long-press font (per-book) ---
        const body=q('.body'); const br=body.getBoundingClientRect();
        body.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:br.right-20,clientY:br.bottom-20})); await sleep(300);
        const plus=qa('.font-ctl .icon').find(b=>(b.getAttribute('title')||'').includes('增大字号'));
        out.fontCtl=qa('.font-ctl .icon').map(b=>b.getAttribute('title'));
        const fs0=bcfg()[bid]?.fontSize;
        const pr=plus.getBoundingClientRect();
        plus.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:pr.left+5,clientY:pr.top+5,button:0,pointerId:7}));
        await sleep(1400);
        plus.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:pr.left+5,clientY:pr.top+5,button:0,pointerId:7}));
        await sleep(200);
        out.fontSize={before:fs0, after:bcfg()[bid]?.fontSize};
        // --- image zoom + fit (per-book) ---
        const zin=qa('.font-ctl .icon').find(b=>(b.getAttribute('title')||'').includes('放大图片'));
        const fit=qa('.font-ctl .icon').find(b=>(b.getAttribute('title')||'').includes('适应阅读器宽度'));
        const zr=zin.getBoundingClientRect();
        zin.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:zr.left+5,clientY:zr.top+5,button:0,pointerId:8}));
        await sleep(700);
        zin.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:zr.left+5,clientY:zr.top+5,button:0,pointerId:8}));
        await sleep(200);
        out.imgScale=bcfg()[bid]?.imgScale;
        fit.click(); await sleep(300);
        out.fitWidth=bcfg()[bid]?.fitWidth; out.fitwClass=q('.reader')?.classList.contains('fitw');
        return out;`));
    } else if (SCENARIO === "imgzoom") {
      await main.evalJs(H(`
        const id=${(added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id};
        let pick=-1;
        for(let i=0;i<120;i++){ try{ const c=await inv('get_chapter',{bookId:id,chapterIndex:i,keepStyles:false}); if(/<img[^>]*width="[4-9]\d\d"/i.test(c.html)){ pick=i; break; } if(pick<0 && /<img/i.test(c.html)) pick=i; }catch(e){} }
        if(pick>=0) window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:pick}));
        return {pick};`));
      await sleep(2500);
      out.z = await main.evalJs(H(`
        const rd=q('.reader');
        const big=()=>{ const all=qa('#chapter-content img'); let b=null,bn=null; for(const im of all){ const r=im.getBoundingClientRect(); if(!b||r.width*r.height>b.width*b.height){b=r;bn=im;} } return b?{w:Math.round(b.width),h:Math.round(b.height),nw:bn.naturalWidth,nh:bn.naturalHeight,cls:bn.className}:null; };
        const info=()=>{ const im=q('#chapter-content img'); const cs=im?getComputedStyle(im):null;
          return { scale:getComputedStyle(rd).getPropertyValue('--rdr-img-scale').trim(), ovr:rd.classList.contains('ovr'), fitw:rd.classList.contains('fitw'), imgw:rd.classList.contains('imgw'),
            count:qa('#chapter-content img').length, big:big(),
            img: im?{w:Math.round(im.getBoundingClientRect().width),h:Math.round(im.getBoundingClientRect().height),maxW:cs.maxWidth,maxH:cs.maxHeight,nw:im.naturalWidth,nh:im.naturalHeight}:null }; };
        const before=info();
        // reveal controls
        const body=q('.body'); const br=body.getBoundingClientRect();
        body.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:br.right-20,clientY:br.bottom-20})); await sleep(300);
        const plus=qa('.font-ctl .icon').find(b=>(b.getAttribute('title')||'').includes('放大图片'));
        for(let k=0;k<3;k++){ const r=plus.getBoundingClientRect(); plus.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:r.left+5,clientY:r.top+5,button:0,pointerId:11})); await sleep(60); plus.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:r.left+5,clientY:r.top+5,button:0,pointerId:11})); await sleep(120); }
        await sleep(400);
        const afterZoom=info();
        const fit=qa('.font-ctl .icon').find(b=>(b.getAttribute('title')||'').includes('适应阅读器宽度'));
        fit.click(); await sleep(500);
        const afterFit=info();
        return {before,afterZoom,afterFit};`));
      out.scaleCss = await main.evalJs(H(`return getComputedStyle(q('.reader')).getPropertyValue('--rdr-img-scale').trim();`));
    } else if (SCENARIO === "libprogress") {
      // register a big folder first so the refresh scan is observable
      out.bigFolder = await main.evalJs(H(`
        const dir=${JSON.stringify(path.join(SANDBOX_ROOT, "ui_biglib")).replace(/\\/g, "\\\\")};
        try{ await inv('add_folder',{path:dir}); }catch(e){ return {err:String(e)}; }
        // wait for the initial recursive scan to settle
        for(let i=0;i<120;i++){ await sleep(500); const c=document.querySelectorAll('.card,.row').length; const t=(q('.statusbar span')?.textContent||''); if(/[0-9]+ 本书/.test(t) && !/^0 /.test(t)) break; }
        await sleep(1500);
        return {total:(q('.statusbar span')?.textContent||'')};`));
      log("bigFolder " + JSON.stringify(out.bigFolder));
      // click refresh and sample the statusbar progress bar while it runs
      await main.evalJs(H(`
        const btn=q('[data-testid="lib-btn-refresh"]');
        btn.click();
        window.__samples=[];
        for(let i=0;i<80;i++){ await sleep(50); const p=q('.scan-progress'); window.__samples.push({t:i*50, bar:!!p, text:p?p.textContent.trim():''}); }
        return 1;`));
      out.samples = await main.evalJs(`window.__samples`);
      out.libBtns = await main.evalJs(H(`return qa('.toolbar .lib-tb-btn').map(b=>b.getAttribute('title'));`));
      out.libDrag = await main.evalJs(H(`
        const btns=qa('.toolbar .lib-tb-btn');
        const from=document.querySelector('[data-testid="lib-btn-random"]');
        const to=btns.find(b=>(b.getAttribute('title')||'').includes('WebDAV'));
        try{ const dt=new DataTransfer();
          from.dispatchEvent(new DragEvent('dragstart',{bubbles:true,dataTransfer:dt}));
          to.dispatchEvent(new DragEvent('dragover',{bubbles:true,cancelable:true,dataTransfer:dt}));
          to.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:dt}));
          from.dispatchEvent(new DragEvent('dragend',{bubbles:true,dataTransfer:dt}));
        }catch(e){ return {err:String(e)}; }
        await sleep(500);
        return JSON.parse(localStorage.getItem('tidepage.config.v1')).library.topbarOrder;`));
    } else if (SCENARIO === "manga") {
      out.manga = await main.evalJs(H(`
        const id=${(added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id};
        const o=await inv('open_book',{id});
        return { title:o.book?.title, isManga:o.is_manga, toc:o.toc?.length, images:o.images?.length };`));
      out.chHtml = await main.evalJs(H(`
        const id=${(added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id};
        const c=await inv('get_chapter',{bookId:id,chapterIndex:20,keepStyles:false});
        const low=c.html.toLowerCase();
        return { len:c.html.length, img:(low.match(/<img/gi)||[]).length, image:(low.match(/<image/gi)||[]).length, svg:(low.match(/<svg/gi)||[]).length, head:c.html.slice(0,260) };`));
      // jump to a real manga page chapter (many page images)
      await main.evalJs(H(`window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:20})); return 1;`));
      await sleep(2500);
      out.guideApply = await main.evalJs(H(`
        const rd=q('.reader');
        const scale=()=>getComputedStyle(rd).getPropertyValue('--rdr-img-scale').trim();
        const bigSize=()=>{ const all=qa('#chapter-content img').map(x=>x.getBoundingClientRect()); let big=null; for(const r of all){ if(!big||r.width*r.height>big.width*big.height) big=r; } return big?{w:Math.round(big.width),h:Math.round(big.height)}:null; };
        // wait for the first page image to actually load (large zip → on-demand image reads can be slow)
        for(let k=0;k<40;k++){ const f=q('#chapter-content img'); if(f && f.complete && f.naturalWidth>0) break; await sleep(250); }
        const before=scale(); const bigBefore=bigSize();
        const slider=q('.manga-guide input[type=range]');
        if(slider){ slider.value='1.5'; slider.dispatchEvent(new Event('input',{bubbles:true})); await sleep(150); }
        qa('.manga-guide .tb-menu-item').find(b=>b.textContent.includes('应用'))?.click();
        await sleep(900);
        const after=scale(); const bigAfter=bigSize();
        const f=q('#chapter-content img'); const fr=f?f.getBoundingClientRect():null;
        return { before, after, bigBefore, bigAfter, imgCount:qa('#chapter-content img').length,
          contentH:q('#chapter-content')?.getBoundingClientRect().height,
          first: f?{w:Math.round(fr.width),h:Math.round(fr.height),nw:f.naturalWidth,nh:f.naturalHeight,disp:getComputedStyle(f).display,zoom:getComputedStyle(f).zoom,complete:f.complete,src:(f.getAttribute('src')||'').slice(0,90),attrW:f.getAttribute('width'),attrH:f.getAttribute('height')}:null,
          mangaImageScale:JSON.parse(localStorage.getItem('tidepage.config.v1')).reader.mangaImageScale };`));
    } else if (SCENARIO === "fonthold") {
      out.hold = await main.evalJs(H(`
        // ensure font ctl visible
        const body=q('.body'); const r=body.getBoundingClientRect();
        body.dispatchEvent(new MouseEvent('mousemove',{bubbles:true,clientX:r.right-20,clientY:r.bottom-20}));
        await sleep(300);
        const plus=qa('.font-ctl .icon').find(b=>b.getAttribute('title')?.includes('增大'));
        if(!plus) return {err:'no plus', ctl:!!q('.font-ctl')};
        const before=JSON.parse(localStorage.getItem('tidepage.config.v1')).reader.fontSize;
        const b=plus.getBoundingClientRect();
        plus.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,clientX:b.left+b.width/2,clientY:b.top+b.height/2,button:0,pointerId:2}));
        await sleep(1500);
        const during=JSON.parse(localStorage.getItem('tidepage.config.v1')).reader.fontSize;
        plus.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:b.left+b.width/2,clientY:b.top+b.height/2,button:0,pointerId:2}));
        await sleep(300);
        const after=JSON.parse(localStorage.getItem('tidepage.config.v1')).reader.fontSize;
        return { before, during, after, controls: qa('.font-ctl .icon').map(x=>x.getAttribute('title')) };`));
    }

    main.close();
  } finally {
    if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
    fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
    console.log("WROTE " + outFile);
  }
}
main().catch((e) => { console.error(e); process.exit(1); });

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
