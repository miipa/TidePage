// C 端到端（真实，不 patch invoke）：临时书入索引 → 删盘 → 搜索定位 → dblclick → 真确认弹窗；取消/确认两条路径。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const HELPER = fs.readFileSync("_test/suite_live.mjs", "utf8").match(/const HELPER = `([\s\S]*?)\n`;/)[1];
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const dir = path.join(SANDBOX_ROOT, "fr_books");
const file = path.join(dir, "删除测试书.txt");
fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(file, "这是用于验证打开已删除书籍的临时文件。\n".repeat(20));

const out = {};
const M = await connectTarget(9444, (t) => t.type === "page");

out.idx = await M.evalJs(H(`
  const I = window.__TAURI_INTERNALS__.invoke;
  try { await I("remove_folder", { path: ${JSON.stringify(dir)} }); } catch(e) {}
  await I("add_folder", { path: ${JSON.stringify(dir)} });
  let found = null;
  for (let i=0;i<60;i++){ const bs=await I("list_books"); found=bs.find(b=>b.path===${JSON.stringify(file)}); if(found) break; await sleep(400); }
  return { added: !!found, path: found && found.path, id: found && found.id };
`));

fs.unlinkSync(file); // 磁盘删除
out.deletedFromDisk = !fs.existsSync(file);

out.flow = await M.evalJs(H(`
  const I = window.__TAURI_INTERNALS__.invoke;
  // 回书库
  for (let i=0;i<80 && !document.querySelector('.library');i++){ const b=[...document.querySelectorAll('.topbar button')].find(x=>x.textContent.includes('←')); if(b) b.click(); else break; await sleep(150); }
  // 搜索定位临时书
  const s = await wait(()=>document.querySelector('input.search'),'search');
  s.value = '删除测试书'; s.dispatchEvent(new Event('input',{bubbles:true}));
  const card = await wait(()=>[...document.querySelectorAll('.card,.row')].find(c=>c.textContent.includes('删除测试书')),'target-card',80);
  // 取消路径
  card.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
  const dlg = await wait(()=>document.querySelector('.confirm-msg'),'confirm');
  const dialogText = dlg.textContent.slice(0,100);
  document.querySelector('[data-testid="confirm-cancel"]').click();
  await sleep(400);
  const afterCancel = { inReader: !!document.querySelector('#chapter-content'), inLibrary: !!document.querySelector('.library') };
  // 确认路径
  card.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
  await wait(()=>document.querySelector('.confirm-msg'),'confirm2');
  document.querySelector('[data-testid="confirm-ok"]').click();
  let toastMsg = '', toastDom = '';
  for (let i=0;i<300;i++){
    await sleep(200);
    toastDom = [...document.querySelectorAll('.toast-card')].map(t=>t.textContent).join(' | ');
    try { const m = await import('/src/lib/stores.svelte.ts'); toastMsg = ((m.toasts && m.toasts.items) || []).map(t=>t.msg).join(' | '); } catch(e){ toastMsg = 'ERR:'+e; }
    if (toastDom || toastMsg) break;
  }
  const afterConfirm = { inReader: !!document.querySelector('#chapter-content'), inLibrary: !!document.querySelector('.library'), toast: toastMsg.slice(0,80), toastDom: toastDom.slice(0,80) };
  // 该书是否已从索引消失
  const bs = await I("list_books");
  return { dialogText, afterCancel, afterConfirm, stillIndexed: bs.some(b=>b.path===${JSON.stringify(file)}) };
`));

out.cleanup = await M.evalJs(H(`
  const I = window.__TAURI_INTERNALS__.invoke;
  try { await I("remove_folder", { path: ${JSON.stringify(dir)} }); } catch(e) { return { err: String(e) }; }
  const bs = await I("list_books");
  return { anyUnderDir: bs.some(b=>b.path.startsWith(${JSON.stringify(dir)})) };
`));

M.close();
fs.rmSync(dir, { recursive: true, force: true });
fs.writeFileSync(path.join(SANDBOX_ROOT, "c_out.json"), JSON.stringify(out, null, 1));
console.log(JSON.stringify(out, null, 1));

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
