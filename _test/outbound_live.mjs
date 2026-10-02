// 必改1 真证据：抓两种协议的 mock 出站请求体（脱敏），断言键集且无 think/reasoning_effort/thinking。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { sendTestPrompt } from "./send.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const LOG = path.join(SANDBOX_ROOT, "mock_reqs.jsonl");
const reqs = () => (fs.existsSync(LOG) ? fs.readFileSync(LOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);

const out = {};
const M = await connectTarget(9444, (t) => t.type === "page" && !t.url.includes("ai=1"));
await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));

// ollama auto
await M.evalJs(H(`const c=cfgRead(); c.ai.profiles[0]={name:'EP',provider:'ollama',baseUrl:'http://127.0.0.1:18081',apiKey:'',model:'mock',proxyMode:'direct',proxyUrl:''}; localStorage.setItem('tidepage.config.v1',JSON.stringify(c)); return 1`));
await M.evalJs(`(()=>{setTimeout(()=>location.reload(),150);return 1})()`);
await sleep(4500);
await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
let n = reqs().length;
out.ollama_send = await sendTestPrompt(M);
out.ollama = reqs()[n];

// openai-compat auto
await M.evalJs(H(`const c=cfgRead(); c.ai.profiles[0]={name:'EP',provider:'openai',baseUrl:'http://127.0.0.1:18081',apiKey:'sk-LOCAL-TEST',model:'mock',proxyMode:'direct',proxyUrl:''}; localStorage.setItem('tidepage.config.v1',JSON.stringify(c)); return 1`));
await M.evalJs(`(()=>{setTimeout(()=>location.reload(),150);return 1})()`);
await sleep(4500);
await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
n = reqs().length;
out.openai_send = await sendTestPrompt(M);
out.openai = reqs()[n];

const forbidden = ["think", "reasoning_effort", "thinking"];
const summarize = (r) => ({
  url: r.url,
  keys: r.keys,
  forbiddenPresent: forbidden.filter((k) => Object.prototype.hasOwnProperty.call(r.body || {}, k)),
  body: r.body,
});
out.ollama_summary = summarize(out.ollama);
out.openai_summary = summarize(out.openai);
out.ollama_keys_exact = JSON.stringify(out.ollama.keys) === JSON.stringify(["messages", "model", "options", "stream"]);
out.openai_keys_exact = JSON.stringify(out.openai.keys) === JSON.stringify(["model", "messages", "stream", "temperature"]);
out.ollama_keys_setEqual = JSON.stringify([...out.ollama.keys].sort()) === JSON.stringify(["messages", "model", "options", "stream"]);
out.openai_keys_setEqual = JSON.stringify([...out.openai.keys].sort()) === JSON.stringify(["messages", "model", "stream", "temperature"]);

M.close();
fs.writeFileSync(path.join(SANDBOX_ROOT, "outbound_out.json"), JSON.stringify(out, null, 1));
console.log(JSON.stringify({ ollama_keys: out.ollama.keys, openai_keys: out.openai.keys, ollama_forbidden: out.ollama_summary.forbiddenPresent, openai_forbidden: out.openai_summary.forbiddenPresent, ollama_send_ok: !out.ollama_send.err, openai_send_ok: !out.openai_send.err }, null, 1));

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
