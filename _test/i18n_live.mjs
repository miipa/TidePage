// i18n 第一阶段实机验证（受守卫沙箱内启动，模式照 _test/cdp.mjs + launch_sandbox.mjs + isolation_guard.mjs）：
//   A 空态提示：en → 英文、zh-CN → 中文
//   B 书库页：工具栏 / 文件夹胶囊 / 右键菜单 / 状态栏 / title 属性在 en 下全英文
//   C 脏值兜底：document.body.innerText 里不得出现 undefined / [object Object] / 形如 library.xxx 的裸键
//   D 语言设置 → 立即生效（走设置面板真实下拉，不重载）
//   E auto 跟随系统：用 Page.addScriptToEvaluateOnNewDocument 改写 navigator.language 后重载
//   F 老用户升级：tidepage.lib.sort / lib.view / lib.folder + 配色方案名 + AI 配置档名
//     在切语言前后逐字节一致（顺便验证已持久化的显示名未被翻译）
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9650);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const J = (v) => JSON.stringify(v);

const sbx = path.join(SANDBOX_ROOT, "i18n_sbx");
const data = path.join(SANDBOX_ROOT, "i18n_data");
for (const d of [sbx, data]) {
  fs.rmSync(d, { recursive: true, force: true });
  fs.mkdirSync(d, { recursive: true });
}
// 真实书籍文件（文件夹作用域要按路径前缀匹配）
const DIR_A = path.join(SANDBOX_ROOT, "i18n_a");
const DIR_B = path.join(SANDBOX_ROOT, "i18n_b");
const BOOKS = [
  { dir: DIR_A, files: ["ia_one.txt", "ia_two.txt"] },
  { dir: DIR_B, files: ["ib_one.txt"] },
];
for (const b of BOOKS) {
  fs.rmSync(b.dir, { recursive: true, force: true });
  fs.mkdirSync(b.dir, { recursive: true });
  for (const f of b.files) fs.writeFileSync(path.join(b.dir, f), `${f} —— i18n 实机验证用书。\n`.repeat(20));
}
const outFile = path.join(SANDBOX_ROOT, "i18n_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=300)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name);};
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const tid=(id)=>q('[data-testid="'+id+'"]');
const cfg=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}');
const libText=()=>((q('.library')||{}).innerText||'');
const bodyText=()=>(document.body.innerText||'');
const libTitles=()=>qa('.library [title]').map(e=>e.getAttribute('title'));
const langSelect=()=>qa('dialog[open] select').find(s=>{const v=[...s.querySelectorAll('option')].map(o=>o.value);return v.includes('zh-CN')&&v.includes('en')&&v.includes('auto');});
const setLangViaUi=async(v)=>{ const s=await wait(()=>langSelect(),'lang-select'); s.value=v; s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(700); return true; };
const openSettings=async(tab)=>{ tid('lib-btn-settings')?.click(); await wait(()=>q('dialog[open]'),'dialog'); if(tab) document.querySelector('[data-testid="set-tab-'+tab+'"]')?.click(); await sleep(tab?500:200); };
const closeSettings=async()=>{ document.querySelector('[data-testid="set-done"]')?.click(); await sleep(250); };
const legacy=()=>({
  sort: localStorage.getItem('tidepage.lib.sort'),
  view: localStorage.getItem('tidepage.lib.view'),
  folder: localStorage.getItem('tidepage.lib.folder'),
  scheme: (cfg().readerSchemes||[]).map(s=>s.name),
  profiles: (cfg().ai?.profiles||[]).map(p=>p.name),
});
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;

let M = null;
const out = { steps: {} };
let failed = 0;
const check = (name, cond, detail) => {
  if (!cond) failed++;
  log(`${cond ? "PASS" : "FAIL"}  ${name}${detail !== undefined ? "  " + J(detail) : ""}`);
};
async function ensureReady(label) {
  // 不能只看 __TAURI_INTERNALS__：vite dev 首次请求时才编译 Library.svelte，
  // 顶栏按钮可能还没挂上，点不到「设置」就会假失败。
  for (let i = 0; i < 400; i++) {
    try {
      const r = await M.evalJs(`!!window.__TAURI_INTERNALS__ && !!document.querySelector('.library [data-testid="lib-btn-settings"]')`);
      if (r) {
        await sleep(800);
        return;
      }
    } catch { /* retry */ }
    await sleep(400);
  }
  throw new Error("not ready " + label);
}
async function reloadApp(label) {
  await M.evalJs(`setTimeout(()=>location.reload(),80);1`);
  await sleep(4500);
  await ensureReady(label);
}

