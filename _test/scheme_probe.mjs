import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";
const PORT = Number(process.env.FR_PORT || 9599);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "probe_sbx");
const data = path.join(SANDBOX_ROOT, "probe_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const openSettings=async(tab)=>{ q('[data-testid="lib-btn-settings"]')?.click(); await wait(()=>q('dialog[open]'),'dialog'); [...document.querySelectorAll('.tabs button')].find(x=>x.textContent.trim()===tab)?.click(); await sleep(350); };\n${b} })()`;
let app=null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "probe.log") });
  let M=null; for(let i=0;i<240;i++){try{M=await connectTarget(PORT,isApp);break}catch{await sleep(2000)}}
  for(let i=0;i<300;i++){try{const r=await M.evalJs(`!!window.__TAURI_INTERNALS__`);if(r)break}catch{}await sleep(400)}
  await sleep(800);
  const r = await M.evalJs(H(`
    const out0={ settingsBtns: qa('[data-testid="lib-btn-settings"]').length, url: location.href };
    for(let k=0;k<20 && !q('dialog[open]');k++){ q('[data-testid="lib-btn-settings"]')?.click(); await sleep(500); }
    out0.opened=!!q('dialog[open]');
    out0.tabs=qa('.tabs button').map(b=>b.textContent.trim());
    [...document.querySelectorAll('.tabs button')].find(x=>x.textContent.trim()==='阅读')?.click(); await sleep(350);
    const out=out0;
    out.saveBtn = qa('.scheme-actions button').map(b=>b.textContent.trim());
    out.cards = qa('.scheme-card').length;
    out.previews = qa('.scheme-preview').length;
    // 点击保存
    qa('.scheme-actions button')[0]?.click(); await sleep(400);
    out.saveOpen = !!q('.scheme-save');
    out.modalBtns = qa('.scheme-save .save-target').map(b=>b.textContent.trim());
    out.newBtn = qa('.ss-new button').map(b=>b.textContent.trim());
    out.hasInput = !!q('.ss-new input');
    // 新建
    const inp=q('.ss-new input'); if(inp){ inp.value='新方案X'; inp.dispatchEvent(new Event('input',{bubbles:true})); }
    await sleep(100);
    let clickErr=null;
    try { qa('.ss-new button')[0]?.click(); } catch(e){ clickErr=String(e); }
    await sleep(600);
    out.toast=qa('.toast-card').map(t=>t.textContent).join(' | ');
    out.nameInputVal=q('.ss-new input')?.value;
    const raw=JSON.parse(localStorage.getItem('tidepage.config.v1'));
    out.clickErr=clickErr;
    out.rawSchemeType=Array.isArray(raw.readerSchemes)?'array':typeof raw.readerSchemes;
    out.rawSchemes=raw.readerSchemes;
    out.afterNewNames = (raw.readerSchemes||[]).map(s=>s.name);
    out.saveClosed = !q('.scheme-save');
    return out;
  `));
  console.log(JSON.stringify(r,null,1));
  M.close();
} finally { if(app){killTree(app.child.pid); await sleep(1200); sweepSandboxWebviews("fr_test_profiles");} }
