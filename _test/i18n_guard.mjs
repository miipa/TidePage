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
const FILE_WHITELIST = {};



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
// ---- 文案表全量校验（任务要求 5）：键集合 / 重复键 / 占位符逐键一致 / en 无中文 / 无空值 / 无“未翻译” ----
const readTable = (name) => {
  const out = new Map();
  const src = fs.readFileSync(path.join(SRC, "lib", "i18n", name), "utf8");
  for (const m of src.matchAll(/^\s{2}"((?:[^"\\]|\\.)*)":\s*"((?:[^"\\]|\\.)*)"/gm))
    out.set(JSON.parse(`"${m[1]}"`), JSON.parse(`"${m[2]}"`));
  return out;
};
const zhTable = readTable("zh-CN.ts");
const enTable = readTable("en.ts");
const phOf = (v) => [...new Set([...v.matchAll(/\{(\w+)\}/g)].map((x) => x[1]))].sort().join(",");
/** 允许 zh/en 完全相同的键：专有名词与格式名（WebDAV / EPUB / TXT / MiniMax…），不是漏译。 */
const SAME_ALLOWED = new Set([
  "settings.language.zhCN", // 语言名按惯例显示各自语言名（endonym），中英下取值相同
  "settings.tab.webdav", "settings.fileDlg.txt", "dict.splitPreset.chapterEn",
  "dict.aiProvider.gemini", "dict.aiProvider.azure-openai",
  "dict.ttsProtocol.azure", "dict.ttsProtocol.minimax", "dict.ttsProtocol.fish",
  "dict.ttsModel.dashscope", "dict.ttsModel.minimax", "dict.ttsModel.fish",
  "dict.ttsModel.volcano", "settings.language.en",
  // 以下是纯字形/符号键，两种语言下本就一致（不是漏译）
  "palette.addGlyph", "reader.font.decGlyph", "reader.font.incGlyph",
]);
/** en 里允许残留的“非文案中文”：全角 ＋/‹›/「」/图 等排版符号，非中文词。 */
const EN_GLYPH_ALLOWED = new Set([
  "palette.addGlyph", "reader.font.incGlyph", "reader.img.incGlyph", "reader.bookmarks.empty",
  "selection.paletteTip", "library.toolbar.addFolder", "settings.scheme.newBtn",
  "settings.manga.scopeTip", "settings.manga.imgScaleTip",
  "settings.tts.add", "settings.ai.add", "settings.ai.addPrompt",
]);
const phMismatch = [], cjkInEn = [], emptyVal = [], untranslated = [];
for (const k of zhKeys) {
  const z = zhTable.get(k), e = enTable.get(k);
  if (phOf(z) !== phOf(e)) phMismatch.push(k + " zh{" + phOf(z) + "} en{" + phOf(e) + "}");
  if (CJK.test(e) && !EN_GLYPH_ALLOWED.has(k) && k !== "settings.language.zhCN") cjkInEn.push(k);
  if (!e.trim() || !z.trim()) emptyVal.push(k);
  if (z === e && !SAME_ALLOWED.has(k)) untranslated.push(k + " = " + JSON.stringify(z));
}

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


/**
 * 行级豁免表：键 = "<文件名>:<行号>"，值 = 豁免原因（必填）。
 * 只允许两类：a) 夹具书籍/正文的真实内容（它就是要断言的对象）；b) 脚本自身的中文报错/日志。
 * 任何「查无残留」的条目都会报错，防止陈旧豁免长期掩盖新问题。
 */
