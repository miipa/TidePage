// 回归实测：「文件夹作用域下删除书籍后书库整片空白」。
// 场景（全部走真实 UI，不 patch 业务逻辑）：
//   种子：add_folder 建 A/B/C 三个文件夹，各放 2-3 本真实 .txt；
//         胶囊顺序 = 全部 / A / B / C（B 即「第二个文件夹胶囊」）。
//   S1 选中 B（第二个文件夹胶囊，非「全部」）→ 网格视图 → 右键 →「仅移出书库」→ 确定
//   S2 切到「全部」胶囊 → 网格视图 → 右键 →「删除本地文件」→ 确定（对照）
//   S3 切列表视图（localStorage['tidepage.lib.view']='list'）→ 选中 C → 右键 →「仅移出书库」→ 确定
// 断言：删除后 300ms / 1s / 3s 三个时刻，`.card`(或 `.row`) 数量 > 0、等于剩余书数、`.card-slot` 数量为 0；
//      并统计删除窗口内应用自身 console 计时 "[tidepage] rebuildIndex" 的条数（= 索引重建次数），
//      验证「删除成功后界面一定刷新、且只刷新一次」。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9641);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const sbx = path.join(SANDBOX_ROOT, "delblank_sbx");
const data = path.join(SANDBOX_ROOT, "delblank_data");
for (const d of [sbx, data]) {
  fs.rmSync(d, { recursive: true, force: true });
  fs.mkdirSync(d, { recursive: true });
}

// ---- 真实书籍文件（后端按 path 前缀做文件夹作用域，故文件必须真在目录里）----
const DIR_A = path.join(SANDBOX_ROOT, "delblank_a");
const DIR_B = path.join(SANDBOX_ROOT, "delblank_b");
const DIR_C = path.join(SANDBOX_ROOT, "delblank_c");
const BOOKS = [
  { dir: DIR_A, files: ["a_one.txt", "a_two.txt"] },
  { dir: DIR_B, files: ["b_one.txt", "b_two.txt", "b_three.txt"] },
  { dir: DIR_C, files: ["c_one.txt", "c_two.txt", "c_three.txt"] },
];
const totalFiles = BOOKS.reduce((n, b) => n + b.files.length, 0);
for (const b of BOOKS) {
  fs.rmSync(b.dir, { recursive: true, force: true });
  fs.mkdirSync(b.dir, { recursive: true });
  for (const f of b.files) {
    fs.writeFileSync(path.join(b.dir, f), `${f} —— 文件夹作用域删书空白回归实测用书。\n`.repeat(24));
  }
}
const J = (v) => JSON.stringify(v);
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

let M = null;
const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=400)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name);};
const inv=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const items=()=>qa('.card,.row').filter(e=>!e.classList.contains('card-slot'));
const titleOf=(e)=>(e.querySelector('.meta .t,.row-meta .t')||{}).textContent||'';
const sample=()=>({ cards:qa('.card').length, slots:qa('.card-slot').length, rows:qa('.row').length,
  titles:items().map(titleOf), bottom:((q('.statusbar')||{}).textContent||'').trim().slice(0,12) });
const dbTotal=async(folder)=>(await inv('list_books_page',{query:'',sort:'title',offset:0,limit:1,formats:null,states:null,desc:false,folder:folder||null})).total;
const chip=(p)=>qa('.folders .folder-chip').find(c=>c.textContent.includes(p));
const chipAll=()=>q('[data-testid="lib-folder-all"]');
const markAt=async(t0,want)=>{ const marks={}; let i=0;
  while(Date.now()-t0 < want[want.length-1]+250){ const dt=Date.now()-t0;
    while(i<want.length && dt>=want[i]){ marks[want[i]]={...sample(), actualAt:Date.now()-t0}; i++; }
    await sleep(20); }
  for(;i<want.length;i++) marks[want[i]]={...sample(), actualAt:Date.now()-t0};
  return marks; };
