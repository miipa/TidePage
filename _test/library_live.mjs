// 书库页观感重做验证：布局不变、token 生效、悬停/圆角/间距符合规范。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9600);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "lib_sbx");
const data = path.join(SANDBOX_ROOT, "lib_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "lib_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=200)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const cards=(rounds=200)=>wait(()=>{const c=qa('.card,.row');return c.length?c:null;},'cards',rounds);
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const cs=(el,p)=>el?getComputedStyle(el).getPropertyValue(p).trim():null;
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
async function ensureReady(M, label) {
  for (let i = 0; i < 300; i++) { try { const r = await M.evalJs(`!!window.__TAURI_INTERNALS__`); if (r) return; } catch {} await sleep(400); }
  throw new Error("not ready " + label);
}

const out = { steps: {} };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "lib_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  await ensureReady(M, "boot");
  const p = path.join(BOOKS, "zhen_dian.epub").replace(/\\/g, "\\\\");
  await M.evalJs(H(`try { await inv('add_dropped_book',{path:"${p}"}); } catch(e){}`));
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close();
  M = await connectTarget(PORT, isApp);
  await ensureReady(M, "after add");

  out.steps.library = await M.evalJs(H(`
    await cards(400);
    const lib=q('.library'), tb=q('.toolbar'), grid=q('.grid');
    const card=q('.card'), cover=card?.querySelector('.cover');
    const t=card?.querySelector('.meta .t'), a=card?.querySelector('.meta .a');
    const viewBtn=qa('.statusbar .icon');
    return {
      layout: { library:!!lib, toolbar:!!tb, grid:!!grid, cardGrid: !!q('.grid-inner') },
      card: { radius:cs(card,'border-radius'), padding:cs(card,'padding'), boxShadow:cs(card,'box-shadow') },
      cover: { radius:cs(cover,'border-radius'), outline:cs(cover,'outline-width') },
      title: { fs:cs(t,'font-size'), fw:cs(t,'font-weight') },
      author: { fs:cs(a,'font-size'), color:cs(a,'color') },
      toolbarPad: cs(tb,'padding'),
      gridPad: cs(grid,'padding'),
      fmt: (()=>{ const f=card?.querySelector('.fmt'); return f?{fs:cs(f,'font-size'), ls:cs(f,'letter-spacing'), radius:cs(f,'border-radius')}:null; })(),
      accentVar: getComputedStyle(document.documentElement).getPropertyValue('--accent').trim(),
    };
  `));
  log("library: " + JSON.stringify(out.steps.library));

  // 悬停态：用真实鼠标移动（CDP Input）触发 CSS :hover
  const cardRect = await M.evalJs(H(`const c=q('.card').getBoundingClientRect(); return {x:Math.round(c.left+c.width/2), y:Math.round(c.top+c.height/2)};`));
  out.steps.hover = await (async () => {
    const before = await M.evalJs(H(`const c=q('.card'); return { bg:cs(c,'background-color'), tf:cs(c,'transform'), sh:cs(c,'box-shadow'), imgTf:cs(c.querySelector('.cover img'),'transform') };`));
    await M.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: cardRect.x, y: cardRect.y });
    await sleep(400);
    const after = await M.evalJs(H(`const c=q('.card'); return { bg:cs(c,'background-color'), tf:cs(c,'transform'), sh:cs(c,'box-shadow'), imgTf:cs(c.querySelector('.cover img'),'transform') };`));
    return { before, after, lifted: before.tf !== after.tf, bgChanged: before.bg !== after.bg, shadowChanged: before.sh !== after.sh, imgScaled: before.imgTf !== after.imgTf };
  })();
  log("hover: " + JSON.stringify(out.steps.hover));

  // 列表视图
  out.steps.list = await M.evalJs(H(`
    [...document.querySelectorAll('.statusbar button')].find(b=>b.getAttribute('title')==='列表视图')?.click();
    await sleep(500);
    const row=q('.row');
    return { list:!!q('.list-inner'), rows:qa('.row').length, rowRadius:cs(row,'border-radius'), rowH:cs(row,'height') };
  `));
  log("list: " + JSON.stringify(out.steps.list));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
