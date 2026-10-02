// 用真机 WebView 把封面 SVG 光栅化为 PNG 截图（F2/F4 视觉证据）。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9699);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "shot_sbx"); const data = path.join(SANDBOX_ROOT, "shot_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outDir = path.join(SANDBOX_ROOT, "cover_shots"); fs.rmSync(outDir, { recursive: true, force: true }); fs.mkdirSync(outDir, { recursive: true });
const outFile = path.join(SANDBOX_ROOT, "shots_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

const out = { shots: [] };
let app = null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "shot_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 光栅化函数（页面内）：SVG 字符串 → PNG dataURL
  const rasterize = (svgStr) => H(`
    return await new Promise((resolve)=>{
      const svg=${JSON.stringify(svgStr)};
      const blob=new Blob([svg],{type:'image/svg+xml'});
      const url=URL.createObjectURL(blob);
      const img=new Image();
      img.onload=()=>{
        const c=document.createElement('canvas'); c.width=320; c.height=480;
        const ctx=c.getContext('2d'); ctx.drawImage(img,0,0,320,480);
        URL.revokeObjectURL(url);
        resolve(c.toDataURL('image/png'));
      };
      img.onerror=()=>resolve(null);
      img.src=url;
    });
  `);

  // 需要的封面：直接调用后端 render_default_cover 不暴露 → 用临时书 + cover-default 端点。
  // 通过设置书名：创建 TXT 文件名即书名/作者。
  const cases = [
    { tag: "short-cn", name: "围城 (钱钟书).txt" },
    { tag: "long-cn", name: "这是一本特别特别特别特别特别特别特别特别特别特别长的中文书名测试用 (某作者).txt" },
    { tag: "long-en", name: "An Extremely Long English Book Title That Goes On And On Forever And Ever (Some Author).txt" },
    { tag: "subtitle", name: "主标题：一个带副标题的书名 (作者名).txt" },
    { tag: "normal", name: "普通书名 (普通作者).txt" },
  ];
  const tmp = path.join(SANDBOX_ROOT, "shot_books"); fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  for (const c of cases) fs.writeFileSync(path.join(tmp, c.name), "正文。".repeat(30), "utf8");
  for (const c of cases) {
    const p = path.join(tmp, c.name).replace(/\\/g, "\\\\");
    await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${p}"});}catch(e){}`));
  }
  await sleep(2000);
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(4000);
  M.close(); M = await connectTarget(PORT, isApp);
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }
  await M.evalJs(H(`for(let i=0;i<80;i++){ const s=await inv('meta_debug_stats'); if(!s.running) break; await sleep(400);} return 1;`));
  await sleep(1500);

  // 对每个用例：取 cover-default SVG → 光栅化 → 存 PNG
  for (const c of cases) {
    const svg = await M.evalJs(H(`
      const pg=await inv('list_books_page',{query:'', sort:'title', offset:0, limit:200});
      const key=${JSON.stringify(c.name.replace(/\.txt$/, "").slice(0, 12))};
      const b=pg.rows.find(r=>r.path.includes(${JSON.stringify(c.name.replace(/\.txt$/, ""))}));
      if(!b) return null;
      const t=await (await fetch('http://tidepage.localhost/cover-default/'+b.id)).text();
      return { svg:t, title:b.title, author:b.author };
    `));
    if (!svg) { out.shots.push({ tag: c.tag, err: "no book/svg" }); continue; }
    const dataUrl = await M.evalJs(rasterize(svg.svg));
    if (dataUrl && dataUrl.startsWith("data:image/png;base64,")) {
      const buf = Buffer.from(dataUrl.split(",")[1], "base64");
      fs.writeFileSync(path.join(outDir, c.tag + ".png"), buf);
      out.shots.push({ tag: c.tag, title: svg.title, author: svg.author, bytes: buf.length,
        fontMatch: svg.svg.match(/font-size="(\d+)" font-weight="700"/)?.[1] });
    } else {
      out.shots.push({ tag: c.tag, err: "rasterize failed" });
    }
    log(`${c.tag}: ${JSON.stringify(out.shots[out.shots.length-1])}`);
  }

  M.close();
} catch (e) { process.exitCode = 1; /* 失败必须体现在退出码上 */ out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile + " ; PNGs in " + outDir);
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
