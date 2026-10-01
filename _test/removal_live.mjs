// 实机验证（连接已在运行的沙箱实例 9444）：
// E 旧配置删除 reasoningOff*/reasoningMode 键、发送恒 auto、两章请求形状一致、重启无 AI 报错；
// D/A/B 真实 invoke refresh_library / file_exists（计数 + 无新增监听进程）；
// C 打开已删书 → 弹确认；确认→刷新+toast+停留书库；取消→不崩、停留书库。
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { connectTarget } from "./cdp.mjs";
import { sendTestPrompt } from "./send.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const PORT = 9444;
const LOG = path.join(SANDBOX_ROOT, "mock_reqs.jsonl");
const reqs = () => (fs.existsSync(LOG) ? fs.readFileSync(LOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
async function ensureReady(c, ms = 60000) { const t0 = Date.now(); for (;;) { try { await c.evalJs(`localStorage.getItem('tidepage.config.v1');location.href;1`); return; } catch (e) { if (Date.now() - t0 > ms) throw e; await sleep(400); } } }
function procCount() {
  const f = path.join(SANDBOX_ROOT, "pc.ps1");
  fs.writeFileSync(f, "$fr=@(Get-Process tidepage -EA SilentlyContinue|?{$_.Path -notlike '*\\Local\\TidePage\\*'}).Count; $wv=@(Get-CimInstance Win32_Process|?{$_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'fr_sbx'}).Count; $n=@(Get-Process node -EA SilentlyContinue).Count; \"$fr|$wv|$n\"");
  const r = execFileSync("powershell.exe", ["-NoProfile", "-Command", "& '" + f + "'"], { encoding: "utf8" }).trim().split("|");
  return { tidepage: +r[0], sandboxWebview: +r[1], node: +r[2] };
}

const out = {};
const M = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));
await ensureReady(M);

// ===== E: 写入带旧键的配置 → 重载 → 物理删除 =====
out.E_seed = await M.evalJs(H(`
  const cfg={reader:{fontFamily:'方正兰亭圆_gbk',fontSize:27,mode:'page'},
    ai:{enabled:true,autoOnChapter:false,system:'',active:0,reasoningMode:'off',display:'reader',style:{followReader:true},
      prompts:[{id:1,name:'probe',key:'F1',text:'请总结：{chapter}'}],
      profiles:[{name:'TEST-EP',provider:'ollama',baseUrl:'http://127.0.0.1:18081',apiKey:'',model:'mock',proxyMode:'direct',proxyUrl:'',
        reasoningOffModel:'TARGET-OFF',reasoningOffStrategy:'chat-json-think-false',protocolProbe:{baseUrl:'http://127.0.0.1:18081',model:'mock',strategies:['chat-json-think-false'],at:1}}]},
    webdav:{enabled:false},regex:''};
  localStorage.setItem('tidepage.config.v1',JSON.stringify(cfg)); setTimeout(()=>location.reload(),200); return 'seeded-with-legacy-keys';
`));
await sleep(4500); await ensureReady(M);
out.E_afterLoad = await M.evalJs(H(`
  const c=cfgRead(); const p=c.ai.profiles[0];
  const has=(o,k)=>Object.prototype.hasOwnProperty.call(o,k);
  return {
    aiHasReasoningMode: has(c.ai,'reasoningMode'),
    profileHasOffModel: has(p,'reasoningOffModel'),
    profileHasOffStrategy: has(p,'reasoningOffStrategy'),
    profileHasProbe: has(p,'protocolProbe'),
    aiKeys: Object.keys(c.ai), profileKeys: Object.keys(p),
  };
`));
// 发送两章（换章节内容）→ 请求体形状一致、无 think/thinking
out.libCount = await M.evalJs(`window.__TAURI_INTERNALS__.invoke("list_books").then(b=>b.length)`);
if (out.libCount > 0) {
  await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
  let n0 = reqs().length;
  out.E_send_chapterA = await sendTestPrompt(M);
  const reqA = reqs()[n0];
  n0 = reqs().length;
  out.E_send_chapterB = await sendTestPrompt(M);
  const reqB = reqs()[n0];
  const brief = (r) => r && ({ url: r.url, model: r.body?.model, keys: r.keys, thinkInBody: Object.prototype.hasOwnProperty.call(r.body || {}, "think"), auth: r.auth });
  out.E_requests = { A: brief(reqA), B: brief(reqB), identicalKeys: JSON.stringify(reqA?.keys) === JSON.stringify(reqB?.keys), identicalModel: reqA?.body?.model === reqB?.body?.model };
  out.E_noErrorToast = await M.evalJs(H(`return { toasts: toastsNow().slice(0,120), hasError: toastsNow().includes('[错误]') || toastsNow().includes('错误') };`));
} else {
  out.E_requests = "SKIPPED: 沙箱共享库为空，无书可开";
}
// “重启”：再次 reload，确认无 AI 报错、键仍不存在
await M.evalJs(`(()=>{setTimeout(()=>location.reload(),150);return 1})()`);
await sleep(4000); await ensureReady(M);
out.E_afterRestart = await M.evalJs(H(`
  const c=cfgRead(); const p=c.ai.profiles[0];
  return { aiHasReasoningMode:Object.prototype.hasOwnProperty.call(c.ai,'reasoningMode'), profileHasOffModel:Object.prototype.hasOwnProperty.call(p,'reasoningOffModel'), toasts: toastsNow().slice(0,80) };
`));

// ===== D/A/B: 真实 invoke refresh_library / file_exists =====
const before = procCount();
out.RF_invoke = await M.evalJs(`window.__TAURI_INTERNALS__.invoke("refresh_library").then(r=>JSON.stringify(r))`);
await sleep(500);
const after = procCount();
out.RF_procs = { before, after, unchanged: JSON.stringify(before) === JSON.stringify(after) };
out.RF_fileExists = await M.evalJs(`(async()=>{
  const books = await window.__TAURI_INTERNALS__.invoke("list_books");
  const real = books.length ? books[0].path : "";
  const okReal = real ? await window.__TAURI_INTERNALS__.invoke("file_exists",{path:real}) : null;
  const fake = await window.__TAURI_INTERNALS__.invoke("file_exists",{path:"C:\\\\__no_such_file__.txt"});
  return { samplePath: real, realExists: okReal, fakeExists: fake };
})()`);

// C 由 _test/c_live.mjs 以真实端到端覆盖（临时书入索引→删盘→真弹窗→取消/确认）。
M.close();
fs.writeFileSync(path.join(SANDBOX_ROOT, "removal_out.json"), JSON.stringify(out, null, 1));
console.log("REMOVAL DONE");
