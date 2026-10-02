// i18n 防遗漏守门脚本（无新依赖，node 直接跑）：
//   node _test/i18n_guard.mjs
//   FR_GUARD_VERBOSE=1 node _test/i18n_guard.mjs    额外打印行内豁免明细
//
// 判定口径：src/**/*.svelte 与 src/**/*.ts 里「界面位置」残留的中文字面量。
//   - 跳过 src/lib/i18n/**（文案表本身）、*.d.ts、src/lib/api.ts（纯 IPC 封装）。
//   - 跳过行注释、块注释、正则字面量、Svelte 模板里的 HTML 注释与 <style>/<script> 块
//     （<script> 内的字符串另按「字符串字面量」扫）。
//   - Svelte：模板文本节点、title|aria-label|placeholder|alt 属性值、其余字符串字面量。
//   - TS：字符串字面量。
// 豁免（两种，都必须写明原因）：
//   1) 违规行本身或其上一非空行带 `// i18n-allow:原因` → 只豁免那一行。
//   2) 下方 FILE_WHITELIST 里**逐文件**列出 → 豁免整个文件（禁止整目录/通配）。
// 退出码：非白名单文件仍有残留中文 → 1。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.resolve(HERE, "..", "src");
const VERBOSE = process.env.FR_GUARD_VERBOSE === "1";

/** 永远跳过的目录（相对 src，正斜杠）。 */
const SKIP_DIRS = ["lib/i18n"];
/** 永远跳过的文件（相对 src，正斜杠）。 */
const SKIP_FILES = ["lib/api.ts"];

/**
 * 「本阶段尚未迁移」的逐文件白名单。
 * 规则：一条 = 一个文件 + 原因；新增/移除都必须显式改这里，禁止目录级通配。
 * 第一阶段已迁移：components/Library.svelte、routes/+page.svelte，
 * 以及 components/SettingsModal.svelte 里的「通用→界面语言」一小块（故该文件仍在名单内）。
 */
const FILE_WHITELIST = {
  "components/SettingsModal.svelte":
    "第三阶段主战场：本阶段只接了「通用→界面语言」一小块，其余分区（阅读排版/外观/漫画/朗读/分屏/AI/按键/WebDAV/其他）全部保留中文",
};


const CJK = /[\u3000-\u303f\u4e00-\u9fff\uff00-\uffef]/;
const ALLOW_RE = /\/\/\s*i18n-allow\s*[:：]\s*(.+)$/;
const ATTR_NAMES = "title|aria-label|placeholder|alt";
/** 正则字面量可以出现的前一个实义字符（简化判定：本仓库足够用）。 */
const REGEX_PREV = new Set("(,=:[!&|?;{}".split(""));
const KEYWORD_PREV = /(^|[^\w$])(return|typeof|case|in|of|do|else|yield|await|new|delete|void)$/;

const rel = (p) => path.relative(SRC, p).split(path.sep).join("/");
function walk(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (/\.(svelte|ts)$/.test(e.name)) out.push(p);
  }
  return out;
}
const blankRange = (a, s, e) => {
  for (let i = Math.max(0, s); i < e && i < a.length; i++) if (a[i] !== "\n") a[i] = " ";
};
const clone = (src) => src.split("");

/** 挖掉行注释/块注释（字符串与正则保留给后续扫描），字符数不变以便行号映射。 */
function stripJsComments(src) {
  const out = clone(src);
  const n = src.length;
  let i = 0;
  while (i < n) {
    const c = src[i];
    if (c === "/" && src[i + 1] === "/") {
      let j = i;
      while (j < n && src[j] !== "\n") j++;
      blankRange(out, i, j);
      i = j;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      let j = i;
      while (j < n && !(src[j] === "*" && src[j + 1] === "/")) j++;
      j = Math.min(n, j + 2);
      blankRange(out, i, j);
      i = j;
      continue;
    }
    i++;
  }
  return out.join("");
}

