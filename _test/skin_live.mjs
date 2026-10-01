// 真机验证：5 套新皮肤 + 旧 id 迁移 + 玻璃移除 + 按钮对比度。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9611);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "skin_sbx");
const data = path.join(SANDBOX_ROOT, "skin_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "skin_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const cfgRead=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}'); const cv=(n)=>getComputedStyle(document.documentElement).getPropertyValue(n).trim(); const openSettings=async(t)=>{ q('button[title="设置"]')?.click(); await wait(()=>q('dialog[open]'),'dialog'); [...document.querySelectorAll('.tabs button')].find(x=>x.textContent.trim()===t)?.click(); await sleep(350); };\n${b} })()`;

const out = { steps: {} };
let app = null;
try {
  // 预置旧皮肤 id，验证迁移
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "skin_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await sleep(800);

  // 写入旧皮肤 id（eye/ink），验证迁移
  await M.evalJs(`(()=>{ const c=JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}'); c.uiSkinLight='eye'; c.uiSkinDark='ink'; c.uiTheme='system'; c.glass=true; localStorage.setItem('tidepage.config.v1', JSON.stringify(c)); setTimeout(()=>location.reload(),100); return 1; })()`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  out.steps.migrate = await M.evalJs(H(`
    const c=cfgRead();
    return { light:c.uiSkinLight, dark:c.uiSkinDark, glassInCfg: 'glass' in c, glassVar: cv('--glass') || '(none)' };
  `));
  log("migrate: " + JSON.stringify(out.steps.migrate));

  out.steps.skins = await M.evalJs(H(`
    await openSettings('外观');
    const light=qa('.skin-card').filter(b=>['奶油','麻布','冷雾'].includes(b.textContent.replace(/[✓\\s]/g,''))).map(b=>b.textContent.replace(/[✓\\s]/g,''));
    return { cards: qa('.skin-card').map(b=>b.textContent.replace(/[✓\\s]/g,'')).filter(Boolean),
      hasGlassSwitch: !!qa('input.fr-switch').length, accent: cv('--accent'), bg: cv('--bg'), fg: cv('--fg') };
  `));
  log("skins: " + JSON.stringify(out.steps.skins));

  // 依次切换每套皮肤，记录变量差异
  out.steps.each = await M.evalJs(H(`
    const names=['奶油','麻布','冷雾','夜行','石墨'];
    const res={};
    const darkByName={夜行:1,石墨:1};
    for(const n of names){
      // 选明暗模式
      const modeBtns=[...document.querySelectorAll('.fr-seg button')];
      modeBtns.find(b=>b.textContent.trim()===(darkByName[n]?'深色':'浅色'))?.click();
      await sleep(250);
      const card=qa('.skin-card').find(b=>b.textContent.replace(/[✓\\s]/g,'')===n);
      card?.click(); await sleep(350);
      res[n]={ bg:cv('--bg'), accent:cv('--accent'), fg:cv('--fg'), accentFg:cv('--accent-fg'), radius:cv('--radius-card'), shadow:cv('--shadow-card') };
    }
    return res;
  `));
  log("each: " + JSON.stringify(out.steps.each));

  // 按钮对比度：主按钮与图标按钮
  out.steps.buttons = await M.evalJs(H(`
    // 主按钮（设置弹窗"关闭"是 primary）
    const primary=document.querySelector('dialog button.primary');
    const icon=qa('dialog button.icon')[0] || qa('button.icon')[0];
    const gc=(el,p)=>el?getComputedStyle(el).getPropertyValue(p).trim():null;
    return {
      primary: primary?{ bg:gc(primary,'background-color'), color:gc(primary,'color') }:null,
      icon: icon?{ color:gc(icon,'color'), opacity:gc(icon,'opacity') }:null,
      disabledOpacity: (()=>{ const b=document.querySelector('button:disabled'); return b?gc(b,'opacity'):null; })(),
    };
  `));
  log("buttons: " + JSON.stringify(out.steps.buttons));

  // 玻璃已移除：dialog 不应有 backdrop-filter
  out.steps.glassRemoved = await M.evalJs(H(`
    const d=document.querySelector('dialog');
    const bf=d?getComputedStyle(d).backdropFilter||getComputedStyle(d).webkitBackdropFilter:'';
    return { backdropFilter: bf || '(none)', htmlGlassAttr: document.documentElement.getAttribute('data-glass') };
  `));
  log("glassRemoved: " + JSON.stringify(out.steps.glassRemoved));

  // 持久化
  await M.evalJs(`localStorage.setItem('fr.t.light', JSON.parse(localStorage.getItem('tidepage.config.v1')).uiSkinLight); localStorage.setItem('fr.t.dark', JSON.parse(localStorage.getItem('tidepage.config.v1')).uiSkinDark); setTimeout(()=>location.reload(),80);1`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  out.steps.persist = await M.evalJs(`(()=>{ const c=JSON.parse(localStorage.getItem('tidepage.config.v1')); return { light:c.uiSkinLight, dark:c.uiSkinDark, lightOk:c.uiSkinLight===localStorage.getItem('fr.t.light'), darkOk:c.uiSkinDark===localStorage.getItem('fr.t.dark') }; })()`);
  log("persist: " + JSON.stringify(out.steps.persist));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
