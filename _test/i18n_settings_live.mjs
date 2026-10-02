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

/** 用户自建/自命名数据：en 界面下也必须原样出现，故从「中文残留」判定里排除。 */
const USER_DATA = ["我的老方案", "默认", "我的朗读服务", "我的字体", "my_voice_id_中文",
  "方正兰亭圆_gbk", "仿宋", "幼圆", "隶书", "思源黑体", "思源宋体", "简体中文"];
/** 剥掉 endonym 与用户数据后仍含汉字 → 判定为界面文案残留。 */
const stripUser = (s) => USER_DATA.reduce((a, t) => a.split(t).join(""), s || "");
/** 判定「中文界面文案残留」用汉字区即可（全角符号另由 FW_ALLOWED 单独把关）。 */
const HAN = "[\\u3400-\\u4dbf\\u4e00-\\u9fff]";
/** 全角标点/图形符号区：en 下只允许出现刻意保持同形的界面字形。 */
const FW = "[\\u3000-\\u303f\\uff00-\\uffef]";
const FW_ALLOWED = ["＋"]; // 与守卫 EN_GLYPH_ALLOWED 同口径：全角加号是图形符号，不是文案
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
const openSettings=async(tab)=>{ tid('lib-btn-settings')?.click(); await wait(()=>q('dialog[open]'),'dialog'); if(tab) tid('set-tab-'+tab)?.click(); await sleep(tab?600:250); };
const closeSettings=async()=>{ tid('set-done')?.click(); await sleep(300); };
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
    addChip: (q('[data-pane="keys"] .key-add')||{}).textContent?.trim(),
    addChipTitles: qa('[data-pane="keys"] .key-add').map(e=>e.getAttribute('title')),
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
    await M.evalJs(H(`await openSettings(); await setLangViaUi(${J(lang)}); closeSettings(); return 1;`));
    await sleep(500);
    await M.evalJs(H(`await openSettings('keys'); await sleep(700); return 1;`));
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
  check("D4 en：用户自建方案名原样（我的老方案 / 默认），未被翻译",
    en.schemeNames.includes("我的老方案") && en.schemeNames.includes("默认"), en.schemeNames);
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
  check("H1 en：「＋」按钮文本就是 ＋ 本身", en.addChip === "＋", en.addChip);
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
  check("H5 zh-CN：「＋」按钮文本就是 ＋ 本身", zh.addChip === "＋", zh.addChip);
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
  await M.evalJs(H(`await openSettings(); await setLangViaUi('auto'); closeSettings(); return 1;`));
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