/** 跳过正则字面量，返回结束下标（含 flags）；不是正则则返回 -1。 */
function skipRegex(src, i) {
  let j = i + 1;
  let inClass = false;
  while (j < src.length) {
    const c = src[j];
    if (c === "\\") { j += 2; continue; }
    if (c === "\n") return -1;
    if (c === "[") inClass = true;
    else if (c === "]") inClass = false;
    else if (c === "/" && !inClass) {
      j++;
      while (j < src.length && /[a-z]/.test(src[j])) j++;
      return j;
    }
    j++;
  }
  return -1;
}

/** 抽取字符串字面量（含模板字符串），跳过正则字面量。返回 [{start, content}]。 */
function literals(src) {
  const out = [];
  const n = src.length;
  let i = 0;
  let prevSig = "";
  while (i < n) {
    const c = src[i];
    if (c === '"' || c === "'" || c === "`") {
      const q = c;
      let j = i + 1;
      let body = "";
      while (j < n) {
        if (src[j] === "\\") { body += src[j] + (src[j + 1] ?? ""); j += 2; continue; }
        if (src[j] === q) { j++; break; }
        if (src[j] === "\n" && q !== "`") break; // 未闭合的单行字符串
        body += src[j];
        j++;
      }
      out.push({ start: i, content: body });
      i = j;
      prevSig = "x";
      continue;
    }
    if (c === "/" && src[i + 1] !== "/" && src[i + 1] !== "*") {
      const canBeRegex = REGEX_PREV.has(prevSig) || KEYWORD_PREV.test(src.slice(Math.max(0, i - 12), i));
      if (canBeRegex) {
        const j = skipRegex(src, i);
        if (j > 0) { i = j; prevSig = "x"; continue; }
      }
    }
    if (!/\s/.test(c)) prevSig = c;
    i++;
  }
  return out;
}

/** 挖掉 svelte 的 HTML 注释与 <style>/<script> 块（字符数不变）。 */
function svelteRegions(src) {
  const out = clone(src);
  for (const [open, close] of [["<!--", "-->"], ["<style", "</style>"], ["<script", "</script>"]]) {
    let i = 0;
    while (i < src.length) {
      const s = src.indexOf(open, i);
      if (s < 0) break;
      let e = -1;
      if (close === "-->") e = src.indexOf(close, s);
      else {
        const gt = src.indexOf(">", s);
        e = gt < 0 ? -1 : src.indexOf(close, gt);
      }
      if (e < 0) break;
      blankRange(out, s, e + close.length);
      i = e + close.length;
    }
  }
  return out.join("");
}

/** 按花括号配对读出 {…} 的结束下标（含右括号）。 */
function matchBrace(src, open) {
  let depth = 0;
  for (let j = open; j < src.length; j++) {
    if (src[j] === "{") depth++;
    else if (src[j] === "}") { depth--; if (depth === 0) return j + 1; }
  }
  return src.length;
}

/** 找出 <tag …>…</tag> 的**内容**区间列表（跳过嵌套外的同名标签，Svelte 无同名嵌套）。 */
function tagRanges(src, tag) {
  const out = [];
  let i = 0;
  while (i < src.length) {
    const s = src.indexOf("<" + tag, i);
    if (s < 0) break;
    const gt = src.indexOf(">", s);
    if (gt < 0) break;
    const close = "</" + tag + ">";
    const e = src.indexOf(close, gt);
    if (e < 0) break;
    out.push([gt + 1, e]);
    i = e + close.length;
  }
  return out;
}

/* ============================== 采集 ============================== */

const files = walk(SRC, [])
  .filter((p) => {
    const r = rel(p);
    if (p.endsWith(".d.ts")) return false;
    if (SKIP_FILES.includes(r)) return false;
    if (SKIP_DIRS.some((d) => r === d || r.startsWith(d + "/"))) return false;
    return true;
  })
  .sort();

const perFile = new Map();
const lineNo = (src, i) => src.slice(0, i).split("\n").length;

