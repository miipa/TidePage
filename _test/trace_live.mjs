// 真实请求 pre/post trace：本地 mock 端点 + FLOWREAD_AI_TRACE=1，抓 Rust stderr 的 [ai-trace]。
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";
import { sendTestPrompt } from "./send.mjs";

const PORT = Number(process.env.FR_PORT || 9822);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "trc_sbx"); const data = path.join(SANDBOX_ROOT, "trc_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const diag = path.join(SANDBOX_ROOT, "trace_diag.log");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

let app = null, mock = null;
try {
  mock = spawn(process.execPath, [path.join(process.cwd(), "_test", "mock_ai_server.mjs")], { stdio: "ignore", windowsHide: true });
  await sleep(800);
  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data, FLOWREAD_AI_TRACE: "1" },
    cdpPort: PORT, logFile: diag,
  });
  console.log("[trace-live] launched pid", app.child.pid);
  let M = null; for (let i = 0; i < 200; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 200; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 加一本书
  const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
  const bp = path.join(BOOKS, "test_conv.epub").replace(/\\/g, "\\\\");
  await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${bp}"});}catch(e){}`));
  await sleep(1200);
  // 种本地端点
  await M.evalJs(H(`
    const raw=localStorage.getItem('flowread.config.v1');
    const c=raw?JSON.parse(raw):{};
    c.ai=Object.assign({},c.ai||{},{enabled:true,autoOnChapter:false,active:0,
      profiles:[{id:1,name:'MOCK',provider:'ollama',baseUrl:'http://127.0.0.1:18081',apiKey:'',model:'mock',deployment:'',apiVersion:'',proxyMode:'direct',proxyUrl:''}],
      prompts:[{id:1,name:'总结',key:'F1',text:'总结：{chapter}'}],display:'reader'});
    localStorage.setItem('flowread.config.v1',JSON.stringify(c));
    return 1;
  `));
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 200; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 开书 + 开 AI 抽屉 + 发送
  await M.evalJs(H(`
    const c=await wait(()=>{const x=qa('.card,.row');return x.length?x:null;},'cards',300);
    c[0].dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('#chapter-content'),'chapter',400); await sleep(1500);
    localStorage.setItem('fr.ai.mode','drawer');
    [...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='AI')?.click();
    await wait(()=>q('.ai-panel'),'panel',100);
    const pan=q('.ai-panel');
    if(!pan.classList.contains('reader')){ [...pan.querySelectorAll('button')].find(x=>x.textContent.includes('阅读样式'))?.click(); await sleep(400); }
    return 1;
  `));
  const sent = await sendTestPrompt(M, { timeoutMs: 60000 });
  console.log("[trace-live] send result:", JSON.stringify(sent));
  await sleep(1500);
  M.close();
} catch (e) {
  console.error("[trace-live] ERROR", String(e));
} finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  if (mock) killTree(mock.pid);
  console.log("=== [ai-trace] lines from stderr ===");
  try {
    const txt = fs.readFileSync(diag, "utf8");
    const lines = txt.split(/\r?\n/).filter((l) => l.includes("[ai-trace]"));
    console.log(lines.join("\n") || "(none found)");
  } catch (e) { console.log("(diag read err)", String(e)); }
}
