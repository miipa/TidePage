// 默认封面底图整改 证据：3 本书（1 本有内嵌封面 epub + 2 本无封面 txt）
// 换底图/恢复内置/损坏文件 三态下，covers 目录里 per-book 封面文件必须原封不动。
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9150);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);

const sbx = path.join(SANDBOX_ROOT, "dcf_sbx");
const data = path.join(SANDBOX_ROOT, "dcf_data");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "dcf_evidence");
fs.rmSync(evid, { recursive: true, force: true });
fs.mkdirSync(evid, { recursive: true });
const outFile = path.join(SANDBOX_ROOT, "dcf_out.json");

const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=250)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;
const SHOT = async (M, n) => {
  try {
    const s = await M.call("Page.captureScreenshot", { format: "png" });
    if (s?.data) { fs.writeFileSync(path.join(evid, n), Buffer.from(s.data, "base64")); return true; }
  } catch {}
  return false;
};

function crc32(buf){const T=[];for(let n=0;n<256;n++){let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;T[n]=c>>>0}let c=0xffffffff;for(let i=0;i<buf.length;i++)c=T[(c^buf[i])&0xff]^(c>>>8);return (c^0xffffffff)>>>0}
function chunk(t,b){const l=Buffer.alloc(4);l.writeUInt32BE(b.length,0);const tt=Buffer.from(t,"ascii");const cr=Buffer.alloc(4);cr.writeUInt32BE(crc32(Buffer.concat([tt,b])),0);return Buffer.concat([l,tt,b,cr])}
function makePng(w,h,px){const raw=Buffer.alloc((w*3+1)*h);let o=0;for(let y=0;y<h;y++){raw[o++]=0;for(let x=0;x<w;x++){const[r,g,b]=px(x,y);raw[o++]=r;raw[o++]=g;raw[o++]=b}}const ih=Buffer.alloc(13);ih.writeUInt32BE(w,0);ih.writeUInt32BE(h,4);ih[8]=8;ih[9]=2;return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk("IHDR",ih),chunk("IDAT",zlib.deflateSync(raw)),chunk("IEND",Buffer.alloc(0))])}
// 1x1 合法 JPEG（用于验证非 PNG 底图的 MIME 嗅探）
const TINY_JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q==",
  "base64",
);

const coversList = () => {
  const d = path.join(data, "covers");
  return fs.existsSync(d) ? fs.readdirSync(d).sort() : [];
};
const baseExists = () => fs.existsSync(path.join(data, "default-cover.img"));

// 页面内：按 api.ts 规则构造封面 URL，并探测返回内容
const PROBE = `
  const rows=(await inv('list_books_page',{query:'', sort:'title', offset:0, limit:500})).rows.filter(r=>/dcf_books/i.test(r.path));
  const fnv=(b)=>{let h=0x811c9dc5;for(let i=0;i<b.length;i++){h^=b[i];h=(h*0x01000193)>>>0;}return h.toString(16);};
  const res=[];
  for(const b of rows){
    const url = b.cover_path ? 'http://tidepage.localhost/covers/'+encodeURIComponent(b.cover_path) : 'http://tidepage.localhost/cover-default/'+b.id;
    const isPerBook=!!b.cover_path;
    let info={ id:b.id, title:b.title, author:b.author, path:b.path, cover_path:b.cover_path, kind:isPerBook?'per-book':'shared-default', url };
    if(isPerBook){
      const buf=new Uint8Array(await (await fetch(url)).arrayBuffer());
      info.bytes=buf.length; info.isPng=buf[0]===0x89&&buf[1]===0x50; info.isSvg=buf[0]===0x3c; info.hash=fnv(buf);
    } else {
      const t=await (await fetch(url)).text();
      info.isSvg=t.includes('<svg'); info.hasImageData=t.includes('<image href="data:');
      info.hasBuiltinGradient=t.includes('url(#cvbg)');
      info.dataMime=(t.match(/data:(image\\/[a-z]+);base64/)||[])[1]||null;
      info.svgTitle=(t.match(/font-weight="700"[^>]*>([^<]*)</)||[])[1]||null;
      info.svgAuthor=(t.match(/fill-opacity="0.85"[^>]*>([^<]*)</)||[])[1]||null;
    }
    res.push(info);
  }
  return res;
`;

