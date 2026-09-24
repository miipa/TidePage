// 验证：即使 DB 里 meta_busy='1'（旧版卡死状态），启动后仍能生成封面。
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9644);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "meta_sbx");
const data = path.join(SANDBOX_ROOT, "meta_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outFile = path.join(SANDBOX_ROOT, "meta_out.json");
const BOOKS = path.join(SANDBOX_ROOT, "scroll_books");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const wait=async(fn,n,r=300)=>{for(let i=0;i<r;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+n)}; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

const out = { steps: {} };
let app = null;
try {
  // 先在沙箱数据目录建库并写入卡死的 meta_busy（用 py/sqlite? 无 cli → 用 app 自己生成后改）
  // 策略：先正常启动生成 DB，再手动 sqlite 写入 meta_busy=1（用 rust probe 不行，改用手工构造）
  // 简化：直接把用户库副本的 settings 表拷进来是不现实的；改为启动后用 CDP 通过 set_setting_kv 写 meta_busy=1，
  //        加入书籍 → 重启 → 观察封面是否仍生成。
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "meta_diag.log") });
  let M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 加入多本书 + 写入卡死的 meta_busy=1
  const tmp = path.join(SANDBOX_ROOT, "meta_books");
  fs.rmSync(tmp, { recursive: true, force: true }); fs.mkdirSync(tmp, { recursive: true });
  const paths = [];
  for (let i = 0; i < 6; i++) {
    const d = path.join(tmp, `m_${i}.epub`);
    fs.copyFileSync(path.join(BOOKS, "test_conv.epub"), d);
    paths.push(d);
  }
  for (const p of paths) {
    const pp = p.replace(/\\/g, "\\\\");
    await M.evalJs(H(`try { await inv('add_dropped_book',{path:"${pp}"}); } catch(e){}`));
  }
  await sleep(1200);
  await M.evalJs(H(`try { await inv('set_setting_kv',{key:'meta_busy', value:'1'}); } catch(e){} return 1;`));
  out.steps.stuckSet = await M.evalJs(H(`const v=await inv('get_setting_kv',{key:'meta_busy'}); return { meta_busy: v };`));
  log("stuckSet: " + JSON.stringify(out.steps.stuckSet));

  // 重启 → 观察封面是否仍被生成
  await M.evalJs(`setTimeout(()=>location.reload(),80);1`);
  await sleep(4500);

  app.child && execFileSync("taskkill", ["/PID", String(app.child.pid), "/T", "/F"], { stdio: "ignore" });
  await sleep(1500);
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, FLOWREAD_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "meta_diag2.log") });
  M = null; for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp2");
  for (let i = 0; i < 300; i++) { try { if (await M.evalJs(`!!window.__TAURI_INTERNALS__`)) break; } catch {} await sleep(400); }

  // 等待 meta_done 增加（最多 60s）
  out.steps.after = await M.evalJs(H(`
    const inv2=inv;
    let best={done:0,total:0,withCover:0};
    for(let i=0;i<120;i++){
      // 通过 list_books_page 检查
      const pg=await inv2('list_books_page',{query:'', sort:'added', offset:0, limit:200});
      const rows=pg.rows.filter(r=>/meta_books/i.test(r.path));
      const done=rows.filter(r=>r.meta_done===1).length;
      const withCover=rows.filter(r=>r.cover_path).length;
      best={ found:rows.length, done, withCover, total:rows.length,
        covers:rows.map(r=>r.cover_path).filter(Boolean).slice(0,3) };
      if(done>0) break;
      await sleep(500);
    }
    const mb=await inv2('get_setting_kv',{key:'meta_busy'});
    return { ...best, meta_busy: mb };
  `));
  log("after: " + JSON.stringify(out.steps.after));

  M.close();
} catch (e) { out.error = String(e); console.error(e); }
finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
}
