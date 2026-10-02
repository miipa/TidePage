// 复现/验证：书库网格在任意窗口宽度下，卡片不得超出所在单元格或被遮挡。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9633);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "grid_sbx");
const data = path.join(SANDBOX_ROOT, "grid_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "grid_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=200)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

const out = { widths: [] };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "grid_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  // 复制出多份临时书（独立路径）以形成多行网格
  const tmp = path.join(SANDBOX_ROOT, "grid_books");
  fs.rmSync(tmp, { recursive: true, force: true });
  fs.mkdirSync(tmp, { recursive: true });
  const src = path.join(BOOKS, "test_conv.epub");
  const made = [];
  for (let i = 0; i < 24; i++) {
    const dest = path.join(tmp, `book_${String(i).padStart(2, "0")}.epub`);
    fs.copyFileSync(src, dest);
    made.push(dest);
  }
  for (const p of made) {
    const pp = p.replace(/\\/g, "\\\\");
    await M.evalJs(H(`try { await inv('add_dropped_book',{path:"${pp}"}); } catch(e){}`));
  }
  await sleep(1500);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(3500);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await M.evalJs(H(`await wait(()=>{const c=qa('.card');return c.length?1:0;},'cards',300); return 1;`));

  // 通过真实窗口 resize 遍历宽度
  for (const w of [700, 900, 1100, 1380, 1700, 2200, 960]) {
    await M.call("Emulation.setDeviceMetricsOverride", { width: w, height: 800, deviceScaleFactor: 1, mobile: false }).catch(() => {});
    await sleep(700);
    const r = await M.evalJs(H(`
      const grid=q('.grid'); const cards=qa('.card');
      const gr=grid.getBoundingClientRect();
      const cs=getComputedStyle(grid);
      const padL=parseFloat(cs.paddingLeft)||0, padR=parseFloat(cs.paddingRight)||0;
      const avail=gr.width-padL-padR;
      // 列数
      const row=cards.slice(0, cards.length).map(c=>c.getBoundingClientRect());
      // 每张卡右边界不得超出可用区右边界（容器右 - padding）
      const rightLimit = gr.right - padR;
      const leftLimit = gr.left + padL;
      let overflow=0, minLeft=1e9, maxRight=-1e9;
      for(const c of cards){ const b=c.getBoundingClientRect(); minLeft=Math.min(minLeft,b.left); maxRight=Math.max(maxRight,b.right);
        if(b.right > rightLimit+1 || b.left < leftLimit-1) overflow++; }
      // 网格实际列数（按唯一 left 值）
      const lefts=[...new Set(cards.map(c=>Math.round(c.getBoundingClientRect().left)))].sort((a,b)=>a-b);
      // 卡片实际高度 vs 行预算
      const ch=cards[0]?.getBoundingClientRect().height||0;
      // 相邻两行是否重叠：取第一列前两张不同 top 的卡
      const tops=[...new Set(cards.map(c=>Math.round(c.getBoundingClientRect().top)))].sort((a,b)=>a-b);
      let rowOverlap=false;
      if(tops.length>=2){ const t0=tops[0],t1=tops[1];
        const inRow0=cards.filter(c=>Math.abs(c.getBoundingClientRect().top-t0)<2);
        const bottom0=Math.max(...inRow0.map(c=>c.getBoundingClientRect().bottom));
        if(bottom0 > t1+1) rowOverlap=true; }
      return { gridW:Math.round(gr.width), avail:Math.round(avail), cols:lefts.length, cards:cards.length,
        overflow, cardH:Math.round(ch), rowGap: tops.length>=2?(tops[1]-tops[0]):null, rowOverlap };
    `));
    out.widths.push({ width: w, ...r });
    log(`w=${w} cols=${r.cols} cardH=${r.cardH} rowGap=${r.rowGap} overflow=${r.overflow} rowOverlap=${r.rowOverlap}`);
  }
  M.close();
} catch (e) { process.exitCode = 1; /* 失败必须体现在退出码上 */ out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