const TEST_ALLOW_LINES = {
  "b1fix_f_live.mjs:55": "夹具书籍名与正文（书名/作者/正文就是要断言的对象）",
  "b1fix_f_live.mjs:57": "夹具书籍名与正文（书名/作者/正文就是要断言的对象）",
  "b1fix_f_live.mjs:58": "夹具书籍名与正文（书名/作者/正文就是要断言的对象）",
  "b1fix_f_live.mjs:59": "夹具书籍名与正文（书名/作者/正文就是要断言的对象）",
  "b1fix_f_live.mjs:61": "夹具书籍名与正文（书名/作者/正文就是要断言的对象）",
  "batch1_live.mjs:117": "脚本自有断言/日志文案（书库页行为已改用 data-testid 选择）",
  "batch1_live.mjs:52": "脚本自有断言/日志文案（书库页行为已改用 data-testid 选择）",
  "batch1_live.mjs:87": "脚本自有断言/日志文案（书库页行为已改用 data-testid 选择）",
  "c_live.mjs:11": "夹具书籍名与正文（删除已打开书籍的回归用例）",
  "c_live.mjs:13": "夹具书籍名与正文（删除已打开书籍的回归用例）",
  "c_live.mjs:30": "夹具书籍名与正文（删除已打开书籍的回归用例）",
  "cdp.mjs:4": "工具脚本自身的中文报错信息",
  "cover_shots.mjs:48": "夹具书籍名与正文（封面截图用例）",
  "cover_shots.mjs:49": "夹具书籍名与正文（封面截图用例）",
  "cover_shots.mjs:51": "夹具书籍名与正文（封面截图用例）",
  "cover_shots.mjs:52": "夹具书籍名与正文（封面截图用例）",
  "cover_shots.mjs:55": "夹具书籍名与正文（封面截图用例）",
  "defaultcover_fix_live.mjs:81": "夹具书籍名与正文（默认封面修复用例）",
  "defaultcover_fix_live.mjs:82": "夹具书籍名与正文（默认封面修复用例）",
  "defaultcover_fix_live.mjs:82": "夹具书籍名与正文（默认封面修复用例）",
  "defaultcover_fix_live.mjs:83": "夹具书籍名与正文（默认封面修复用例）",
  "defaultcover_fix_live.mjs:83": "夹具书籍名与正文（默认封面修复用例）",
  "defaultcover_live.mjs:41": "脚本自有断言/日志文案",
  "defaultcover_live.mjs:55": "脚本自有断言/日志文案",
  "e_portal.mjs:59": "全文搜索的输入关键词（用户输入值，非界面文案）",
  "extremes.mjs:35": "夹具书籍名与正文（极值用例）",
  "extremes.mjs:35": "夹具书籍名与正文（极值用例）",
  "extremes.mjs:36": "夹具书籍名与正文（极值用例）",
  "f2_live.mjs:35": "夹具书籍名与正文（批量用例）",
  "f2_live.mjs:35": "夹具书籍名与正文（批量用例）",
  "final_evidence.mjs:103": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:124": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:125": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:27": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:28": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:35": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:54": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:68": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence.mjs:84": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:120": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:26": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:27": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:28": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:29": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:30": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:31": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:38": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:45": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:66": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:82": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final_evidence2.mjs:96": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:38": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:39": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:40": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:41": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:48": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:54": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:67": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "final3_evidence.mjs:79": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "fixfont_app.mjs:54": "真实书路径与书名（字体修复用例）+ 脚本自有断言",
  "fixfont_app.mjs:61": "真实书路径与书名（字体修复用例）+ 脚本自有断言",
  "fixfont_app.mjs:84": "真实书路径与书名（字体修复用例）+ 脚本自有断言",
  "fixscheme_app.mjs:44": "真实书路径与书名（阅读方案用例）+ 脚本自有断言",
  "fixscheme_app.mjs:57": "真实书路径与书名（阅读方案用例）+ 脚本自有断言",
  "gh_live.mjs:28": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "gh_live.mjs:29": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "gh_live.mjs:30": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "gh_live.mjs:31": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "gh_live.mjs:39": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "gh_live.mjs:48": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "gh_live.mjs:74": "夹具书籍名/正文/章节名 + 脚本自有断言",
  "i18n_live.mjs:120": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:121": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:121": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:122": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:123": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:132": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:133": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:133": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:134": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:134": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:135": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:135": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:145": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:163": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:166": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:167": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:195": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:196": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:197": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:198": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:199": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:200": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:201": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:204": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:205": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:206": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:212": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:244": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:245": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:246": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:250": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:269": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:273": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:290": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:290": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:290": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:290": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:290": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:291": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:292": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:292": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:298": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:303": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:331": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:344": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:344": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:344": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:353": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:354": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:355": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:355": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:36": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:374": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:437": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:442": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:447": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:458": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:459": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:460": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:460": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:460": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:460": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:460": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:461": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:471": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:472": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:473": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "i18n_live.mjs:481": "i18n 实机验证脚本自身：断言标签与中英对照期望值",
  "isolation_guard.mjs:28": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:33": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:37": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:46": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:47": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:54": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:62": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:66": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:77": "隔离守卫自身的中文报错信息",
  "isolation_guard.mjs:79": "隔离守卫自身的中文报错信息",
  "lib_delete_blank_live.mjs:124": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:125": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:126": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:127": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:128": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:157": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:158": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:159": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:168": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:169": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:190": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:191": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:192": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:193": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:194": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:211": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:212": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:213": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:214": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:215": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:239": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:240": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:241": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:242": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:243": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:244": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:245": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:251": "回归脚本自身的 PASS/FAIL 中文标签",
  "lib_delete_blank_live.mjs:42": "回归脚本自身的 PASS/FAIL 中文标签",
  "manga_flow_regress.mjs:106": "漫画瀑布流回归脚本自身的中文报错",
  "manga_flow_regress.mjs:21": "漫画瀑布流回归脚本自身的中文报错",
  "manga_flow_scroll.mjs:143": "夹具书名 + 脚本自身的中文报错",
  "manga_flow_scroll.mjs:175": "夹具书名 + 脚本自身的中文报错",
  "manga_flow_scroll.mjs:187": "夹具书名 + 脚本自身的中文报错",
  "manga_flow_scroll.mjs:26": "夹具书名 + 脚本自身的中文报错",
  "manga_live.mjs:134": "夹具书名/章节名 + 脚本自身的中文断言",
  "manga_live.mjs:178": "夹具书名/章节名 + 脚本自身的中文断言",
  "manga_live.mjs:254": "夹具书名/章节名 + 脚本自身的中文断言",
  "mock_ai_server.mjs:11": "mock 服务的固定文本载荷 + 端口守卫报错",
  "mock_ai_server.mjs:47": "mock 服务的固定文本载荷 + 端口守卫报错",
  "removal_live.mjs:31": "种子字体族名/提示词种子（刻意与真实用户数据一致）+ 脚本自有日志",
  "removal_live.mjs:64": "种子字体族名/提示词种子（刻意与真实用户数据一致）+ 脚本自有日志",
  "removal_live.mjs:66": "种子字体族名/提示词种子（刻意与真实用户数据一致）+ 脚本自有日志",
  "scheme_probe.mjs:20": "脚本自身的探针输出",
  "send.mjs:11": "发送守卫自身的中文报错",
  "send.mjs:49": "发送守卫自身的中文报错",
  "send.mjs:69": "发送守卫自身的中文报错",
  "send.mjs:73": "发送守卫自身的中文报错",
  "send.mjs:74": "发送守卫自身的中文报错",
  "send.mjs:75": "发送守卫自身的中文报错",
  "settings_ui.mjs:36": "脚本自有断言：读 DOM 文本做对照（选择器已全部换成 data-testid）",
  "settings_ui.mjs:46": "脚本自有断言：读 DOM 文本做对照（选择器已全部换成 data-testid）",
  "suite_live.mjs:100": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:103": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:103": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:122": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:129": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:132": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:142": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:147": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:160": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:21": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:53": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "suite_live.mjs:63": "种子字体族名/提示词种子 + 脚本自有断言与日志",
  "theme_live.mjs:117": "持久化的阅读配色方案名（如 纸质）+ 脚本自有断言",
  "theme_live.mjs:147": "持久化的阅读配色方案名（如 纸质）+ 脚本自有断言",
  "theme_live.mjs:161": "持久化的阅读配色方案名（如 纸质）+ 脚本自有断言",
  "theme_live.mjs:174": "持久化的阅读配色方案名（如 纸质）+ 脚本自有断言",
  "theme_live.mjs:72": "持久化的阅读配色方案名（如 纸质）+ 脚本自有断言",
  "theme_live.mjs:92": "持久化的阅读配色方案名（如 纸质）+ 脚本自有断言",
  "trace_live.mjs:38": "脚本自身的探针输出",
  "trace_live.mjs:53": "脚本自身的探针输出",
  "ui_repro.mjs:194": "书库路径 + 脚本自身的探针文案",
  "ui_repro.mjs:289": "书库路径 + 脚本自身的探针文案",
  "ui_repro.mjs:311": "书库路径 + 脚本自身的探针文案",
  "ui_repro.mjs:353": "书库路径 + 脚本自身的探针文案",
  "ui_repro.mjs:371": "书库路径 + 脚本自身的探针文案",
};

