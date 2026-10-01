// 验证设置弹窗新外壳：左导航+内容、导航切换、搜索过滤、各标签可渲染。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9655);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "set_sbx"); const data = path.join(SANDBOX_ROOT, "set_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "set_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const gc=(el,p)=>el?getComputedStyle(el).getPropertyValue(p).trim():null; const rect=(el)=>{if(!el)return null;const r=el.getBoundingClientRect();return{x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}};\n${b} })()`;

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "set_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(800);

  out.steps.shell = await M.evalJs(H(`
    for(let k=0;k<20 && !q('dialog[open]');k++){ q('button[title="设置"]')?.click(); await sleep(400); }
    const root=q('.settings-root'); const head=q('.set-head'); const nav=q('.set-nav'); const content=q('.set-content'); const foot=q('.set-foot');
    return { root:rect(root), head:!!head, nav:rect(nav), content:rect(content), foot:!!foot,
      navItems:qa('.set-nav-item').length, groups:qa('.set-nav-group').map(g=>g.textContent.trim()),
      hasSearch: !!q('.set-search input'), crumb:q('.set-crumb-now')?.textContent };
  `));
  log("shell: " + JSON.stringify(out.steps.shell));

  out.steps.switch = await M.evalJs(H(`
    const before={ crumb:q('.set-crumb-now')?.textContent, active:q('.set-nav-item.active')?.textContent.trim() };
    // 点"漫画模式"
    [...document.querySelectorAll('.set-nav-item')].find(b=>b.textContent.includes('漫画模式'))?.click();
    await sleep(400);
    const after={ crumb:q('.set-crumb-now')?.textContent, active:q('.set-nav-item.active')?.textContent.trim(), hasMangaFields: !!q('.set-content .field') };
    return { before, after };
  `));
  log("switch: " + JSON.stringify(out.steps.switch));

  out.steps.search = await M.evalJs(H(`
    const inp=q('.set-search input'); inp.value='语'; inp.dispatchEvent(new Event('input',{bubbles:true}));
    await sleep(300);
    const items=qa('.set-nav-item').map(b=>b.textContent.trim());
    inp.value=''; inp.dispatchEvent(new Event('input',{bubbles:true})); await sleep(200);
    return { filtered:items };
  `));
  log("search: " + JSON.stringify(out.steps.search));

  out.steps.tabs = await M.evalJs(H(`
    const names=['界面外观','阅读排版','漫画模式','语音朗读','分章规则','AI 助手','按键与手势','WebDAV','其他与关于'];
    const res={};
    for(const n of names){
      [...document.querySelectorAll('.set-nav-item')].find(b=>b.textContent.includes(n))?.click();
      await sleep(250);
      res[n]={ fields: qa('.set-content .field').length, crumb:q('.set-crumb-now')?.textContent, contentH:Math.round(q('.set-content')?.scrollHeight||0) };
    }
    return res;
  `));
  log("tabs: " + JSON.stringify(out.steps.tabs));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