for (const f of files) {
  const raw = fs.readFileSync(f, "utf8");
  const hits = [];
  const push = (line, kind, text) => hits.push({ line, kind, text: String(text).replace(/\s+/g, " ").trim().slice(0, 90) });

  if (f.endsWith(".svelte")) {
    const tpl = svelteRegions(raw);

    // (1) title/aria-label/placeholder/alt 属性值（引号形式或 {…} 表达式形式）
    const attrOut = clone(tpl);
    const attrRe = new RegExp(`\\b(?:${ATTR_NAMES})\\s*=\\s*(["'\`{])`, "g");
    let a;
    while ((a = attrRe.exec(tpl))) {
      const q = a[1];
      let end;
      let value;
      if (q === "{") {
        end = matchBrace(tpl, a.index + a[0].length - 1);
        value = tpl.slice(a.index + a[0].length, end - 1);
      } else {
        end = tpl.indexOf(q, a.index + a[0].length);
        end = end < 0 ? tpl.length : end + 1;
        value = tpl.slice(a.index + a[0].length, end - 1);
      }
      blankRange(attrOut, a.index, end);
      if (CJK.test(value)) push(lineNo(tpl, a.index), "属性值", value);
    }

    // (2) <script> 块里的字符串字面量（先挖注释再抽字面量）
    for (const [s, e] of tagRanges(raw, "script")) {
      const clean = stripJsComments(raw.slice(s, e));
      for (const lit of literals(clean)) {
        if (CJK.test(lit.content)) push(lineNo(raw, s + lit.start), "字符串字面量", lit.content);
      }
    }

    // (3) 模板表达式（{…}）里剩下的字符串字面量
    for (const lit of literals(attrOut.join(""))) {
      if (CJK.test(lit.content)) push(lineNo(tpl, lit.start), "字符串字面量", lit.content);
    }

    // (4) 模板文本节点：标签内全部挖空，再按花括号配对挖掉 {…} 表达式
    const text = clone(tpl);
    let inTag = false;
    for (let i = 0; i < text.length; i++) {
      if (text[i] === "{") {
        const end = matchBrace(tpl, i);
        blankRange(text, i, end);
        i = end - 1;
        continue;
      }
      if (text[i] === "<") inTag = true;
      else if (text[i] === ">") inTag = false;
      else if (inTag && text[i] !== "\n") text[i] = " ";
    }
    text.join("").split("\n").forEach((ln, i) => {
      if (CJK.test(ln)) push(i + 1, "模板文本节点", ln);
    });
  } else {
    const clean = stripJsComments(raw);
    for (const lit of literals(clean)) {
      if (CJK.test(lit.content)) push(lineNo(clean, lit.start), "字符串字面量", lit.content);
    }
  }

  if (hits.length) {
    hits.sort((x, y) => x.line - y.line);
    perFile.set(rel(f), hits);
  }
}

/* ============================== 统计与豁免 ============================== */

function allowedAt(src, line) {
  const lines = src.split(/\r?\n/);
  for (let i = line - 1; i >= Math.max(0, line - 2); i--) {
    const m = ALLOW_RE.exec(lines[i] ?? "");
    if (m) return m[1].trim();
  }
  return null;
}

const violations = [];
const exemptLines = [];
const whitelisted = [];
for (const [r, hits] of perFile) {
  const raw = fs.readFileSync(path.join(SRC, r.split("/").join(path.sep)), "utf8");
  const left = [];
  for (const h of hits) {
    const why = allowedAt(raw, h.line);
    if (why) exemptLines.push({ file: r, ...h, why });
    else left.push(h);
  }
  if (!left.length) continue;
  const note = FILE_WHITELIST[r];
  if (note) whitelisted.push({ file: r, count: left.length, note });
  else violations.push({ file: r, hits: left });
}
const stale = Object.keys(FILE_WHITELIST).filter((r) => !whitelisted.some((w) => w.file === r));