const testViolations = [];
const testWhitelisted = [];
if (fs.existsSync(TEST_DIR)) {
  for (const f of fs
    .readdirSync(TEST_DIR)
    .filter((x) => x.endsWith(".mjs"))
    .filter((x) => x !== "i18n_guard.mjs") // 本脚本自身：中文只有输出标签与行级豁免原因，逐行豁免会自我指涉
    .sort()) {
    const clean = stripJsComments(fs.readFileSync(path.join(TEST_DIR, f), "utf8"));
    const hits = literals(clean).filter((l) => CJK.test(l.content));
    if (!hits.length) continue;
    const rows = hits.map((l) => ({
      line: clean.slice(0, l.start).split("\n").length,
      text: l.content.replace(/\s+/g, " ").slice(0, 90),
    }));
    const left = [];
    for (const r of rows) {
      const why = TEST_ALLOW_LINES[f + ":" + r.line];
      if (why) testWhitelisted.push({ file: f, line: r.line, why, text: r.text });
      else left.push(r);
    }
    if (left.length) testViolations.push({ file: f, hits: left });
  }
}
const usedAllowKeys = new Set(testWhitelisted.map((w) => w.file + ":" + w.line));
const testStale = Object.keys(TEST_ALLOW_LINES).filter((k) => !usedAllowKeys.has(k));

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

/* ============ 检查 3：zh 表的值必须是中文（否则说明 zh 侧被写成了英文/漏翻） ============ */

/**
 * 允许 zh 值为非中文的键，逐条写明原因。允许的类别仅限：
 *   a) 专有名词 / 产品名 / 服务商名（Google Gemini、Azure OpenAI、MiniMax、Fish Audio、WebDAV）
 *   b) 格式名（TXT）
 *   c) 界面字形（A− 等以字母/符号为主、不含中文的键）
 *   d) 语言 endonym（settings.language.en）
 *   e) 纯占位符模板（本项目暂无命中，占位以 {x} 形式本身不含 CJK 时才需要）
 * 值发生修改后若不再需要豁免，此处即为「陈旧豁免」，检查 3 会 FAIL。
 */
const ZH_NON_CJK_ALLOWED = {
  "reader.font.decGlyph": "界面字形：A−（字号减小符号，非中文文案）",
  "settings.language.en": "语言 endonym：按惯例显示 English 自身写法",
  "settings.fileDlg.txt": "格式名：TXT",
  "dict.aiProvider.gemini": "专有名词：Google Gemini",
  "dict.aiProvider.azure-openai": "专有名词：Azure OpenAI",
  "dict.ttsProtocol.azure": "专有名词：Azure OpenAI",
  "dict.ttsProtocol.minimax": "专有名词：MiniMax",
  "dict.ttsProtocol.fish": "专有名词：Fish Audio",
  "dict.ttsModel.dashscope": "专有名词/取值：音色名 Cherry（服务商给定的固定 id）",
  "dict.ttsModel.minimax": "专有名词/取值：音色 id male-qn-qingse（服务商给定的固定 id）",
  "dict.ttsModel.fish": "专有名词/取值：reference_id（服务商给定的字段名）",
  "dict.splitPreset.chapterEn": "专有名词/格式约定：Chapter N 是英文分章预设的原名",
  "settings.tab.webdav": "专有名词：WebDAV",
};
const zhNonCjk = [];
for (const k of zhKeys) if (!CJK.test(zhTable.get(k)) && !ZH_NON_CJK_ALLOWED[k]) zhNonCjk.push(k + " = " + JSON.stringify(zhTable.get(k)));
const zhAllowStale = Object.keys(ZH_NON_CJK_ALLOWED).filter((k) => CJK.test(zhTable.get(k) ?? ""));

