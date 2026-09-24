// 封面增量扫描自测：
// 场景1：扫描进行中换默认封面 → 进度条数字不回退/不归零，扫描继续到完。
// 场景2：导入 3 本带内嵌封面的新书 → 底部 0/3 → 3/3 后消失；3 本自动从底图变成各自封面。
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9150);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);

const sbx = path.join(SANDBOX_ROOT, "cs_sbx");
const data = path.join(SANDBOX_ROOT, "cs_data");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "cs_evidence");
fs.rmSync(evid, { recursive: true, force: true });
fs.mkdirSync(evid, { recursive: true });
const outFile = path.join(SANDBOX_ROOT, "cs_out.json");

const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

function crc32(buf){const T=[];for(let n=0;n<256;n++){let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;T[n]=c>>>0}let c=0xffffffff;for(let i=0;i<buf.length;i++)c=T[(c^buf[i])&0xff]^(c>>>8);return (c^0xffffffff)>>>0}
function chunk(t,b){const l=Buffer.alloc(4);l.writeUInt32BE(b.length,0);const tt=Buffer.from(t,"ascii");const cr=Buffer.alloc(4);cr.writeUInt32BE(crc32(Buffer.concat([tt,b])),0);return Buffer.concat([l,tt,b,cr])}
function makePng(w,h,px){const raw=Buffer.alloc((w*3+1)*h);let o=0;for(let y=0;y<h;y++){raw[o++]=0;for(let x=0;x<w;x++){const[r,g,b]=px(x,y);raw[o++]=r;raw[o++]=g;raw[o++]=b}}const ih=Buffer.alloc(13);ih.writeUInt32BE(w,0);ih.writeUInt32BE(h,4);ih[8]=8;ih[9]=2;return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk("IHDR",ih),chunk("IDAT",zlib.deflateSync(raw)),chunk("IEND",Buffer.alloc(0))])}

const sample = async (M) => {
  try {
    return await M.evalJs(H(`
      const el=q('.meta-progress .scan-text');
      const st=await inv('meta_debug_stats');
      return { text: el?el.textContent.trim():null, hasBar: !!q('.meta-progress'), running: st.running, queued: st.queued, startCount: st.startCount };
    `));
  } catch (e) { return { err: String(e) }; }
};

const parse = (s) => {
  const m = /(\d+)\/(\d+)/.exec(s?.text || "");
  return m ? { done: Number(m[1]), total: Number(m[2]) } : null;
};

const out = { steps: {} };
let app = null;
try {
  // 场景1素材：150 本 test_conv.epub 副本（解析较慢，保证换底图时扫描仍在进行）
  const bulk = path.join(SANDBOX_ROOT, "cs_books", "bulk");
  fs.rmSync(bulk, { recursive: true, force: true });
  fs.mkdirSync(bulk, { recursive: true });
  const srcConv = path.join(SANDBOX_ROOT, "scroll_books", "test_conv.epub");
  for (let i = 0; i < 300; i++) fs.copyFileSync(srcConv, path.join(bulk, `bulk_${String(i).padStart(3, "0")}.epub`));
  // 场景2素材：3 本 zhen_dian.epub 副本（有内嵌封面）
  const trio = path.join(SANDBOX_ROOT, "cs_books", "trio");
  fs.rmSync(trio, { recursive: true, force: true });
  fs.mkdirSync(trio, { recursive: true });
  const srcZhen = path.join(SANDBOX_ROOT, "scroll_books", "zhen_dian.epub");
  for (let i = 0; i < 3; i++) fs.copyFileSync(srcZhen, path.join(trio, `trio_${i}.epub`));

  const basePng = path.join(evid, "base_midscan.png");
  fs.writeFileSync(basePng, makePng(400, 600, (x, y) => (((x >> 5) + (y >> 5)) % 2 ? [200, 60, 40] : [245, 225, 220])));

  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data },
    cdpPort: PORT,
    logFile: path.join(SANDBOX_ROOT, "cs_diag.log"),
  });
  log("launched pid " + app.child.pid);
  let M = null;
  for (let i = 0; i < 30; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  log("cdp ready");

  // 记录后端原始事件序列（meta-progress / scan-finished），便于核验。
  await M.evalJs(H(`
    window.__prog = [];
    const t = window.__TAURI_INTERNALS__.transformCallback;
    const h1 = t((e)=>{ window.__prog.push({ k:'p', p:e.payload }); });
    await inv('plugin:event|listen',{ event:'meta-progress', target:{kind:'Any'}, handler:h1 });
    const h2 = t(()=>{ window.__prog.push({ k:'fin' }); });
    await inv('plugin:event|listen',{ event:'scan-finished', target:{kind:'Any'}, handler:h2 });
    // 页内 DOM 记录器（40ms）：精确记录底部进度条文字的变化序列。
    window.__dom = [];
    window.__domTimer = setInterval(()=>{ const el=document.querySelector('.meta-progress .scan-text'); const t=el?el.textContent.trim():null; if(window.__dom[window.__dom.length-1]!==t) window.__dom.push(t); }, 40);
    return 1;
  `));

  // ---------- 场景1 ----------
  await M.evalJs(H(`await inv('add_folder',{path:${JSON.stringify(bulk)}}); return 1;`));
  const s1 = [];
  let switched = false, doneBefore = null, doneAfter = null;
  for (let i = 0; i < 500; i++) {
    const smp = await sample(M);
    s1.push(smp);
    if (!switched && smp.hasBar && (smp.running || smp.queued > 0)) {
      doneBefore = parse(smp);
      // 扫描进行中换默认封面
      await M.evalJs(H(`await inv('set_default_cover',{imagePath:${JSON.stringify(basePng)}}); return 1;`));
      const smp2 = await sample(M);
      doneAfter = parse(smp2);
      switched = true;
      log("switched base mid-scan; before=" + JSON.stringify(doneBefore) + " after=" + JSON.stringify(doneAfter));
    }
    if (switched && !smp.running && smp.queued === 0 && !smp.hasBar) break;
    await sleep(120);
  }
  out.steps.scenario1 = {
    switched,
    before: doneBefore,
    after: doneAfter,
    finalRunning: s1[s1.length - 1]?.running,
    finalQueued: s1[s1.length - 1]?.queued,
    finalText: s1[s1.length - 1]?.text,
    samples: s1.map((s) => parse(s)).filter(Boolean),
    startCounts: [...new Set(s1.map((s) => s.startCount))],
  };
  // 断言：done 单调不减，且不出现「有进度后回 0」
  const seq = out.steps.scenario1.samples;
  let monotonic = true, resetToZero = false;
  for (let i = 1; i < seq.length; i++) {
    if (seq[i].done < seq[i - 1].done) monotonic = false;
    if (seq[i - 1].done > 0 && seq[i].done === 0) resetToZero = true;
  }
  out.steps.scenario1.monotonic = monotonic;
  out.steps.scenario1.resetToZero = resetToZero;
  out.steps.scenario1.totalSeen = seq.length ? seq[seq.length - 1].total : 0;
  log("S1 " + JSON.stringify({ switched, before: doneBefore, after: doneAfter, monotonic, resetToZero, finalText: out.steps.scenario1.finalText, last: seq[seq.length - 1] }));
  await sleep(1000);
  out.steps.scenario1.domSeq = await M.evalJs(`JSON.stringify(window.__dom)`);
  out.steps.s1Events = await M.evalJs(`window.__prog.map(x=>x.k==='fin'?'FIN':(x.p.done+'/'+x.p.total)).join(' ')`);
  log("S1 DOM seq (tail): " + out.steps.scenario1.domSeq.slice(-220));
  await M.evalJs(`window.__prog.length = 0; 1`);

  // ---------- 场景2 ----------
  const s2 = [];
  await M.evalJs(`window.__dom = []; 1`);
  await M.evalJs(H(`await inv('add_folder',{path:${JSON.stringify(trio)}}); return 1;`));
  let saw03 = false, saw33 = false, barGone = false;
  for (let i = 0; i < 300; i++) {
    const smp = await sample(M);
    s2.push(smp);
    const p = parse(smp);
    if (p && p.total === 3 && p.done === 0) saw03 = true;
    if (p && p.total === 3 && p.done === 3) saw33 = true;
    if (saw03 && !smp.hasBar) { barGone = true; break; }
    await sleep(100);
  }
  out.steps.scenario2 = {
    saw03, saw33, barGone,
    samples: s2.map((s) => parse(s)).filter(Boolean).filter((p) => p.total === 3),
    finalText: s2[s2.length - 1]?.text,
  };
  log("S2 progress " + JSON.stringify({ saw03, saw33, barGone, seq: out.steps.scenario2.samples }));
  out.steps.scenario2.domSeq = await M.evalJs(`JSON.stringify(window.__dom)`);
  out.steps.s2Events = await M.evalJs(`window.__prog.map(x=>x.k==='fin'?'FIN':(x.p.done+'/'+x.p.total)).join(' ')`);
  log("S2 DOM seq: " + out.steps.scenario2.domSeq);
  log("S2 events: " + out.steps.s2Events);

  // 等 meta 完成并刷新页面，检查 3 本是否变成各自封面
  await M.evalJs(H(`let saw=false; for(let i=0;i<200;i++){const s=await inv('meta_debug_stats'); if(s.running) saw=true; else if(saw||s.queued===0) break; await sleep(300);} return 1;`));
  await sleep(1200);
  out.steps.scenario2.books = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:1000});
    const rows=pg.rows.filter(r=>/trio_\\d+\\.epub$/i.test(r.path));
    const fnv=(b)=>{let h=0x811c9dc5;for(let i=0;i<b.length;i++){h^=b[i];h=(h*0x01000193)>>>0;}return h.toString(16);};
    const res=[];
    for(const b of rows){
      const perBook = !!b.cover_path;
      const url = perBook ? 'http://flowread.localhost/book-cover/'+b.id+'?v=0-'+b.cover_ver : 'http://flowread.localhost/cover-default/'+b.id;
      const buf=new Uint8Array(await (await fetch(url)).arrayBuffer());
      res.push({ id:b.id, cover_path:b.cover_path, cover_ver:b.cover_ver, meta_done:b.meta_done, perBook, isPng:buf[0]===0x89&&buf[1]===0x50, isSvg:buf[0]===0x3c, bytes:buf.length, hash:fnv(buf) });
    }
    return res;
  `));
  log("S2 books " + JSON.stringify(out.steps.scenario2.books));

  // 场景1 的 150 本：换底图后 meta_done 必须仍为 1（未被重置）
  out.steps.scenario1.bulkMeta = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:1000});
    const rows=pg.rows.filter(r=>/bulk_\\d+\\.epub$/i.test(r.path));
    return { count: rows.length, allDone: rows.every(r=>r.meta_done!==0), done1: rows.filter(r=>r.meta_done===1).length, done2: rows.filter(r=>r.meta_done===2).length, done0: rows.filter(r=>r.meta_done===0).length };
  `));
  log("S1 bulkMeta " + JSON.stringify(out.steps.scenario1.bulkMeta));

  M.close();
} catch (e) {
  out.error = String(e);
  console.error("[cs] ERROR", e);
} finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile + "  evidence: " + evid);
}