// 文案表统计
const readKeys = (name) => [...fs.readFileSync(path.join(SRC, "lib", "i18n", name), "utf8").matchAll(/^\s{2}"([^"]+)":/gm)].map((x) => x[1]);
const zhKeys = readKeys("zh-CN.ts");
const enKeys = readKeys("en.ts");
const keySet = new Set(zhKeys);

// 键引用：t("…") 直呼 + 键表里以字面量形式出现（labelKey: "…" / STATE_LABEL_KEYS / ERR_CODES）
const tCallRe = /(?<![\w.])t\(\s*"([^"]+)"/g;
const usedAll = new Set();
const usedMigrated = new Set();
let callSites = 0;
for (const f of [...files, ...walk(path.join(SRC, "lib", "i18n"), [])]) {
  const raw = fs.readFileSync(f, "utf8");
  const wl = !!FILE_WHITELIST[rel(f)];
  // 键引用既可能在模板表达式里，也可能在 <script> 的键表里（labelKey: "…"），
  // ERR_CODES 这类表则住在 src/lib/i18n 自身，故这里连 i18n 目录一起扫。
  for (const lit of literals(stripJsComments(raw))) if (keySet.has(lit.content)) usedAll.add(lit.content);
  if (wl) continue;
  let m;
  while ((m = tCallRe.exec(raw))) {
    usedAll.add(m[1]);
    usedMigrated.add(m[1]);
    callSites++;
  }
}
const unusedKeys = zhKeys.filter((k) => !usedAll.has(k));
// 键集合必须完全一致：只比数量的话「en 少一个 A、多一个 B」会被漏掉。
const zhSet = new Set(zhKeys), enSet = new Set(enKeys);
const onlyZh = zhKeys.filter((k) => !enSet.has(k));
const onlyEn = enKeys.filter((k) => !zhSet.has(k));
const dupZh = zhKeys.filter((k, i) => zhKeys.indexOf(k) !== i);
const dupEn = enKeys.filter((k, i) => enKeys.indexOf(k) !== i);

/* ====================== 检查 2：_test/*.mjs 里的中文选择器 ====================== */

const TEST_DIR = path.resolve(HERE, "..", "_test");

/**
 * 「用于选择器/断言的中文字面量」= _test 脚本里（去注释后）出现在字符串字面量中的中文。
 * 逐文件白名单必须写明原因；命中即报错退出。
 * 允许的原因只有两类：
 *   a) 夹具/书籍内容本身（书名、正文、章节名、真实书路径、字体族名、提示词种子）——它们就是要断言的对象；
 *   b) 目标元素位于 SettingsModal（第三阶段迁移）或工具脚本自身的中文报错/日志。
 */