// 右键目标卡片（nth 或 titleNeedle）→ 点指定菜单项（按 data-testid）→ 确认弹窗「确定」→ 返回删除发起时刻
const norm=(s)=>String(s).replace(/[\\s_\\-]/g,'').toLowerCase();
const ctxDelete=async(menuTestId,opt)=>{
  opt=opt||{};
  const list=items();
  const card = opt.needle ? list.find(e=>norm(titleOf(e)).includes(norm(opt.needle))) : list[opt.nth||0];
  if(!card) throw new Error('no card for '+JSON.stringify(opt));
  const title=titleOf(card);
  card.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:120,clientY:120}));
  await wait(()=>q('.ctx'),'ctx');
  const item=q('[data-testid="'+menuTestId+'"]');
  if(!item) throw new Error('no ctx item '+menuTestId);
  item.click();
  await wait(()=>q('.confirm-msg'),'confirm');
  const msg=q('.confirm-msg').textContent.replace(/\\s+/g,' ').slice(0,50);
  const ok=q('[data-testid="confirm-ok"]');
  if(!ok) throw new Error('no confirm ok');
  const t0=Date.now(); ok.click();
  return { msg, t0, title };
};
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;

async function ensureReady(label) {
  for (let i = 0; i < 300; i++) {
    try { const r = await M.evalJs(`!!window.__TAURI_INTERNALS__`); if (r) return; } catch { /* retry */ }
    await sleep(400);
  }
  throw new Error("not ready " + label);
}
async function reloadApp(label) {
  await M.evalJs(`setTimeout(()=>location.reload(),100);1`);
  await sleep(4000);
  await ensureReady(label);
}
/** 取出自上次 drain 以来的 console 计时行（应用自身 console.timeEnd 的输出）。 */
const timerBatch = () =>
  M.drainEvents()
    .filter((e) => e.method === "Runtime.consoleAPICalled")
    .map((e) => (e.params?.args || []).map((a) => String(a.value ?? a.description ?? "")).join(" "));
const countTimers = (lines, needle) => lines.filter((l) => l.includes(needle)).length;
/** 让封面/元数据后台扫描先静默，并清空计时基线。 */
const quiet = async () => { await sleep(1200); M.drainEvents(); };

const out = { steps: {} };
let app = null;
let failed = 0;
const check = (name, cond, detail) => {
  if (!cond) failed++;
  log(`${cond ? "PASS" : "FAIL"}  ${name}${detail !== undefined ? "  " + JSON.stringify(detail) : ""}`);
};
/** 三个时刻的统一断言：数量>0、等于剩余书数、空卡槽为 0、被删那本消失、底栏计数一致。 */
const assertMarks = (tag, marks, expectN, goneTitle) => {
  for (const t of ["300", "1000", "3000"]) {
    const m = marks[t];
    const n = m.rows > 0 ? m.rows : m.cards;
    check(`${tag} @${t}ms：渲染数=${n} > 0`, n > 0, n);
    check(`${tag} @${t}ms：渲染数等于剩余书数 ${expectN}`, n === expectN, { n, cards: m.cards, rows: m.rows, slots: m.slots, bottom: m.bottom, actualAt: m.actualAt });
    check(`${tag} @${t}ms：.card-slot=0`, m.slots === 0, m.slots);
    check(`${tag} @${t}ms：被删那本《${goneTitle}》已消失`, !m.titles.includes(goneTitle), m.titles);
    check(`${tag} @${t}ms：底栏计数为 ${expectN}`, String(m.bottom).includes(String(expectN)), m.bottom);
  }
};

