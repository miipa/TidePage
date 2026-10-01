// 6e 验证：设置里默认封面控件三态截图 + 换底图后书库刷新 + 缓存文件数 + 损坏文件错误处理。
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9833);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "dc_sbx"); const data = path.join(SANDBOX_ROOT, "dc_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const evid = path.join(SANDBOX_ROOT, "dc_evidence"); fs.rmSync(evid, { recursive: true, force: true }); fs.mkdirSync(evid, { recursive: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=250)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;
const SHOT = async (M, n) => { try { const s = await M.call("Page.captureScreenshot", { format: "png" }); if (s?.data) { fs.writeFileSync(path.join(evid, n), Buffer.from(s.data, "base64")); return true; } } catch {} return false; };
const countCovers = () => { const d = path.join(data, "covers"); return fs.existsSync(d) ? fs.readdirSync(d).length : 0; };
const hasBase = () => fs.existsSync(path.join(data, "default-cover.img"));

// 生成一张可用的 PNG
function crc32(buf){const T=[];for(let n=0;n<256;n++){let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;T[n]=c>>>0}let c=0xffffffff;for(let i=0;i<buf.length;i++)c=T[(c^buf[i])&0xff]^(c>>>8);return (c^0xffffffff)>>>0}
function chunk(t,b){const l=Buffer.alloc(4);l.writeUInt32BE(b.length,0);const tt=Buffer.from(t,"ascii");const cr=Buffer.alloc(4);cr.writeUInt32BE(crc32(Buffer.concat([tt,b])),0);return Buffer.concat([l,tt,b,cr])}
function makePng(w,h,px){const raw=Buffer.alloc((w*3+1)*h);let o=0;for(let y=0;y<h;y++){raw[o++]=0;for(let x=0;x<w;x++){const[r,g,b]=px(x,y);raw[o++]=r;raw[o++]=g;raw[o++]=b}}const ih=Buffer.alloc(13);ih.writeUInt32BE(w,0);ih.writeUInt32BE(h,4);ih[8]=8;ih[9]=2;return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk("IHDR",ih),chunk("IDAT",zlib.deflateSync(raw)),chunk("IEND",Buffer.alloc(0))])}

const out = { steps: {} };
let app = null;
try {
  const goodPng = path.join(evid, "base.png");
  fs.writeFileSync(goodPng, makePng(400, 600, (x, y) => [30 + (x % 200), 60, 120 + (y % 100)]));
  const badFile = path.join(evid, "bad.txt");
  fs.writeFileSync(badFile, "this is definitely not an image", "utf8");

  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "dc_diag.log") });
  console.log("[dc] launched pid", app.child.pid);
  let M = null; for (let i = 0; i < 150; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 200; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 打开 设置 → 外观
  await M.evalJs(H(`
    q('button[title="设置"]')?.click();
    await wait(()=>q('dialog[open]'),'dlg');
    [...document.querySelectorAll('.set-nav-item')].find(x=>x.textContent.includes('界面外观'))?.click();
    await sleep(600); return 1;
  `));
  out.steps.initial = await M.evalJs(H(`
    const img=q('.cover-preview');
    return { hasCtl:!!q('.cover-ctl'), previewSrc: img?img.getAttribute('src'):null, btns:qa('.cover-ctl-acts button').map(b=>b.textContent.trim()) };
  `));
  console.log("[dc] initial", JSON.stringify(out.steps.initial));
  await SHOT(M, "state1_builtin.png");

  // 选图（直接用命令 + 刷新状态；文件选择器无法在无头下操作）
  out.steps.set = await M.evalJs(H(`
    await inv('set_default_cover',{imagePath:${JSON.stringify(goodPng.replace(/\\/g, "\\\\"))}});
    const has=await inv('has_default_cover');
    // 触发状态刷新：切走再切回外观
    [...document.querySelectorAll('.set-nav-item')].find(x=>x.textContent.includes('阅读排版'))?.click(); await sleep(200);
    [...document.querySelectorAll('.set-nav-item')].find(x=>x.textContent.includes('界面外观'))?.click(); await sleep(600);
    return { hasDefault:has, tipHasCustom: q('.cover-ctl .tip')?.textContent?.includes('自定义')||false };
  `));
  console.log("[dc] set", JSON.stringify(out.steps.set));
  await SHOT(M, "state2_custom.png");

  out.steps.afterSetFiles = { coversDirFiles: countCovers(), baseExists: hasBase() };

  // 关设置，看库封面 & 缓存文件数
  out.steps.library = await M.evalJs(H(`
    q('dialog button.primary')?.click(); await sleep(400);
    return 1;
  `));
  await SHOT(M, "library_custom.png");

  // 损坏文件 → 报错、底图不变
  const beforeBase = fs.existsSync(path.join(data, "default-cover.img")) ? fs.readFileSync(path.join(data, "default-cover.img")) : null;
  out.steps.badErr = await M.evalJs(H(`
    let err=null;
    try { await inv('set_default_cover',{imagePath:${JSON.stringify(badFile.replace(/\\/g, "\\\\"))}}); } catch(e){ err=String(e); }
    return { err, hasStill: await inv('has_default_cover') };
  `));
  const afterBase = fs.existsSync(path.join(data, "default-cover.img")) ? fs.readFileSync(path.join(data, "default-cover.img")) : null;
  out.steps.badErr.baseUnchanged = beforeBase && afterBase && beforeBase.equals(afterBase);
  out.steps.badErr.tmpResidue = fs.existsSync(path.join(data, "default-cover.img.tmp"));
  console.log("[dc] badErr", JSON.stringify(out.steps.badErr));

  // 恢复内置
  out.steps.reset = await M.evalJs(H(`
    await inv('clear_default_cover');
    const has=await inv('has_default_cover');
    return { hasDefault:has };
  `));
  out.steps.afterResetFiles = { coversDirFiles: countCovers(), baseExists: hasBase() };
  console.log("[dc] reset", JSON.stringify(out.steps.reset));
  await SHOT(M, "state3_builtin_again.png");

  console.log("[dc] files", JSON.stringify({ afterSetFiles: out.steps.afterSetFiles, afterResetFiles: out.steps.afterResetFiles }));
  M.close();
} catch (e) { out.error = String(e); console.error("[dc] ERROR", String(e)); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(path.join(SANDBOX_ROOT, "dc_out.json"), JSON.stringify(out, null, 1));
  console.log("evidence:", evid);
}
