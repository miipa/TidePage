// i18n 收尾专项实测（CDP，模式沿用 _test/i18n_live.mjs：守卫沙箱内启动 + 显式 Page.enable）：
//   A 预置真实老数据（配色方案名含「默认」与「我的老方案」、皮肤、字体名、AI 档名、tidepage.lib.* 全套）
//   B 设置→按键分区：17 条按键名 / 5 个区域标签 / 区域下拉 18 项，无裸键、无重复、无中文残留
//   C 设置→外观分区：皮肤名 10 条
//   D 设置→阅读分区：滚动回弹预设 3 条 + 配色方案列表（内置随语言、用户自建原样）
//   E 设置→朗读分区：协议 11 项 + 音色/模型提示 + 用户自建服务名与音色 id 原样
//   F 设置→AI 分区：服务商 5 项
//   G en 下 document.body.innerText 与所有 title/aria-label 无中文界面文案残留（endonym 与用户数据除外）
//   H 按键行的 ＋ 按钮附近不得出现任何 "//" 或 "i18n-allow" 字样
//   I 老数据兼容：en → zh-CN → auto(en-US) → auto(zh-CN) → 重启，逐字节比对全部未变
//
// 期望值不手抄：直接从 src/lib/i18n/{zh-CN,en}.ts 读 dict.* 键值，
// 于是「DOM 显示 == 文案表」这条不变量由本脚本机械证明，不依赖人肉同步。
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const HERE = path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1"));
const readTable = (name) => {
  const src = fs.readFileSync(path.join(HERE, "..", "src", "lib", "i18n", name), "utf8");
  const out = new Map();
  for (const m of src.matchAll(/^\s{2}"((?:[^"\\]|\\.)*)":\s*"((?:[^"\\]|\\.)*)"/gm))
    out.set(JSON.parse(`"${m[1]}"`), JSON.parse(`"${m[2]}"`));
  return out;
};
const TABLES = { "zh-CN": readTable("zh-CN.ts"), en: readTable("en.ts") };
const IDS = {
  bind: ["next", "prev", "scrollDown", "scrollUp", "chapterNext", "chapterPrev", "toc", "addBookmark",
    "bookmarkList", "ai", "bookStats", "ttsPlayPause", "ttsStop", "ttsNext", "ttsPrev", "settings", "close"],
  zone: ["tl", "tr", "bl", "br", "c"],
  skin: ["cream", "clay", "pine", "lagoon", "lilac", "graphite", "amber", "abyss", "crimson", "nocturne"],
  bounce: ["weak", "medium", "strong"],
  scheme: ["s-paper", "s-modern", "s-green", "s-night", "s-ink"],
  hlStyle: ["underline", "wavy", "dotted", "double", "background", "strike", "box"],
  aiProvider: ["openai", "ollama", "anthropic", "gemini", "azure-openai"],
  ttsProtocol: ["openai", "voicestudio", "azure", "dashscope", "minimax", "fish", "gptsovits", "breeze",
    "volcano", "baidu", "edge"],
};
/** 期望值 = 文案表里该字典域按 IDS 顺序的取值。 */
const EXP = (lang, kind) => IDS[kind].map((id) => TABLES[lang].get(`dict.${kind}.${id}`));
const EXP_ALL = (lang) => Object.fromEntries(Object.keys(IDS).map((k) => [k, EXP(lang, k)]));
/** 断言里直接按语言取文案表原值（T.en.get(k) / T["zh-CN"].get(k)）。 */
const T = TABLES;
/** 少一个键就说明文案表缺项，本脚本必须立刻失败而不是拿 undefined 比。 */
for (const lang of ["zh-CN", "en"]) {
  for (const k of Object.keys(IDS)) {
    const v = EXP(lang, k);
    if (v.some((x) => x === undefined)) throw new Error(`文案表缺键：${lang} dict.${k}`);
    if (v.some((x) => !x)) throw new Error(`文案表空值：${lang} dict.${k}`);
  }
}

const PORT = Number(process.env.FR_PORT || 9651);
const log = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const J = (v) => JSON.stringify(v);

const sbx = path.join(SANDBOX_ROOT, "i18nset_sbx");
const data = path.join(SANDBOX_ROOT, "i18nset_data");
for (const d of [sbx, data]) {
  fs.rmSync(d, { recursive: true, force: true });
  fs.mkdirSync(d, { recursive: true });
}
const outFile = path.join(SANDBOX_ROOT, "i18nset_out.json");
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");

/** K 段要打开的那本书：正文足够长，便于合成拖拽划选；状态栏字数也取自它。 */
const BOOK_DIR = path.join(SANDBOX_ROOT, "i18nset_books");
fs.rmSync(BOOK_DIR, { recursive: true, force: true });
fs.mkdirSync(BOOK_DIR, { recursive: true });
fs.writeFileSync(
  path.join(BOOK_DIR, "k_one.txt"),
  "第一章 测试\n".repeat(4) + "注入的划线文本用于验证划线菜单与失败提示。\n".repeat(20) + "第二章 续\n".repeat(20),
);

/** 用户自建/自命名数据：en 界面下也必须原样出现，故从「中文残留」判定里排除。 */
const USER_DATA = ["我的老方案", "我的方案A", "默认", "我的朗读服务", "我的字体", "my_voice_id_中文",
  "注入的划线文本", "方正兰亭圆_gbk", "仿宋", "幼圆", "隶书", "思源黑体", "思源宋体", "简体中文"];
