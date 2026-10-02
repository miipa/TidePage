// 漫画模式（纵向瀑布流 .manga-flow-viewport）滚动实测：卡死 / 回弹 / 帧率 / 窗口有界 四项断言。
// 参照 _test/manga_live.mjs 的开书步骤，但 manga_live.mjs 只测了横向，本脚本专测纵向。
// 夹具：_test/make_manga_fixture.py 生成的 ≥60 章、每章 1 张 ~380KB 大图的 EPUB。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9588);
const TAG = process.env.MFLOW_TAG || "run";
const CHAPTERS = Number(process.env.MFLOW_CHAPTERS || 55);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const sbx = path.join(SANDBOX_ROOT, "mflow_sbx");
const data = path.join(SANDBOX_ROOT, "mflow_data");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
for (const dir of [sbx, data]) {
  for (const f of fs.readdirSync(dir)) fs.rmSync(path.join(dir, f), { recursive: true, force: true });
}
const BOOKS = path.join(SANDBOX_ROOT, "mflow_books");
const FIXTURE = path.join(BOOKS, "manga_flow_fixture.epub");
if (!fs.existsSync(FIXTURE)) {
  console.error(`缺少夹具：${FIXTURE}\n先跑：python _test/make_manga_fixture.py "${FIXTURE}" 60`);
  process.exit(2);
}
const outFile = path.join(SANDBOX_ROOT, `mflow_${TAG}.json`);
const diagLog = path.join(SANDBOX_ROOT, `mflow_${TAG}_diag.log`);

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const raf=()=>new Promise(r=>requestAnimationFrame(()=>r()));
const wait=async(fn,name,rounds=300)=>{for(let i=0;i<rounds;i++){const v=await fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const cards=(rounds=300)=>wait(()=>{const c=qa('.card,.row');return c.length?c:null;},'cards',rounds);
const cfg=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}');
const pct=(a,p)=>(a.length?a[Math.min(a.length-1,Math.floor(a.length*p))]:0);
`;
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

/** 页面内探针：逐帧记录 scrollTop / 回弹位移 / 图片 load 计数（只读行内 style，不触发强制样式重算）。 */
const PROBE = String.raw`
window.__mf = (() => {
  const vp = q('.manga-flow-viewport');
  const reader = q('.reader');
  if (!vp) return null;
  const frames = [];
  let last = 0, prevBounce = 0, imgLoads = 0, stop = false;
  const onImg = () => { imgLoads++; };
  reader.addEventListener('load', onImg, true);
  const readBounce = () => {
    const t = vp.style.transform;
    if (!t) return 0;
    const m = /translateY\(([-0-9.]+)px\)/.exec(t);
    return m ? parseFloat(m[1]) : 0;
  };
  const loop = (t) => {
    if (stop) return;
    if (last) frames.push({ dt: +(t - last).toFixed(2), top: vp.scrollTop, bounce: prevBounce, imgLoad: imgLoads });
    last = t;
    prevBounce = readBounce();
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
  const blocks = () => qa('.manga-flow-viewport .manga-block').map(b => ({ i: Number(b.dataset.index), top: b.offsetTop, h: b.offsetHeight }));
  return {
    vp, frames,
    snap() {
      const bs = blocks();
      // 视觉锚点：视口顶部所在章 + 章内偏移。头裁剪会让 scrollTop 锯齿状回退（内容不变、
      // 视觉不跳），所以「是否倒退」必须按视觉锚点判定，而不是裸 scrollTop。
      let ac = -1, ao = 0, blockH = 0;
      for (const b of bs) if (b.top + b.h > vp.scrollTop) { ac = b.i; ao = Math.round(vp.scrollTop - b.top); blockH = b.h; break; }
      return {
        top: vp.scrollTop,
        scrollHeight: vp.scrollHeight,
        clientHeight: vp.clientHeight,
        blocks: bs.length,
        first: bs.length ? bs[0].i : -1,
        last: bs.length ? bs[bs.length-1].i : -1,
        topChapter: ac, anchorOffset: ao, blockH,
        bounce: prevBounce,
        transform: vp.style.transform || '',
        imgLoads,
      };
    },
    wheel(dy) { reader.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: dy, deltaMode: 0 })); },
    stop() { stop = true; reader.removeEventListener('load', onImg, true); },
    ranges() {
      const bs = blocks();
      const seen = new Set(bs.map(b => b.i));
      const ranges = [];
      for (const b of bs) {
        const tail = ranges[ranges.length - 1];
        if (tail && b.i === tail[1] + 1) tail[1] = b.i; else ranges.push([b.i, b.i]);
      }
      return { ranges, duplicates: bs.length - seen.size };
    },
  };
})();
`;

const out = { tag: TAG, fixture: FIXTURE, steps: {}, asserts: {} };
let app = null;
try {
  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data },
    cdpPort: PORT,
    logFile: diagLog,
  });
  log("launched pid " + app.child.pid);
  let M = null;
  for (let i = 0; i < 240; i++) {
    try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); }
  }
  if (!M) throw new Error("no cdp");
  const ready = async () => {
    for (let i = 0; i < 300; i++) {
      try { const r = await M.evalJs(`({'t':!!window.__TAURI_INTERNALS__})`); if (r && r.t) return r; } catch { /* wait */ }
      await sleep(400);
    }
    throw new Error("app not ready");
  };
  await ready(M);
  log("app ready");

  // ---- 加书（只读打开夹具） ----
  out.steps.add = await M.evalJs(H(`return await inv('add_dropped_book', { path: ${JSON.stringify(FIXTURE)} });`));
  log("added: " + JSON.stringify(out.steps.add));
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(4000);
  M.close();
  M = await connectTarget(PORT, isApp);
  await ready();
  log("reloaded");

  // ---- 开书：确认为漫画 + 切到纵向瀑布流 ----
  out.steps.open = await M.evalJs(H(`
    const c = await cards();
    const b = c.find(x => x.textContent.includes('manga_flow_fixture')) || c[0];
    b.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await wait(() => q('.reader'), 'reader', 400);
    await sleep(2500);
    const id = ${out.steps.add?.id ?? 0};
    const o = await inv('open_book', { id });
    const c0 = cfg();
    c0.reader = c0.reader || {};
    c0.reader.mode = 'scroll';
    c0.reader.manga = Object.assign({}, c0.reader.manga || {}, { layout: 'flow' });
    localStorage.setItem('tidepage.config.v1', JSON.stringify(c0));
    await sleep(400);
    // 漫画模式若未自动开启，走顶栏真实按钮
    if (!q('.reader').classList.contains('manga')) {
      const btn = qa('.tb-r button').find(x => (x.getAttribute('title')||'').includes('漫画模式'));
      btn && btn.click();
      await sleep(1800);
    }
    await sleep(2500);
    const vp = q('.manga-flow-viewport');
    return {
      id, isManga: o.is_manga, title: o.book?.title, toc: o.toc?.length, images: o.images?.length,
      mangaClass: q('.reader').classList.contains('manga'),
      flowVp: !!vp, hViewport: !!q('.manga-h-viewport'),
      blocks: qa('.manga-block').length,
      clientHeight: vp ? vp.clientHeight : 0,
      layout: cfg().reader?.manga?.layout,
    };
  `));
  log("open: " + JSON.stringify(out.steps.open));
  if (!out.steps.open.flowVp) throw new Error("未进入 .manga-flow-viewport 纵向瀑布流路径");
  await sleep(1500);

  // ---- 装探针 ----
  out.steps.probe = await M.evalJs(H(`
    ${PROBE}
    if (!window.__mf) return 'no-viewport';
    // 等首屏窗口与懒加载图片稳定，避免把加载中的块数当成稳态。
    for (let i = 0; i < 60; i++) { await sleep(200); const n = qa('.manga-block').length; if (n >= 8 && i > 6) break; }
    return { ok: true, blocks: qa('.manga-block').length, imgs: qa('.manga-flow-viewport img').length, snap: window.__mf.snap() };
  `));
  log("probe: " + JSON.stringify(out.steps.probe));
  if (!out.steps.probe || out.steps.probe === "no-viewport") throw new Error("probe failed: 不是 .manga-flow-viewport");

  // ---- 断言 1 主体：从顶部连续滚轮下滚，记录每一步几何 ----
  out.steps.scroll = await M.evalJs(H(`
    const mf = window.__mf;
    mf.frames.length = 0;
    const samples = [];
    const events = [];
    const chaptersSeen = new Set();
    let stalled = 0, lastTop = -1;
    for (let i = 0; i < 400; i++) {
      mf.wheel(600);
      await raf(); await raf();
      await sleep(55);
      const s = mf.snap();
      s.i = i;
      samples.push(s);
      chaptersSeen.add(s.topChapter);
      if (s.top === lastTop) {
        stalled++;
        if (stalled >= 3) { events.push({ kind: 'stalled', i, s, ranges: mf.ranges() }); break; }
      } else stalled = 0;
      lastTop = s.top;
      if (s.topChapter >= ${CHAPTERS}) { events.push({ kind: 'reachedTarget', i, s }); break; }
    }
    return { samples, events, chaptersSeen: chaptersSeen.size, ranges: mf.ranges(), end: mf.snap() };
  `));
  log(`scroll: ${out.steps.scroll.samples.length} samples, chaptersSeen=${out.steps.scroll.chaptersSeen}, ranges=${JSON.stringify(out.steps.scroll.ranges.ranges)}`);

  // ---- 断言 1：缓冲不塌 + scrollTop 单调递增 + 跨章 ----
  out.asserts.a1 = await M.evalJs(H(`
    const ss = ${JSON.stringify(out.steps.scroll.samples)};
    const total = ${JSON.stringify(out.steps.open.toc)};
    const target = ${CHAPTERS};
    const events = ${JSON.stringify(out.steps.scroll.events)};
    const end = ss[ss.length - 1];
    const atBookEnd = end.last >= total - 1;
    let minBuffer = Infinity, bufFail = null;
    for (const s of ss) {
      const buf = s.scrollHeight - s.clientHeight;
      if (buf < minBuffer) minBuffer = buf;
      // 未到书尾时预载缓冲不许塌到一屏以内
      if (!(s.last >= total - 1) && buf < 1.5 * s.clientHeight) {
        bufFail = { i: s.i, buf: Math.round(buf), need: Math.round(1.5 * s.clientHeight), scrollHeight: s.scrollHeight, clientHeight: s.clientHeight, first: s.first, last: s.last, top: s.top };
        break;
      }
    }
    let decreases = 0, monoWorst = null, maxBackJump = 0;
    for (let k = 1; k < ss.length; k++) {
      // 视觉锚点单调性：章号不回退；同章内章内偏移不回退（容差 1px）。
      const a = ss[k-1], b = ss[k];
      if (b.topChapter < a.topChapter || (b.topChapter === a.topChapter && b.anchorOffset < a.anchorOffset - 1)) {
        decreases++;
        if (!monoWorst) monoWorst = { i: b.i, from: [a.topChapter, a.anchorOffset], to: [b.topChapter, b.anchorOffset] };
      }
      const d = b.top - a.top;
      if (d < -maxBackJump) maxBackJump = -d;
    }
    // 「卡住」度量：视觉锚点连续不变的采样数（平滑滚动时每采样都应推进）。
    let anchorStall = 0, run = 0;
    for (let k = 1; k < ss.length; k++) {
      const a = ss[k-1], b = ss[k];
      if (b.topChapter === a.topChapter && b.anchorOffset === a.anchorOffset) { run++; if (run > anchorStall) anchorStall = run; }
      else run = 0;
    }
    return {
      samples: ss.length, total, target, atBookEnd,
      reachedTarget: end.topChapter >= target,
      minBuffer: minBuffer === Infinity ? null : Math.round(minBuffer),
      bufferFail: bufFail, bufferOk: !bufFail,
      monotonic: decreases === 0, decreases, monoWorst,
      maxBackJump: Math.round(maxBackJump), anchorStall,
      maxBlockH: Math.max(...ss.map(s => s.blockH)),
      chaptersSeen: ${out.steps.scroll.chaptersSeen},
      maxBlocks: Math.max(...ss.map(s => s.blocks)),
      maxScrollHeight: Math.max(...ss.map(s => s.scrollHeight)),
      tailScrollHeight: Math.max(...ss.slice(-8).map(s => s.scrollHeight)),
      stall: events.filter(e => e.kind === 'stalled'),
      endWindow: { first: end.first, last: end.last, blocks: end.blocks },
    };
  `));

  // ---- 断言 2：无回弹（纯下滚阶段内 scrollTop 不得逐帧回退；回弹 transform 必须恒为 0） ----
  out.asserts.a2 = await M.evalJs(H(`
    const mf = window.__mf;
    mf.frames.length = 0;
    // 先反向滚回上方一段（让续章/裁剪两侧都被走一遍），再只做下滚并采样。
    for (let i = 0; i < 14; i++) { mf.wheel(-600); await raf(); await raf(); await sleep(50); }
    const midUp = mf.snap();
    mf.frames.length = 0;
    const mark = mf.frames.length;
    for (let i = 0; i < 26; i++) { mf.wheel(600); await raf(); await raf(); await sleep(50); }
    const fr = mf.frames.map(f => ({ ...f }));
    // 方向切换后的减速尾巴（前 6 帧）不计：那是平滑减速本身，不是回弹。
    const body = fr.slice(6);
    let decRun = 0, maxDecRun = 0;
    const decSamples = [];
    for (let k = 1; k < body.length; k++) {
      const d = body[k].top - body[k-1].top;
      if (d < -0.5) {
        decRun++;
        if (decRun > maxDecRun) maxDecRun = decRun;
        if (decSamples.length < 5) decSamples.push({ k: k + 6, from: body[k-1].top, to: body[k].top, dt: body[k].dt });
      } else decRun = 0;
    }
    const worstBounce = Math.max(0, ...fr.map(f => Math.abs(f.bounce)));
    const worstBounceAt = fr.find(f => Math.abs(f.bounce) > 0.01) || null;
    const anyTransform = fr.some(f => f.top > 0) && fr.filter(f => f.transformSeen).length;
    return { frames: fr.length, midUp, maxDecRun, decSamples, worstBounce: +worstBounce.toFixed(2),
             worstBounceAt, bounceOk: maxDecRun <= 1 && worstBounce === 0, end: mf.snap() };
  `));

  // ---- 断言 3：帧率 p50/p95/max（图片解码帧单独标注，不计入长帧） ----
  out.asserts.a3 = await M.evalJs(H(`
    const mf = window.__mf;
    mf.frames.length = 0;
    for (let i = 0; i < 30; i++) { mf.wheel(600); await raf(); await raf(); await sleep(50); }
    const fr = mf.frames.map(f => ({ ...f }));
    const dts = fr.map(f => f.dt).filter(v => v > 0).sort((a, b) => a - b);
    if (!dts.length) return { err: 'no frames' };
    const imgLoads = fr.length ? fr[fr.length-1].imgLoad - fr[0].imgLoad : 0;
    // 图片 load 之后的 300ms 内的帧归为「解码帧」，单独标注不计入长帧。
    const imgFrames = new Set();
    let lastLoad = -1;
    fr.forEach((f, k) => { if (lastLoad >= 0 || (k > 0 && f.imgLoad > fr[k-1].imgLoad)) lastLoad = k; if (lastLoad >= 0 && k - lastLoad < 18) imgFrames.add(k); });
    const code = dts.filter((_, k) => !imgFrames.has(k));
    const long = fr.map((f, k) => ({ k, dt: f.dt, img: imgFrames.has(k) })).filter(x => x.dt > 250);
    return {
      frames: dts.length, imgLoads,
      p50: pct(dts, 0.5), p95: pct(dts, 0.95), max: +dts[dts.length-1].toFixed(1),
      codeFrames: code.length, codeP50: pct(code, 0.5), codeP95: pct(code, 0.95),
      codeMax: code.length ? +code[code.length-1].toFixed(1) : null,
      longFrames: long, longCodeFrames: long.filter(x => !x.img),
      smoothOk: long.filter(x => !x.img).length === 0,
    };
  `));

  // ---- 断言 4：窗口有界 ----
  out.asserts.a4 = await M.evalJs(H(`
    const s = window.__mf.snap();
    const r = window.__mf.ranges();
    return { blocks: s.blocks, first: s.first, last: s.last, topChapter: s.topChapter,
             scrollHeight: s.scrollHeight, clientHeight: s.clientHeight,
             imgNodes: qa('.manga-flow-viewport img').length,
             budget: 50, ok: s.blocks <= 50 && qa('.manga-flow-viewport img').length <= 50,
             ranges: r.ranges, duplicates: r.duplicates };
  `));

  await M.evalJs(`(()=>{ window.__mf && window.__mf.stop(); return 1; })()`);
  M.close();
} catch (e) {
  out.error = String(e);
  console.error(e);
} finally {
  if (app) {
    killTree(app.child.pid);
    await sleep(1500);
    sweepSandboxWebviews("mflow_sbx");
  }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
  const a = out.asserts;
  console.log("=== SUMMARY " + TAG + " ===");
  console.log(JSON.stringify({
    open: out.steps.open,
    a1: a.a1 && {
      samples: a.a1.samples, reachedTarget: a.a1.reachedTarget, minBuffer: a.a1.minBuffer,
      bufferOk: a.a1.bufferOk, bufferFail: a.a1.bufferFail, monotonicAnchor: a.a1.monotonic,
      anchorDecreases: a.a1.decreases, monoWorst: a.a1.monoWorst,
      maxBackJump: a.a1.maxBackJump, anchorStall: a.a1.anchorStall, maxBlockH: a.a1.maxBlockH,
      chaptersSeen: a.a1.chaptersSeen, maxBlocks: a.a1.maxBlocks,
      maxScrollHeight: a.a1.maxScrollHeight, tailScrollHeight: a.a1.tailScrollHeight,
      endWindow: a.a1.endWindow, stall: a.a1.stall,
    },
    a2: a.a2 && { frames: a.a2.frames, maxDecRun: a.a2.maxDecRun, decSamples: a.a2.decSamples, worstBounce: a.a2.worstBounce, bounceOk: a.a2.bounceOk, end: a.a2.end },
    a3: a.a3 && { frames: a.a3.frames, imgLoads: a.a3.imgLoads, p50: a.a3.p50, p95: a.a3.p95, max: a.a3.max, codeP50: a.a3.codeP50, codeP95: a.a3.codeP95, codeMax: a.a3.codeMax, longCodeFrames: a.a3.longCodeFrames, smoothOk: a.a3.smoothOk },
    a4: a.a4 && { blocks: a.a4.blocks, imgNodes: a.a4.imgNodes, first: a.a4.first, last: a.a4.last, topChapter: a.a4.topChapter, ok: a.a4.ok, ranges: a.a4.ranges },
    error: out.error,
  }, null, 1));
}