const TEST_WHITELIST = {
  "b1fix_f_live.mjs": "夹具书籍名与正文（书名/作者/正文就是要断言的对象）",
  "batch1_live.mjs": "断言的是书库页长按刷新菜单项文案 + 存档点提示，目标按钮已改 data-testid，剩余中文属脚本自有断言",
  "c_live.mjs": "夹具书籍名与正文（删除已打开书籍的回归用例）",
  "cdp.mjs": "工具脚本自身的中文报错信息",
  "cover_shots.mjs": "夹具书籍名与正文（封面截图用例）",
  "defaultcover_fix_live.mjs": "夹具书籍名与正文（默认封面修复用例）",
  "defaultcover_live.mjs": "设置面板内的导航文案（SettingsModal，第三阶段迁移）",
  "e_portal.mjs": "全文搜索的输入关键词（用户输入值，非界面文案）",
  "extremes.mjs": "夹具书籍名与正文（极值用例）",
  "f2_live.mjs": "夹具书籍名与正文（批量用例）",
  "final3_evidence.mjs": "夹具书籍名/正文/章节名（端到端取证用例）",
  "final_evidence.mjs": "夹具书籍名/正文/章节名（端到端取证用例）",
  "final_evidence2.mjs": "夹具书籍名/正文/章节名 + 设置面板分区名（SettingsModal，第三阶段迁移）",
  "fixfont_app.mjs": "真实书路径与书名（字体修复用例）+ 设置面板内的导航文案",
  "fixscheme_app.mjs": "真实书路径与书名（阅读方案用例）+ 设置面板内的按钮文案",
  "gh_live.mjs": "夹具书籍名/正文/章节名（划线回归用例）",
  "i18n_guard.mjs": "守门脚本自身：白名单理由、统计输出的中文标签",
  "i18n_live.mjs": "i18n 实机验证脚本自身：断言标签与中文对照期望值",
  "isolation_guard.mjs": "隔离守卫自身的中文报错信息",
  "lib_delete_blank_live.mjs": "回归脚本自身的 PASS/FAIL 中文标签",
  "manga_flow_regress.mjs": "漫画瀑布流回归脚本自身的中文报错",
  "manga_flow_scroll.mjs": "夹具书名 + 漫画瀑布流脚本自身的中文报错",
  "manga_live.mjs": "设置面板分区名（SettingsModal，第三阶段迁移）+ 夹具书名",
  "mock_ai_server.mjs": "mock 服务的固定文本载荷 + 端口守卫报错",
  "removal_live.mjs": "种子字体族名/提示词种子（刻意与真实用户数据一致）+ 脚本自有日志",
  "scheme_probe.mjs": "脚本自身的探针输出",
  "send.mjs": "发送守卫自身的中文报错",
  "settings_ui.mjs": "设置面板内的导航/搜索文案（SettingsModal，第三阶段迁移）",
  "skin_live.mjs": "UI 皮肤显示名（数据在 UI_SKINS，消费方 SettingsModal，第三阶段迁移）",
  "suite_live.mjs": "种子字体族名/提示词种子 + 设置面板内的导航与下拉文案（SettingsModal，第三阶段迁移）",
  "theme_live.mjs": "阅读配色方案显示名「纸质」等（已持久化数据，规则禁止翻译）+ 设置面板导航",
  "trace_live.mjs": "脚本自身的探针输出",
  "ui_repro.mjs": "书库路径 + 脚本自身的探针文案",
};

const testViolations = [];
const testWhitelisted = [];
if (fs.existsSync(TEST_DIR)) {
  for (const f of fs.readdirSync(TEST_DIR).filter((x) => x.endsWith(".mjs")).sort()) {
    const clean = stripJsComments(fs.readFileSync(path.join(TEST_DIR, f), "utf8"));
    const hits = literals(clean).filter((l) => CJK.test(l.content));
    if (!hits.length) continue;
    const note = TEST_WHITELIST[f];
    if (note) testWhitelisted.push({ file: f, count: hits.length, note });
    else testViolations.push({ file: f, hits: hits.map((l) => ({ line: clean.slice(0, l.start).split("\n").length, text: l.content.replace(/\s+/g, " ").slice(0, 90) })) });
  }
}
const testStale = Object.keys(TEST_WHITELIST).filter((f) => !testWhitelisted.some((w) => w.file === f));

// 行内豁免按「文件 + 原因」聚合：让「哪些是持久化值/用户内容」一眼可见
const exemptGroups = new Map();
for (const e of exemptLines) {
  const k = e.file + "\u0000" + e.why;
  exemptGroups.set(k, (exemptGroups.get(k) || 0) + 1);
}
const groups = [...exemptGroups].map(([k, n]) => {
  const [file, why] = k.split("\u0000");
  return { file, why, n };
});

/* ============================== 输出 ============================== */

