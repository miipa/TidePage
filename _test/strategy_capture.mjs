// OpenAI 兼容【协议策略】出站全验证（sandbox + 本地 mock；prompt 固定文本，绝不携带章节正文）。
// 覆盖：探针前禁用/拦截、注册后 chat/completions 双路由与双解析器、协议不匹配显式报错、
// model-complete 零出站、换端点验证失效、auto 零残留、legacy 名映射、model 严格=TARGET。
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { connectTarget } from "./cdp.mjs";
import { sendTestPrompt } from "./send.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const PORT = 9444;
const LOG = path.join(SANDBOX_ROOT, "mock_reqs.jsonl");
const reqs = () => (fs.existsSync(LOG) ? fs.readFileSync(LOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;

async function waitForCdp(timeoutMs = 420000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try { const l = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json(); if (l.some((t) => t.type === "page" && t.url.includes("localhost:1420"))) return; } catch {}
    await sleep(2000);
  }
  throw new Error("cdp not ready");
}
async function ensureReady(c, ms = 60000) { const t0 = Date.now(); for (;;) { try { await c.evalJs(`localStorage.getItem('flowread.config.v1');location.href;1`); return; } catch (e) { if (Date.now() - t0 > ms) throw e; await sleep(400); } } }
function audit() {
  const f = path.join(SANDBOX_ROOT, "audit2.ps1");
  fs.writeFileSync(f, "$fr=@(Get-Process flowread -EA SilentlyContinue|?{$_.Path -notlike '*\\Local\\FlowRead\\*'}).Count; $ls=@(Get-NetTCPConnection -State Listen -EA SilentlyContinue|?{$_.LocalPort -in 9444,1420}).Count; \"$fr|$ls\"");
  const r = execFileSync("powershell.exe", ["-NoProfile", "-Command", "& '" + f + "'"], { encoding: "utf8" }).trim().split("|");
  return { flowread: +r[0], listeners: +r[1] };
}

const out = {};
const sandboxDir = path.join(SANDBOX_ROOT, "fr_sbx");
fs.mkdirSync(sandboxDir, { recursive: true });
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sandboxDir }, cdpPort: PORT });
  out.appTreeRootPid = app.child.pid;
  await waitForCdp();
  const M = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));
  await ensureReady(M);

  await M.evalJs(H(`
    const cfg={reader:{fontFamily:'方正兰亭圆_gbk',fontSize:27,letterSpacing:0.05,mode:'page',punctuationCompression:false},
      ai:{enabled:true,autoOnChapter:false,system:'',active:0,reasoningMode:'off',display:'reader',style:{followReader:true},
        prompts:[{id:1,name:'probe',key:'F1',text:'ping-protocol-probe'}],
        profiles:[{name:'S-OPENAI',provider:'openai',baseUrl:'http://127.0.0.1:18081',apiKey:'',model:'TARGET',proxyMode:'direct',proxyUrl:'',reasoningOffModel:'TARGET-OFF',reasoningOffStrategy:'chat-json-think-false'}]},
      webdav:{enabled:false},regex:''};
    localStorage.setItem('flowread.config.v1',JSON.stringify(cfg)); setTimeout(()=>location.reload(),200); return 'seeded';
  `));
  await sleep(4500); await ensureReady(M);
  await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));

  const setCfg = async (patch) => {
    const stmts = [];
    stmts.push(`c.ai.reasoningMode=${JSON.stringify(patch.mode ?? "off")}`);
    if ("strategy" in patch) stmts.push(`p.reasoningOffStrategy=${JSON.stringify(patch.strategy)}`);
    if ("model" in patch) stmts.push(`p.model=${JSON.stringify(patch.model)}`);
    if ("baseUrl" in patch) stmts.push(`p.baseUrl=${JSON.stringify(patch.baseUrl)}`);
    if ("probe" in patch) stmts.push(`p.protocolProbe=${JSON.stringify(patch.probe)}`);
    await M.evalJs(`(()=>{const c=JSON.parse(localStorage.getItem('flowread.config.v1'));const p=c.ai.profiles[0];${stmts.join(";")};localStorage.setItem('flowread.config.v1',JSON.stringify(c));setTimeout(()=>location.reload(),150);return 1})()`);
    await sleep(3500); await ensureReady(M); await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
  };
  const domState = () => M.evalJs(H(`
    await openSettings('AI');
    const s=await wait(()=>field('思考模式')?.querySelector('select'),'rs');
    const strat=document.querySelector('#ai-reasoning-strategy');
    const opts=strat?Object.fromEntries([...strat.options].map(o=>[o.value,o.disabled])):null;
    closeSettings();
    return { modeValue:s.value, modeOffDisabled:s.querySelector('option[value="off"]').disabled, stratOpts:opts, probe: (cfgRead().ai.profiles[0].protocolProbe||null) };
  `));
  const runCase = async (name, patch, { allowUnsupported = false } = {}) => {
    const n0 = reqs().length;
    await setCfg(patch);
    let blocked = null, send = null;
    try { send = await sendTestPrompt(M, { allowUnsupported }); }
    catch (e) { blocked = e.message.slice(0, 70); }
    await sleep(500);
    const newReqs = reqs().slice(n0);
    const cap = newReqs[0] || null;
    out[name] = {
      blocked, send, delta: newReqs.length,
      cap: cap && { url: cap.url, method: cap.method, keys: cap.keys, model: cap.body?.model, think: cap.body?.think ?? "(absent)", hasMessages: (cap.keys || []).includes("messages"), hasPrompt: (cap.keys || []).includes("prompt"), thinkHeader: cap.thinkHeader, auth: cap.auth },
    };
  };

  // 1) 探针注册前：chat 策略禁用 + 双层拦截 + 零出站
  out["gate_ui"] = await domState();
  {
    const n0 = reqs().length;
    try { await sendTestPrompt(M); out["1_pre_probe_blocked"] = "NOT-BLOCKED!!"; }
    catch (e) { out["1_pre_probe_blocked"] = e.message.slice(0, 80); }
    out["1_pre_probe_delta"] = reqs().length - n0;
  }
  // 1b) 应用层拦截：经唯一入口 allowUnsupported 点击 → toast + 零 marks + 零出站
  {
    const n0 = reqs().length;
    await sendTestPrompt(M, { allowUnsupported: true });
    await sleep(700);
    const appGuard = await M.evalJs(H(`
      return { toastBlocked: toastsNow().includes('拦截'), marks: document.querySelectorAll('.ai-panel .ai-mark').length };
    `));
    out["1_app_guard"] = { ...appGuard, delta: reqs().length - n0 };
  }

  // 2) 通过设置页 UI 注册探针结果（chat + completions 验证通过；含一个 model-complete 也不得放行）
  const probeLine = 'FLOWREAD_PROTOCOL=' + JSON.stringify({ baseUrl: "http://127.0.0.1:18081", model: "TARGET", strategies: ["chat-json-think-false", "completions-json-think-false", "model-complete-query-think-false"], at: Date.now() });
  out["2_register"] = await M.evalJs(H(`
    await openSettings('AI');
    const ta=await wait(()=>document.querySelector('#ai-probe-input'),'probe ta');
    ta.value=${JSON.stringify(probeLine)}; ta.dispatchEvent(new Event('input',{bubbles:true})); await sleep(100);
    [...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='注册验证结果').click();
    await sleep(400);
    const t=toastsNow();
    const strat=document.querySelector('#ai-reasoning-strategy');
    const opts=Object.fromEntries([...strat.options].map(o=>[o.value,{disabled:o.disabled, text:o.textContent.slice(0,30)}]));
    closeSettings();
    return { toastOk: t.includes('已注册探针验证'), probe: cfgRead().ai.profiles[0].protocolProbe, opts };
  `));

  // 3) chat+json off → chat url、body think=false、messages、model TARGET
  await runCase("3_chat_off", { strategy: "chat-json-think-false" });
  // 4) completions off → /v1/completions、prompt、think=false、text 帧解析完成
  await runCase("4_completions_off", { strategy: "completions-json-think-false" });
  // 5) 协议不匹配显式报错：completions 路由返回 delta 帧（mock-wrongshape）→ 不得空白回复
  await setCfg({ strategy: "completions-json-think-false", model: "mock-wrongshape" });
  {
    const pl = 'FLOWREAD_PROTOCOL=' + JSON.stringify({ baseUrl: "http://127.0.0.1:18081", model: "mock-wrongshape", strategies: ["completions-json-think-false"], at: Date.now() });
    out["5a_re_register_for_wrongshape"] = await M.evalJs(H(`
      await openSettings('AI');
      const ta=document.querySelector('#ai-probe-input'); ta.value=${JSON.stringify(pl)}; ta.dispatchEvent(new Event('input',{bubbles:true})); await sleep(100);
      [...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='注册验证结果').click(); await sleep(300);
      const t=toastsNow(); closeSettings();
      return { toastOk: t.includes('已注册探针验证') };
    `));
    let done;
    try { done = await sendTestPrompt(M); } catch (e) { done = { threw: e.message.slice(0, 60) }; }
    const bodyText = await M.evalJs(H(`const ms=document.querySelectorAll('.ai-panel .ai-mark'); const m=ms[ms.length-1]; return { head: m.textContent.slice(0,70), hasMismatch: m.textContent.includes('响应协议不匹配'), len: m.textContent.length };`));
    out["5b_mismatch_error"] = { done, bodyText };
  }
  // 6) model-complete：探针虽记录但解析器未实现 → 选项保持禁用；测试层+应用层双拦截；/complete 零命中
  await setCfg({ strategy: "model-complete-query-think-false", model: "TARGET" });
  {
    out["6a_ui"] = await M.evalJs(H(`
      await openSettings('AI');
      const strat=document.querySelector('#ai-reasoning-strategy');
      const o=Object.fromEntries([...strat.options].map(x=>[x.value,x.disabled]));
      const sel=strat.value;
      closeSettings();
      return { opts:o, selectedStill: sel };
    `));
    const n0 = reqs().length;
    let blocked1 = null;
    try { await sendTestPrompt(M); blocked1 = "NOT-BLOCKED!!"; } catch (e) { blocked1 = e.message.slice(0, 80); }
    const started = await sendTestPrompt(M, { allowUnsupported: true });
    await sleep(700);
    const appToast = await M.evalJs(H(`
      return { toastBlocked: toastsNow().includes('未实现') || toastsNow().includes('拦截'), marks: document.querySelectorAll('.ai-panel .ai-mark').length };
    `));
    const newR = reqs().slice(n0);
    out["6_model_complete"] = { blocked1, startedOnly: !!started.startedOnly, appToast, urls: newR.map((r) => r.url), completeHits: newR.filter((r) => r.url.includes("/complete")).length };
  }
  // 7) 换端点（18081→18082）→ 验证失效：禁用 + 拦截 + 零出站
  await setCfg({ strategy: "chat-json-think-false", baseUrl: "http://127.0.0.1:18082" });
  {
    const n0 = reqs().length;
    out["7_expiry_ui"] = await M.evalJs(H(`
      await openSettings('AI');
      const strat=document.querySelector('#ai-reasoning-strategy');
      const chatOpt=[...strat.options].find(o=>o.value==='chat-json-think-false');
      const probe=cfgRead().ai.profiles[0].protocolProbe;
      closeSettings();
      return { chatOptDisabledAfterBaseChange: chatOpt.disabled, probeStillStored: !!probe };
    `));
    let blocked = null;
    try { await sendTestPrompt(M); } catch (e) { blocked = e.message.slice(0, 80); }
    out["7_expiry"] = { blocked, delta: reqs().length - n0 };
  }
  // 8) auto：即便策略是 chat-json，也不得残留 think/query/header
  await setCfg({ strategy: "chat-json-think-false", baseUrl: "http://127.0.0.1:18081", mode: "auto" });
  await runCase("8_auto_clean", { strategy: "chat-json-think-false", mode: "auto" }, {});
  // 9) model 走私回归（探针有效也拦）
  await setCfg({ mode: "off" });
  await runCase("9_smuggle", { strategy: "chat-json-think-false", model: 'TARGET" -H "X: 1' });
  // 10) legacy 名映射：json-think-false → chat-json-think-false（重新注册 TARGET 探针：case5 改过 model）
  await setCfg({
    strategy: "json-think-false", model: "TARGET", baseUrl: "http://127.0.0.1:18081",
    probe: { baseUrl: "http://127.0.0.1:18081", model: "TARGET", strategies: ["chat-json-think-false"], at: Date.now() },
  });
  await runCase("10_legacy_name", {});
  M.close();
} finally {
  if (app) { killTree(app.child.pid); await sleep(2000); sweepSandboxWebviews("fr_test_profiles"); }
  await sleep(1500);
  out.finalAudit = audit();
  fs.writeFileSync(path.join(SANDBOX_ROOT, "strategy_out.json"), JSON.stringify(out, null, 1));
  console.log("STRATEGY DONE");
}
