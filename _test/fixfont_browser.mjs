// 真机(真 Blink)验证 readFix：用 headless Edge + CDP，加载真实 MOBI 解析出的 HTML，
// 调用浏览器里打包后的实际模块 applyReadFix，读控制台与 computed style。
// 由于 WebView2(Chromium 153) 不再开放 CDP 套接字，改用同为 Blink 的 Edge 浏览器执行。
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { execFileSync, spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import esbuild from "esbuild";
import { connectTarget } from "./cdp.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..");
const SANDBOX = path.join(process.env.LOCALAPPDATA || "", "Temp", "fr_test_profiles");
const DUMP = path.join(SANDBOX, "mobi_dump.html");
const EDGE = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const CDP_PORT = Number(process.env.FR_EDGE_PORT || 9811);
const HTTP_PORT = Number(process.env.FR_HTTP_PORT || 8611);
const UD = path.join(SANDBOX, "edge_ud_fixfont");
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 1) 打包真实模块（用 esbuild JS API，避免 Node24 不能起 .cmd）
await esbuild.build({
  entryPoints: [path.join(REPO, "src", "lib", "readFix.ts")],
  bundle: true,
  format: "esm",
  outfile: path.join(HERE, "_readfix.js"),
});
const html = fs.readFileSync(DUMP, "utf8");
fs.writeFileSync(path.join(HERE, "_mobi_html.js"), "export default " + JSON.stringify(html) + ";\n");
log(`bundled readFix + mobi html (${html.length} bytes, <font> x${(html.match(/<font/gi) || []).length})`);

const fixture = `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>fixfont</title>
<style>
  /* 模拟“书籍自带样式”对抗阅读器：改 inline 也能被 !important 覆盖 */
  #chapter-content p { font-family: "BookFont", serif; font-size: 12px; line-height: 4; color: rgb(0,0,255); background: rgb(255,0,0); }
</style></head><body>
<div id="chapter-content"></div>
<script type="module">
  import { applyReadFix } from './_readfix.js';
  import mobiHtml from './_mobi_html.js';
  const cc = document.getElementById('chapter-content');
  cc.innerHTML = mobiHtml;
  const opts = {
    fontFamily: '"ReaderFont", "Microsoft YaHei", serif',
    monoFont: 'Consolas, monospace',
    fontSize: 31,
    lineHeight: 1.85,
    color: 'rgb(44,44,44)',
    background: 'rgb(253,245,230)',
  };
  window.__opts = opts;
  window.__run = (o) => applyReadFix(o || opts);
  window.__mobiLen = mobiHtml.length;
  window.__run(opts);           // 第一次注入
  window.__afterFirst = { styles: document.querySelectorAll('#__read_fix').length };
  window.__run({ ...opts, fontFamily: '"ReaderFont2", serif', fontSize: 41 }); // 幂等 + 新值
  window.__afterSecond = { styles: document.querySelectorAll('#__read_fix').length };
  window.__ready = true;
</script></body></html>`;
fs.writeFileSync(path.join(HERE, "_fixfont_fixture.html"), fixture);

// 2) 静态服务器（file:// 下 ES module 会被 CORS 拦，必须走 http）
const server = http.createServer((req, res) => {
  const p = path.join(HERE, decodeURIComponent(req.url.split("?")[0]));
  try {
    const buf = fs.readFileSync(p);
    res.writeHead(200, { "content-type": p.endsWith(".js") ? "text/javascript" : p.endsWith(".html") ? "text/html" : "application/octet-stream" });
    res.end(buf);
  } catch { res.writeHead(404); res.end("nf"); }
});
await new Promise((r) => server.listen(HTTP_PORT, "127.0.0.1", r));
log("http server on " + HTTP_PORT);

// 3) headless Edge + CDP
fs.rmSync(UD, { recursive: true, force: true });
// 本机固定调试端口被安全策略拦截，只有 :0（临时端口）可用 → 用 DevToolsActivePort 拿真实端口
const edge = spawn(EDGE, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--user-data-dir=" + UD, "--remote-debugging-port=0", "--remote-allow-origins=*",
  `http://127.0.0.1:${HTTP_PORT}/_fixfont_fixture.html`,
], { stdio: "ignore" });

const out = { steps: {} };
try {
  let realPort = 0;
  for (let i = 0; i < 60; i++) {
    try { realPort = Number(fs.readFileSync(path.join(UD, "DevToolsActivePort"), "utf8").split(/\r?\n/)[0]); if (realPort) break; } catch { /* wait */ }
    await sleep(500);
  }
  if (!realPort) throw new Error("no DevToolsActivePort");
  log("edge DevTools port = " + realPort);
  let M = null;
  for (let i = 0; i < 40; i++) { try { M = await connectTarget(realPort, (t) => t.type === "page"); break; } catch { await sleep(500); } }
  if (!M) throw new Error("no edge cdp");
  log("cdp connected");
  for (let i = 0; i < 60; i++) { try { if (await M.evalJs("!!window.__ready")) break; } catch { /* wait */ } await sleep(300); }
  await sleep(500);

  // 控制台日志（真实浏览器输出）
  out.logs = M.consoleLogs().filter((l) => l.includes("fixfont") || l.includes("READ_CSS"));
  const inj = out.logs.map((l) => l.match(/\[fixfont\] injected, stripped=(\d+), html=(\d+)/)).filter(Boolean);
  out.injected = inj.map((m) => ({ stripped: Number(m[1]), html: Number(m[2]) }));
  const reads = out.logs.filter((l) => l.includes("READ_CSS"));
  out.readCss = reads[reads.length - 1] || null;

  // DOM + computed style 实测
  out.dom = await M.evalJs(`(async()=>{
    const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
    const cc=document.getElementById('chapter-content');
    const fonts=[...cc.querySelectorAll('font')];
    const still=fonts.filter(f=>f.hasAttribute('face')||f.hasAttribute('size')||f.hasAttribute('color')).length;
    // 找一个原本带 <font> 的文本元素，读它的计算样式
    const probe = fonts[0] ? fonts[0] : cc.querySelector('p,div');
    const cs = getComputedStyle(probe);
    const csParent = getComputedStyle(probe.parentElement || cc);
    return {
      mobiLen: window.__mobiLen,
      fontsTotal: fonts.length,
      fontsStillWithAttrs: still,
      styleCount: document.querySelectorAll('#__read_fix').length,
      styleLen: (document.getElementById('__read_fix')?.textContent||'').length,
      cssHead: (document.getElementById('__read_fix')?.textContent||'').slice(0,160),
      probeTag: probe.tagName,
      probeFontFamily: cs.fontFamily,
      probeFontSize: cs.fontSize,
      probeColor: cs.color,
      probeBg: cs.backgroundColor,
      ccBg: getComputedStyle(cc).backgroundColor,
      bodyBg: getComputedStyle(document.body).backgroundColor,
      htmlBg: getComputedStyle(document.documentElement).backgroundColor,
      pRuleBeaten: (()=>{ const p=cc.querySelector('p'); if(!p) return null; const s=getComputedStyle(p); return { fontFamily:s.fontFamily, fontSize:s.fontSize, color:s.color, bg:s.backgroundColor }; })(),
      afterFirstStyleCount: window.__afterFirst.styles,
      afterSecondStyleCount: window.__afterSecond.styles,
    };
  })()`);
  log("dom: " + JSON.stringify(out.dom));

  // 缺失 #chapter-content 时必须 warn 且不静默
  out.missing = await M.evalJs(`(async()=>{
    const cc=document.getElementById('chapter-content'); cc.id='chapter-content-x';
    const r=window.__run(); cc.id='chapter-content';
    return { returned: r===null };
  })()`);
  await sleep(300);
  out.warn = M.consoleLogs().filter((l) => l.includes("NO #chapter-content"));
  log("missing: " + JSON.stringify(out.missing) + " warn=" + JSON.stringify(out.warn));

  M.close();
} catch (e) {
    process.exitCode = 1; /* 失败必须体现在退出码上 */
  out.error = String(e);
  console.error(e);
} finally {
  try { edge.kill(); } catch { /* ignore */ }
  try { execFileSync("taskkill", ["/IM", "msedge.exe", "/F"], { stdio: "ignore" }); } catch { /* ignore */ }
  server.close();
  fs.writeFileSync(path.join(SANDBOX, "fixfont_browser_out.json"), JSON.stringify(out, null, 1));
  console.log("WROTE " + path.join(SANDBOX, "fixfont_browser_out.json"));
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
