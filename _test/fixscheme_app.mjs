// 验证「清除本书方案」：先给本书写入方案覆盖 -> 字体被钉住 -> 点清除 -> 回到全局。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sbx = path.join(SANDBOX_ROOT, "fixscheme_sbx");
const data = path.join(SANDBOX_ROOT, "fixscheme_data");
for (const d of [sbx, data]) fs.rmSync(d, { recursive: true, force: true });
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
const DTP = path.join(sbx, "EBWebView", "DevToolsActivePort"); fs.rmSync(DTP, { force: true });
const diag = fs.openSync(path.join(SANDBOX_ROOT, "fixscheme_diag.log"), "a");
const outFile = path.join(SANDBOX_ROOT, "fixscheme_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=400)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)];
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const cfg=()=>JSON.parse(localStorage.getItem('flowread.config.v1')||'{}');
const bc=()=>JSON.parse(localStorage.getItem('flowread.bookcfg.v1')||'{}');
const fontOf=()=>{const p=q('#chapter-content p,#chapter-content div,#chapter-content font');return p?getComputedStyle(p).fontFamily:null};
const openSettings=async()=>{ (q('button[data-tip="设置"]')||q('button[aria-label="设置"]')||q('button[title="设置"]'))?.click(); await wait(()=>q('dialog[open]'),'dialog'); };
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
const out = { steps: {} };
let child = null;
try {
  const env = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FR_TEST_CDP_PORT: "0", FLOWREAD_DATA_DIR: data };
  child = spawn("cmd.exe", ["/d", "/s", "/c", "npm run tauri dev"], { cwd: REPO, env, stdio: ["ignore", diag, diag], windowsHide: true });
  let port = 0;
  for (let i = 0; i < 180; i++) { try { port = Number(fs.readFileSync(DTP, "utf8").split(/\r?\n/)[0]); if (port) break; } catch { /* wait */ } await sleep(1000); }
  log("port=" + port); if (!port) throw new Error("no port");
  let M = null;
  for (let i = 0; i < 60; i++) { try { M = await connectTarget(port, isApp); break; } catch { await sleep(1000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch { /* wait */ } await sleep(400); }

  const mobi = (process.env.FR_MOBI || "D:\\Books\\历史的观念.mobi").replace(/\\/g, "\\\\");
  out.steps.added = await M.evalJs(H(`try { return await inv('add_dropped_book',{path:"${mobi}"}); } catch(e){ return {err:String(e)}; }`));
  const id = out.steps.added.id;
  // 写入“仅本书方案”（模拟用户状态）
  out.steps.seed = await M.evalJs(H(`
    const m=bc(); m["${id}"]={ fontSize:30, overrideBookCss:true, schemeTheme:"white", schemeBg:"#FAFAF9", schemeFg:"#22252A",
      schemeFontFamily:'"Microsoft YaHei","PingFang SC",sans-serif', schemeLineHeight:1.7, schemeParaSpacing:0.9, schemeIndent:"0em", schemeTextAlign:"left" };
    localStorage.setItem('flowread.bookcfg.v1', JSON.stringify(m)); return Object.keys(m);
  `));
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(4000);
  M.close(); M = await connectTarget(port, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs("!!window.__TAURI_INTERNALS__")) break; } catch { /* wait */ } await sleep(400); }

  out.steps.pinned = await M.evalJs(H(`
    const c=await wait(()=>{const x=qa('.card,.row');return x.length?x:null;},'cards',600);
    const b=c.find(x=>x.textContent.includes('历史的观念'))||c[0];
    b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true})); await wait(()=>q('#chapter-content'),'chapter',800); await sleep(2000);
    return { font: fontOf(), ovr: q('.reader')?.classList.contains('ovr'), hasScheme: !!bc()["${id}"]?.schemeTheme };
  `));
  log("pinned: " + JSON.stringify(out.steps.pinned));

  out.steps.cleared = await M.evalJs(H(`
    await openSettings();
    const btn=qa('button.link-btn').find(b=>b.textContent.includes('清除本书方案'));
    if(!btn) return { err:'no clear button', buttons: qa('button.link-btn').map(b=>b.textContent.trim()) };
    btn.click(); await sleep(800);
    return { entry: bc()["${id}"]||null, font: fontOf() };
  `));
  log("cleared: " + JSON.stringify(out.steps.cleared));
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (child) { try { execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch { /* ignore */ } try { execFileSync("taskkill", ["/IM", "msedgewebview2.exe", "/F"], { stdio: "ignore" }); } catch { /* ignore */ } }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