const out = { steps: {} };
let app = null;
try {
  // 造 3 本：1 本有内嵌封面 epub + 2 本无封面 txt
  const tmp = path.join(SANDBOX_ROOT, "dcf_books");
  fs.rmSync(tmp, { recursive: true, force: true });
  fs.mkdirSync(tmp, { recursive: true });
  fs.copyFileSync(path.join(SANDBOX_ROOT, "scroll_books", "zhen_dian.epub"), path.join(tmp, "有封面书.epub"));
  fs.writeFileSync(path.join(tmp, "无封面甲 (作者甲).txt"), "正文甲。".repeat(60), "utf8");
  fs.writeFileSync(path.join(tmp, "无封面乙 (作者乙).txt"), "正文乙。".repeat(60), "utf8");

  // CDP 端口只在 debug 构建注入（FR_TEST_CDP_PORT），故用 tauri dev 沙箱启动。
  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data },
    cdpPort: PORT,
    logFile: path.join(SANDBOX_ROOT, "dcf_diag.log"),
  });
  log("launched pid " + app.child.pid);

  let M = null;
  for (let i = 0; i < 30; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  log("cdp ready");

  // 加入 3 本
  for (const n of fs.readdirSync(tmp)) {
    const p = path.join(tmp, n).replace(/\\/g, "\\\\");
    await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${p}"});}catch(e){return String(e);} return 1;`));
  }
  // 触发元数据/封面提取（add_dropped_book 本身不拉起 worker），并等待完成
  await M.evalJs(H(`try{await inv('refresh_covers');}catch(e){} return 1;`));
  await sleep(1200);
  await M.evalJs(H(`let saw=false; for(let i=0;i<300;i++){ const s=await inv('meta_debug_stats'); if(s.running) saw=true; else if(saw) break; await sleep(500);} return 1;`));
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close();
  M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await M.evalJs(H(`let saw=false; for(let i=0;i<300;i++){ const s=await inv('meta_debug_stats'); if(s.running) saw=true; else if(saw) break; await sleep(500);} return 1;`));
  await sleep(1200);

  // ---------- BEFORE ----------
  out.steps.before = {
    coversDir: coversList(),
    coversDirCount: coversList().length,
    baseExists: baseExists(),
    books: await M.evalJs(H(PROBE)),
  };
  await SHOT(M, "1_before_library.png");
  log("BEFORE " + JSON.stringify({ coversDirCount: out.steps.before.coversDirCount, baseExists: out.steps.before.baseExists, books: out.steps.before.books.map(b=>({t:b.title,k:b.kind,mime:b.dataMime||null})) }));

  // 记录有内嵌封面书的封面字节哈希（换底图/重置后必须一致）
  const perBookBefore = out.steps.before.books.filter((b) => b.kind === "per-book").map((b) => ({ id: b.id, hash: b.hash, bytes: b.bytes }));

  // ---------- 换底图（醒目条纹 PNG） ----------
  const basePng = path.join(evid, "base_new.png");
  fs.writeFileSync(basePng, makePng(400, 600, (x, y) => (((x >> 5) + (y >> 5)) % 2 ? [16, 148, 130] : [230, 245, 240])));
  out.steps.afterSet = await M.evalJs(H(`
    await inv('set_default_cover',{imagePath:${JSON.stringify(basePng.replace(/\\/g, "\\\\"))}});
    await sleep(900);
    return { hasDefault: await inv('has_default_cover') };
  `));
  await sleep(1200);
  out.steps.after = {
    coversDir: coversList(),
    coversDirCount: coversList().length,
    baseExists: baseExists(),
    books: await M.evalJs(H(PROBE)),
  };
  await SHOT(M, "2_after_library.png");
  out.steps.after.perBookPreserved = out.steps.after.books
    .filter((b) => b.kind === "per-book")
    .map((b) => { const before = perBookBefore.find((p) => p.id === b.id); return { id: b.id, sameHash: !!before && before.hash === b.hash, hash: b.hash, before: before?.hash }; });
  log("AFTER " + JSON.stringify({ coversDirCount: out.steps.after.coversDirCount, baseExists: out.steps.after.baseExists, perBookPreserved: out.steps.after.perBookPreserved, books: out.steps.after.books.map(b=>({t:b.title,k:b.kind,mime:b.dataMime||null,grad:b.hasBuiltinGradient})) }));

  // ---------- 恢复内置 ----------
  out.steps.reset = await M.evalJs(H(`
    await inv('clear_default_cover');
    await sleep(900);
    return { hasDefault: await inv('has_default_cover') };
  `));
  await sleep(1200);
  out.steps.afterReset = {
    coversDir: coversList(),
    coversDirCount: coversList().length,
    baseExists: baseExists(),
    books: await M.evalJs(H(PROBE)),
  };
  await SHOT(M, "3_reset_library.png");
  out.steps.afterReset.perBookPreserved = out.steps.afterReset.books
    .filter((b) => b.kind === "per-book")
    .map((b) => { const before = perBookBefore.find((p) => p.id === b.id); return { id: b.id, sameHash: !!before && before.hash === b.hash, hash: b.hash, before: before?.hash }; });
  log("RESET " + JSON.stringify({ coversDirCount: out.steps.afterReset.coversDirCount, baseExists: out.steps.afterReset.baseExists, perBookPreserved: out.steps.afterReset.perBookPreserved, books: out.steps.afterReset.books.map(b=>({t:b.title,k:b.kind,grad:b.hasBuiltinGradient,img:b.hasImageData})) }));

  // ---------- 损坏文件 ----------
  const badFile = path.join(evid, "bad.txt");
  fs.writeFileSync(badFile, "definitely not an image", "utf8");
  out.steps.bad = await M.evalJs(H(`
    let err=null;
    try { await inv('set_default_cover',{imagePath:${JSON.stringify(badFile.replace(/\\/g, "\\\\"))}}); } catch(e){ err=String(e); }
    await sleep(500);
    return { err, hasDefault: await inv('has_default_cover') };
  `));
  out.steps.bad.coversDirCount = coversList().length;
  out.steps.bad.tmpResidue = fs.existsSync(path.join(data, "default-cover.img.tmp"));
  out.steps.bad.books = await M.evalJs(H(PROBE));
  out.steps.bad.perBookPreserved = out.steps.bad.books
    .filter((b) => b.kind === "per-book")
    .map((b) => { const before = perBookBefore.find((p) => p.id === b.id); return { id: b.id, sameHash: !!before && before.hash === b.hash }; });
  log("BAD " + JSON.stringify({ err: out.steps.bad.err, hasDefault: out.steps.bad.hasDefault, coversDirCount: out.steps.bad.coversDirCount, perBookPreserved: out.steps.bad.perBookPreserved }));

  // ---------- 非 PNG 底图：验证 MIME 按字节嗅探 ----------
  const jpg = path.join(evid, "base_new.jpg");
  fs.writeFileSync(jpg, TINY_JPEG);
  out.steps.jpeg = await M.evalJs(H(`
    let err=null;
    try { await inv('set_default_cover',{imagePath:${JSON.stringify(jpg.replace(/\\/g, "\\\\"))}}); } catch(e){ err=String(e); }
    await sleep(800);
    const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:500});
    const b=pg.rows.filter(r=>/dcf_books/i.test(r.path) && !r.cover_path)[0];
    let mime=null, svgTitle=null;
    if(b){ const t=await (await fetch('http://tidepage.localhost/cover-default/'+b.id)).text(); mime=(t.match(/data:(image\\/[a-z]+);base64/)||[])[1]||null; svgTitle=(t.match(/font-weight="700"[^>]*>([^<]*)</)||[])[1]||null; }
    return { err, hasDefault: await inv('has_default_cover'), dataMime: mime, svgTitle };
  `));
  log("JPEG " + JSON.stringify(out.steps.jpeg));

  M.close();
} catch (e) {
  out.error = String(e);
  console.error("[dcf] ERROR", e);
} finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile + "  evidence: " + evid);
}