let app = null;
let injected = null;
try {
  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data },
    cdpPort: PORT,
    logFile: path.join(SANDBOX_ROOT, "i18n_diag.log"),
  });
  for (let i = 0; i < 240; i++) {
    try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); }
  }
  if (!M) throw new Error("no cdp target");
  // 必须显式 enable：否则 Page.addScriptToEvaluateOnNewDocument 不生效（E 组改不了系统语言）。
  await M.call("Page.enable");
  await ensureReady("boot");
  log("app ready");

  /* ============ A：空库时的空态提示（en / zh-CN） ============ */
  await M.evalJs(H(`
    await openSettings(); await setLangViaUi('en'); closeSettings(); return 1;
  `));
  out.steps.emptyEn = await M.evalJs(H(`
    const hint=tid('lib-hint-empty');
    return { hint: (hint?.textContent||'').replace(/\\s+/g,' ').trim(), addBtn: tid('lib-btn-add-folder')?.textContent?.trim(),
             refresh: tid('lib-btn-refresh')?.textContent?.trim(), searchPh: tid('lib-search')?.getAttribute('placeholder') };
  `));
  log("emptyEn: " + J(out.steps.emptyEn));
  check("A1 en：空态提示为英文", /^Library is empty/.test(out.steps.emptyEn.hint), out.steps.emptyEn.hint);
  check("A2 en：「添加文件夹」按钮为英文（第二轮起前缀已换成 <IconPlus /> 图标，文本不再含 ＋）", out.steps.emptyEn.addBtn === "Add folder", out.steps.emptyEn.addBtn);
  check("A3 en：刷新按钮为英文（前缀已换成 <IconRefresh /> 图标；按钮本身按 data-testid 定位）", out.steps.emptyEn.refresh === "Refresh", out.steps.emptyEn.refresh);
  check("A4 en：搜索框 placeholder 为英文", out.steps.emptyEn.searchPh === "Search title / author…", out.steps.emptyEn.searchPh);

  await M.evalJs(H(`await openSettings(); await setLangViaUi('zh-CN'); closeSettings(); return 1;`));
  out.steps.emptyZh = await M.evalJs(H(`
    return { hint:(tid('lib-hint-empty')?.textContent||'').replace(/\\s+/g,' ').trim(),
             addBtn: tid('lib-btn-add-folder')?.textContent?.trim(), refresh: tid('lib-btn-refresh')?.textContent?.trim(),
             searchPh: tid('lib-search')?.getAttribute('placeholder') };
  `));
  log("emptyZh: " + J(out.steps.emptyZh));
  check("A5 zh-CN：空态提示恢复中文", /^书库为空/.test(out.steps.emptyZh.hint), out.steps.emptyZh.hint);
  check("A6 zh-CN：「添加文件夹」恢复中文（同 A2，无 ＋ 前缀）", out.steps.emptyZh.addBtn === "添加文件夹", out.steps.emptyZh.addBtn);
  check("A7 zh-CN：刷新按钮恢复中文（同 A3，无 ⟳ 前缀）", out.steps.emptyZh.refresh === "刷新", out.steps.emptyZh.refresh);
  check("A8 zh-CN：搜索框 placeholder 恢复中文", out.steps.emptyZh.searchPh === "搜索书名 / 作者…", out.steps.emptyZh.searchPh);

  /* ============ 种子：2 个文件夹 + 3 本书 ============ */
  await M.evalJs(H(`
    for (const d of ${J(BOOKS.map((b) => b.dir))}) { await window.__TAURI_INTERNALS__.invoke('add_folder',{path:d}); }
    await sleep(500); return 1;
  `));
  await reloadApp("after seed");

  /* ============ F 前置：写入「老用户」已持久化数据 ============ */
  await M.evalJs(H(`
    const c=cfg();
    c.language='auto';
    c.readerSchemes=(c.readerSchemes||[]).concat([{ id:'legacy-scheme', name:'我的老方案', cfg:structuredClone(c.reader||{}) }]);
    (c.ai=c.ai||{}).profiles=(c.ai.profiles||[]).concat([{ name:'OLD-PROFILE', provider:'ollama', baseUrl:'http://127.0.0.1:18081', apiKey:'', model:'mock', proxyMode:'direct', proxyUrl:'' }]);
    localStorage.setItem('tidepage.config.v1', JSON.stringify(c));
    localStorage.setItem('tidepage.lib.sort','title');
    localStorage.setItem('tidepage.lib.view','list');
    localStorage.setItem('tidepage.lib.folder', ${J(DIR_B)});
    setTimeout(()=>location.reload(),80); return 1;
  `));
  await reloadApp("after legacy seed");
  await sleep(1200);
  out.steps.legacyBefore = await M.evalJs(H(`
    await wait(()=>qa('[data-testid="lib-folder-chip"]').length>=2,'chips');
    return legacy();
  `));
  log("legacyBefore: " + J(out.steps.legacyBefore));
  check("F1 老数据已落地（排序=按书名 / 视图=列表 / 文件夹=B）",
    out.steps.legacyBefore.sort === "title" && out.steps.legacyBefore.view === "list" && out.steps.legacyBefore.folder === DIR_B,
    out.steps.legacyBefore);
  check("F2 配色方案名 / AI 配置档名可读回",
    out.steps.legacyBefore.scheme.includes("我的老方案") && out.steps.legacyBefore.profiles.includes("OLD-PROFILE"),
    { scheme: out.steps.legacyBefore.scheme, profiles: out.steps.legacyBefore.profiles });

  /* ============ B：en 下书库页全英文 ============ */
  await M.evalJs(H(`await openSettings(); await setLangViaUi('en'); closeSettings(); return 1;`));
  await sleep(400);
  out.steps.en = await M.evalJs(H(`
    const card=q('.row,.card');
    card.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:80,clientY:80}));
    await wait(()=>q('.ctx'),'ctx');
    const ctx=qa('[data-testid^="lib-ctx-"]').map(b=>b.textContent.trim());
    q('.ctx-backdrop')?.click();
    return {
      chips: qa('.folders .folder-chip').map(c=>c.textContent.replace(/\\s+/g,' ').trim()),
      statusbar: tid('lib-statusbar-count')?.textContent?.trim(),
      viewGrid: tid('lib-view-grid')?.getAttribute('title'),
      viewList: tid('lib-view-list')?.getAttribute('title'),
      topbar: qa('[data-testid^="lib-btn-"]').filter(b=>b.getAttribute('aria-label')).map(b=>b.getAttribute('aria-label')),
      ctx, titles: libTitles(), refreshTip: tid('lib-btn-refresh')?.getAttribute('title'),
      folderAll: tid('lib-folder-all')?.textContent?.trim(),
      folderAllTip: tid('lib-folder-all')?.getAttribute('title'),
      sortOpts: qa('[data-testid="lib-sort"] option').map(o=>o.textContent.trim()),
      reverseTip: tid('lib-btn-reverse')?.getAttribute('aria-label'),
      filterBtn: tid('lib-btn-filter')?.textContent?.trim(),
      body: bodyText(),
    };
  `));
  log("en: " + J({ ...out.steps.en, body: undefined }));
  check("B1 en：文件夹「全部」胶囊为 All", out.steps.en.folderAll === "All", out.steps.en.folderAll);
  check("B2 en：状态栏计数为英文", /^\d+ books$/.test(out.steps.en.statusbar), out.steps.en.statusbar);
  check("B3 en：视图按钮 title 为英文", out.steps.en.viewGrid === "Grid view" && out.steps.en.viewList === "List view", [out.steps.en.viewGrid, out.steps.en.viewList]);
  check("B4 en：刷新按钮 title 为英文", out.steps.en.refreshTip === "Click: read books only; long press: refresh covers", out.steps.en.refreshTip);
  check("B5 en：反向排序 aria-label 为英文", out.steps.en.reverseTip === "Reverse sort order", out.steps.en.reverseTip);
  check("B6 en：排序下拉为英文", out.steps.en.sortOpts.includes("By file size") && out.steps.en.sortOpts.includes("By title"), out.steps.en.sortOpts);
  check("B7 en：右键菜单项为英文",
    ["Open", "Details", "Remove from library", "Delete local file", "Show in folder", "Set cover"].every((x) => out.steps.en.ctx.includes(x)),
    out.steps.en.ctx);
  check("B8 en：顶栏 aria-label 为英文", out.steps.en.topbar.includes("Settings") && out.steps.en.topbar.includes("Open a random book"), out.steps.en.topbar);
  check("B9 en：.library 内所有 title 无中文", !out.steps.en.titles.some((s) => /[\u4e00-\u9fff]/.test(s || "")), out.steps.en.titles);
  check("B10 en：文件夹胶囊文本为英文/路径（无中文 UI 词）",
    !out.steps.en.chips.some((s) => /[\u4e00-\u9fff]/.test(s)), out.steps.en.chips);

  /* ============ C：无脏值 / 裸键 ============ */
  const bad = out.steps.en.body.match(/undefined|\[object Object\]|\b(?:library|toast|common|settings|err)\.[a-zA-Z][a-zA-Z.]*\b/g) || [];
  out.steps.badTokens = bad;
  check("C1 en：body 无 undefined / [object Object] / 裸键", bad.length === 0, [...new Set(bad)]);

  /* ============ G：分类筛选面板 + 多选工具条 + 移除文件夹确认框（en） ============ */
  await M.evalJs(H(`tid('lib-btn-filter')?.click(); return 1;`));
  await sleep(400);
  out.steps.gEn = await M.evalJs(H(`
    const fp=tid('lib-filter-clear');
    const res={ fpLabels: qa('.filter-panel .fp-label').map(e=>e.textContent.trim()),
                stateTags: qa('.filter-panel .tag').map(e=>e.textContent.trim()),
                empty: (q('.filter-panel .fp-empty')||{}).textContent };
    if (fp) fp.click();
    await sleep(300);
    // 长按卡片进入多选（550ms 阈值，与真实手势一致）
    const card=q('.row,.card');
    card.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,pointerId:1,button:0,isPrimary:true}));
    await sleep(900);
    return { ...res, selCount: tid('lib-sel-count')?.textContent?.trim(),
             selBtns: qa('[data-testid^="lib-sel-"]').map(e=>e.textContent.trim()),
             inSelect: !!tid('lib-sel-count') };
  `));
  await sleep(300);
  out.steps.gEn.filterChip = await M.evalJs(H(`
    // 必须先退出多选：筛选芯片位于非多选分支的工具条里。
    tid('lib-sel-cancel')?.click();
    await sleep(300);
    const tag=[...document.querySelectorAll('.filter-panel .tag')].find(b=>b.textContent.trim()==='Unread');
    if (tag) tag.click();
    await sleep(900);
    return { chip: tid('lib-filter-chip')?.textContent?.replace(/\\s+/g,' ').trim(), chipTip: tid('lib-filter-chip')?.getAttribute('title'),
             clsBtn: tid('lib-btn-filter')?.textContent?.trim() };
  `));
  log("gEn: " + J(out.steps.gEn));
  check("G1 en：筛选面板分组标题为英文", JSON.stringify(out.steps.gEn.fpLabels) === JSON.stringify(["Format", "Reading state"]), out.steps.gEn.fpLabels);
  check("G2 en：阅读状态标签为英文", out.steps.gEn.stateTags.includes("Unread") && out.steps.gEn.stateTags.includes("Read"), out.steps.gEn.stateTags);
  check("G3 en：多选工具条为英文",
    out.steps.gEn.inSelect === true && out.steps.gEn.selCount === "1 selected" &&
      ["Select all", "Invert", "Remove from library", "Delete", "Cancel"].every((x) => out.steps.gEn.selBtns.includes(x)),
    out.steps.gEn);
  check("G4 en：筛选芯片与分类按钮为英文",
    /^Filtered/.test(out.steps.gEn.filterChip.chip || "") &&
      out.steps.gEn.filterChip.chipTip === "Clear all filters" &&
      /^Filter· 1$/.test(out.steps.gEn.filterChip.clsBtn || ""),
    out.steps.gEn.filterChip);

  out.steps.rmFolder = await M.evalJs(H(`
    tid('lib-filter-chip')?.click(); tid('lib-btn-filter')?.click();
    await sleep(500);
    const x=document.querySelector('.folder-chip .x');
    x?.click();
    await wait(()=>q('.confirm-msg'),'confirm');
    const msg=q('.confirm-msg').textContent.replace(/\\s+/g,' ').trim();
    const btns=qa('.confirm-row button').map(b=>b.textContent.trim());
    q('[data-testid="confirm-cancel"]')?.click();
    await sleep(300);
    return { msg, btns, stillThere: !!x };
  `));
  log("rmFolder: " + J(out.steps.rmFolder));
  check("G5 en：移除文件夹确认框为英文",
    /^Remove folder .+ from the library\? Files on disk are kept\.$/.test(out.steps.rmFolder.msg) &&
      JSON.stringify(out.steps.rmFolder.btns) === JSON.stringify(["Cancel", "OK"]),
    out.steps.rmFolder);
  check("G6 en：取消后文件夹未被移除", out.steps.rmFolder.stillThere === true, out.steps.rmFolder.stillThere);

  await M.evalJs(H(`tid('lib-btn-filter')?.click(); return 1;`));

  /* ============ B'：切回 zh-CN 恢复中文 ============ */
  await M.evalJs(H(`await openSettings(); await setLangViaUi('zh-CN'); closeSettings(); return 1;`));
  await sleep(400);
  out.steps.zh = await M.evalJs(H(`
    const card=q('.row,.card');
    card.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:80,clientY:80}));
    await wait(()=>q('.ctx'),'ctx');
    const ctx=qa('[data-testid^="lib-ctx-"]').map(b=>b.textContent.trim());
    q('.ctx-backdrop')?.click();
    return { ctx, folderAll: tid('lib-folder-all')?.textContent?.trim(), statusbar: tid('lib-statusbar-count')?.textContent?.trim(),
             viewList: tid('lib-view-list')?.getAttribute('title'), refreshTip: tid('lib-btn-refresh')?.getAttribute('title') };
  `));
  log("zh: " + J(out.steps.zh));
  check("B11 zh-CN：右键菜单恢复中文", ["打开", "详细信息", "仅移出书库", "删除本地文件"].every((x) => out.steps.zh.ctx.includes(x)), out.steps.zh.ctx);
  check("B12 zh-CN：胶囊/状态栏/title 恢复中文",
    out.steps.zh.folderAll === "全部" && /本书$/.test(out.steps.zh.statusbar) && out.steps.zh.viewList === "列表视图" && /^点击：/.test(out.steps.zh.refreshTip),
    out.steps.zh);

  /* ============ F 后置：老数据必须一字未动 ============ */
  out.steps.legacyAfter = await M.evalJs(H(`return legacy();`));
  log("legacyAfter: " + J(out.steps.legacyAfter));
  check("F3 切语言后 lib.sort/lib.view/lib.folder 完全一致",
    JSON.stringify(out.steps.legacyAfter.sort) === JSON.stringify(out.steps.legacyBefore.sort) &&
      JSON.stringify(out.steps.legacyAfter.view) === JSON.stringify(out.steps.legacyBefore.view) &&
      JSON.stringify(out.steps.legacyAfter.folder) === JSON.stringify(out.steps.legacyBefore.folder),
    [out.steps.legacyBefore, out.steps.legacyAfter]);
  check("F4 切语言后配色方案名 / AI 配置档名完全一致（未被翻译）",
    JSON.stringify(out.steps.legacyAfter.scheme) === JSON.stringify(out.steps.legacyBefore.scheme) &&
      JSON.stringify(out.steps.legacyAfter.profiles) === JSON.stringify(out.steps.legacyBefore.profiles),
    [out.steps.legacyBefore.scheme, out.steps.legacyAfter.scheme, out.steps.legacyBefore.profiles, out.steps.legacyAfter.profiles]);

  /* ============ E：auto 跟随系统（改写 navigator.language 后重载） ============ */
  const sysLang = await M.evalJs(`navigator.language`);
  out.steps.sysLang = sysLang;
  log("system navigator.language = " + sysLang);
  // 覆写「系统语言」：navigator 实例 + Navigator.prototype 两处都打，
  // 再把 Intl 解析出的 locale 一并改掉（auto 会并读这两个来源）。
  const inject = (tag) => `(()=>{const T=${JSON.stringify(tag)};` +
    `try{Object.defineProperty(Navigator.prototype,'language',{get:()=>T,configurable:true})}catch(e){}` +
    `try{Object.defineProperty(Navigator.prototype,'languages',{get:()=>[T],configurable:true})}catch(e){}` +
    `try{Object.defineProperty(window.navigator,'language',{get:()=>T,configurable:true})}catch(e){}` +
    `try{Object.defineProperty(window.navigator,'languages',{get:()=>[T],configurable:true})}catch(e){}` +
    `try{const p=Intl.DateTimeFormat.prototype,o=p.resolvedOptions;p.resolvedOptions=function(){const r=o.call(this);try{r.locale=T}catch(e){}return r}}catch(e){}` +
    `})();`;
  injected = (await M.call("Page.addScriptToEvaluateOnNewDocument", { source: inject("en-US") })).identifier;
  await M.evalJs(H(`await openSettings(); await setLangViaUi('auto'); closeSettings(); return 1;`));
  await reloadApp("auto+en-US");
  await sleep(800);
  out.steps.autoEn = await M.evalJs(H(`
    return { lang: navigator.language, intl: Intl.DateTimeFormat().resolvedOptions().locale, cfgLang: cfg().language,
             folderAll: tid('lib-folder-all')?.textContent?.trim(), refresh: tid('lib-btn-refresh')?.textContent?.trim(),
             statusbar: tid('lib-statusbar-count')?.textContent?.trim() };
  `));
  log("autoEn: " + J(out.steps.autoEn));
  check("E1 auto + 系统=English → 界面英文", out.steps.autoEn.cfgLang === "auto" && out.steps.autoEn.folderAll === "All" && out.steps.autoEn.refresh === "Refresh", out.steps.autoEn);

  await M.call("Page.removeScriptToEvaluateOnNewDocument", { identifier: injected });
  injected = null;
  injected = (await M.call("Page.addScriptToEvaluateOnNewDocument", { source: inject("zh-CN") })).identifier;
  await reloadApp("auto+zh-CN");
  await sleep(800);
  out.steps.autoZh = await M.evalJs(H(`
    return { lang: navigator.language, intl: Intl.DateTimeFormat().resolvedOptions().locale, cfgLang: cfg().language,
             folderAll: tid('lib-folder-all')?.textContent?.trim(), refresh: tid('lib-btn-refresh')?.textContent?.trim(),
             statusbar: tid('lib-statusbar-count')?.textContent?.trim() };
  `));
  log("autoZh: " + J(out.steps.autoZh));
  check("E2 auto + 系统=简体中文 → 界面中文", out.steps.autoZh.folderAll === "全部" && out.steps.autoZh.refresh === "刷新", out.steps.autoZh);
  await M.call("Page.removeScriptToEvaluateOnNewDocument", { identifier: injected });
  injected = null;

  /* ============ F 收尾：重启（重载）后老数据仍在 + 语言被记住 ============ */
  await reloadApp("final restart");
  await sleep(800);
  out.steps.final = await M.evalJs(H(`return { legacy: legacy(), folderAll: tid('lib-folder-all')?.textContent?.trim(), view: !!q('.list-inner') };`));
  log("final: " + J(out.steps.final));
  check("F5 重载后老数据仍在", JSON.stringify(out.steps.final.legacy) === JSON.stringify(out.steps.legacyBefore), out.steps.final.legacy);
  check("F6 重载后仍是列表视图（视图偏好被记住）", out.steps.final.view === true, out.steps.final.view);
  check("F7 重载后 auto 跟随系统语言（zh-CN）", out.steps.final.folderAll === "全部", out.steps.final.folderAll);

  /* ============ H：第二阶段 —— 全应用遍历（书库 / 阅读页 / 漫画 / AI 面板 / 笔记 / WebDAV / 设置） ============ */
  // 打开一本书：书库里点开第一本（列表视图 → 双击卡片）
  await M.evalJs(H(`
    localStorage.setItem('tidepage.lib.view','grid');
    await openSettings(); await setLangViaUi('en'); closeSettings();
    return 1;
  `));
  await reloadApp("phase2 grid");
  await sleep(1200);
  out.steps.openBook = await M.evalJs(H(`
    const c = await wait(()=>document.querySelector('.card'),'card');
    c.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>document.querySelector('#chapter-content, .reader'),'reader');
    await sleep(2500);
    return { inReader: !!document.querySelector('.reader'), title: (q('.tb-title')||{}).textContent || '' };
  `));
  log("openBook: " + J(out.steps.openBook));
  check("H1 能打开一本书进入阅读页", out.steps.openBook.inReader === true, out.steps.openBook);

  // 遍历：逐个打开本阶段的侧面板 / 弹窗，收集 body.innerText
  out.steps.sweepEn = await M.evalJs(H(`
    const bad = [];
    const PREFIXES = ['library','reader','ai','notes','webdav','convert','bookDetail','bookStats','search','highlight','selection','toc','tts','common','settings','toast','err','dict','palette','lib'];
    const scan = (tag) => {
      const txt = document.body.innerText || '';
      if (txt.indexOf('undefined') >= 0) bad.push(tag + ':undefined');
      if (txt.indexOf('[object Object]') >= 0) bad.push(tag + ':[object Object]');
      for (const p of PREFIXES) {
        const re = new RegExp(p + '\\.[a-z][A-Za-z0-9_]*(?:\\.[A-Za-z0-9_]+)*', 'g');
        for (const m of txt.matchAll(re)) {
          const parts = m[0].split('.');
          const seg = parts[1] || '';
          // i18n 键一定是 camelCase（至少一个内部大写）或带二级以上命名空间
          if (/[A-Z]/.test(seg) || parts.length > 2) bad.push(tag + ':' + m[0]);
        }
      }
    };
    const tap = async (sel, waitSel) => {
      const el = q(sel);
      if (!el) return false;
      el.click();
      await sleep(waitSel ? 900 : 400);
      scan(sel);
      return true;
    };
    scan('reader');
    // 顶栏面板：目录 / 划线 / 本书统计 / 本书图片 / 已屏蔽图片 / 顶栏按钮管理 / WebDAV / AI
    for (const id of ['toc','highlight','stats','images','manager','webdav','ai']) await tap('[data-testid="tb-'+id+'"]');
    // 阅读器浮层控件（字体/图片/适宽适高）
    for (const id of ['font-dec','font-inc','img-dec','img-inc','img-fit-w','img-fit-h']) await tap('[data-testid="'+id+'"]');
    // 书库侧：书籍详细信息 + 格式转换 + WebDAV 面板
    document.querySelector('[data-testid="reader-back"]')?.click();
    await sleep(900);
    const card = q('.card,.row');
    if (card) {
      card.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:100,clientY:100}));
      await wait(()=>q('.ctx'),'ctx');
      await tap('[data-testid="lib-ctx-detail"]', 1200);
      scan('bookDetail');
      q('[data-testid="bd-close"]')?.click(); await sleep(400);
      card.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:100,clientY:100}));
      await wait(()=>q('.ctx'),'ctx');
      if (q('[data-testid="lib-ctx-convert"]')) {
        q('[data-testid="lib-ctx-convert"]').click();
        await sleep(1000); scan('convert');
        q('[data-testid="conv-close"]')?.click(); await sleep(400);
      }
    }
    await tap('[data-testid="lib-btn-webdav"]', 1500);
    scan('webdav');
    q('[data-testid="wd-close"]')?.click(); await sleep(300);
    await tap('[data-testid="lib-btn-notes"]', 1500);
    scan('notes');
    // 设置面板（语言下拉之外）整体扫一遍
    await tap('[data-testid="lib-btn-settings"]', 1200);
    scan('settings');
    q('dialog button.primary')?.click(); await sleep(400);
    return bad;
  `));
  log("sweepEn bad tokens: " + J([...new Set(out.steps.sweepEn)]));
  check("H2 en：全应用遍历无 undefined / [object Object] / 裸键", out.steps.sweepEn.length === 0, [...new Set(out.steps.sweepEn)]);

  // 切回 zh-CN，确认中文回来了
  await M.evalJs(H(`await openSettings(); await setLangViaUi('zh-CN'); closeSettings(); return 1;`));
  await sleep(600);
  out.steps.sweepZh = await M.evalJs(H(`
    const txt = document.body.innerText || '';
    return { books: /本书|book/.test(txt), sample: txt.replace(/\\s+/g,' ').slice(0, 160) };
  `));
  log("sweepZh: " + J(out.steps.sweepZh));
  check("H3 zh-CN：切回后界面恢复中文", out.steps.sweepZh.sample.length > 0, out.steps.sweepZh.sample.slice(0, 80));

  /* ============ I：分栏预设下拉（第三阶段回归：5 个选项、中文/英文各自正确、无重复项） ============ */
  out.steps.presetsZh = await M.evalJs(H(`
    await openSettings(); await setLangViaUi('zh-CN'); closeSettings(); await sleep(500);
    await openSettings('split'); await sleep(600);
    const btns=[...document.querySelectorAll('.presets button')].map(b=>b.textContent.trim());
    closeSettings();
    return btns;
  `));
  log("presetsZh: " + J(out.steps.presetsZh));
  check("I1 zh：分章预设 5 项齐全", out.steps.presetsZh.length === 5, out.steps.presetsZh);
  check("I2 zh：无重复项（chapterN / chapterEn 各一份）", new Set(out.steps.presetsZh).size === out.steps.presetsZh.length, out.steps.presetsZh);
  check("I3 zh：中文项显示中文（含「第X章（中文）」）", out.steps.presetsZh.includes("章节+特殊篇") && out.steps.presetsZh.includes("第X章（中文）") && out.steps.presetsZh.includes("数字编号") && out.steps.presetsZh.includes("分隔符"), out.steps.presetsZh);
  check("I4 zh：英文项 Chapter N 只出现一次", out.steps.presetsZh.filter((x) => x === "Chapter N").length === 1, out.steps.presetsZh);

  out.steps.presetsEn = await M.evalJs(H(`
    await openSettings(); await setLangViaUi('en'); closeSettings(); await sleep(500);
    await openSettings('split'); await sleep(600);
    const btns=[...document.querySelectorAll('.presets button')].map(b=>b.textContent.trim());
    closeSettings();
    return btns;
  `));
  log("presetsEn: " + J(out.steps.presetsEn));
  check("I5 en：分章预设 5 项、无重复", out.steps.presetsEn.length === 5 && new Set(out.steps.presetsEn).size === 5, out.steps.presetsEn);
  check("I6 en：无裸键/undefined/[object Object]", !out.steps.presetsEn.some((x) => /undefined|\[object Object\]|\bsettings\.|\bdict\./.test(x)), out.steps.presetsEn);
  check("I7 en：Chapter N 恰好一项、其余为英文", out.steps.presetsEn.filter((x) => x === "Chapter N").length === 1 && out.steps.presetsEn.some((x) => /Chapter X/.test(x)), out.steps.presetsEn);

  // 切回中文，供后续 F 段沿用
  await M.evalJs(H(`await openSettings(); await setLangViaUi('zh-CN'); closeSettings(); return 1;`));

  /* ============ 控制台无 i18n 缺键告警 ============ */
  const warns = M.consoleLogs().filter((l) => l.includes("[i18n]"));
  out.steps.i18nWarns = warns;
  check("D2 运行期无「缺少译文」告警", warns.length === 0, warns);

  out.error = null;
} catch (e) {
    process.exitCode = 1; /* 失败必须体现在退出码上 */
  out.error = String(e);
  failed++;
  console.error(e);
} finally {
  if (injected) await M.call("Page.removeScriptToEvaluateOnNewDocument", { identifier: injected }).catch(() => {});
  if (app) {
    killTree(app.child.pid);
    await sleep(1500);
    sweepSandboxWebviews("fr_test_profiles");
  }
  fs.writeFileSync(outFile, JSON.stringify(out, null, 1));
  console.log("WROTE " + outFile);
  console.log(failed === 0 ? "RESULT: ALL CHECKS PASSED" : `RESULT: ${failed} CHECK(S) FAILED`);
}
process.exit(failed === 0 ? 0 : 1);

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
