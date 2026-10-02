// 批1遗留必改 + 批2(F) 证据：worker 互斥、清缓存不并发、封面统一/智能书名、极端底图清晰度。
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9677);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "f_sbx"); const data = path.join(SANDBOX_ROOT, "f_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "f_evidence"); fs.mkdirSync(evid, { recursive: true });
const outFile = path.join(SANDBOX_ROOT, "b1fix_f_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=250)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

// 生成 4 张极端底图 PNG（纯白/纯黑/中间调灰绿/半深半浅花图）——极简 PNG 编码器
function pngChunk(type, buf) {
  const len = Buffer.alloc(4); len.writeUInt32BE(buf.length, 0);
  const t = Buffer.from(type, "ascii");
  const crcBuf = Buffer.concat([t, buf]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(crcBuf) >>> 0, 0);
  return Buffer.concat([len, t, buf, crc]);
}
let CRC_T = null;
function crc32(buf) {
  if (!CRC_T) { CRC_T = []; for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; CRC_T[n] = c >>> 0; } }
  let c = 0xffffffff; for (let i = 0; i < buf.length; i++) c = CRC_T[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function makePng(w, h, px) {
  const raw = Buffer.alloc((w * 3 + 1) * h);
  let o = 0;
  for (let y = 0; y < h; y++) { raw[o++] = 0; for (let x = 0; x < w; x++) { const [r, g, b] = px(x, y); raw[o++] = r; raw[o++] = g; raw[o++] = b; } }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  return Buffer.concat([sig, pngChunk("IHDR", ihdr), pngChunk("IDAT", zlib.deflateSync(raw)), pngChunk("IEND", Buffer.alloc(0))]);
}

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "f_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 造含多本 TXT 的书库（文件名覆盖边界用例）
  const tmp = path.join(SANDBOX_ROOT, "f_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  const names = [
    "普通书名 (某作者).txt",
    "12345.txt",
    "《围城》.txt",
    "超长中文书名测试专用特别特别特别特别特别特别长.txt",
    "解忧杂货店 - 东野圭吾.txt",
  ];
  for (const n of names) fs.writeFileSync(path.join(tmp, n), "正文内容测试。".repeat(50), "utf8");
  for (const n of names) {
    const p = path.join(tmp, n).replace(/\\/g, "\\\\");
    await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${p}"});}catch(e){}`));
  }
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(1500);

  // 批1-1: worker 互斥 —— 连点刷新封面 5 次 + 清封面，startCount 应为 1
  out.steps.mutex = await M.evalJs(H(`
    // 先等当前 worker 结束
    for(let i=0;i<40;i++){ const s=await inv('meta_debug_stats'); if(!s.running) break; await sleep(300); }
    const before=await inv('meta_debug_stats');
    // 并发触发 5 次 refresh_covers（不 await，模拟连点）
    const ps=[]; for(let i=0;i<5;i++){ ps.push(inv('refresh_covers')); }
    await Promise.all(ps);
    const midRunning=(await inv('meta_debug_stats')).running;
    await sleep(300);
    const afterConcurrent=await inv('meta_debug_stats');
    // 再等全部结束
    for(let i=0;i<40;i++){ const s=await inv('meta_debug_stats'); if(!s.running) break; await sleep(300); }
    const end=await inv('meta_debug_stats');
    return { beforeStart: before.startCount, afterConcurrentStart: afterConcurrent.startCount,
      deltaConcurrent: afterConcurrent.startCount-before.startCount, midRunning,
      totalDelta: end.startCount-before.startCount };
  `));
  log("mutex: " + JSON.stringify(out.steps.mutex));

  // 批1-2: 生成中途清封面 —— 无报错、无半成品
  out.steps.clearMid = await M.evalJs(H(`
    let err=null;
    try { await inv('refresh_covers'); await sleep(300); await inv('clear_cache',{what:'covers'}); } catch(e){ err=String(e); }
    return { err };
  `));
  log("clearMid: " + JSON.stringify(out.steps.clearMid));

  // F3: 文件名解析（后端返回的 title/author）
  out.steps.parse = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:200});
    const rows=pg.rows.filter(r=>/f_books/i.test(r.path)).map(r=>({name:r.path.split(/[\\\\/]/).pop(), title:r.title, author:r.author}));
    return rows;
  `));
  log("parse: " + JSON.stringify(out.steps.parse));

  // F1/F2: 取默认封面 SVG 内容（验证暗化层+白字）
  out.steps.coverSvg = await M.evalJs(H(`
    for(let i=0;i<80;i++){
      const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:200});
      const rows=pg.rows.filter(r=>/f_books/i.test(r.path));
      const withCover=rows.filter(r=>r.cover_path);
      if(withCover.length){
        const r=withCover[0];
        const resp=await fetch('http://tidepage.localhost/covers/'+encodeURIComponent(r.cover_path));
        const txt=await resp.text();
        return { total:rows.length, withCover:withCover.length, cover:r.cover_path, len:txt.length,
          hasOverlay: txt.includes('fill-opacity="0.60"'), hasWhite: txt.includes('fill="#ffffff"'),
          title:r.title, allCovers: rows.map(x=>({t:x.title, c:x.cover_path})) };
      }
      await sleep(500);
    }
    return { err:'no cover generated' };
  `));
  log("coverSvg: " + JSON.stringify(out.steps.coverSvg));

  // F2: 换默认底图 → 缓存失效（covers 目录被清 + meta_done 复位）
  const whitePng = path.join(evid, "base-white.png");
  fs.writeFileSync(whitePng, makePng(400, 600, () => [255, 255, 255]));
  const darkPng = path.join(evid, "base-dark.png");
  fs.writeFileSync(darkPng, makePng(400, 600, (x, y) => (x + y) % 2 === 0 ? [10, 12, 16] : [230, 235, 240]));
  out.steps.swapBase = await M.evalJs(H(`
    const before=await inv('storage_stats');
    await inv('set_default_cover',{imagePath:${JSON.stringify(whitePng.replace(/\\/g, "\\\\"))}});
    await sleep(1500);
    const mid=await inv('storage_stats');
    const stats=await inv('meta_debug_stats');
    return { coversBefore: before.covers_bytes, coversAfterSet: mid.covers_bytes, workerRunning: stats.running, hasDefault: await inv('has_default_cover') };
  `));
  log("swapBase: " + JSON.stringify(out.steps.swapBase));

  // F2: 4 张极端底图 —— 各生成封面并把 SVG 存盘（验证结构一致：暗化层+白字）
  const bases = {
    "base1-white": makePng(400, 600, () => [255, 255, 255]),
    "base2-black": makePng(400, 600, () => [8, 8, 8]),
    "base3-mid": makePng(400, 600, (x) => x < 200 ? [110, 125, 118] : [104, 116, 132]),
    "base4-split": makePng(400, 600, (x, y) => (x + y) % 2 === 0 ? [20, 22, 28] : [235, 238, 242]),
  };
  out.steps.extremeBases = {};
  for (const [tag, buf] of Object.entries(bases)) {
    const p = path.join(evid, tag + ".png");
    fs.writeFileSync(p, buf);
    const r = await M.evalJs(H(`
      await inv('set_default_cover',{imagePath:${JSON.stringify(p.replace(/\\/g, "\\\\"))}});
      await sleep(1800);
      for(let i=0;i<60;i++){
        const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:200});
        const rows=pg.rows.filter(x=>/f_books/i.test(x.path) && x.cover_path);
        if(rows.length>=3){
          const resp=await fetch('http://tidepage.localhost/covers/'+encodeURIComponent(rows[0].cover_path));
          const txt=await resp.text();
          return { cover:rows[0].cover_path, len:txt.length, hasOverlay: txt.includes('fill-opacity="0.60"'), hasWhite: txt.includes('fill="#ffffff"'), hasSlice: txt.includes('preserveAspectRatio="xMidYMid slice"'), svg: txt };
        }
        await sleep(500);
      }
      return { err:'timeout' };
    `));
    if (r && r.svg) { fs.writeFileSync(path.join(evid, tag + ".svg"), r.svg); delete r.svg; }
    out.steps.extremeBases[tag] = r;
    log(`base ${tag}: ` + JSON.stringify(r));
  }

  M.close();
} catch (e) { process.exitCode = 1; /* 失败必须体现在退出码上 */ out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
