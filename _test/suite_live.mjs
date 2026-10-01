// TidePage 实机回归（“关闭思考”功能已彻底移除后的 auto-only 版本）。
// 覆盖：seed 后旧键不残留、非本机端点守卫、ollama/openai 兼容发送恒 auto（无 think/reasoning 键、形状一致）、
// AI 分页不跳末页（切换「每行字数上限」触发重排）、发送密钥不泄露、故障注入树回收。
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { connectTarget } from "./cdp.mjs";
import { sendTestPrompt, expectSendBlocked } from "./send.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = 9444;
const HERE = path.dirname(fileURLToPath(import.meta.url));
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const MOCK_LOG = path.join(SANDBOX_ROOT, "mock_reqs.jsonl");
const mockReqs = () => (fs.existsSync(MOCK_LOG) ? fs.readFileSync(MOCK_LOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);
const brief = (r) => (r ? { path: r.path, model: r.body?.model, keys: r.keys, think: Object.prototype.hasOwnProperty.call(r.body || {}, "think") } : "n/a");

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=150)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const field=(t)=>[...document.querySelectorAll('.field')].find(f=>f.textContent.includes(t));
const cfgRead=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}');
const openBook=async()=>{ if(!document.querySelector('#chapter-content')){ const c=await wait(()=>document.querySelector('.card,.row'),'card'); c.dispatchEvent(new MouseEvent('dblclick',{bubbles:true})); await wait(()=>document.querySelector('#chapter-content'),'chapter'); } await sleep(500); };
const openDrawer=async()=>{ localStorage.setItem('fr.ai.mode','drawer'); if(!document.querySelector('.ai-panel')) [...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='AI')?.click(); const d=await wait(()=>document.querySelector('.ai-panel'),'panel'); if(!d.classList.contains('reader')){[...d.querySelectorAll('button')].find(x=>x.textContent.includes('阅读样式'))?.click();await sleep(400);} return d; };
const openSettings=async(tab)=>{ document.querySelector('button[title="设置"]')?.click(); await wait(()=>document.querySelector('dialog[open]'),'dialog'); [...document.querySelectorAll('.tabs button')].find(x=>x.textContent.trim()===tab)?.click(); await sleep(250); };
const closeSettings=async()=>{ document.querySelector('dialog button.primary')?.click(); await sleep(200); };
const uiSet=async(labelText,val)=>{ const s=await wait(()=>field(labelText)?.querySelector('select'),'uiSet '+labelText); s.value=val; s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(150); };
const uiInput=async(labelText,val)=>{ const s=await wait(()=>field(labelText)?.querySelector('input'),'uiInput '+labelText); s.value=val; s.dispatchEvent(new Event('input',{bubbles:true})); s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(150); };
const labelField=(t)=>[...document.querySelectorAll('.field')].find(f=>f.querySelector('label')?.textContent.includes(t));
const toastsNow=()=>[...document.querySelectorAll('.toast-card')].map(t=>t.textContent).join(' | ');
const widthToggle=async()=>{ await openSettings('阅读'); const cur=Number(field('每行字数上限')?.querySelector('input')?.value)||0; const want=cur===0?32:0; await uiInput('每行字数上限', String(want)); await sleep(400); return want; };
`;
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;

async function waitForCdp(timeoutMs = 360_000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      if (list.some((t) => t.type === "page" && t.url.includes("localhost:1420"))) return Date.now() - t0;
    } catch { /* not up */ }
    await sleep(2000);
  }
  throw new Error("CDP 未在超时内就绪");
}
async function ensureReady(cdp, timeoutMs = 60_000) {
  const t0 = Date.now();
  for (;;) {
    try { await cdp.evalJs(`(()=>{localStorage.getItem('tidepage.config.v1');return location.href})()`); return; }
    catch (e) { if (Date.now() - t0 > timeoutMs) throw e; await sleep(500); }
  }
}
function auditTree() {
  const psScript =
    "$fr=@(Get-Process tidepage -EA SilentlyContinue).Count;" +
    "$dev=@(Get-CimInstance Win32_Process | Where-Object { ($_.Name -match '^(node|cargo|rustc|tidepage)\\.exe$' -and $_.CommandLine -match 'tauri dev|vite|tidepage_lib') -or ($_.Name -eq 'cmd.exe' -and $_.CommandLine -match 'tauri dev') }).Count;" +
    "$wv=@(Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'fr_test_profiles' }).Count;" +
    "$ls=@(Get-NetTCPConnection -State Listen -EA SilentlyContinue | Where-Object { $_.LocalPort -in 9444,1420 }).Count;" +
    "\"$fr|$dev|$wv|$ls\"";
  try {
    const [fr, dev, wv, ls] = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", psScript], { encoding: "utf8" }).trim().split("|");
    return { tidepage: Number(fr), tauriDev: Number(dev), sandboxWebView: Number(wv), listeners9444_1420: Number(ls) };
  } catch { return { tidepage: -1, tauriDev: -1, sandboxWebView: -1, listeners9444_1420: -1 }; }
}

const out = {};
const sandboxDir = path.join(SANDBOX_ROOT, "fr_sbx");
fs.mkdirSync(sandboxDir, { recursive: true });
let app = null;
let mock = null;
try {
  mock = spawn(process.execPath, [path.join(HERE, "mock_ai_server.mjs")], { stdio: "ignore", windowsHide: true });
  await sleep(500);
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sandboxDir }, cdpPort: PORT });
  out.appTreeRootPid = app.child.pid;
  log("launched pid " + app.child.pid);
  out.cdpReadyMs = await waitForCdp();
  log("cdp up");
  if (process.env.FR_FAULT_INJECT === "1") throw new Error("故障注入：人为在应用启动后失败（验证 finally 树回收）");

  const main = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));
  await ensureReady(main);
  log("seed");
  out.seed = await main.evalJs(H(`
    const base=cfgRead();
    const cfg={ reader: { fontFamily: "方正兰亭圆_gbk", fontSize: 27, mode: "page" },
      ai: { enabled:true, autoOnChapter:false, system: base.ai?.system||"", active:0,
        profiles:[{name:"TEST-EP",provider:"ollama",baseUrl:"http://127.0.0.1:18081",apiKey:"sk-GUARDTEST-PLACEHOLDER-9f3k2l",model:"mock",proxyMode:"direct",proxyUrl:""}],
        prompts:[{id:1,name:"测试总结",key:"F1",text:"总结：{chapter}"}], display:"reader", style: base.ai?.style||{} },
      webdav:{enabled:false}, regex:"" };
    localStorage.setItem("tidepage.config.v1", JSON.stringify(cfg));
    setTimeout(()=>location.reload(),200);
    return "seeded+reloading";
  `));
  await sleep(5000);
  main.close();

  const M = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));
  await ensureReady(M);
  await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
  out.seedReadback = await M.evalJs(H(`
    const c=cfgRead(); const p=c.ai.profiles[0];
    const has=(o,k)=>Object.prototype.hasOwnProperty.call(o,k);
    return { name:p.name, provider:p.provider, baseUrl:p.baseUrl, model:p.model, enabled:c.ai.enabled,
      old_ai_reasoningMode:has(c.ai,'reasoningMode'), old_p_offModel:has(p,'reasoningOffModel'),
      old_p_offStrategy:has(p,'reasoningOffStrategy'), old_p_probe:has(p,'protocolProbe'),
      profileKeys:Object.keys(p) };
  `));

  log("B 非本机端点守卫");
  {
    const n0 = mockReqs().length;
    await M.evalJs(H(`
      const c=cfgRead(); c.ai.profiles.push({name:"EVIL-DEEPSEEK",provider:"openai",baseUrl:"https://api.deepseek.com",apiKey:"x",model:"deepseek-chat",proxyMode:"auto",proxyUrl:""});
      localStorage.setItem("tidepage.config.v1", JSON.stringify(c)); return 1`));
    out.B_blockedWith = await expectSendBlocked(M, "非本机端点");
    out.B_mockDelta = mockReqs().length - n0;
    await M.evalJs(H(`const c=cfgRead(); c.ai.profiles=c.ai.profiles.filter(p=>p.name!=="EVIL-DEEPSEEK"); localStorage.setItem("tidepage.config.v1", JSON.stringify(c)); return 1`));
  }

  log("S 发送恒 auto（ollama 两章 + openai 兼容）");
  {
    let n0 = mockReqs().length;
    out.S_ollama_A = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
    n0 = mockReqs().length;
    out.S_ollama_B = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
    out.S_ollama_identicalShape = JSON.stringify(mockReqs()[mockReqs().length - 2]?.keys) === JSON.stringify(mockReqs()[mockReqs().length - 1]?.keys);
    out.S_noThinkingUI = await M.evalJs(H(`return { thinkUI: document.querySelectorAll('[class*=think],[class*=reason]').length, errorToast: toastsNow().includes('[错误]') };`));
    // openai 兼容路径：不注入 think/query/header，模型不被替换
    await M.evalJs(H(`await openSettings('AI'); await uiSet('服务商','openai'); await uiInput('模型','mock'); closeSettings(); await sleep(300); return 1`));
    n0 = mockReqs().length;
    out.S_openai = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
    await M.evalJs(H(`await openSettings('AI'); await uiSet('服务商','ollama'); closeSettings(); await sleep(300); return 1`));
  }

  log("G AI 分页不跳末页");
  {
    await M.evalJs(H(`await widthToggle(); closeSettings(); await sleep(600); return 1`));
    await M.evalJs(H(`await openDrawer(); return 1`));
    const hasMark = await M.evalJs(H(`return !!document.querySelector('.ai-panel .ai-mark')`));
    if (!hasMark) await sendTestPrompt(M);
    out.G_start = await M.evalJs(H(`
      const pg=()=>Number(document.querySelector('.pctinfo')?.textContent.match(/^(\\d+)\\//)?.[1]||1);
      let g=0; while(pg()<5 && g++<10){ window.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true})); await sleep(220);}
      const d=await openDrawer();
      const cur=()=>Number((d.querySelector('.ai-pg-text')?.textContent||'').match(/第 (\\d+)/)?.[1]||1);
      g=0; while(cur()>1 && g++<80){ d.querySelector('button[title^="上一页"]')?.click(); await sleep(120);}
      return { chapter: document.querySelector('.pctinfo')?.textContent.trim(), ai: d.querySelector('.ai-pg-text')?.textContent.trim() };
    `));
    const flip = async () => {
      const want = await M.evalJs(H(`return await widthToggle();`));
      await M.evalJs(H(`closeSettings(); await sleep(1300); return 1`));
      const st8 = await M.evalJs(H(`
        const d=document.querySelector('.ai-panel');
        return { maxLineChars: cfgRead().reader.maxLineChars, chapter: document.querySelector('.pctinfo')?.textContent.trim(), ai: d?.querySelector('.ai-pg-text')?.textContent.trim() };
      `));
      return { wantMaxLineChars: want, ...st8 };
    };
    out.G_toggle1 = await flip();
    out.G_toggle2 = await flip();
    out.G_toggle3 = await flip();
    const before = await M.evalJs(H(`return document.querySelectorAll('.ai-panel .ai-mark').length`));
    const done = await sendTestPrompt(M);
    await sleep(700);
    const stt = await M.evalJs(H(`
      const d=document.querySelector('.ai-panel'); const t=d.querySelector('.ai-pg-text')?.textContent.trim()||'';
      const m=t.match(/第 (\\d+)\\/(\\d+) 页/);
      return { aiText:t, atLast: m?Number(m[1])===Number(m[2]):null, chapter: document.querySelector('.pctinfo')?.textContent.trim() };
    `));
    out.G_newReply = { grew: done.marks > before, ...stt };
  }

  log("final");
  out.finalReadback = await M.evalJs(H(`
    const c=cfgRead(); const p=c.ai.profiles[0];
    return {name:p.name,provider:p.provider,baseUrl:p.baseUrl,model:p.model,keyLen:(p.apiKey||'').length,enabled:c.ai.enabled};
  `));
  out.mockKeyLeak = mockReqs().some((r) => JSON.stringify(r).includes("sk-GUARDTEST"));
  M.close();
} finally {
  log("teardown");
  if (app) {
    killTree(app.child.pid);
    await sleep(2000);
    sweepSandboxWebviews("fr_test_profiles");
  }
  if (mock) killTree(mock.pid);
  let a = null;
  for (let i = 0; i < 10; i++) {
    a = auditTree();
    if (a.tidepage === 0 && a.tauriDev === 0 && a.sandboxWebView === 0 && a.listeners9444_1420 === 0) break;
    await sleep(2000);
  }
  out.teardownAudit = a;
  fs.writeFileSync(path.join(SANDBOX_ROOT, "suite_out.json"), JSON.stringify(out, null, 1));
  console.log(process.env.FR_FAULT_INJECT === "1" ? "FAULT-INJECT RUN DONE" : "SUITE DONE");
}
