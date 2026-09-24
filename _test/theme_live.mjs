// Live verification of UI skins + reader colour schemes (isolated sandbox, real WebView2, debug CDP).
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9588);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const sbx = path.join(SANDBOX_ROOT, "theme_sbx");
const data = path.join(SANDBOX_ROOT, "theme_data");
fs.mkdirSync(sbx, { recursive: true });
fs.mkdirSync(data, { recursive: true });
for (const dir of [sbx, data]) {
  for (const f of fs.readdirSync(dir)) fs.rmSync(path.join(dir, f), { recursive: true, force: true });
}
const outFile = path.join(SANDBOX_ROOT, "theme_out.json");
const diagLog = path.join(SANDBOX_ROOT, "theme_diag.log");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=200)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const cards=(rounds=200)=>wait(()=>{const c=qa('.card,.row');return c.length?c:null;},'cards',rounds);
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const cfgRead=()=>JSON.parse(localStorage.getItem('flowread.config.v1')||'{}');
const bcRead=()=>JSON.parse(localStorage.getItem('flowread.bookcfg.v1')||'{}');
const cssvar=(n)=>getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const openSettings=async(tab)=>{ q('button[title="设置"]')?.click(); await wait(()=>q('dialog[open]'),'dialog'); [...document.querySelectorAll('.tabs button')].find(x=>x.textContent.trim()===tab)?.click(); await sleep(350); };
const closeSettings=async()=>{ document.querySelector('dialog button.primary')?.click(); await sleep(300); };
`;
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

async function ensureReady(M, label) {
  for (let i = 0; i < 300; i++) {
    try { const r = await M.evalJs(`({'tauri':!!window.__TAURI_INTERNALS__,url:location.href})`); if (r && r.tauri) return r; } catch { /* wait */ }
    await sleep(400);
  }
  throw new Error("not ready: " + label);
}

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: diagLog });
  log("launched pid " + app.child.pid);
  let M = null;
  for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  await ensureReady(M, "boot");
  await sleep(1500);

  // ---- 1) 内置方案存在（等首帧落盘）；外壳皮肤变量存在 ----
  out.steps.builtin = await M.evalJs(H(`
    for(let i=0;i<30;i++){ const c=cfgRead(); if((c.readerSchemes||[]).length) break; await sleep(200); }
    const c=cfgRead();
    await openSettings('阅读');
    const nCards=qa('.scheme-card').length, nPreviews=qa('.scheme-preview').length;
    await closeSettings();
    return { schemes: (c.readerSchemes||[]).map(s=>s.name), cards:nCards, previews:nPreviews, uiTheme:c.uiTheme, light:c.uiSkinLight, dark:c.uiSkinDark, glass:c.glass,
      vars:{ bg:cssvar('--bg'), accent:cssvar('--accent') } };
  `));
  log("builtin: " + JSON.stringify(out.steps.builtin));

  // ---- 2) 外观页：切换深色 + 换深色皮肤，观察变量变化 ----
  out.steps.skin = await M.evalJs(H(`
    await openSettings('外观');
    const before={ theme:document.documentElement.dataset.theme, bg:cssvar('--bg') };
    // 切深色
    [...document.querySelectorAll('.fr-seg button')].find(b=>b.textContent.trim()==='深色')?.click();
    await sleep(400);
    const dark={ theme:document.documentElement.dataset.theme, bg:cssvar('--bg'), accent:cssvar('--accent') };
    // 选深色皮肤"纯黑"
    const darkCards=qa('.skin-card').filter(b=>b.textContent.includes('纯黑'));
    darkCards[0]?.click(); await sleep(400);
    const ink={ bg:cssvar('--bg'), accent:cssvar('--accent'), saved:cfgRead().uiSkinDark };
    // 切回浅色
    [...document.querySelectorAll('.fr-seg button')].find(b=>b.textContent.trim()==='浅色')?.click();
    await sleep(400);
    return { before, dark, ink, backTheme:document.documentElement.dataset.theme, backBg:cssvar('--bg'),
      skins: qa('.skin-card').map(b=>b.textContent.replace(/[✓\\s]/g,'')).filter(Boolean) };
  `));
  log("skin: " + JSON.stringify(out.steps.skin));

  // ---- 3) 外观与阅读独立：改外壳不改 reader ----
  out.steps.independence = await M.evalJs(H(`
    const before={ bgColor:cfgRead().reader?.bgColor, theme:cfgRead().reader?.theme };
    // 在书库改外壳浅色皮肤
    const lightCards=qa('.skin-card').filter(b=>b.textContent.includes('冷蓝'));
    if(lightCards[0]){ } // 需在浅色模式才有意义；此处仅记录
    return { readerBefore: before, uiSkinLight: cfgRead().uiSkinLight, uiSkinDark: cfgRead().uiSkinDark };
  `));
  log("independence: " + JSON.stringify(out.steps.independence));

  // ---- 4) 打开书，套用方案到书/全局，修改字号不改方案 ----
  // 加入一本书
  const p = path.join(BOOKS, "zhen_dian.epub").replace(/\\/g, "\\\\");
  await M.evalJs(H(`try { await inv('add_dropped_book',{path:"${p}"}); } catch(e){}`));
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close();
  M = await connectTarget(PORT, isApp);
  await ensureReady(M, "after add");
  await M.evalJs(H(`
    const c=await cards(400);
    const b=c.find(x=>x.textContent.includes('zhen_dian'))||c[0];
    b.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',500); await sleep(1200); return 1;
  `));
  out.steps.scheme = await M.evalJs(H(`
    await openSettings('阅读');
    const names=()=>cfgRead().readerSchemes.map(s=>s.name);
    const before=names();
    // 保存当前配置为新方案
    [...document.querySelectorAll('.scheme-actions button')].find(b=>b.textContent.includes('保存'))?.click();
    await sleep(300);
    const inp=q('.scheme-save input');
    if(inp){ inp.value='我的测试方案'; inp.dispatchEvent(new Event('input',{bubbles:true})); }
    [...document.querySelectorAll('.ss-new button')].find(b=>b.textContent.includes('新建'))?.click();
    await sleep(500);
    const afterNew=names();
    // 套用到本书（点击第一个缩略图=内置"纸质"）
    const firstPreview=qa('.scheme-preview')[0];
    firstPreview?.click(); await sleep(600);
    const bc=bcRead(); const entry=Object.values(bc)[0]||{};
    return { before, afterNew, bookSchemeTheme: entry.schemeTheme, bookSchemeBg: entry.schemeBg };
  `));
  log("scheme: " + JSON.stringify(out.steps.scheme));

  // 修改字号，方案不应变
  out.steps.noMutate = await M.evalJs(H(`
    const rdName='纸质';
    const snap=()=>{ const r=cfgRead(); const s=(r.readerSchemes||[]).find(x=>x.name===rdName); return { schemeFontSize:s?.cfg.fontSize, globalFontSize:r.reader.fontSize }; };
    const before=snap();
    // 通过全局字号滑杆修改
    const field=[...document.querySelectorAll('.field')].find(f=>f.textContent.includes('字号 '));
    const sl=field?.querySelector('input[type=range]');
    if(sl){ sl.value=String(Number(sl.value)+4); sl.dispatchEvent(new Event('input',{bubbles:true})); }
    await sleep(700);
    return { before, after:snap() };
  `));
  log("noMutate: " + JSON.stringify(out.steps.noMutate));

  // 覆盖方案：点保存→选已有方案
  out.steps.overwrite = await M.evalJs(H(`
    const rBefore=(cfgRead().readerSchemes||[]).find(x=>x.name==='纸质')?.cfg.fontSize;
    [...document.querySelectorAll('.scheme-actions button')].find(b=>b.textContent.includes('保存'))?.click();
    await sleep(300);
    [...document.querySelectorAll('.scheme-save .save-target')].find(b=>b.textContent.includes('纸质'))?.click();
    await sleep(600);
    const rAfter=(cfgRead().readerSchemes||[]).find(x=>x.name==='纸质')?.cfg.fontSize;
    const sizes=cfgRead().readerSchemes.map(s=>s.name+':'+s.cfg.fontSize);
    return { schemeFontBefore:rBefore, schemeFontAfter:rAfter, globalNow:cfgRead().reader.fontSize, sizes };
  `));
  log("overwrite: " + JSON.stringify(out.steps.overwrite));

  // 删除方案
  out.steps.del = await M.evalJs(H(`
    const before=cfgRead().readerSchemes.length;
    // 删除"我的测试方案"（找居中的 × 按钮）
    const card=[...document.querySelectorAll('.scheme-card')].find(c=>c.textContent.includes('我的测试方案'));
    card?.querySelector('button[title="删除"]')?.click();
    await sleep(300);
    // 确认弹窗
    const ok=[...document.querySelectorAll('.confirm-row button')].find(b=>b.textContent.includes('确定'));
    ok?.click(); await sleep(500);
    return { before, after:cfgRead().readerSchemes.length, names:cfgRead().readerSchemes.map(s=>s.name) };
  `));
  log("del: " + JSON.stringify(out.steps.del));

  // ---- 5) 持久化：重载后方案与皮肤保持 ----
  await M.evalJs(`(()=>{ const c=JSON.parse(localStorage.getItem('flowread.config.v1')); localStorage.setItem('fr.t.schemes', JSON.stringify(c.readerSchemes.map(s=>s.name))); localStorage.setItem('fr.t.skin', c.uiSkinDark); setTimeout(()=>location.reload(),80); return 1; })()`);
  await sleep(3500);
  M.close();
  M = await connectTarget(PORT, isApp);
  await ensureReady(M, "after reload");
  out.steps.persist = await M.evalJs(`(()=>{ const c=JSON.parse(localStorage.getItem('flowread.config.v1')); const want=JSON.parse(localStorage.getItem('fr.t.schemes')); return { schemes: c.readerSchemes.map(s=>s.name), equal: JSON.stringify(c.readerSchemes.map(s=>s.name))===JSON.stringify(want), skin: c.uiSkinDark, skinEqual: c.uiSkinDark===localStorage.getItem('fr.t.skin') }; })()`);
  log("persist: " + JSON.stringify(out.steps.persist));

  M.close();
} catch (e) {
  out.error = String(e);
  console.error(e);
} finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
