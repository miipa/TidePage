// 真机验证：taste-skill 改造（字体栈/字号阶梯/着色阴影/视图高亮/alt）。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9622);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "taste_sbx");
const data = path.join(SANDBOX_ROOT, "taste_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "taste_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a); const cv=(n)=>getComputedStyle(document.documentElement).getPropertyValue(n).trim(); const gc=(el,p)=>el?getComputedStyle(el).getPropertyValue(p).trim():null;\n${b} })()`;

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "taste_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  const p = path.join(BOOKS, "zhen_dian.epub").replace(/\\/g, "\\\\");
  await M.evalJs(H(`try { await inv('add_dropped_book',{path:"${p}"}); } catch(e){}`));
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  out.steps.font = await M.evalJs(H(`return { stack: gc(document.body,'font-family') };`));
  log("font: " + JSON.stringify(out.steps.font));

  out.steps.shadows = await M.evalJs(H(`
    return { sm: cv('--shadow-sm'), md: cv('--shadow-md'), lg: cv('--shadow-lg') };
  `));
  log("shadows: " + JSON.stringify(out.steps.shadows));

  await M.evalJs(H(`const c=await wait(()=>{const x=qa('.card,.row');return x.length?x:null;},'cards',300); return 1;`));
  out.steps.card = await M.evalJs(H(`
    const card=q('.card'); const t=card?.querySelector('.meta .t'); const a=card?.querySelector('.meta .a');
    const img=card?.querySelector('img.cover, .cover img') || card?.querySelector('img');
    return { title:{ fs:gc(t,'font-size'), wrap:gc(t,'text-wrap') }, author:{ fs:gc(a,'font-size') },
      coverAlt: img?img.getAttribute('alt'):null,
      cardShadow: gc(card,'box-shadow') };
  `));
  log("card: " + JSON.stringify(out.steps.card));

  // 视图按钮 active 状态
  out.steps.viewBtn = await M.evalJs(H(`
    const btns=qa('.view-btn');
    const grid=q('[data-testid="lib-view-grid"]'), list=q('[data-testid="lib-view-list"]');
    const before={ grid:grid?.classList.contains('active'), list:list?.classList.contains('active') };
    list?.click(); await sleep(400);
    const after={ grid:grid?.classList.contains('active'), list:list?.classList.contains('active'),
      listBg:gc(list,'background-color'), listColor:gc(list,'color') };
    return { before, after };
  `));
  log("viewBtn: " + JSON.stringify(out.steps.viewBtn));

  // 悬停（真实鼠标）
  await M.evalJs(H(`document.querySelector('[data-testid="lib-view-grid"]')?.click(); await sleep(400); return 1;`));
  const rect = await M.evalJs(H(`const c=q('.card').getBoundingClientRect(); return {x:Math.round(c.left+c.width/2), y:Math.round(c.top+c.height/2)};`));
  const before = await M.evalJs(H(`const c=q('.card'); return { tf:gc(c,'transform'), sh:gc(c,'box-shadow') };`));
  await M.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: rect.x, y: rect.y });
  await sleep(400);
  const after = await M.evalJs(H(`const c=q('.card'); return { tf:gc(c,'transform'), sh:gc(c,'box-shadow') };`));
  out.steps.hover = { before, after, lifted: before.tf !== after.tf, shadowChanged: before.sh !== after.sh };
  log("hover: " + JSON.stringify(out.steps.hover));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
