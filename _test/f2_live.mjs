// F2 架构验证：缓存封面文件数=1（只有底图/无内嵌封面者不落盘）；文字实时叠、各不相同；极端底图。
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9688);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "f2_sbx"); const data = path.join(SANDBOX_ROOT, "f2_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "f2_evidence"); fs.rmSync(evid, { recursive: true, force: true }); fs.mkdirSync(evid, { recursive: true });
const outFile = path.join(SANDBOX_ROOT, "f2_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=250)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

function crc32(buf){const T=[];for(let n=0;n<256;n++){let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;T[n]=c>>>0}let c=0xffffffff;for(let i=0;i<buf.length;i++)c=T[(c^buf[i])&0xff]^(c>>>8);return (c^0xffffffff)>>>0}
function pngChunk(type,buf){const len=Buffer.alloc(4);len.writeUInt32BE(buf.length,0);const t=Buffer.from(type,"ascii");const crc=Buffer.alloc(4);crc.writeUInt32BE(crc32(Buffer.concat([t,buf])),0);return Buffer.concat([len,t,buf,crc])}
function makePng(w,h,px){const raw=Buffer.alloc((w*3+1)*h);let o=0;for(let y=0;y<h;y++){raw[o++]=0;for(let x=0;x<w;x++){const [r,g,b]=px(x,y);raw[o++]=r;raw[o++]=g;raw[o++]=b}}const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(w,0);ihdr.writeUInt32BE(h,4);ihdr[8]=8;ihdr[9]=2;return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),pngChunk("IHDR",ihdr),pngChunk("IDAT",zlib.deflateSync(raw)),pngChunk("IEND",Buffer.alloc(0))])}

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "f2_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 造 100 本无封面 TXT
  const tmp = path.join(SANDBOX_ROOT, "f2_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  for (let i = 0; i < 100; i++) {
    fs.writeFileSync(path.join(tmp, `书_${String(i).padStart(3,"0")} (作者${i}).txt`), "正文测试。".repeat(30), "utf8");
  }
  const files = fs.readdirSync(tmp);
  for (const n of files) {
    const p = path.join(tmp, n).replace(/\\/g, "\\\\");
    await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${p}"});}catch(e){}`));
  }
  await sleep(2500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(4000);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  // 等 meta 全部完成
  await M.evalJs(H(`for(let i=0;i<120;i++){ const s=await inv('meta_debug_stats'); if(!s.running) break; await sleep(500);} return 1;`));
  await sleep(2000);

  // 统计 covers 目录文件数
  const coversDir = path.join(data, "covers");
  const coverFiles = fs.existsSync(coversDir) ? fs.readdirSync(coversDir) : [];
  out.steps.cacheCount = { coversDirFiles: coverFiles.length, sample: coverFiles.slice(0, 5) };
  const defaultCover = fs.existsSync(path.join(data, "default-cover.img"));
  out.steps.cacheCount.hasDefaultImg = defaultCover;
  log("cacheCount: " + JSON.stringify(out.steps.cacheCount));

  // 3 本不同书 → 各自 URL 返回的 SVG 文字不同（证明实时叠）
  out.steps.threeBooks = await M.evalJs(H(`
    const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:200});
    const rows=pg.rows.filter(r=>/f2_books/i.test(r.path)).slice(0,3);
    const res=[];
    for(const b of rows){
      const url='http://flowread.localhost/cover-default/'+b.id;
      const t=await (await fetch(url)).text();
      const m=t.match(/font-weight="700"[^>]*>([^<]*)</);
      res.push({ id:b.id, dbTitle:b.title, dbAuthor:b.author, svgTitle:m?m[1]:null, hasShade:t.includes('id="shade"'), hasSlice:t.includes('slice')||false });
    }
    // 存第一个 svg 供肉眼检查
    if(rows[0]){ const t=await (await fetch('http://flowread.localhost/cover-default/'+rows[0].id)).text(); window.__svg0=t; }
    return { count:rows.length, res, svg0: window.__svg0 ? window.__svg0.slice(0,400) : null };
  `));
  log("threeBooks: " + JSON.stringify(out.steps.threeBooks));

  // 极端底图：4 张，取每张渲染结果
  const bases = {
    "f2-white": makePng(400,600,()=>[255,255,255]),
    "f2-black": makePng(400,600,()=>[8,8,8]),
    "f2-mid": makePng(400,600,(x)=>x<200?[110,125,118]:[104,116,132]),
    "f2-split": makePng(400,600,(x,y)=>(x+y)%2===0?[20,22,28]:[235,238,242]),
  };
  out.steps.bases = {};
  for (const [tag, buf] of Object.entries(bases)) {
    const p = path.join(evid, tag + ".png"); fs.writeFileSync(p, buf);
    const r = await M.evalJs(H(`
      await inv('set_default_cover',{imagePath:${JSON.stringify(p.replace(/\\/g, "\\\\"))}});
      await sleep(600);
      const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:200});
      const b=pg.rows.filter(x=>/f2_books/i.test(x.path))[0];
      const t=await (await fetch('http://flowread.localhost/cover-default/'+b.id)).text();
      return { len:t.length, hasShade:t.includes('id="shade"'), hasWhite:t.includes('fill="#ffffff"'), hasSlice:t.includes('preserveAspectRatio="xMidYMid slice"'), svg:t.slice(0,300) };
    `));
    out.steps.bases[tag] = { len: r.len, hasShade: r.hasShade, hasWhite: r.hasWhite, hasSlice: r.hasSlice };
    log(`base ${tag}: ` + JSON.stringify(out.steps.bases[tag]));
  }
  // 换底图后仍只有 1 份底图、无每本封面
  const after = fs.existsSync(coversDir) ? fs.readdirSync(coversDir) : [];
  out.steps.afterSwap = { coversDirFiles: after.length };
  log("afterSwap: " + JSON.stringify(out.steps.afterSwap));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
