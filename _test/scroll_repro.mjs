// Real-app reproduction/verification for scroll mode. Launches the real Tauri app
// (dev via tauri dev, or a release exe via FR_EXE) in an isolated sandbox, drives real
// scrolling through CDP, and samples per-frame scrollTop / visible text / chapter state
// to detect flicker (jumps), blank regions and toc-jump blanks.
// Data isolation: WEBVIEW2_USER_DATA_FOLDER + TIDEPAGE_DATA_DIR both point outside the repo;
// the user's real DB is never touched.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const isApp = (t) => t.type === "page" && (t.url.includes("localhost:1420") || t.url.includes("tauri.localhost"));

const PORT = Number(process.env.FR_PORT || 9555);
const SCENARIO = process.argv[2] || "scroll";
const BOOK_MATCH = process.argv[3] || "zhen_dian";
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const sbx = path.join(SANDBOX_ROOT, "scroll_sbx");
const data = path.join(SANDBOX_ROOT, "scroll_data");
const booksDir = path.join(SANDBOX_ROOT, "scroll_books");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
if (process.env.FR_KEEP_DATA !== "1") {
  for (const f of fs.readdirSync(data)) fs.rmSync(path.join(data, f), { recursive: true, force: true });
}
const outFile = path.join(SANDBOX_ROOT, `scroll_out_${SCENARIO}.json`);
const diagLog = path.join(SANDBOX_ROOT, `scroll_diag_${SCENARIO}.log`);

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=200)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const area=()=>document.querySelector('.scroll-area');
const content=()=>document.querySelector('#chapter-content');
const visText=()=>{ const a=area(); if(!a) return -1;
  const r=a.getBoundingClientRect(); let hit=0,total=0,samples=[];
  for(let iy=0.1; iy<=0.9; iy+=0.2){ for(let ix=0.15; ix<=0.9; ix+=0.25){
    total++;
    const x=r.left+r.width*ix, y=r.top+r.height*iy;
    let t='';
    const rg=document.caretRangeFromPoint?document.caretRangeFromPoint(x,y):null;
    if(rg&&rg.startContainer){ const n=rg.startContainer; t=(n.nodeValue||n.textContent||'').slice(0,200); }
    const clean=t.replace(/\\s+/g,'');
    if(clean.length>0){hit++;samples.push(clean.slice(0,12));}
  }}
  return {hit,total,samples};
};
const state=()=>{ const a=area(); const c=content();
  let cr=null; if(a&&c){const ar=a.getBoundingClientRect(), rr=c.getBoundingClientRect(); cr={top:Math.round(rr.top-ar.top), bottom:Math.round(rr.bottom-ar.top), h:Math.round(rr.height)};}
  return { st:a?a.scrollTop:-1, sh:a?a.scrollHeight:-1, ch:a?a.clientHeight:-1,
    contentLen:c?c.textContent.length:-1, contentRect:cr, vis:visText(),
    foot:(document.querySelector('.finfo-l')?.textContent||'')+'|'+(document.querySelector('.finfo-r')?.textContent||''),
    title:(document.querySelector('.btitle')?.textContent||'') };
};
`;

const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;

async function main() {
  const out = { scenario: SCENARIO, book: BOOK_MATCH, frames: [], steps: [] };
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
        cdpPort: PORT,
        logFile: diagLog,
      });
    }
    log("launched pid " + app.child.pid);
    let main = null;
    for (let i = 0; i < 240; i++) {
      try { main = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); }
    }
    if (!main) throw new Error("no cdp");
    let ready = false;
    for (let i = 0; i < 300; i++) {
      try {
        const r = await main.evalJs(`({'url':location.href,'tauri':!!window.__TAURI_INTERNALS__,'body':!!document.body})`);
        if (r && r.tauri) { ready = true; log("app ready: " + JSON.stringify(r)); break; }
        if (i % 10 === 0) log("waiting app: " + JSON.stringify(r));
      } catch { /* context not ready */ }
      await sleep(500);
    }
    if (!ready) throw new Error("app internals never ready");

    const names = fs.readdirSync(booksDir).filter((f) => f.endsWith(".epub") && f.includes(BOOK_MATCH));
    const added = [];
    for (const n of names) {
      const p = path.join(booksDir, n).replace(/\\/g, "\\\\");
      try {
        const r = await main.evalJs(H(`return await inv('add_dropped_book',{path:"${p}"});`));
        added.push({ n, id: r?.id, title: r?.title });
      } catch (e) { added.push({ n, err: String(e) }); }
    }
    out.added = added;
    log("added: " + JSON.stringify(added));

    // Wait for the background metadata scan (parse_with_chars, CPU heavy) to finish so it
    // does not starve get_chapter's blocking pool (test-only confound).
    await sleep(3000);
    out.metaWait = await main.evalJs(H(`
      for(let i=0;i<180;i++){ let v=null; try{ v=await inv('get_setting_kv',{key:'meta_busy'}); }catch(e){}
        if(!v) return {done:true, waited:i}; await sleep(1000); }
      return {done:false};`));
    log("meta wait: " + JSON.stringify(out.metaWait));

    if (SCENARIO === "probe") {
      const bookId = (added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id;
      const open = await main.evalJs(H(`return await inv('open_book',{id:${bookId}});`));
      out.open = {
        title: open?.book?.title, tocLen: open?.toc?.length,
        toc0: (open?.toc || []).slice(0, 6).map((t) => ({ title: t.title, index: t.index, chars: t.chars })),
        docs0: (open?.docs || []).slice(0, 6),
        images: open?.images?.length,
      };
      out.chapters = await main.evalJs(H(`
        const id=${bookId}; const o=[];
        for(let i=0;i<40;i++){ try{ const c=await inv('get_chapter',{bookId:id,chapterIndex:i,keepStyles:false}); o.push({i,title:c.title,len:c.html.length,chars:c.chars,head:c.html.slice(0,90)});}catch(e){o.push({i,err:String(e)});} }
        return o;`));
      log("probe done tocLen=" + out.open.tocLen);
      main.close();
      return;
    }

    await main.evalJs(H(`
      const raw = localStorage.getItem('tidepage.config.v1');
      const cfg = raw ? JSON.parse(raw) : {};
      cfg.reader = Object.assign({}, cfg.reader || {}, { mode: '${process.env.FR_MODE || "scroll"}' });
      localStorage.setItem('tidepage.config.v1', JSON.stringify(cfg));
      return 1;`));
    await main.evalJs(`setTimeout(()=>location.reload(),100); 1`);
    await sleep(4000);

    main.close();
    main = await connectTarget(PORT, isApp);
    for (let i = 0; i < 300; i++) {
      try { const r = await main.evalJs(`({'tauri':!!window.__TAURI_INTERNALS__})`); if (r && r.tauri) break; } catch { /* retry */ }
      await sleep(400);
    }

    await main.evalJs(H(`
      const cards = await wait(()=>{const c=[...document.querySelectorAll('.card,.row')];return c.length?c:null;},'cards');
      const b = cards.find(x=>x.textContent.includes(${JSON.stringify(BOOK_MATCH)})) || cards[0];
      b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
      await wait(()=>document.querySelector('#chapter-content'),'chapter');
      await sleep(800);
      return document.querySelector('.btitle')?.textContent;`));
    out.opened = await main.evalJs(H(`return { mode: (JSON.parse(localStorage.getItem('tidepage.config.v1')).reader||{}).mode, tocOpen: !!document.querySelector('.toc-item') };`));

    out.diag0 = await main.evalJs(H(`
      const c=content();
      const imgs=[...document.querySelectorAll('#chapter-content img')].map(i=>({src:(i.getAttribute('src')||'').slice(0,80),nw:i.naturalWidth,complete:i.complete,w:i.width,h:i.height}));
      const html=c?c.innerHTML:'';
      let seen=0, prevented=0;
      const rd=document.querySelector('.reader');
      const h=(e)=>{seen++; if(e.defaultPrevented) prevented++;};
      rd.addEventListener('wheel',h,true);
      for(let i=0;i<8;i++){ const a=area(); if(a) a.dispatchEvent(new WheelEvent('wheel',{deltaY:520,bubbles:true,cancelable:true})); await sleep(30); }
      await sleep(500);
      rd.removeEventListener('wheel',h,true);
      return { seen, prevented, htmlLen: html.length, htmlHead: html.slice(0,240), imgs,
        loading: document.querySelector('.loading')?.textContent||'', s: state() };`));

    if (SCENARIO === "scroll") {
      out.steps.push(await main.evalJs(H(`
        window.__frFrames=[]; window.__frStop=false;
        const DELTA=${Number(process.env.FR_DELTA || 120)};
        const INTERVAL=${Number(process.env.FR_INTERVAL || 40)};
        const TICKS=${Number(process.env.FR_TICKS || 800)};
        const rec=()=>{ if(window.__frStop){return;} const s=state();
          window.__frFrames.push({t:Math.round(performance.now()), st:s.st, sh:s.sh, ch:s.ch, len:s.contentLen, hit:s.vis&&s.vis.hit, vtot:s.vis&&s.vis.total, cr:s.contentRect});
          requestAnimationFrame(rec); };
        requestAnimationFrame(rec);
        const target=area();
        for(let i=0;i<TICKS;i++){
          target.dispatchEvent(new WheelEvent('wheel',{deltaY:DELTA,bubbles:true,cancelable:true}));
          await sleep(INTERVAL);
          if(i%100===0){ window.__frSteps=window.__frSteps||[]; window.__frSteps.push({i,...state()}); }
        }
        await sleep(600);
        window.__frStop=true;
        return { frames: window.__frFrames.length, steps: window.__frSteps||[] };
      `)));
      out.frames = await main.evalJs(`window.__frFrames`);
      {
        const bid = (added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id;
        out.ipc = await main.evalJs(H(`
          const t=(p,ms)=>Promise.race([p.then(v=>({ok:true,v})).catch(e=>({ok:false,e:String(e)})), new Promise(r=>setTimeout(()=>r({timeout:true}),ms))]);
          const r1=await t(inv('app_data_dir'),3000);
          const r2=await t(inv('get_chapter',{bookId:${bid},chapterIndex:33,keepStyles:false}),8000);
          const r3=await t(inv('get_chapter',{bookId:${bid},chapterIndex:40,keepStyles:false}),8000);
          return { appData:r1, ch33:(r2.ok?{len:r2.v.html.length}:(r2.timeout?'TIMEOUT':r2)), ch40:(r3.ok?{len:r3.v.html.length}:(r3.timeout?'TIMEOUT':r3)) };`));
      }
    } else if (SCENARIO === "flicker") {
      const bid = (added.find((a) => a.n.includes(BOOK_MATCH)) || added[0]).id;
      out.pickImgChapter = await main.evalJs(H(`
        const id=${bid};
        for(let i=6;i<60;i++){ try{ const c=await inv('get_chapter',{bookId:id,chapterIndex:i,keepStyles:false});
          const tags=c.html.match(/<img[^>]*>/gi)||[]; if(tags.length>0) return {i,imgs:tags.length,len:c.html.length,tags:tags.slice(0,4)}; }catch(e){} }
        return null;`));
      if (out.pickImgChapter) {
        await main.evalJs(`window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:${out.pickImgChapter.i}})); 1`);
        await sleep(2000);
      }
      await main.evalJs(H(`
        window.__cls=[]; window.__clsStart=performance.now();
        try{ new PerformanceObserver((l)=>{ for(const e of l.getEntries()){ if(!e.hadRecentInput) window.__cls.push({t:Math.round(e.startTime-window.__clsStart), v:+e.value.toFixed(4), src:(e.sources||[]).map(s=>s.node?.tagName||'?').join(',')}); } }).observe({type:'layout-shift',buffered:true}); }catch(e){ window.__clsErr=String(e); }
        window.__imgLoads=[]; document.querySelector('#chapter-content')?.addEventListener('load',(e)=>{ if(e.target?.tagName==='IMG') window.__imgLoads.push({t:Math.round(performance.now()-window.__clsStart), src:(e.target.getAttribute('src')||'').slice(0,60)}); },true);
        return 1;`));
      await main.evalJs(H(`
        window.__frFrames=[]; window.__frStop=false;
        const DELTA=${Number(process.env.FR_DELTA || 100)};
        const INTERVAL=${Number(process.env.FR_INTERVAL || 50)};
        const TICKS=${Number(process.env.FR_TICKS || 400)};
        const rec=()=>{ if(window.__frStop)return; const s=state(); window.__frFrames.push({t:Math.round(performance.now()), st:s.st, sh:s.sh, len:s.contentLen}); requestAnimationFrame(rec); };
        requestAnimationFrame(rec);
        const target=area();
        for(let i=0;i<TICKS;i++){ target.dispatchEvent(new WheelEvent('wheel',{deltaY:DELTA,bubbles:true,cancelable:true})); await sleep(INTERVAL); }
        window.__frStop=true; return 1;`));
      out.frames = await main.evalJs(`window.__frFrames`);
      out.cls = await main.evalJs(`window.__cls`);
      out.imgLoads = await main.evalJs(`window.__imgLoads`);
    } else if (SCENARIO === "flash") {
      await main.evalJs(H(`
        window.dispatchEvent(new CustomEvent('tidepage-jump',{detail:8})); return 1;`));
      await sleep(2500);
      await main.evalJs(H(`
        window.__frFrames=[]; window.__frStop=false;
        const rec=()=>{ if(window.__frStop)return; const s=state(); window.__frFrames.push({t:Math.round(performance.now()), st:s.st, sh:s.sh, len:s.contentLen}); requestAnimationFrame(rec); };
        requestAnimationFrame(rec); return 1;`));
      const runP = main.call("Runtime.evaluate", {
        expression: H(`
          const DELTA=${Number(process.env.FR_DELTA || 140)};
          const INTERVAL=${Number(process.env.FR_INTERVAL || 40)};
          const TICKS=${Number(process.env.FR_TICKS || 400)};
          const target=area();
          for(let i=0;i<TICKS;i++){ target.dispatchEvent(new WheelEvent('wheel',{deltaY:DELTA,bubbles:true,cancelable:true})); await sleep(INTERVAL); }
          window.__frStop=true; return 'done';`),
        awaitPromise: true, returnByValue: true,
      });
      const sizes = [];
      for (let i = 0; i < 70; i++) {
        try {
          const r = await main.call("Page.captureScreenshot", { format: "jpeg", quality: 40 });
          sizes.push({ i, n: (r?.data || "").length });
        } catch (e) { sizes.push({ i, err: String(e).slice(0, 60) }); }
        await sleep(30);
      }
      out.sizes = sizes;
      await runP.catch(() => {});
      out.frames = await main.evalJs(`window.__frFrames || []`);
    } else if (SCENARIO === "pages") {
      out.p0 = await main.evalJs(H(`
        const x=()=>{ const el=document.querySelector('.col-page'); const t=el?getComputedStyle(el).transform:''; const m=t.match(/matrix\\(1, 0, 0, 1, (-?[\\d.]+)/); return { tx:m?Number(m[1]):0, len:content()?content().textContent.length:-1, foot:(document.querySelector('.finfo-l')?.textContent||'')+'|'+(document.querySelector('.finfo-r')?.textContent||'') }; };
        return x();`));
      out.pages = await main.evalJs(H(`
        const x=()=>{ const el=document.querySelector('.col-page'); const t=el?getComputedStyle(el).transform:''; const m=t.match(/matrix\\(1, 0, 0, 1, (-?[\\d.]+)/); return { tx:m?Number(m[1]):0, len:content()?content().textContent.length:-1 }; };
        const out=[]; for(let i=0;i<6;i++){ window.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true})); await sleep(350); out.push(x()); }
        return out;`));
      out.tocPage = await main.evalJs(H(`
        document.querySelector('[data-testid="tb-toc"]')?.click();
        const items = await wait(()=>{const x=[...document.querySelectorAll('.toc-item')];return x.length?x:null;},'toc items');
        const target = items[Math.min(items.length-1, Math.floor(items.length*0.5))];
        const t=target.textContent.trim(); target.click(); await sleep(1500);
        const el=document.querySelector('.col-page'); const tr=el?getComputedStyle(el).transform:'';
        return { picked:t, len:content()?content().textContent.length:-1, tx:tr };`));
      out.img = await main.evalJs(H(`
        const imgs=[...document.querySelectorAll('#chapter-content img')].map(i=>({src:(i.getAttribute('src')||'').slice(0,60),nw:i.naturalWidth,w:i.width,h:i.height}));
        return imgs.slice(0,5);`));
    } else if (SCENARIO === "toc") {
      out.before = await main.evalJs(H(`return state();`));
      out.toc = await main.evalJs(H(`
        document.querySelector('[data-testid="tb-toc"]')?.click();
        const items = await wait(()=>{const x=[...document.querySelectorAll('.toc-item')];return x.length?x:null;},'toc items');
        const target = items[Math.min(items.length-1, Math.floor(items.length*0.6))];
        const t = target.textContent.trim();
        target.click();
        window.__poll=[];
        for(let i=0;i<30;i++){ await sleep(100); const c=content(); window.__poll.push({t:i*100, len:c?c.textContent.length:-1, hl:(c?c.innerHTML.length:-1)}); }
        return { picked: t, items: items.length };`));
      out.poll = await main.evalJs(`window.__poll`);
      out.after = await main.evalJs(H(`return state();`));
      out.afterHtml = await main.evalJs(H(`return (content()?.innerHTML||'').slice(0,200);`));
    }

    main.close();
  } finally {
    if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
    fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
    console.log("WROTE " + outFile);
  }
}
main().catch((e) => { console.error(e); process.exit(1); });