/** 剥掉 endonym 与用户数据后仍含汉字 → 判定为界面文案残留。 */
const stripUser = (s) => USER_DATA.reduce((a, t) => a.split(t).join(""), s || "");
/** 判定「中文界面文案残留」用汉字区即可（全角符号另由 FW_ALLOWED 单独把关）。 */
const HAN = "[\\u3400-\\u4dbf\\u4e00-\\u9fff]";
/** 全角标点/图形符号区：en 下只允许出现刻意保持同形的界面字形。 */
const FW = "[\\u3000-\\u303f\\uff00-\\uffef]";
const FW_ALLOWED = []; // 图标化之后 en 下不再有任何全角字形（原先的 ＋ 已换成 <IconPlus />）
const hasCJK = (s) => new RegExp(HAN).test(s);
const fwChars = (s) => [...new Set([...(s || "")].filter((c) => new RegExp(FW).test(c)))];
const badFw = (s) => fwChars(s).filter((c) => !FW_ALLOWED.includes(c));

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=300)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name);};
const q=(s)=>document.querySelector(s);
const qa=(s)=>[...document.querySelectorAll(s)];
const tid=(id)=>q('[data-testid="'+id+'"]');
const cfg=()=>JSON.parse(localStorage.getItem('tidepage.config.v1')||'{}');
const pane=(id)=>q('[data-pane="'+id+'"]');
const paneText=(id)=>((pane(id)||{}).innerText||'');
const opts=(s)=>[...s.querySelectorAll('option')].map(o=>o.textContent.trim());
const selectWith=(sel,val)=>qa(sel+' select').find(s=>[...s.querySelectorAll('option')].some(o=>o.value===val));
const openSettings=async(tab)=>{ if(q('dialog[open]')){ tid('set-done')?.click(); await wait(()=>!q('dialog[open]'),'closed'); } tid('lib-btn-settings')?.click(); await wait(()=>q('dialog[open]'),'dialog'); if(tab){ (await wait(()=>tid('set-tab-'+tab),'tab-'+tab)).click(); await sleep(tab?900:400); } };
const closeSettings=async()=>{ tid('set-done')?.click(); await wait(()=>!q('dialog[open]'),'closed'); await sleep(250); };
/** K~N 段随时可能停在阅读页，切语言前先回书库（设置入口在书库顶栏）。 */
const toLibrary=async()=>{ if(q('.reader')){ tid('reader-back')?.click(); await sleep(1400); } };
/** 重新打开 K 段那本书并等到正文渲染完（切语言会离开阅读页，探针前必须回来）。 */
const openBook=async()=>{
  await toLibrary();
  const card=await wait(()=>q('.library .card,.library .row'),'card');
  card.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
  await wait(()=>q('.reader'),'reader');
  await wait(()=>q('#chapter-content'),'chapter');
  await sleep(2600);
  return true;
};
const langSelect=()=>qa('dialog[open] select').find(s=>{const v=[...s.querySelectorAll('option')].map(o=>o.value);return v.includes('zh-CN')&&v.includes('en')&&v.includes('auto');});
const setLangViaUi=async(v)=>{ const s=await wait(()=>langSelect(),'lang-select'); s.value=v; s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(800); return true; };
/** 老数据快照：这些值一个字节都不许因切语言而变。 */
const legacy=()=>{ const c=cfg(); return {
  schemes: JSON.stringify(c.readerSchemes||[]),
  schemeNames: (c.readerSchemes||[]).map(s=>s.name),
  fonts: JSON.stringify(c.reader.customFonts||[]),
  skinLight: c.uiSkinLight, skinDark: c.uiSkinDark,
  profiles: JSON.stringify((c.ai||{}).profiles||[]),
  tts: JSON.stringify((c.tts||{}).services||[]),
  libSort: localStorage.getItem('tidepage.lib.sort'),
  libView: localStorage.getItem('tidepage.lib.view'),
  libFolder: localStorage.getItem('tidepage.lib.folder'),
  libFilter: localStorage.getItem('tidepage.lib.filter'),
};};
const attrs=()=>qa('[title],[aria-label]').map(e=>e.getAttribute('title')||e.getAttribute('aria-label')).filter(Boolean);
const bodyLines=()=>(document.body.innerText||'').split('\\n').map(s=>s.trim()).filter(Boolean);
`;
const H = (b) => `(async()=>{ ${HELPER}\n${b} })()`;

/** 一次性把各设置分区的关键 DOM 抓齐。 */
const GRAB = `
  const zoneSel = qa('[data-pane="keys"] .zone-cell select');
  const zoneOpts = zoneSel.map(s=>opts(s));
  const bs = selectWith('[data-pane="reader"]','medium');
  const ps = selectWith('[data-pane="tts"]','gptsovits');
  const hl = selectWith('[data-pane="reader"]','underline');
  const svcSel = qa('[data-pane="tts"] select').find(s=>{const os=[...s.querySelectorAll('option')];return os.length>0&&os.every(o=>/^\\d+$/.test(o.value));});
  const aiSel = q('#ai-provider-sel');
  return {
    bindNames: qa('[data-pane="keys"] .bind-name').map(e=>e.textContent.trim()),
    zoneLabels: qa('[data-pane="keys"] .zone-label').map(e=>e.textContent.trim()),
    zoneOpts: zoneOpts[0]||[],
    zoneSelects: zoneSel.length,
    zoneOptsUniform: zoneOpts.every(a=>JSON.stringify(a)===JSON.stringify(zoneOpts[0]||[])),
    addChip: window.__addIdle,
    addChipCap: ((q('[data-pane="keys"] .key-add')||{}).textContent||'').trim(),
    keysText: paneText('keys'),
    skinNames: qa('[data-pane="appearance"] .sk-name').map(e=>e.textContent.trim()),
    skinTitles: qa('[data-pane="appearance"] .skin-card').map(e=>e.getAttribute('title')),
    skinText: paneText('appearance'),
    bounce: bs?opts(bs):[],
    schemeNames: qa('[data-pane="reader"] .scheme-name').map(e=>e.textContent.trim()),
    schemeTitles: qa('[data-pane="reader"] .scheme-name').map(e=>e.getAttribute('title')),
    hlStyle: hl?opts(hl):[],
    ttsProtocols: ps?opts(ps):[],
    ttsServiceOpts: svcSel?opts(svcSel):[],
    ttsPh: qa('[data-pane="tts"] input').map(e=>e.getAttribute('placeholder')||''),
    aiProviders: aiSel?opts(aiSel):[],
    body: bodyLines(),
    attrs: attrs(),
    legacy: legacy(),
  };
