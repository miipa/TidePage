// F 第三处：独立 AI 窗口的 probe（主窗 pop-out → 唯一入口发送 → 三态测宽）
import { connectTarget } from "./cdp.mjs";
import { sendTestPrompt } from "./send.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import fs from "node:fs";
import path from "node:path";

const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;
const CHIP = ".prompt-chip:not(.add)";

const main = await connectTarget(9444, (t) => t.type === "page" && !t.url.includes("ai=1"));
await main.evalJs(H(`await openBook(); return 1`));
const popped = await main.evalJs(`(()=>{const b=[...document.querySelectorAll('button')].find(x=>x.title&&x.title.includes('弹出为独立窗口'));if(b){b.click();return 1}return 0})()`);
console.log("popout:", popped);
await new Promise((r) => setTimeout(r, 5000));
const st = await connectTarget(9444, (t) => t.url.includes("ai=1"));
const panel = await st.evalJs(H(`
  const p = await wait(() => document.querySelector('.ai-panel'),'sp');
  if (!p.classList.contains('reader')) { [...p.querySelectorAll('button')].find(x=>x.textContent.includes('阅读样式'))?.click(); await sleep(400); }
  return { hasMark: !!p.querySelector('.ai-mark') };
`));
if (!panel.hasMark) {
  await st.evalJs(H(`const p=document.querySelector('.ai-panel'); p.querySelector('.ai-page')?.insertAdjacentHTML('beforeend','<div class="ai-page ai-reply"><div class="ai-mark"><p>，。！？（）《》，。！？（）《》</p></div></div>'); return 1`));
}
const probeStandalone = () => st.evalJs(H(`
  const p=document.querySelector('.ai-panel'); const ms=p.querySelectorAll('.ai-mark');
  return probeEl(ms[ms.length-1]);
`));
console.log("standalone palt-off:", JSON.stringify(await probeStandalone()));
await main.evalJs(H(`await paltBox(true); closeSettings(); return 1`));
await new Promise((r) => setTimeout(r, 900));
console.log("standalone palt-on:", JSON.stringify(await probeStandalone()));
const paltNow = await main.evalJs(H(`return cfgRead().reader.punctuationCompression`));
if (paltNow) await main.evalJs(H(`await paltBox(false); closeSettings(); return 1`));
main.close(); st.close();
