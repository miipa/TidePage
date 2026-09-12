// OpenAI 兼容 off 三策略出站形状捕获（sandbox + 本地 mock）。prompt 固定文本，绝不携带章节正文�?
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
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
async function waitForCdp(timeoutMs = 360000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try { const l = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json(); if (l.some((t) => t.type === "page" && t.url.includes("localhost:1420"))) return; } catch {}
    await sleep(2000);
  }
  throw new Error("cdp not ready");
}
async function ensureReady(c, ms = 60000) { const t0 = Date.now(); for (;;) { try { await c.evalJs(`localStorage.getItem('flowread.config.v1');location.href;1`); return; } catch (e) { if (Date.now() - t0 > ms) throw e; await sleep(400); } } }

const sandboxDir = path.join(SANDBOX_ROOT, "fr_sbx");
fs.mkdirSync(sandboxDir, { recursive: true });
let app = null; let mock = null; const out = {};
async function audit() {
  const ps = "$fr=@(Get-Process flowread -EA SilentlyContinue|?{$_.Path -notlike '*\\Local\\FlowRead\\*'}).Count; $ls=@(Get-NetTCPConnection -State Listen -EA SilentlyContinue|?{$_.LocalPort -in 9444,1420}).Count; \"$fr|$ls\"";
  const f = path.join(SANDBOX_ROOT, "audit.ps1"); fs.writeFileSync(f, ps);
  const r = execFileSync("powershell.exe", ["-NoProfile", "-Command", "& '" + f + "'"], { encoding: "utf8" }).trim().split("|");
  return { flowread: +r[0], listeners: +r[1] };
}
try {
  mock = null; // 复用已在 18081 运行�?mock，避免端口冲突；�?length 增量判定
  await sleep(200);
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sandboxDir }, cdpPort: PORT });
  out.pid = app.child.pid;
  await waitForCdp();
  const M = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));
  await ensureReady(M);

  // 初始化：openai 兼容 mock 端点，模�?TARGET，固定探�?prompt，off+switch-model+offModel=TARGET-OFF
  await M.evalJs(H(`
    const p=document.querySelector('.card,.row')||1;
    const cfg={reader:{fontFamily:'方正兰亭圆_gbk',fontSize:27,letterSpacing:0.05,mode:'page',punctuationCompression:false},
      ai:{enabled:true,autoOnChapter:false,system:'',active:0,reasoningMode:'off',display:'reader',style:{followReader:true},
        prompts:[{id:1,name:'probe',key:'F1',text:'ping-protocol-probe'}],
        profiles:[{name:'S-OPENAI',provider:'openai',baseUrl:'http://127.0.0.1:18081',apiKey:'',model:'TARGET',proxyMode:'direct',proxyUrl:'',reasoningOffModel:'TARGET-OFF',reasoningOffStrategy:'switch-model'}]},
      webdav:{enabled:false},regex:''};
    localStorage.setItem('flowread.config.v1',JSON.stringify(cfg)); setTimeout(()=>location.reload(),200); return 'seeded';
  `));
  await sleep(4500); await ensureReady(M);
  await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));

  const setCfg = async (patch) => {
    await M.evalJs(`(()=>{const c=JSON.parse(localStorage.getItem('flowread.config.v1'));c.ai.reasoningMode=${JSON.stringify(patch.mode)};const p=c.ai.profiles[0];p.reasoningOffStrategy=${JSON.stringify(patch.strategy)};if('offModel' in patch)p.reasoningOffModel=patch.offModel;if('model' in patch)p.model=patch.model;localStorage.setItem('flowread.config.v1',JSON.stringify(c));setTimeout(()=>location.reload(),150);return 1})()`);
    await sleep(3500); await ensureReady(M); await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
  };

  const lastReq = () => reqs()[reqs().length - 1];
  const runCase = async (name, patch) => {
    const n0 = reqs().length;
    await setCfg(patch);
    const blocked = [];
    try { await sendTestPrompt(M, { allowUnsupported: patch.expectBlocked }); } catch (e) { blocked.push(e.message.slice(0, 50)); }
    await sleep(400);
    const sent = reqs().length > n0;
    const r = lastReq();
    out[name] = { sent, blocked, shape: r && reqs().length > n0 ? { url: r.url, model: r.body?.model, thinkInBody: r.body?.think ?? "(absent)", thinkHeader: r.thinkHeader, auth: r.auth } : null };
  };

  // 1 switch-model + off
  await runCase("1_switch_model_off", { mode: "off", strategy: "switch-model", offModel: "TARGET-OFF" });
  // 2 json-think-false + off（model 保持 TARGET�?  await runCase("2_json_off", { mode: "off", strategy: "json-think-false" });
  // 3 query-think-false + off
  await runCase("3_query_off", { mode: "off", strategy: "query-think-false" });
  // 4 header-think-false + off
  await runCase("4_header_off", { mode: "off", strategy: "header-think-false" });
  // 5 auto + json 策略（不得残�?think/query/header�?  await runCase("5_auto_with_json_strategy", { mode: "auto", strategy: "json-think-false" });
  // 6 注入尝试：model �?header/JSON/URL �?必须拦截、零出站
  await runCase("6_model_header_smuggle", { mode: "off", strategy: "json-think-false", model: 'TARGET" -H "X-Think: 1', expectBlocked: true });
  await runCase("6_model_json_smuggle", { mode: "off", strategy: "json-think-false", model: '{"model":"x","think":false}', expectBlocked: true });
  await runCase("6_model_url_smuggle", { mode: "off", strategy: "query-think-false", model: "http://evil/v1/completions?think=false", expectBlocked: true });
  await runCase("6_model_query_smuggle", { mode: "off", strategy: "json-think-false", model: "TARGET?think=false", expectBlocked: true });

  // 7 SSE choices[].delta.content 正文正常结束：任一条成功用例的 send 已完成（sendTestPrompt 无抛错）
  // 校验：切�?json off 成功后回复文本非�?  await setCfg({ mode: "off", strategy: "json-think-false" });
  const done = await sendTestPrompt(M);
  out["7_sse_completion"] = { len: done.len, err: done.err, completed: !done.err && done.len > 100 };
  M.close();
} finally {
  if (app) { killTree(app.child.pid); await sleep(2000); sweepSandboxWebviews("fr_sandbox|fr_test_profiles"); }
  if (mock) { try { execFileSync("taskkill", ["/PID", String(mock.pid), "/T", "/F"], { stdio: "ignore" }); } catch {} }
  await sleep(1500);
  out.finalAudit = await audit();
  fs.writeFileSync(path.join(SANDBOX_ROOT, "strategy_out.json"), JSON.stringify(out, null, 1));
  console.log("STRATEGY DONE");
}
