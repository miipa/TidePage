// 应用级(真 WebView2)验证：临时放开临时调试端口(0) + DevToolsActivePort 定位真实端口。
// 覆盖：真实 MOBI -> [fixfont] injected stripped>0、READ_CSS、改字体后不重启 READ_CSS 新值、computed style。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sbx = path.join(SANDBOX_ROOT, "fixapp_sbx");
const data = path.join(SANDBOX_ROOT, "fixapp_data");
for (const d of [sbx, data]) fs.rmSync(d, { recursive: true, force: true });
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
const DTP = path.join(sbx, "EBWebView", "DevToolsActivePort");
fs.rmSync(DTP, { force: true });
const diag = fs.openSync(path.join(SANDBOX_ROOT, "fixapp_diag.log"), "a");
const outFile = path.join(SANDBOX_ROOT, "fixapp_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=400)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)];
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const field=(t)=>[...document.querySelectorAll('.field')].find(f=>f.textContent.includes(t));
const openSettings=async()=>{ (q('button[data-tip="设置"]')||q('button[aria-label="设置"]')||q('button[title="设置"]'))?.click(); await wait(()=>q('dialog[open]'),'dialog'); };
const closeSettings=async()=>{ document.querySelector('dialog button.primary')?.click(); await sleep(300); };
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;

const out = { steps: {} };
let child = null;
try {
  const env = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FR_TEST_CDP_PORT: "0", TIDEPAGE_DATA_DIR: data };
  child = spawn("cmd.exe", ["/d", "/s", "/c", "npm run tauri dev"], { cwd: REPO, env, stdio: ["ignore", diag, diag], windowsHide: true });
  log("spawned cmd pid " + child.pid);

  let port = 0;
  for (let i = 0; i < 180; i++) {
    try { const t = fs.readFileSync(path.join(sbx, "EBWebView", "DevToolsActivePort"), "utf8").split(/\r?\n/)[0]; port = Number(t); if (port) break; } catch { /* wait */ }
    await sleep(1000);
  }
  log("webview DevTools port = " + port);
  if (!port) throw new Error("no DevToolsActivePort from webview");

  let M = null;
  for (let i = 0; i < 60; i++) { try { M = await connectTarget(port, isApp); break; } catch { await sleep(1000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch { /* wait */ } await sleep(400); }
  log("app ready");

  const mobi = (process.env.FR_MOBI || "D:\\Books\\历史的观念.mobi").replace(/\\/g, "\\\\");
  out.steps.added = await M.evalJs(H(`try { return await inv('add_dropped_book',{path:"${mobi}"}); } catch(e){ return {err:String(e)}; }`));
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(4000);
  M.close(); M = await connectTarget(port, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch { /* wait */ } await sleep(400); }

  out.steps.open = await M.evalJs(H(`
    const c=await wait(()=>{const x=qa('.card,.row');return x.length?x:null;},'cards',600);
    const b=c.find(x=>x.textContent.includes('历史的观念'))||c[0];
    b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',800); await sleep(2500);
    return { title: b.textContent.trim().slice(0,20) };
  `));
  log("open: " + JSON.stringify(out.steps.open));

  // 跳到字体标签更多的一章，拿到更有代表性的 stripped
  out.logs1 = M.consoleLogs().filter((l) => l.includes("fixfont") || l.includes("READ_CSS"));
  const inj = out.logs1.map((l) => l.match(/\[fixfont\] injected, stripped=(\d+), html=(\d+)/)).filter(Boolean);
  out.injected = inj.map((m) => ({ stripped: Number(m[1]), html: Number(m[2]) }));
  log("injected: " + JSON.stringify(out.injected));

  out.steps.before = await M.evalJs(H(`
    const cc=q('#chapter-content');
    return { fonts: qa('#chapter-content font').length,
      fontsWithAttrs: qa('#chapter-content font').filter(f=>f.hasAttribute('face')||f.hasAttribute('size')||f.hasAttribute('color')).length,
      hasFixStyle: !!document.getElementById('__read_fix') };
  `));
  log("before: " + JSON.stringify(out.steps.before));

  out.steps.set = await M.evalJs(H(`
    await openSettings();
    const fi=qa('input.font-value')[0]; fi.value='SimSun'; fi.dispatchEvent(new Event('input',{bubbles:true})); fi.dispatchEvent(new Event('change',{bubbles:true}));
    await sleep(200);
    const r=field('字号')?.querySelector('input[type=range]'); if(r){ r.value='40'; r.dispatchEvent(new Event('input',{bubbles:true})); r.dispatchEvent(new Event('change',{bubbles:true})); }
    await sleep(400);
    const cfg=JSON.parse(localStorage.getItem('tidepage.config.v1'));
    return { font: cfg.reader.fontFamily, size: cfg.reader.fontSize };
  `));
  log("set: " + JSON.stringify(out.steps.set));

  await M.evalJs(H(`window.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',code:'ArrowRight',bubbles:true})); await sleep(1500); await closeSettings(); await sleep(400); return 1;`)).catch(() => {});
  const reads = M.consoleLogs().filter((l) => l.includes("READ_CSS"));
  out.readCss = reads.slice(-3);
  log("readCss(last): " + JSON.stringify(reads.slice(-1)));

  out.steps.after = await M.evalJs(H(`
    const cc=q('#chapter-content'); const p=cc.querySelector('p')||cc; const cs=getComputedStyle(p);
    return { pFontFamily: cs.fontFamily, pFontSize: cs.fontSize,
      ccBg: getComputedStyle(cc).backgroundColor, bodyBg: getComputedStyle(document.body).backgroundColor,
      styleLen: (document.getElementById('__read_fix')?.textContent||'').length,
      fontsWithAttrs: qa('#chapter-content font').filter(f=>f.hasAttribute('face')||f.hasAttribute('size')||f.hasAttribute('color')).length };
  `));
  log("after: " + JSON.stringify(out.steps.after));
  M.close();
} catch (e) {
  out.error = String(e); console.error(e);
} finally {
  if (child) {
    try { execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch { /* ignore */ }
    try { execFileSync("taskkill", ["/IM", "msedgewebview2.exe", "/F"], { stdio: "ignore" }); } catch { /* ignore */ }
  }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