try {
  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data },
    cdpPort: PORT,
    logFile: path.join(SANDBOX_ROOT, "delblank_diag.log"),
  });
  for (let i = 0; i < 240; i++) { try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); } }
  if (!M) throw new Error("no cdp target");
  await ensureReady("boot");
  log("app ready");

  // ---- 种子：建 3 个文件夹并等扫描入库 ----
  out.steps.seed = await M.evalJs(H(`
    for (const d of ${J(BOOKS.map((b) => b.dir))}) { await inv('add_folder', { path: d }); }
    const want = ${J(Object.fromEntries(BOOKS.map((b) => [b.dir, b.files.length])))};
    const got = {};
    for (let i=0;i<300;i++){
      let done = true;
      for (const d of Object.keys(want)) { got[d] = await dbTotal(d); if (got[d] !== want[d]) done = false; }
      if (done) break;
      await sleep(200);
    }
    return { folders: await inv('list_folders'), got, total: await dbTotal(null) };
  `));
  log("seed: " + JSON.stringify(out.steps.seed));
  check("种子：三个文件夹已登记", (out.steps.seed.folders || []).length === 3, (out.steps.seed.folders || []).length);
  check("种子：各文件夹书数正确", BOOKS.every((b) => out.steps.seed.got[b.dir] === b.files.length), out.steps.seed.got);
  check("种子：全库书数正确", out.steps.seed.total === totalFiles, out.steps.seed.total);

  // ---- 重载页面：让 onMount 的 loadFolders() 渲染胶囊 ----
  await reloadApp("after reload");
  out.steps.chips = await M.evalJs(H(`
    await wait(()=>qa('.folders .folder-chip').length>=4,'chips');
    return qa('.folders .folder-chip').map((c,i)=>({ i, text:c.textContent.replace(/\\s+/g,' ').trim() }));
  `));
  log("chips: " + JSON.stringify(out.steps.chips));
  check("胶囊排：全部 + 3 个文件夹", out.steps.chips.length === 4, out.steps.chips.map((c) => c.i + ":" + c.text));
  check("胶囊排：第 2 个文件夹胶囊 = delblank_b（非「全部」）",
    out.steps.chips.length === 4 && out.steps.chips[2].text.includes("delblank_b") && !out.steps.chips[2].text.includes("delblank_a"),
    out.steps.chips[2]);

  // ================= S1：第二个文件夹胶囊（B）+ 网格 + 仅移出书库 =================
  await M.evalJs(H(`
    const b = chip(${J(DIR_B)});
    if (!b) throw new Error('chip B missing');
    b.click();
    await wait(()=>qa('.card').length===${BOOKS[1].files.length} && qa('.card-slot').length===0,'B cards');
    window.__pre = { before: sample(), dbBefore: await dbTotal(${J(DIR_B)}) };
    return 1;
  `));
  await quiet();
  out.steps.s1 = await M.evalJs(H(`
    const d = await ctxDelete('lib-ctx-remove', { nth: 0 });
    const marks = await markAt(d.t0,[300,1000,3000]);
    return { before: window.__pre.before, dbBefore: window.__pre.dbBefore, dmsg:d.msg, gone:d.title, marks, dbAfter: await dbTotal(${J(DIR_B)}) };
  `));
  const tl1 = timerBatch(); out.steps.s1.rebuilds = countTimers(tl1, "[tidepage] rebuildIndex"); out.steps.s1.libChanged = countTimers(tl1, "library-changed");
  log("S1: " + JSON.stringify(out.steps.s1));
  check("S1 前：B 显示 3 本", out.steps.s1.before.cards === 3 && out.steps.s1.before.slots === 0, out.steps.s1.before);
  check("S1：DB 里确实少了一本", out.steps.s1.dbAfter === out.steps.s1.dbBefore - 1, [out.steps.s1.dbBefore, out.steps.s1.dbAfter]);
  assertMarks("S1(网格/B)", out.steps.s1.marks, out.steps.s1.dbAfter, out.steps.s1.gone);
  check("S1：索引只重建一次（rebuildIndex=1）", out.steps.s1.rebuilds === 1, out.steps.s1.rebuilds);
  check("S1：library-changed 监听刷新一次", out.steps.s1.libChanged === 1, out.steps.s1.libChanged);

  // ================= S2：「全部」胶囊 + 网格 + 删除本地文件（对照） =================
  await M.evalJs(H(`
    chipAll().click();
    await wait(()=>qa('.card').length===${totalFiles - 1} && qa('.card-slot').length===0,'all cards');
    window.__pre = { before: sample(), dbBefore: await dbTotal(null) };
    return 1;
  `));
  await quiet();
  out.steps.s2 = await M.evalJs(H(`
    const d = await ctxDelete('lib-ctx-delete-file', { needle: 'a_one' });   // 固定打 A 文件夹那本，不影响 B/C 计数
    const marks = await markAt(d.t0,[300,1000,3000]);
    return { before: window.__pre.before, dbBefore: window.__pre.dbBefore, dmsg:d.msg, gone:d.title, marks, dbAfter: await dbTotal(null) };
  `));
  const tl2 = timerBatch(); out.steps.s2.rebuilds = countTimers(tl2, "[tidepage] rebuildIndex"); out.steps.s2.libChanged = countTimers(tl2, "library-changed");
  log("S2: " + JSON.stringify(out.steps.s2));
  check("S2 前：「全部」显示剩余全部书", out.steps.s2.before.cards === totalFiles - 1, out.steps.s2.before.cards);
  check("S2：DB 里确实少了一本", out.steps.s2.dbAfter === out.steps.s2.dbBefore - 1, [out.steps.s2.dbBefore, out.steps.s2.dbAfter]);
  assertMarks("S2(网格/全部)", out.steps.s2.marks, out.steps.s2.dbAfter, out.steps.s2.gone);
  check("S2：索引只重建一次", out.steps.s2.rebuilds === 1, out.steps.s2.rebuilds);
  check("S2：library-changed 监听刷新一次", out.steps.s2.libChanged === 1, out.steps.s2.libChanged);

  // ================= S3：列表视图 + 文件夹 C + 仅移出书库 =================
  await M.evalJs(`localStorage.setItem('tidepage.lib.view','list');1`);
  await reloadApp("after list reload");
  const expectC = BOOKS[2].files.length - 1;
  await M.evalJs(H(`
    await wait(()=>qa('.folders .folder-chip').length>=4,'chips2');
    const c = chip(${J(DIR_C)});
    if (!c) throw new Error('chip C missing');
    c.click();
    await wait(()=>qa('.row').length===${BOOKS[2].files.length},'C rows');
    window.__pre = { before: sample(), dbBefore: await dbTotal(${J(DIR_C)}) };
    return 1;
  `));
  await quiet();
  out.steps.s3 = await M.evalJs(H(`
    const d = await ctxDelete('lib-ctx-remove', { nth: 0 });
    const marks = await markAt(d.t0,[300,1000,3000]);
    return { before: window.__pre.before, dbBefore: window.__pre.dbBefore, dmsg:d.msg, gone:d.title, marks,
             dbAfter: await dbTotal(${J(DIR_C)}), listView: !!q('.list-inner') };
  `));
  const tl3 = timerBatch(); out.steps.s3.rebuilds = countTimers(tl3, "[tidepage] rebuildIndex"); out.steps.s3.libChanged = countTimers(tl3, "library-changed");
  log("S3: " + JSON.stringify(out.steps.s3));
  check("S3：确为列表视图", out.steps.s3.listView === true, out.steps.s3.listView);
  check("S3 前：C 显示 3 行", out.steps.s3.before.rows === 3, out.steps.s3.before);
  check("S3：DB 里确实少了一本", out.steps.s3.dbAfter === out.steps.s3.dbBefore - 1, [out.steps.s3.dbBefore, out.steps.s3.dbAfter]);
  check("S3：列表视图剩余书数正确", out.steps.s3.dbAfter === expectC, [out.steps.s3.dbAfter, expectC]);
  assertMarks("S3(列表/C)", out.steps.s3.marks, out.steps.s3.dbAfter, out.steps.s3.gone);
  check("S3：索引只重建一次", out.steps.s3.rebuilds === 1, out.steps.s3.rebuilds);
  check("S3：library-changed 监听刷新一次", out.steps.s3.libChanged === 1, out.steps.s3.libChanged);

  // 静置 3s 再确认一次（用户报告「放着不管永不恢复」）
  await sleep(3000);
  out.steps.settle = await M.evalJs(H(`return sample();`));
  log("settle: " + JSON.stringify(out.steps.settle));
  check("静置 3s 后 C 仍显示 2 行（不靠外部事件自愈）", out.steps.settle.rows === out.steps.s3.dbAfter, out.steps.settle);

  out.error = null;
} catch (e) {
  out.error = String(e);
  failed++;
  console.error(e);
} finally {
  if (app) { killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles"); }
  const outFile = path.join(SANDBOX_ROOT, "delblank_out.json");
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
  console.log(failed === 0 ? "RESULT: ALL CHECKS PASSED" : `RESULT: ${failed} CHECK(S) FAILED`);
}
process.exit(failed === 0 ? 0 : 1);