const pad = (s, n) => String(s) + " ".repeat(Math.max(0, n - String(s).length));
const L = console.log;
L("=".repeat(78));
L("i18n 守门检查 —— 扫描 src/**/*.svelte 与 src/**/*.ts");
L("=".repeat(78));
L(`文案表：zh-CN.ts ${zhKeys.length} 键 / en.ts ${enKeys.length} 键` + (zhKeys.length === enKeys.length ? "（数量一致）" : "  ⚠ 数量不一致！"));
L(`文案表里没有任何地方引用的键：${unusedKeys.length}` + (unusedKeys.length ? " → " + unusedKeys.join(", ") : ""));
L(`en.ts 里 zh-CN 没有的键：${onlyEn.length}` + (onlyEn.length ? " → " + onlyEn.join(", ") : ""));
L(`zh-CN.ts 里 en.ts 没有的键：${onlyZh.length}` + (onlyZh.length ? " → " + onlyZh.join(", ") : ""));
L(`重复键：zh-CN ${dupZh.length} / en ${dupEn.length}` + (dupZh.length + dupEn.length ? " → " + [...dupZh, ...dupEn].join(", ") : ""));
L(`本轮迁移（非白名单文件）：${usedMigrated.size} 个键 / ${callSites} 处 t() 调用点`);
L("");
L("—— 检查 2：_test/*.mjs 中的中文选择器/断言 ——");
L("");
const leftTotal = violations.reduce((n, v) => n + v.hits.length, 0);
L(`剩余中文（未豁免，必须为 0）：${leftTotal}`);
for (const v of violations) {
  L(`  ✗ ${v.file}  (${v.hits.length})`);
  for (const h of v.hits) L(`      L${pad(h.line, 5)} ${pad(h.kind, 14)} ${h.text}`);
}
L("");
const wlTotal = whitelisted.reduce((n, w) => n + w.count, 0);
L(`白名单文件（本阶段尚未迁移，逐文件列出）：${whitelisted.length} 个，合计残留 ${wlTotal} 条`);
for (const w of whitelisted.sort((a, b) => b.count - a.count)) L(`  · ${pad(w.file, 40)} ${pad(w.count + " 条", 7)} ${w.note}`);
if (exemptLines.length) {
  L("");
  L(`行内 // i18n-allow 豁免（规则禁止翻译的持久化值 / 用户内容）：${exemptLines.length} 行，${groups.length} 类`);
  for (const g of groups) L(`  · ${pad(g.file, 30)} ${pad(g.n + " 行", 7)} ${g.why}`);
  if (VERBOSE) { L(""); for (const e of exemptLines) L(`      ${e.file}:${e.line} ${e.kind} ${e.text}`); }
}
if (stale.length) {
  L("");
  L(`白名单里已查无残留（可清理登记）：${stale.join(", ")}`);
}
L("");
const testLeft = testViolations.reduce((n, v) => n + v.hits.length, 0);
L(`_test 剩余中文选择器/断言（未豁免，必须为 0）：${testLeft}`);
for (const v of testViolations) {
  L(`  ✗ ${v.file}  (${v.hits.length})`);
  for (const h of v.hits) L(`      L${pad(h.line, 5)} ${h.text}`);
}
L(``);
L(`_test 白名单（逐文件列出并写明原因）：${testWhitelisted.length} 个，合计 ${testWhitelisted.reduce((n, w) => n + w.count, 0)} 条`);
for (const w of testWhitelisted) L(`  · ${pad(w.file, 28)} ${pad(w.count + " 条", 7)} ${w.note}`);
if (testStale.length) {
  L("");
  L(`⚠ _test 白名单里已查无中文（可清理登记）：${testStale.join(", ")}`);
}
L("=".repeat(78));
const tablesOutOfSync = onlyZh.length + onlyEn.length + dupZh.length + dupEn.length;
if (violations.length || testViolations.length || tablesOutOfSync) {
  L(`RESULT: FAIL —— src 侧 ${violations.length} 个文件 + _test 侧 ${testViolations.length} 个文件仍有未迁移的中文` + (tablesOutOfSync ? `；文案表键集合不一致（${tablesOutOfSync} 处）` : ""));
  process.exit(1);
}
L("RESULT: PASS");
