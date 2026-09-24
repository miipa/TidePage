// E 验证：搜索框 Portal 到 body；拖拽可到视口边缘（clamp 0..innerW/H），不被祖先裁剪。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";
const PORT = Number(process.env.FR_PORT || 9700);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "e2_sbx"); const data = path.join(SANDBOX_ROOT, "e2_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "e2_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;
let app = null; const out = {};
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "e2_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  const p = path.join(BOOKS, "zhen_dian.epub").replace(/\\/g, "\\\\");
  await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${p}"});}catch(e){}`));
  await sleep(1500); await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  // 打开书
  await M.evalJs(H(`const c=await (async()=>{for(let i=0;i<300;i++){const x=qa('.card,.row');if(x.length)return x;await sleep(100);}return [];})(); c[0].dispatchEvent(new MouseEvent('dblclick',{bubbles:true})); return 1;`));
  await sleep(2500);
  // 真实鼠标拖选一段文字
  const pt = await M.evalJs(H(`
    for(let i=0;i<60;i++){ if(q('#chapter-content')) break; await sleep(200); }
    const root=q('#chapter-content'); if(!root) return null;
    const w=document.createTreeWalker(root,NodeFilter.SHOW_TEXT); let nd;
    while(nd=w.nextNode()){ if(nd.nodeValue && nd.nodeValue.trim().length>16) break; }
    if(!nd) return null;
    const r=document.createRange(); r.selectNodeContents(nd);
    const rc=(r.getClientRects()[0])||r.getBoundingClientRect();
    return { x1:Math.round(rc.left+2), y1:Math.round(rc.top+rc.height/2), x2:Math.round(Math.min(rc.right-2,rc.left+140)), y2:Math.round(rc.top+rc.height/2) };
  `));
  // 关键：reader 的 pointerdown 会清掉旧选区，因此顺序必须是
  // pointerdown → 建立选区 → pointerup（moved>4）才被判定为"拖拽出的有效选区"。
  await M.evalJs(H(`
    const root=q('#chapter-content'); const el=q('.content');
    const w=document.createTreeWalker(root,NodeFilter.SHOW_TEXT); let nd;
    while(nd=w.nextNode()){ if(nd.nodeValue && nd.nodeValue.trim().length>16) break; }
    if(!nd) return 0;
    const r=document.createRange(); r.selectNodeContents(nd);
    const rc=(r.getClientRects()[0])||r.getBoundingClientRect();
    const x1=Math.round(rc.left+2), y1=Math.round(rc.top+rc.height/2);
    const x2=Math.round(Math.min(rc.right-2, rc.left+140)), y2=y1;
    el.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true,button:0,buttons:1,pointerId:77,pointerType:'mouse',isPrimary:true,clientX:x1,clientY:y1}));
    await sleep(30);
    const sel=window.getSelection(); sel.removeAllRanges(); sel.addRange(r);
    el.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,cancelable:true,button:0,buttons:0,pointerId:77,pointerType:'mouse',isPrimary:true,clientX:x2,clientY:y2}));
    await sleep(500);
    return 1;
  `));
  // 开发期钩子：直接打开搜索框（DEV only），稳定验证 portal + 拖动。
  await M.evalJs(H(`
    if(window.__frDebug?.openSearch){ window.__frDebug.openSearch('测试'); }
    await sleep(800); return 1;
  `));
  out.popup = await M.evalJs(H(`
    const pop=q('.sb-pop');
    if(!pop) return { err:'no popup' };
    // 祖先扫描：找带 transform/filter/contain/overflow 的祖先
    const bad=[]; let el=pop.parentElement;
    while(el && el!==document.documentElement){
      const cs=getComputedStyle(el);
      if(cs.transform!=='none'||cs.filter!=='none'||(cs.contain&&cs.contain!=='none')||cs.overflow!=='visible')
        bad.push({ tag:el.tagName, cls:el.className, transform:cs.transform, overflow:cs.overflow, contain:cs.contain });
      el=el.parentElement;
    }
    return { parentTag: pop.parentElement?.tagName, parentIsBody: pop.parentElement===document.body, ancestryIssues: bad,
      pos:getComputedStyle(pop).position, z:getComputedStyle(pop).zIndex };
  `));
  // 拖动：真实鼠标从头部按住拖到视口左上角之外方向
  const head = await M.evalJs(H(`const h=q('.sb-head'); const r=h.getBoundingClientRect(); return {x:Math.round(r.left+30), y:Math.round(r.top+14)};`));
  out.before = await M.evalJs(H(`const r=q('.sb-pop').getBoundingClientRect(); return {x:Math.round(r.x),y:Math.round(r.y)};`));
  await M.call("Input.dispatchMouseEvent", { type: "mousePressed", x: head.x, y: head.y, button: "left", clickCount: 1 });
  await M.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 5, y: 5, button: "left", buttons: 1 });
  await M.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: 5, y: 5, button: "left", clickCount: 1 });
  await sleep(400);
  out.after = await M.evalJs(H(`const r=q('.sb-pop').getBoundingClientRect(); return {x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)};`));
  out.moved = out.before.x !== out.after.x || out.before.y !== out.after.y;
  // 截图（拖到左上角后，弹框压在顶栏之上）
  try {
    const shot = await M.call("Page.captureScreenshot", { format: "png" });
    if (shot && shot.data) fs.writeFileSync(path.join(SANDBOX_ROOT, "e2_dragged.png"), Buffer.from(shot.data, "base64"));
  } catch (e) { out.shotErr = String(e); }
  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log(JSON.stringify(out, null, 1));
}
