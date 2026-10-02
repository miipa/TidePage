// 补验：删2本+加1本 → removed=2/added=1 且 toast 与之一致；一级子目录命中、二级不误报。
// 注：运行时 add_folder 不安装 watcher（install_watcher 仅 lib.rs:163 启动调用），故本临时根不被监听，计数可控。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const root = path.join(SANDBOX_ROOT, "fr_rf");
const sub = path.join(root, "sub");
const deep = path.join(sub, "deep");
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(sub, { recursive: true });
fs.mkdirSync(deep, { recursive: true });
fs.writeFileSync(path.join(root, "A.txt"), "A");
fs.writeFileSync(path.join(root, "B.txt"), "B");

const out = {};
const M = await connectTarget(9444, (t) => t.type === "page" && !t.url.includes("ai=1"));
await M.evalJs(H(`
  const I=window.__TAURI_INTERNALS__.invoke;
  for (let i=0;i<80 && !document.querySelector('.library');i++){ const b=[...document.querySelectorAll('.topbar button')].find(x=>x.textContent.includes('←')); if(b)b.click(); else break; await sleep(150); }
  try { await I("remove_folder",{path:${JSON.stringify(root)}}); } catch(e){}
  await I("add_folder",{path:${JSON.stringify(root)}});
  let a=null,b=null;
  for(let i=0;i<60;i++){ const bs=await I("list_books"); a=bs.find(x=>x.path===${JSON.stringify(path.join(root, "A.txt"))}); b=bs.find(x=>x.path===${JSON.stringify(path.join(root, "B.txt"))}); if(a&&b) break; await sleep(300); }
  return { indexedA:!!a, indexedB:!!b };
`));

// 删 2 本 + 加 1 本（一级子目录）+ 1 本二级（不得误报）
fs.unlinkSync(path.join(root, "A.txt"));
fs.unlinkSync(path.join(root, "B.txt"));
fs.writeFileSync(path.join(sub, "C-new.txt"), "C");
fs.writeFileSync(path.join(deep, "D-deep.txt"), "D");

out.flow = await M.evalJs(H(`
  const I=window.__TAURI_INTERNALS__.invoke;
  const s=document.querySelector('input.search'); if(s){ s.value=''; s.dispatchEvent(new Event('input',{bubbles:true})); }
  const btn=document.querySelector('[data-testid="lib-btn-refresh"]');
  if(!btn) return { err:'no refresh button' };
  btn.click();
  let toast='';
  for(let i=0;i<300;i++){ await sleep(200); toast=toastsNow(); if(toast) break; }
  const bs=await I("list_books");
  return {
    toast: toast.slice(0,80),
    removedA: !bs.some(x=>x.path===${JSON.stringify(path.join(root, "A.txt"))}),
    removedB: !bs.some(x=>x.path===${JSON.stringify(path.join(root, "B.txt"))}),
    addedC: bs.some(x=>x.path===${JSON.stringify(path.join(sub, "C-new.txt"))}),
    addedDeep: bs.some(x=>x.path===${JSON.stringify(path.join(deep, "D-deep.txt"))}),
  };
`));

out.cleanup = await M.evalJs(H(`
  const I=window.__TAURI_INTERNALS__.invoke;
  try { await I("remove_folder",{path:${JSON.stringify(root)}}); } catch(e){ return {err:String(e)}; }
  const bs=await I("list_books");
  return { anyUnderRoot: bs.some(x=>x.path.startsWith(${JSON.stringify(root)})) };
`));
M.close();
fs.rmSync(root, { recursive: true, force: true });
fs.writeFileSync(path.join(SANDBOX_ROOT, "refresh_out.json"), JSON.stringify(out, null, 1));
console.log(JSON.stringify(out, null, 1));

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