/* ============ 检查 4：同一文件内同一个 labelKey/titleKey 不得被两个不同 id 复用 ============ */
// 只看承载「id 的显示名」的字段；modelHintKey/voiceHintKey 这类提示文本允许共用。
// 只统计值确实存在于文案表里的，避免把 key: "F1" 之类非 i18n 键误判进来。
const dupLabelKeys = [];
for (const f of files) {
  const raw = fs.readFileSync(f, "utf8");
  const c = new Map();
  for (const m of raw.matchAll(/\b(labelKey|titleKey)\s*:\s*"([^"]+)"/g)) {
    if (!zhSet.has(m[2])) continue;
    c.set(m[2], (c.get(m[2]) || 0) + 1);
  }
  for (const [k, n] of c) if (n > 1) dupLabelKeys.push(rel(f) + "  " + k + " ×" + n);
}

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
L(`占位符逐键一致：不一致 ${phMismatch.length}` + (phMismatch.length ? " → " + phMismatch.join(", ") : ""));
L(`en 值里残留中文（已放行 ${EN_GLYPH_ALLOWED.size} 个含全角符号的键）：${cjkInEn.length}` + (cjkInEn.length ? " → " + cjkInEn.join(", ") : ""));
L(`空值：${emptyVal.length}` + (emptyVal.length ? " → " + emptyVal.join(", ") : ""));
L(`译文与原文完全相同（疑似漏译，已放行 ${SAME_ALLOWED.size} 个专有名词/格式名）：${untranslated.length}` + (untranslated.length ? " → " + untranslated.join(", ") : ""));
L(`—— 检查 3：zh 表的值必须是中文 ——`);
L(`zh 值不含中文（未豁免，必须为 0）：${zhNonCjk.length}` + (zhNonCjk.length ? " → " + zhNonCjk.join(", ") : ""));
L(`zh 值豁免 ${Object.keys(ZH_NON_CJK_ALLOWED).length} 条（陈旧 ${zhAllowStale.length}）` + (zhAllowStale.length ? " → " + zhAllowStale.join(", ") : ""));
L(`—— 检查 4：labelKey/titleKey 不得复用 ——`);
L(`同一文件内同一显示名键被复用（必须为 0）：${dupLabelKeys.length}` + (dupLabelKeys.length ? " → " + dupLabelKeys.join(" | ") : ""));
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
L(`_test 行级豁免（逐行写明原因）：${testWhitelisted.length} 行，分布在 ${new Set(testWhitelisted.map((w) => w.file)).size} 个文件`);
const byFile = new Map();
for (const w of testWhitelisted) {
  if (!byFile.has(w.file)) byFile.set(w.file, []);
  byFile.get(w.file).push(w);
}
for (const [f, ws] of [...byFile].sort()) {
  L(`  · ${pad(f, 28)} ${ws.length} 行`);
  for (const w of ws) L(`      L${pad(w.line, 5)} ${pad(w.text, 46)} ${w.why}`);
}
if (testStale.length) {
  L("");
  L(`⚠ _test 行级豁免里已查无残留（必须删除，否则视为陈旧豁免）：${testStale.join(", ")}`);
}
L("=".repeat(78));
const tablesOutOfSync =
  onlyZh.length + onlyEn.length + dupZh.length + dupEn.length +
  phMismatch.length + cjkInEn.length + emptyVal.length + untranslated.length +
  zhNonCjk.length + zhAllowStale.length + dupLabelKeys.length;
if (violations.length || testViolations.length || tablesOutOfSync || testStale.length || stale.length) {
  L(`RESULT: FAIL —— src 侧 ${violations.length} 个文件 + _test 侧 ${testViolations.length} 个文件仍有未迁移的中文` + (tablesOutOfSync ? `；文案表键集合不一致（${tablesOutOfSync} 处）` : "") + (testStale.length || stale.length ? `；陈旧豁免 src ${stale.length} 条 / _test ${testStale.length} 条` : "") +
    (zhNonCjk.length ? `；zh 非中文值 ${zhNonCjk.length} 处` : "") +
    (dupLabelKeys.length ? `；显示名键复用 ${dupLabelKeys.length} 处` : "") +
    (zhAllowStale.length ? `；zh 豁免陈旧 ${zhAllowStale.length} 条` : ""));
  process.exit(1);
}
L("RESULT: PASS");