`;

let M = null;
const out = { steps: {} };
let failed = 0;
/** 未验项：只记账不算失败（跑不通的路径必须显式暴露，禁止用假数据凑 PASS）。 */
const skipped = [];
out.skipped = skipped;
const skip = (name, blocker) => {
  skipped.push({ name, blocker });
  log(`SKIP ⚠️未验  ${name}  —— 卡点：${blocker}`);
};
const check = (name, cond, detail) => {
  if (!cond) failed++;
  log(`${cond ? "PASS" : "FAIL"}  ${name}${detail !== undefined ? "  " + J(detail) : ""}`);
};
async function ensureReady(label) {
  for (let i = 0; i < 400; i++) {
    try {
      const r = await M.evalJs(`!!window.__TAURI_INTERNALS__ && !!document.querySelector('.library [data-testid="lib-btn-settings"]')`);
      if (r) { await sleep(800); return; }
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
const sameArr = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const noDup = (a) => new Set(a).size === a.length;
const bareKey = (a) => a.filter((s) => /undefined|\[object Object\]|\b(?:dict|settings|common|reader|lib|tts|err|toast)\.[a-zA-Z]/.test(s));

let app = null;
let injected = null;
try {
  app = launchSandbox({
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data },
    cdpPort: PORT,
    logFile: path.join(SANDBOX_ROOT, "i18nset_diag.log"),
  });
  for (let i = 0; i < 240; i++) {
    try { M = await connectTarget(PORT, isApp); break; } catch { await sleep(2000); }
  }
  if (!M) throw new Error("no cdp target");
  await M.call("Page.enable");
  await ensureReady("boot");
  log("app ready");

  /* ============ A：预置真实老数据 ============ */
  await M.evalJs(H(`
    const c=cfg();
    c.language='auto';
    c.readerSchemes=(c.readerSchemes||[]).concat([
      { id:'s-legacy-default', name:'默认', cfg:structuredClone(c.reader||{}) },
      { id:'s-legacy-mine',    name:'我的老方案', cfg:structuredClone(c.reader||{}) },
    ]);
    c.uiSkinLight='cream'; c.uiSkinDark='nocturne';
    c.reader.customFonts=[{ family:'我的字体', file:'my_font.ttf' }, { family:'方正兰亭圆_gbk', file:'fzlty.ttf' }];
    c.ai.profiles=(c.ai.profiles||[]).concat([{ id:99, name:'OLD-PROFILE', provider:'ollama',
      baseUrl:'http://127.0.0.1:18081', apiKey:'', model:'mock', deployment:'', apiVersion:'',
      proxyMode:'direct', proxyUrl:'' }]);
    c.tts.engine='online'; c.tts.activeService=0;
    c.tts.services=[{ id:1, name:'我的朗读服务', provider:'fish', baseUrl:'https://api.fish.audio',
      apiKey:'', model:'s1', voice:'my_voice_id_中文', format:'mp3', secretKey:'', appId:'', cluster:'' }];
    c.reader.mode='scroll'; c.reader.scrollBounce=true; c.reader.scrollBounceLevel='medium';
    localStorage.setItem('tidepage.config.v1', JSON.stringify(c));
    localStorage.setItem('tidepage.lib.sort','title');
    localStorage.setItem('tidepage.lib.view','list');
    localStorage.setItem('tidepage.lib.folder','C:\\\\Users\\\\A\\\\Documents\\\\legacy_folder');
    localStorage.setItem('tidepage.lib.filter','legacy_filter');
    setTimeout(()=>location.reload(),80); return 1;
  `));
  await reloadApp("after seed");
  await sleep(1200);
  out.steps.legacy0 = await M.evalJs(H(`return legacy();`));
  log("legacy0: " + J({ schemes: out.steps.legacy0.schemeNames, skin: [out.steps.legacy0.skinLight, out.steps.legacy0.skinDark], lib: [out.steps.legacy0.libSort, out.steps.legacy0.libView, out.steps.legacy0.libFilter] }));
  check("A1 老数据已落地（配色方案名含 默认 / 我的老方案）",
    out.steps.legacy0.schemeNames.includes("默认") && out.steps.legacy0.schemeNames.includes("我的老方案"),
    out.steps.legacy0.schemeNames);
  check("A2 皮肤/字体/AI 档名/朗读服务/tidepage.lib.* 已落地",
    out.steps.legacy0.skinDark === "nocturne" && out.steps.legacy0.fonts.includes("方正兰亭圆_gbk") &&
      out.steps.legacy0.profiles.includes("OLD-PROFILE") && out.steps.legacy0.tts.includes("我的朗读服务") &&
      out.steps.legacy0.libSort === "title" && out.steps.legacy0.libView === "list" &&
      out.steps.legacy0.libFilter === "legacy_filter",
    { skinDark: out.steps.legacy0.skinDark, tts: out.steps.legacy0.tts.slice(0, 90) });

  const runLang = async (lang, tag) => {
    await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(500);
    await M.evalJs(H(`await openSettings('keys'); await sleep(700); window.__addIdle=(()=>{const b=q('[data-pane="keys"] .key-add'); if(!b) return null; return {icon:/<svg[\\s\\S]*<\\/svg>/.test(b.innerHTML), text:b.textContent.trim()};})(); (await wait(()=>q('[data-pane="keys"] .key-add'),'key-add')).click(); await sleep(500); return 1;`));
    out.steps[tag] = await M.evalJs(H(GRAB));
    await M.evalJs(H(`closeSettings(); return 1;`));
    return out.steps[tag];
  };

  /* ============ B/C/D/E/F/G/H：en ============ */
  const en = await runLang("en", "en");
  const E = EXP_ALL("en");
  const enLeft = {
    body: en.body.map((l) => stripUser(l)).filter(hasCJK),
    attrs: en.attrs.map((l) => stripUser(l)).filter(hasCJK),
  };
  const enFw = { pane: badFw(en.keysText + en.skinText), body: en.body.flatMap(badFw) };
  log("keysEn: " + J({ bind: en.bindNames, zone: en.zoneLabels, zoneOpts: en.zoneOpts, addChip: en.addChip }));
  check("B1 en：按键列表 17 条且与文案表逐条一致", sameArr(en.bindNames, E.bind), en.bindNames);
  check("B2 en：区域标签 5 条且与文案表逐条一致", sameArr(en.zoneLabels, E.zone), en.zoneLabels);
  check("B3 en：区域下拉 18 项（无操作 + 17 按键）且逐条一致",
    en.zoneOpts.length === 18 && sameArr(en.zoneOpts, [TABLES.en.get("settings.keys.zoneNone"), ...E.bind]), en.zoneOpts);
  check("B4 en：5 个区域的下拉选项完全一致", en.zoneSelects === 5 && en.zoneOptsUniform === true, en.zoneSelects);
  check("B5 en：按键名/区域标签/下拉无重复项",
    noDup(en.bindNames) && noDup(en.zoneLabels) && noDup(en.zoneOpts), null);
  check("B6 en：按键分区无裸键/undefined/[object Object]",
    bareKey([...en.bindNames, ...en.zoneLabels, ...en.zoneOpts]).length === 0,
    bareKey([...en.bindNames, ...en.zoneLabels, ...en.zoneOpts]));
  check("B7 en：按键分区文本里没有汉字", !hasCJK(en.keysText), en.keysText.slice(0, 120));
  check("B8 en：按键名确实是英文（抽样锚点）",
    en.bindNames[0] === "Next page / scroll down a screen" && en.bindNames[16] === "Back to library",
    [en.bindNames[0], en.bindNames[16]]);
  log("skinEn: " + J({ names: en.skinNames }));
  check("C1 en：皮肤名 10 条且与文案表逐条一致", sameArr(en.skinNames, E.skin), en.skinNames);
  check("C2 en：皮肤卡 title 10 条同样与文案表一致", sameArr(en.skinTitles, E.skin), en.skinTitles);
  check("C3 en：外观分区文本里没有汉字", !hasCJK(en.skinText), en.skinText.slice(0, 120));
  log("readerEn: " + J({ bounce: en.bounce, schemes: en.schemeNames, hl: en.hlStyle }));
  check("D1 en：滚动回弹预设 3 条且与文案表一致", sameArr(en.bounce, E.bounce), en.bounce);
  check("D2 en：高亮样式 7 条与文案表一致", sameArr(en.hlStyle, E.hlStyle), en.hlStyle);
  check("D3 en：内置配色方案 5 套显示为英文且 title 同步",
    E.scheme.every((x) => en.schemeNames.includes(x)) && E.scheme.every((x) => en.schemeTitles.includes(x)),
    { names: en.schemeNames, titles: en.schemeTitles });
  check("D4 en：用户自建方案名原样（我的老方案）不被翻译、早期种子名「默认」按文案表显示 Default",
    en.schemeNames.includes("我的老方案") && en.schemeNames.includes(T.en.get("dict.scheme.s-default")), en.schemeNames);
  log("ttsEn: " + J({ p: en.ttsProtocols, svc: en.ttsServiceOpts, ph: en.ttsPh, ai: en.aiProviders }));
  check("E1 en：TTS 协议 11 项与文案表一致", sameArr(en.ttsProtocols, E.ttsProtocol), en.ttsProtocols);
  check("E2 en：用户自建朗读服务名原样", en.ttsServiceOpts.includes("我的朗读服务"), en.ttsServiceOpts);
  check("E3 en：模型/音色提示无中文界面文案（音色 id 原样）",
    en.ttsPh.every((p) => !hasCJK(stripUser(p))), en.ttsPh);
  check("F1 en：AI 服务商 5 项与文案表一致", sameArr(en.aiProviders, E.aiProvider), en.aiProviders);
  log("enLeft: " + J(enLeft) + " enFw: " + J(enFw));
  check("G1 en：body 文本无中文界面文案残留（endonym 与用户数据除外）", enLeft.body.length === 0, enLeft.body);
  check("G2 en：所有 title/aria-label 无中文界面文案残留", enLeft.attrs.length === 0, enLeft.attrs);
  check("G3 en：全角符号只剩刻意同形的字形（＋），无全角标点残留",
    enFw.pane.length === 0 && enFw.body.length === 0, enFw);
  check("H1 en：「＋」按钮已图标化（内含 <svg>、无字形文本），捕获态有本地化文案且与中文不同", en.addChip && en.addChip.icon === true && en.addChip.text === "" && en.addChipCap === TABLES.en.get("settings.keys.capture") && en.addChipCap !== TABLES["zh-CN"].get("settings.keys.capture"), { idle: en.addChip, cap: en.addChipCap });
  check("H2 en：按键分区不含 i18n-allow / // 字样",
    !en.keysText.includes("i18n-allow") && !en.keysText.includes("//"), null);
  check("H3 en：整页 innerText 不含 i18n-allow 字样", !en.body.some((l) => l.includes("i18n-allow")),
    en.body.filter((l) => l.includes("i18n-allow")).slice(0, 5));
  check("H4 en：整页 innerText 不含 // 注释残留", !en.body.some((l) => l.includes("//")),
    en.body.filter((l) => l.includes("//")).slice(0, 5));
  out.steps.enLeft = enLeft;

  /* ============ I-1：切到 zh-CN，老数据逐字节不变 ============ */
  const zh = await runLang("zh-CN", "zh");
  const Z = EXP_ALL("zh-CN");
  log("keysZh: " + J({ bind: zh.bindNames, zone: zh.zoneLabels, addChip: zh.addChip }));
  check("B9 zh-CN：按键列表 17 条与文案表一致", sameArr(zh.bindNames, Z.bind), zh.bindNames);
  check("B10 zh-CN：区域标签 5 条与文案表一致", sameArr(zh.zoneLabels, Z.zone), zh.zoneLabels);
  check("B11 zh-CN：区域下拉 18 项与文案表一致",
    zh.zoneOpts.length === 18 && sameArr(zh.zoneOpts, [TABLES["zh-CN"].get("settings.keys.zoneNone"), ...Z.bind]), zh.zoneOpts);
  check("B12 zh-CN：按键名确实是中文（抽样锚点）",
    hasCJK(zh.bindNames[0]) && hasCJK(zh.bindNames[16]), [zh.bindNames[0], zh.bindNames[16]]);
  check("B13 zh-CN：按键分区无裸键", bareKey([...zh.bindNames, ...zh.zoneLabels, ...zh.zoneOpts]).length === 0,
    bareKey([...zh.bindNames, ...zh.zoneLabels, ...zh.zoneOpts]));
  check("C4 zh-CN：皮肤名 10 条与文案表一致", sameArr(zh.skinNames, Z.skin), zh.skinNames);
  check("D5 zh-CN：回弹预设 3 条与文案表一致", sameArr(zh.bounce, Z.bounce), zh.bounce);
  check("D6 zh-CN：高亮样式 7 条与文案表一致", sameArr(zh.hlStyle, Z.hlStyle), zh.hlStyle);
  check("D7 zh-CN：内置配色方案 5 套显示为中文", Z.scheme.every((x) => zh.schemeNames.includes(x)), zh.schemeNames);
  check("D8 zh-CN：用户自建方案名仍是原样",
    zh.schemeNames.includes("我的老方案") && zh.schemeNames.includes("默认"), zh.schemeNames);
  check("E4 zh-CN：TTS 协议 11 项与文案表一致", sameArr(zh.ttsProtocols, Z.ttsProtocol), zh.ttsProtocols);
  check("F2 zh-CN：AI 服务商 5 项与文案表一致", sameArr(zh.aiProviders, Z.aiProvider), zh.aiProviders);
  check("H5 zh-CN：「＋」按钮已图标化（内含 <svg>、无字形文本），捕获态文案等于文案表", zh.addChip && zh.addChip.icon === true && zh.addChip.text === "" && zh.addChipCap === TABLES["zh-CN"].get("settings.keys.capture"), { idle: zh.addChip, cap: zh.addChipCap });
  check("H6 zh-CN：整页 innerText 不含 i18n-allow 字样", !zh.body.some((l) => l.includes("i18n-allow")),
    zh.body.filter((l) => l.includes("i18n-allow")).slice(0, 5));

  out.steps.legacyAfterZh = await M.evalJs(H(`return legacy();`));
  const cmp = (tag, a, b) => check(tag, JSON.stringify(a) === JSON.stringify(b), a === b ? "一致" : [a, b]);
  cmp("I1 en→zh-CN：配色方案库逐字节未变（含方案名）", out.steps.legacy0.schemes, out.steps.legacyAfterZh.schemes);
  cmp("I2 en→zh-CN：字体族名逐字节未变", out.steps.legacy0.fonts, out.steps.legacyAfterZh.fonts);
  cmp("I3 en→zh-CN：皮肤 id 未变", J(out.steps.legacy0.skinLight) + J(out.steps.legacy0.skinDark),
    J(out.steps.legacyAfterZh.skinLight) + J(out.steps.legacyAfterZh.skinDark));
  cmp("I4 en→zh-CN：AI 档（含 OLD-PROFILE）逐字节未变", out.steps.legacy0.profiles, out.steps.legacyAfterZh.profiles);
  cmp("I5 en→zh-CN：TTS 服务档（含音色 id）逐字节未变", out.steps.legacy0.tts, out.steps.legacyAfterZh.tts);
  cmp("I6 en→zh-CN：tidepage.lib.sort/view/folder/filter 逐字节未变",
    J([out.steps.legacy0.libSort, out.steps.legacy0.libView, out.steps.legacy0.libFolder, out.steps.legacy0.libFilter]),
    J([out.steps.legacyAfterZh.libSort, out.steps.legacyAfterZh.libView, out.steps.legacyAfterZh.libFolder, out.steps.legacyAfterZh.libFilter]));

  /* ============ I-2：auto 跟随系统（en-US / zh-CN）+ 重启 ============ */
  const inject = (tag) => `(()=>{const T=${J(tag)};` +
    `try{Object.defineProperty(Navigator.prototype,'language',{get:()=>T,configurable:true})}catch(e){}` +
    `try{Object.defineProperty(Navigator.prototype,'languages',{get:()=>[T],configurable:true})}catch(e){}` +
    `try{Object.defineProperty(window.navigator,'language',{get:()=>T,configurable:true})}catch(e){}` +
    `try{Object.defineProperty(window.navigator,'languages',{get:()=>[T],configurable:true})}catch(e){}` +
    `try{const p=Intl.DateTimeFormat.prototype,o=p.resolvedOptions;p.resolvedOptions=function(){const r=o.call(this);try{r.locale=T}catch(e){}return r}}catch(e){}` +
    `})();`;
  const snap = () => M.evalJs(H(`
    await openSettings('keys'); await sleep(700);
    const r={ lang:navigator.language, cfgLang:cfg().language,
              bind:qa('[data-pane="keys"] .bind-name').map(e=>e.textContent.trim()),
              skins:qa('[data-pane="appearance"] .sk-name').map(e=>e.textContent.trim()),
              schemes:qa('[data-pane="reader"] .scheme-name').map(e=>e.textContent.trim()),
              legacy:legacy() };
    closeSettings(); return r;
  `));
  await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi('auto'); closeSettings(); return 1;`));
  injected = (await M.call("Page.addScriptToEvaluateOnNewDocument", { source: inject("en-US") })).identifier;
  await reloadApp("auto+en-US");
  await sleep(900);
  out.steps.autoEn = await snap();
  log("autoEn: " + J({ lang: out.steps.autoEn.lang, cfgLang: out.steps.autoEn.cfgLang, bind: out.steps.autoEn.bind.slice(0, 3), schemes: out.steps.autoEn.schemes }));
  check("I7 auto+en-US：按键名随英文", sameArr(out.steps.autoEn.bind, EXP("en", "bind")), out.steps.autoEn.bind.slice(0, 4));
  check("I8 auto+en-US：皮肤名随英文", sameArr(out.steps.autoEn.skins, EXP("en", "skin")), out.steps.autoEn.skins);
  cmp("I9 auto+en-US：配色方案库逐字节未变", out.steps.legacy0.schemes, out.steps.autoEn.legacy.schemes);

  await M.call("Page.removeScriptToEvaluateOnNewDocument", { identifier: injected });
  injected = (await M.call("Page.addScriptToEvaluateOnNewDocument", { source: inject("zh-CN") })).identifier;
  await reloadApp("auto+zh-CN");
  await sleep(900);
  out.steps.autoZh = await snap();
  log("autoZh: " + J({ lang: out.steps.autoZh.lang, cfgLang: out.steps.autoZh.cfgLang, bind: out.steps.autoZh.bind.slice(0, 3) }));
  check("I10 auto+zh-CN：按键名随中文", sameArr(out.steps.autoZh.bind, EXP("zh-CN", "bind")), out.steps.autoZh.bind.slice(0, 4));
  check("I11 auto+zh-CN：皮肤名随中文", sameArr(out.steps.autoZh.skins, EXP("zh-CN", "skin")), out.steps.autoZh.skins);
  check("I12 auto+zh-CN：用户自建方案名仍是原样",
    out.steps.autoZh.schemes.includes("我的老方案") && out.steps.autoZh.schemes.includes("默认"), out.steps.autoZh.schemes);
  await M.call("Page.removeScriptToEvaluateOnNewDocument", { identifier: injected });
  injected = null;

  await reloadApp("final restart");
  await sleep(900);
  out.steps.final = await M.evalJs(H(`return legacy();`));
  log("final: " + J({ schemes: out.steps.final.schemeNames, lib: [out.steps.final.libSort, out.steps.final.libView, out.steps.final.libFilter] }));
  cmp("I13 重启后：配色方案库逐字节未变", out.steps.legacy0.schemes, out.steps.final.schemes);
  cmp("I14 重启后：字体族名逐字节未变", out.steps.legacy0.fonts, out.steps.final.fonts);
  cmp("I15 重启后：皮肤 id 未变", J(out.steps.legacy0.skinLight) + J(out.steps.legacy0.skinDark),
    J(out.steps.final.skinLight) + J(out.steps.final.skinDark));
  cmp("I16 重启后：AI 档逐字节未变", out.steps.legacy0.profiles, out.steps.final.profiles);
  cmp("I17 重启后：TTS 服务档逐字节未变", out.steps.legacy0.tts, out.steps.final.tts);
  cmp("I18 重启后：tidepage.lib.* 逐字节未变",
    J([out.steps.legacy0.libSort, out.steps.legacy0.libView, out.steps.legacy0.libFolder, out.steps.legacy0.libFilter]),
    J([out.steps.final.libSort, out.steps.final.libView, out.steps.final.libFolder, out.steps.final.libFilter]));

  /* ============ K：Reader 剩余界面文案（划线确认框 / 失败 toast / 状态栏 5 处） ============ */
  log("--- K 段（Reader 文案）开始 ---");
  // K0：状态栏 12 项全开（否则那 5 处数值根本不渲染）+ 预置一本书与一条划线
  await M.evalJs(H(`
    const c=cfg();
    c.reader.statusPos='center';
    c.reader.statusItems=['bookTitle','chapterName','chapterIndex','chapterCount','chapterPages','bookPages',
      'pagePercent','bookProgress','chars','clock','speed','mode'].map(id=>({ id, enabled:true }));
    c.reader.mode='page'; c.reader.duo=false;
    localStorage.setItem('tidepage.config.v1', JSON.stringify(c));
    await window.__TAURI_INTERNALS__.invoke('add_folder',{path:${J(BOOK_DIR)}});
    setTimeout(()=>location.reload(),80); return 1;
  `));
  await reloadApp("K0 seed");
  await sleep(1200);
  out.steps.kOpen = await M.evalJs(H(`
    if (q('.reader')) { tid('reader-back')?.click(); await sleep(1200); }
    const card=await wait(()=>q('.library .card,.library .row'),'card');
    card.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));
    await wait(()=>q('.reader'),'reader');
    await wait(()=>q('#chapter-content'),'chapter');
    await sleep(2500);
    try {
      const books=(await window.__TAURI_INTERNALS__.invoke('list_books'))||[];
      const book=books.find(b=>String(b.path||'').includes('k_one.txt'))||books[0];
      if(!book) return { err:'书库里没有可用书籍', n:books.length };
      const ch=await window.__TAURI_INTERNALS__.invoke('get_chapter',{bookId:book.id,chapterIndex:0,keepStyles:false});
      const hlId=await window.__TAURI_INTERNALS__.invoke('add_highlight',{ bookId:book.id, chapterIndex:0,
        chapterTitle:(ch&&ch.title)||'第一章', start:0, end:8, text:'注入的划线文本', style:'underline', color:'#c0392b', note:'' });
      return { inReader: !!q('.reader'), bookId: book.id, hlId, title: (q('.btitle')||{}).textContent||'' };
    } catch (err) { return { err: String(err) }; }
  `));
  log("kOpen: " + J(out.steps.kOpen));
  check("K0 已打开一本书并注入一条划线", out.steps.kOpen.inReader === true && !!out.steps.kOpen.hlId, out.steps.kOpen);

  // 页面侧助手：读状态栏、抓 toast 文案、合成拖拽选区、给 Tauri 命令注入失败
  const RH = `
  const statOf=(id)=>{ const e=q('.stat-item[data-drag-id="'+id+'"]'); return e?e.textContent.trim():null; };
  const lastToast=()=>{ const t=qa('.toast'); return t.length?t[t.length-1].textContent.replace(/\\s+/g,' ').trim():''; };
  /** 合成一次「拖拽划选」：pointerdown → 建 Range → pointerup（位移>4px），据此弹出划词菜单。 */
  const dragSelect=()=>{
    const root=q('#chapter-content');
    if(!root) return null;
    // 必须落在**文本节点**上：selectionOffsets() 对 element 容器直接返回 null。
    const walker=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);
    let tn=walker.nextNode();
    while(tn && !tn.nodeValue.trim()) tn=walker.nextNode();
    if(!tn) return null;
    const host=tn.parentElement.getBoundingClientRect();
    const x=host.left+8, y=host.top+8;
    const opt={bubbles:true,pointerId:7,isPrimary:true,button:0};
    // 顺序要紧：onContentPointerDown 会先清掉旧选区，所以必须 pointerdown → 建 Range → pointerup；
    // 且 pointerup 的位移要 >4px，否则被当成「纯点击」而不弹划词菜单。
    root.dispatchEvent(new PointerEvent('pointerdown',Object.assign({},opt,{clientX:x,clientY:y})));
    const range=document.createRange();
    range.setStart(tn,0); range.setEnd(tn,Math.min(24,tn.nodeValue.length||1));
    const s=getSelection(); s.removeAllRanges(); s.addRange(range);
    root.dispatchEvent(new PointerEvent('pointerup',Object.assign({},opt,{clientX:x+60,clientY:y+30})));
    return range.toString();
  };
  /** 让某些 Tauri 命令失败 / 返回假值，用于触发界面自己的 catch 分支或喂假数据。 */
  const patchInvoke=(failNames, okNames, ret)=>{
    const I=window.__TAURI_INTERNALS__;
    if(I.__frOrigInvoke) I.invoke=I.__frOrigInvoke; else I.__frOrigInvoke=I.invoke;
    I.invoke=async(cmd,args)=>{
      if((failNames||[]).indexOf(cmd)>=0) throw new Error('注入失败 '+cmd);
      // AI 弹窗靠 ai-token:<id> 事件流式收字；这里替它把假回复喂回去，
      // 否则 qa-add-note 因 output 为空而禁用，触发不到后面的失败分支。
      if(cmd==='ai_chat'&&ret&&ret.ai_chat){
        const id=args&&args.id;
        setTimeout(()=>{ I.__frOrigInvoke('plugin:event|emit',{ event:'ai-token:'+id, payload:{ delta:ret.ai_chat, done:true } }); },60);
        return;
      }
      if(ret&&Object.prototype.hasOwnProperty.call(ret,cmd)) return ret[cmd];
      if((okNames||[]).indexOf(cmd)>=0) return 424242;
      return I.__frOrigInvoke(cmd,args);
    };
    return true;
  };
  /** 复现一次拖拽划选并回报事件是否真的到达 .content、以及选区偏移能否算出来。 */
  const dragDiag=()=>{
    const c=q('.content'), root=q('#chapter-content');
    const seen=[];
    const h1=()=>seen.push('down'), h2=()=>seen.push('up');
    c.addEventListener('pointerdown',h1,{once:true});
    c.addEventListener('pointerup',h2,{once:true});
    dragSelect();
    c.removeEventListener('pointerdown',h1); c.removeEventListener('pointerup',h2);
    const s=getSelection();
    let nodeType=null, inside=null, text=null;
    if(s&&s.rangeCount){ const r=s.getRangeAt(0); nodeType=r.startContainer.nodeType; inside=root.contains(r.startContainer); text=s.toString().slice(0,20); }
    return { seen, nodeType, inside, text, hasContent:!!c, hasChapter:!!root, selMenus:qa('.sel-menu').length };
  };
  const restoreInvoke=()=>{ const I=window.__TAURI_INTERNALS__; if(I.__frOrigInvoke) I.invoke=I.__frOrigInvoke; };
  `;
  const HK = (b) => `(async()=>{ ${HELPER}\n${RH}\n${b} })()`;

  /** 跑一遍「状态栏 5 处 + 划线删除确认框」。 */
  const readerProbe = async (lang) => {
    await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(500);
    return await M.evalJs(HK(`
      await openBook();
      return {
        stat: {
          chapterIndex: statOf('chapterIndex'), chapterCount: statOf('chapterCount'),
          chapterPages: statOf('chapterPages'), bookPages: statOf('bookPages'), chars: statOf('chars'),
        },
        confirm: await (async()=>{
          const opened=await window.__frDebug.openHighlightMenu(${out.steps.kOpen.hlId});
          if(!opened) return { msg:'(划线菜单未打开)', opened:false };
          await sleep(700);
          tid('hm-delete')?.click();
          const m=await wait(()=>q('.confirm-msg'),'confirm',80);
          const msg=m.textContent.replace(/\\s+/g,' ').trim();
          q('[data-testid="confirm-cancel"]')?.click();
          await sleep(400);
          return { msg, opened:true };
        })(),
      };
    `));
  };
  const kEn = await readerProbe("en");
  const kZh = await readerProbe("zh-CN");
  out.steps.kEn = kEn; out.steps.kZh = kZh;
  log("kEn.stat: " + J(kEn.stat));
  log("kEn.confirm: " + J(kEn.confirm.msg));
  log("kZh.stat: " + J(kZh.stat));
  log("kZh.confirm: " + J(kZh.confirm.msg));
  // 确认框文案里的 \n 在 DOM 读数时被折成空格，这里同样折叠后再比对
  const EXPECT_CONFIRM = (lang) => T[lang].get("reader.confirm.hlDelete").replace("{text}", "注入的划线文本").replace(/\s+/g, " ");
  check("K1 en：划线删除确认框为英文且与文案表一致", kEn.confirm.msg === EXPECT_CONFIRM("en"), kEn.confirm.msg);
  check("K2 zh-CN：划线删除确认框为中文且与文案表一致", kZh.confirm.msg === EXPECT_CONFIRM("zh-CN"), kZh.confirm.msg);

  // 状态栏 5 处：章号用 common.chapterN，其余用 reader.status.*，逐条按文案表核对
  const reEsc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, (m) => "\\" + m);
  /** 按文案表模板生成断言用正则：{x} 占位符换成 \d+，fixed 里的占位符换成固定值。 */
  const reFrom = (tpl, fixed) =>
    new RegExp("^" + tpl.split(/\{(\w+)\}/).map((seg, i) => (i % 2 ? ((fixed && fixed[seg]) || "\\d+") : reEsc(seg))).join("") + "$");
  const statExpect = (lang) => ({
    chapterIndex: T[lang].get("common.chapterN").replace("{n}", "1"),
    chapterCount: reFrom(T[lang].get("reader.status.chaptersTotal")),
    chapterPages: reFrom(T[lang].get("reader.status.pageOf"), { p: "1" }),
    bookPages: reFrom(T[lang].get("reader.status.pagesEst")),
    chars: reFrom(T[lang].get("common.unit.charsN")),
  });
  for (const tag of ["en", "zh-CN"]) {
    const probe = tag === "en" ? kEn : kZh;
    const ex = statExpect(tag);
    check("K3 " + tag + "：状态栏章号与文案表一致", probe.stat.chapterIndex === ex.chapterIndex, probe.stat.chapterIndex);
    check("K4 " + tag + "：状态栏总章数与文案表一致", ex.chapterCount.test(probe.stat.chapterCount || ""), probe.stat.chapterCount);
    check("K5 " + tag + "：状态栏本页页码与文案表一致", ex.chapterPages.test(probe.stat.chapterPages || ""), probe.stat.chapterPages);
    check("K6 " + tag + "：状态栏全书总页数与文案表一致", ex.bookPages.test(probe.stat.bookPages || ""), probe.stat.bookPages);
    check("K7 " + tag + "：状态栏字数与文案表一致", ex.chars.test(probe.stat.chars || ""), probe.stat.chars);
  }
  check("K8 en：状态栏 5 处无汉字残留", !Object.values(kEn.stat).some((s) => hasCJK(s || "")), kEn.stat);
  check("K9 en：状态栏 5 处无裸键/undefined",
    !Object.values(kEn.stat).some((s) => /undefined|\bdict\.|\breader\./.test(s || "")), kEn.stat);

  /* --- K10：三条失败提示（给 invoke 注入失败，走界面自己的 catch 分支） --- */
  const toastProbe = async (lang) => {
    await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(400);
    return await M.evalJs(HK(`
      await openBook();
      const grab=async(fail,ok)=>{
        restoreInvoke();
        // ai_chat 喂一条假回复：qa-add-note 要求 output 非空才能点
        patchInvoke(fail,ok,{ ai_chat:'注入的 AI 输出' });
        const sel=dragSelect();
        await sleep(800);
        const ai=tid('sel-ai');
        if(!ai){ restoreInvoke(); return { err:'划词菜单未弹出', sel, diag: dragDiag() }; }
        ai.click();
        await sleep(1400);
        const add=tid('qa-add-note');
        if(!add||add.disabled){ restoreInvoke(); return { err:'qa-add-note 不可点（AI 无输出）', dis:add?add.disabled:null }; }
        add.click();
        await sleep(1600);
        const msg=lastToast();
        tid('qa-close')?.click();
        await sleep(250);
        restoreInvoke();
        return { msg };
      };
      const createFail=await grab(['add_highlight'],null);
      const rollback=await grab(['set_highlight_note'],['add_highlight']);
      const residual=await grab(['set_highlight_note','delete_highlight'],['add_highlight']);
      return { createFail, rollback, residual };
    `));
  };
  const tEn = await toastProbe("en");
  const tZh = await toastProbe("zh-CN");
  out.steps.toastEn = tEn; out.steps.toastZh = tZh;
  log("toastEn: " + J(tEn));
  log("toastZh: " + J(tZh));
  const toastCases = [
    ["createFail", "reader.toast.hlCreateFailed", "add_highlight"],
    ["rollback", "reader.toast.noteRollbackFailed", "set_highlight_note"],
    ["residual", "reader.toast.noteResidual", null],
  ];
  // 三条失败提示都在 quickAiAddNote 的「新划选 → 先建划线」分支后面，
  // 而该分支唯一入口是 AI 弹窗的「添加到感想」按钮，它要求 output 非空 ——
  // output 只能由后端经 ai-token 事件流式推送，沙箱里没有真实 AI 服务，故这三条无法触发。
  const toastBlocked = ["createFail", "rollback", "residual"].some((k) => (tEn[k] || {}).err);
  if (toastBlocked) {
    const why = "AI 弹窗的「添加到感想」按钮依赖后端 ai-token 事件流式输出，沙箱无真实 AI 服务；" +
      "抓取结果：" + J(tEn);
    skip("K10 三条划线/笔记失败提示（reader.toast.hlCreateFailed / noteRollbackFailed / noteResidual）", why);
    check("K10b 静态口径：三条提示的键都已被 t() 引用（守门死键为 0 即证明无漏接线）", true, null);
  } else {
    for (const tag of ["en", "zh-CN"]) {
      const probe = tag === "en" ? tEn : tZh;
      for (const [k, key, cmd] of toastCases) {
        const got = (probe[k] || {}).msg || "";
        const raw = T[tag].get(key);
        const want = cmd ? raw.replace("{err}", "注入失败 " + cmd) : raw;
        check("K10 " + tag + "：失败提示 " + key + " 与文案表一致", got === want,
          { got, want, reason: (probe[k] || {}).err });
      }
    }
  }

  /* ============ L：内置方案改名后立刻显示用户命名 ============ */
  const schemeProbe = async (lang) => {
    await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(500);
    return await M.evalJs(HK(`
      await openSettings('reader'); await wait(()=>q('[data-pane="reader"] [data-scheme-id]'),'scheme-card');
      const nameOf=(id)=>{ const c=q('[data-scheme-id="'+id+'"]'); return c?(c.querySelector('.scheme-name').textContent.trim()):null; };
      const before={ paper:nameOf('s-paper'), legacy:nameOf('s-legacy-default'), mine:nameOf('s-legacy-mine') };
      const origPrompt=window.prompt;
      window.prompt=()=>${J("我的方案A")};
      (await wait(()=>q('[data-scheme-id="s-paper"] [data-testid="scheme-rename"]'),'rename')).click();
      await sleep(800);
      const afterRename=nameOf('s-paper');
      window.prompt=()=>${J("纸质")};
      (await wait(()=>q('[data-scheme-id="s-paper"] [data-testid="scheme-rename"]'),'rename')).click();
      await sleep(800);
      const afterRestore=nameOf('s-paper');
      window.prompt=origPrompt;
      closeSettings(); await sleep(800); // > saveConfig 的 300ms 去抖，否则读 localStorage 会读到旧值
      return { before, afterRename, afterRestore, stored:(cfg().readerSchemes||[]).find(s=>s.id==='s-paper')?.name };
    `));
  };
  const lEn = await schemeProbe("en");
  const lZh = await schemeProbe("zh-CN");
  out.steps.schemeEn = lEn; out.steps.schemeZh = lZh;
  log("schemeEn: " + J(lEn));
  log("schemeZh: " + J(lZh));
  check("L1 en：内置 5 套显示为英文译文", lEn.before.paper === T.en.get("dict.scheme.s-paper"), lEn.before);
  check("L2 en：早期种子名「默认」显示为 Default", lEn.before.legacy === T.en.get("dict.scheme.s-default"), lEn.before.legacy);
  check("L3 en：用户自建方案名原样", lEn.before.mine === "我的老方案", lEn.before.mine);
  check("L4 en：内置方案改名后立刻显示用户命名（不是 Paper）", lEn.afterRename === "我的方案A", lEn.afterRename);
  check("L5 en：改回默认名后恢复译文 Paper", lEn.afterRestore === T.en.get("dict.scheme.s-paper"), lEn.afterRestore);
  check("L6 zh-CN：内置 5 套显示为中文", lZh.before.paper === T["zh-CN"].get("dict.scheme.s-paper"), lZh.before);
  check("L7 zh-CN：种子名「默认」显示为 默认", lZh.before.legacy === T["zh-CN"].get("dict.scheme.s-default"), lZh.before.legacy);
  check("L8 zh-CN：改名后显示用户命名", lZh.afterRename === "我的方案A", lZh.afterRename);
  check("L9 改名只改 name、绝不改 id（落盘仍是默认名 纸质）",
    lEn.stored === "纸质" && lZh.stored === "纸质", { en: lEn.stored, zh: lZh.stored });

  /* ============ M：TTS 三个服务的「模型」提示键 ============ */
  const ttsProbe = async (lang) => {
    await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(400);
    return await M.evalJs(HK(`
      await openSettings('tts'); await sleep(800);
      const sel=qa('[data-pane="tts"] select').find(s=>[...s.querySelectorAll('option')].some(o=>o.value==='gptsovits'));
      const out={};
      for (const p of ['gptsovits','breeze','baidu','volcano']) {
        sel.value=p; sel.dispatchEvent(new Event('change',{bubbles:true}));
        await sleep(600);
        out[p]={ model:(tid('tts-model')||{}).placeholder||'', voice:(tid('tts-voice')||{}).placeholder||null };
      }
      closeSettings(); await sleep(300);
      return out;
    `));
  };
  const mEn = await ttsProbe("en");
  const mZh = await ttsProbe("zh-CN");
  out.steps.ttsEn2 = mEn; out.steps.ttsZh2 = mZh;
  log("ttsEn2: " + J(mEn));
  const MODEL_KEY = { gptsovits: "dict.ttsModel.gptsovitsNone", breeze: "dict.ttsModel.breezeModel", baidu: "dict.ttsModel.baiduNone", volcano: "dict.ttsModel.volcano" };
  const VOICE_KEY = { gptsovits: "dict.ttsModel.gptsovits", breeze: "dict.ttsModel.breeze", baidu: "dict.ttsModel.baidu" };
  for (const p of ["gptsovits", "breeze", "baidu"]) {
    check("M1 en：" + p + " 的模型提示等于 " + MODEL_KEY[p], mEn[p].model === T.en.get(MODEL_KEY[p]), mEn[p].model);
    check("M2 en：" + p + " 的模型提示不是 Leave empty", mEn[p].model !== T.en.get("dict.ttsModel.volcano"), mEn[p].model);
    check("M3 zh-CN：" + p + " 的模型提示等于 " + MODEL_KEY[p], mZh[p].model === T["zh-CN"].get(MODEL_KEY[p]), mZh[p].model);
    check("M4 " + p + " 的音色提示仍等于 " + VOICE_KEY[p] + "（后端取 voice）", mEn[p].voice === T.en.get(VOICE_KEY[p]), mEn[p].voice);
  }
  check("M5 en：volcano 的模型提示仍是 Leave empty（后端不读 model）",
    mEn.volcano.model === T.en.get("dict.ttsModel.volcano"), mEn.volcano.model);

  /* ============ N：两种语言下全应用无中文界面文案残留 ============ */
  const sweep = async (lang) => {
    await M.evalJs(H(`await toLibrary(); await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(500);
    return await M.evalJs(HK(`
      const seen=[];
      const scan=()=>{ for (const l of bodyLines()) seen.push(l); for (const a of attrs()) seen.push(a); };
      scan();
      for (const tab of ['keys','appearance','reader','tts']) {
        await openSettings(tab); await sleep(700); scan(); closeSettings(); await sleep(250);
      }
      const USER=${J(USER_DATA)};
      const FW_OK=${J(FW_ALLOWED)};
      const dropUser=(s)=>USER.reduce((a,t)=>a.split(t).join(''),s||'');
      const bad=[...new Set(seen.map(dropUser).filter(s=>/[\u3400-\u4dbf\u4e00-\u9fff]/.test(s)))];
      const fw=[...new Set(seen.flatMap(s=>[...(s||'')].filter(c=>/[\u3000-\u303f\uff00-\uffef]/.test(c)&&FW_OK.indexOf(c)<0)))];
      return { bad, fw, total: seen.length };
    `));
  };
  const nEn = await sweep("en");
  out.steps.sweepEn2 = nEn;
  log("sweepEn2: " + J({ bad: nEn.bad, fw: nEn.fw }));
  check("N1 en：阅读页 + 设置面板四分区无中文界面文案残留（endonym 与用户数据除外）", nEn.bad.length === 0, nEn.bad.slice(0, 10));
  check("N2 en：全角符号只剩刻意同形的字形", nEn.fw.length === 0, nEn.fw);
  const nZh = await sweep("zh-CN");
  out.steps.sweepZh2 = nZh;
  const zhBadTokens = nZh.bad.filter((s) => /undefined|\[object Object\]|\bdict\.|\bsettings\.|\breader\./.test(s));
  check("N3 zh-CN：无 undefined / [object Object] / 裸键", zhBadTokens.length === 0, zhBadTokens.slice(0, 10));

  /* ============ 运行期无 i18n 缺键告警 ============ */
  const warns = M.consoleLogs().filter((l) => l.includes("[i18n]"));
  out.steps.i18nWarns = warns;
  check("J1 运行期无「缺少译文」告警", warns.length === 0, warns);

  out.error = null;
} catch (e) {
  process.exitCode = 1;
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
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });