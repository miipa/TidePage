#!/usr/bin/env node
/**
 * 入库守门：源码、提交信息与提交身份里不得出现旧产品名，或由它首字母缩写拼出的旧前缀标识。
 *
 * 扫描范围
 *   1. `git ls-files` 列出的文本文件（含 NUL 字节的按二进制跳过）；
 *   2. `git log -200` 的提交信息（标题 + 正文）；
 *   3. 提交身份：`git config --get user.name` / `user.email`（未设置即跳过并打印说明），
 *      以及 `git log -200 --format=%h|%an|%ae|%cn|%ce` 的作者/提交者姓名与邮箱（去重后逐条检查）。
 *
 * 自测入口（跑反例不必改仓库、不产生提交）
 *   node scripts/check-legacy-name.mjs --identities "<name>|<email>|<cname>|<cemail>"
 *   只检查这一组身份，不扫文件与提交信息；判定规则与退出码与本模式一致。
 *   node scripts/check-legacy-name.mjs --selftest
 *   只跑内置正例/负例断言（覆盖下划线/中缀变体、点号命名空间与豁免），全部通过退出 0，否则 1。
 *
 * 退出码：0 通过；1 命中；2 用法或环境错误（非 git 仓库、git 调用失败、参数不合法）。
 *
 * 排除：`.git/**`（含 hooks）、`_test/` 下的临时产物、二进制文件。
 *
 * 注意：本文件也在扫描范围内，因此旧名与旧前缀一律用字符串拼接构造 —— 直接写字面量会自我命中。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "..");

const EXIT_PASS = 0;
const EXIT_HIT = 1;
const EXIT_USAGE = 2;

/** 旧名首字母缩写（两字母） */
const ABBR = "f" + "r";
const ABBR_UPPER = ABBR.toUpperCase();

/** 旧产品名：由两段拼接，本文件不得出现完整旧名 */
const OLD_NAME = ["Flow", "Read"].join("");

/**
 * 标识符边界：旧前缀前允许行首或任意非字母数字字符（下划线、点号、引号、括号均可）。
 * 不能用单词边界 —— 下划线在正则里属于单词字符，双下划线、中缀、大写下划线前缀都会漏检。
 */
const IDENT_START = "(^|[^A-Za-z0-9])";

/**
 * 语言/地区代码豁免：前缀分隔符后是两位大写地区码，且后面不再接
 * 字母/数字/下划线。地区码是语言标签的一部分，不是旧前缀。
 * 注意：该正则大小写敏感 —— 只有小写语言码的标签才豁免；大写形式仍按旧前缀处理。
 */
const LOCALE_TAIL_RE = /^[A-Z]{2}(?![A-Za-z0-9_])/;

/** 常见顶级域名（后缀）：最后一节命中这里时按域名处理，不算点号命名空间。 */
const COMMON_TLDS = new Set([
  "com", "org", "net", "io", "dev", "cn", "fr", "ai", "co", "uk", "de", "jp", "edu", "gov",
  "info", "me", "app", "xyz", "top", "site", "online", "tech", "cloud", "pro", "biz", "tv",
  "cc", "us", "eu", "au", "ca", "ru", "br", "in", "it", "nl", "se", "no", "fi", "pl", "es",
  "ch", "at", "be", "dk", "kr", "tw", "hk", "sg",
]);

/** 常见文件扩展名：唯一一节命中这里时按文件名处理，不算点号命名空间。 */
const FILE_EXTS = new Set([
  "json", "jsonl", "js", "mjs", "cjs", "ts", "svelte", "md", "html", "css", "txt", "yml",
  "yaml", "toml", "xml", "csv", "log", "lock", "png", "jpg", "jpeg", "webp", "gif", "svg",
  "rs", "py", "sh", "ps1", "bat", "exe", "zip",
]);

/** 点号命名空间规则：前缀 + 点 + token（token 可为空：句末的单前缀句点不报）。 */
const DOT_NS_RE = new RegExp(IDENT_START + ABBR + "\\.([A-Za-z0-9._-]*)", "i");

/** 命中判定：token 形似域名（最后一节是常见 TLD）或文件名（唯一一节是文件扩展名）时豁免。 */
function dotNamespaceHits(match) {
  // IDENT_START 是可选捕获组：行首时 match[1] 为空串，token 在 match[2]。
  const token = match[2] || "";
  if (!token) return false;
  const lower = token.toLowerCase();
  if (token.includes(".")) {
    const last = lower.slice(lower.lastIndexOf(".") + 1);
    return !COMMON_TLDS.has(last);
  }
  return !FILE_EXTS.has(lower);
}

/** 命中判定：前缀后紧跟语言地区码时豁免。 */
function prefixHits(match, line) {
  const rest = line.slice(match.index + match[0].length);
  return !LOCALE_TAIL_RE.test(rest);
}

const CHECKS = [
  {
    label: `旧产品名「${OLD_NAME}」`,
    re: new RegExp(["Flow", "[ _-]?", "Read"].join(""), "i"),
  },
  {
    label: `旧前缀「${ABBR}-」/「${ABBR}_」`,
    re: new RegExp(IDENT_START + ABBR + "[-_]", "i"),
    accept: prefixHits,
  },
  {
    label: `旧点号命名空间「${ABBR}.」`,
    re: DOT_NS_RE,
    accept: dotNamespaceHits,
  },
  {
    label: `旧 DEV 钩子「__${ABBR}Debug」`,
    // 覆盖双下划线、紧跟字母或下划线、大小写混写等变体。
    re: new RegExp("__" + ABBR + "(?=[A-Za-z_])", "i"),
  },
];

/** `_test/` 下由探针生成的临时产物（沙箱 profile、导出结果、截图、日志） */
const TEST_ARTIFACT_RE = /^_test\/(?:profiles\/|.*\.(?:json|jsonl|log|txt|csv|db|png|jpe?g|webp|gif)$)/i;

/** 身份条目的四个字段：角色用于去重，来源用于报告定位。 */
const IDENTITY_FIELDS = [
  { role: "author.name", argLabel: "identities:author.name", commitLabel: "author.name" },
  { role: "author.email", argLabel: "identities:author.email", commitLabel: "author.email" },
  { role: "committer.name", argLabel: "identities:committer.name", commitLabel: "committer.name" },
  { role: "committer.email", argLabel: "identities:committer.email", commitLabel: "committer.email" },
];

/** 当前配置身份对应的键（报告来源按 `config:<键名>` 命名）。 */
const CONFIG_KEYS = ["user.name", "user.email"];

const FIELD_SEP = "\u001f";
const RECORD_SEP = "\u001e";

const problems = [];
const notes = [];
const counts = { files: 0, binary: 0, commits: 0, identities: 0 };
const seenIdentity = new Set();

/* ---------------------------------- 工具 ---------------------------------- */

function usage() {
  return [
    '用法：node scripts/check-legacy-name.mjs [--identities "<name>|<email>|<cname>|<cemail>"] [--selftest]',
    "  无参数：扫 git ls-files 文本文件 + git log -200 提交信息 + 提交身份",
    "  --identities：只检查传入的四个身份字段，不扫文件与提交信息",
    "  --selftest：只跑内置正例/负例断言（不扫仓库）",
    "退出码：0 通过；1 命中；2 用法或环境错误",
  ].join("\n");
}

/** 用法/环境错误：打印原因与用法，退出码 2。 */
function fail(message) {
  console.error(`check-legacy-name: ${message}`);
  console.error(usage());
  process.exit(EXIT_USAGE);
}

/**
 * 跑 git，返回 {ok, out, err, detail}（不抛异常，便于把失败区分为「未设置」与「环境错误」）。
 * `err` 严格取 stderr：`git config --get <未设置的键>` 是退出码 1 且 stderr 为空，
 * 只有把 stderr 与「进程启动失败」的 message 分开，才能不把「未设置」误判成环境错误。
 */
function git(args) {
  try {
    const out = execFileSync("git", args, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return { ok: true, out, err: "", detail: "" };
  } catch (e) {
    const out = e && e.stdout != null ? String(e.stdout) : "";
    const err = e && e.stderr != null ? String(e.stderr) : "";
    const detail = e && e.message ? String(e.message) : "";
    return { ok: false, out, err, detail };
  }
}

/** 失败原因的展示文本：优先 stderr，其次进程级 message。 */
function why(r) {
  return r.err.trim() || r.detail || "未知错误";
}

function rel(p) {
  return p.split(path.sep).join("/");
}

function isExcluded(p) {
  if (p.startsWith(".git/")) return true; // 含 .git/hooks/*
  if (TEST_ARTIFACT_RE.test(p)) return true;
  return false;
}

function isBinary(buf) {
  return buf.includes(0);
}

/** 逐行匹配全部规则，返回 {line, col, label, snippet} 列表。 */
function scanText(text) {
  const out = [];
  const lines = String(text).split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    for (const c of CHECKS) {
      const m = c.re.exec(line);
      if (!m) continue;
      // accept(m, line) 为 false 表示该匹配被豁免（语言地区码 / 域名 / 文件名）。
      if (c.accept && !c.accept(m, line)) continue;
      out.push({ line: i + 1, col: m.index + 1, label: c.label, snippet: m[0] });
    }
  }
  return out;
}

/** 登记一条身份（同一角色 + 同一值只查一次）并立即检查。 */
function addIdentity(role, source, value) {
  const v = String(value ?? "");
  const key = role + FIELD_SEP + v;
  if (seenIdentity.has(key)) return;
  seenIdentity.add(key);
  counts.identities++;
  for (const hit of scanText(v)) {
    problems.push(`${source}:${hit.line}:${hit.col}: ${hit.label} → ${JSON.stringify(hit.snippet)}`);
  }
}

/* -------------------------------- 扫描各来源 -------------------------------- */

function scanFiles() {
  const listed = git(["ls-files", "-z"]);
  if (!listed.ok) fail(`git ls-files 失败：${why(listed)}`);

  const paths = listed.out
    .split("\0")
    .filter(Boolean)
    .map(rel)
    .filter((p) => !isExcluded(p));

  for (const p of paths) {
    const abs = path.join(ROOT, p);
    // 入口已存在但尚未落盘（部分 checkout/子模块残留）时跳过，不让守门脚本自身报错。
    if (!fs.existsSync(abs)) continue;
    const buf = fs.readFileSync(abs);
    if (isBinary(buf)) {
      counts.binary++;
      continue;
    }
    counts.files++;
    for (const hit of scanText(buf.toString("utf8"))) {
      problems.push(`${p}:${hit.line}:${hit.col}: ${hit.label} → ${JSON.stringify(hit.snippet)}`);
    }
  }
}

function scanConfigIdentity() {
  for (const key of CONFIG_KEYS) {
    const r = git(["config", "--get", key]);
    if (!r.ok) {
      // git config 对「未设置」是退出码 1 且 stderr 为空；stderr 有内容说明是环境问题。
      if (r.err.trim()) fail(`git config ${key} 读取失败：${why(r)}`);
      notes.push(`git config ${key} 未设置，跳过该身份项`);
      continue;
    }
    const value = r.out.replace(/\r?\n$/, "");
    if (!value.trim()) {
      notes.push(`git config ${key} 为空值，跳过该身份项`);
      continue;
    }
    addIdentity(key, "config:" + key, value);
  }
}

function scanHistory() {
  const head = git(["rev-parse", "--verify", "-q", "HEAD"]);
  if (!head.ok) {
    notes.push("仓库还没有任何提交（HEAD 不存在），跳过提交信息与历史身份");
    return;
  }

  const format = ["%h", "%an", "%ae", "%cn", "%ce"].join(FIELD_SEP) + RECORD_SEP;
  const log = git(["log", "-200", `--format=${format}`]);
  if (!log.ok) fail(`git log 失败：${why(log)}`);

  for (const record of log.out.split(RECORD_SEP)) {
    const raw = record.replace(/^\r?\n/, "");
    if (!raw.trim()) continue;
    const [hash, an, ae, cn, ce] = raw.split(FIELD_SEP);
    if (!hash) continue;
    counts.commits++;
    const values = [an, ae, cn, ce];
    for (let i = 0; i < IDENTITY_FIELDS.length; i++) {
      const f = IDENTITY_FIELDS[i];
      addIdentity(f.commitLabel, `commit:${hash}:${f.commitLabel}`, values[i]);
    }
  }
}

function scanArgIdentities(fields) {
  console.log("check-legacy-name: --identities 模式（只检查传入身份，不扫文件与提交信息）");
  for (let i = 0; i < IDENTITY_FIELDS.length; i++) {
    addIdentity(IDENTITY_FIELDS[i].role, IDENTITY_FIELDS[i].argLabel, fields[i]);
  }
}

/* ------------------------------- --selftest ------------------------------- */

/**
 * 自测：正例必须命中、负例必须不命中。样例同样用拼接构造，避免脚本自我命中。
 * 覆盖规则要求的下划线/中缀变体、点号命名空间、语言地区码与域名/文件名豁免。
 */
function runSelftest() {
  const lowercase = ABBR;
  const uppercase = ABBR_UPPER;
  const positives = [
    ["双下划线 + 连字符", "__" + lowercase + "-x"],
    ["BEM 中缀", "block__" + lowercase + "-item"],
    ["大写下划线前缀", "MY_" + uppercase + "_THEME"],
    ["点号命名空间 ai.winw", lowercase + ".ai.winw"],
    ["点号命名空间 ai.open", lowercase + ".ai.open"],
    ["DEV 钩子", "__" + lowercase + "Debug"],
    ["行首下划线前缀", lowercase + "_theme"],
  ];
  const negatives = [
    ["英文单词 front", "front"],
    ["英文单词 frame", "frame"],
    ["英文单词 from", "from"],
    ["英文单词 affront", "affront"],
    ["语言地区代码连字符", 'lang="' + lowercase + '-FR"'],
    ["语言地区代码下划线", "locale " + lowercase + "_CA"],
    ["域名", lowercase + ".wikipedia.org"],
    ["URL 中的语言域名", "https://example." + lowercase + "/x"],
    ["文件名", lowercase + ".json"],
  ];
  let failed = 0;
  for (const [name, sample] of positives) {
    const hits = scanText(sample);
    if (!hits.length) {
      failed++;
      console.error(`check-legacy-name: --selftest 正例未命中：${name} → ${JSON.stringify(sample)}`);
    }
  }
  for (const [name, sample] of negatives) {
    const hits = scanText(sample);
    if (hits.length) {
      failed++;
      console.error(
        `check-legacy-name: --selftest 负例误报：${name} → ${JSON.stringify(sample)} 命中 ${JSON.stringify(hits.map((h) => h.snippet))}`,
      );
    }
  }
  if (failed) {
    console.error(`check-legacy-name: --selftest 失败（${failed} 条）`);
    process.exit(EXIT_HIT);
  }
  console.log(`check-legacy-name: --selftest 通过（${positives.length} 条正例、${negatives.length} 条负例）`);
  process.exit(EXIT_PASS);
}

/* ---------------------------------- 入口 ---------------------------------- */

function parseArgs(argv) {
  let identities = null;
  let selftest = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--identities") {
      const raw = argv[++i];
      if (raw === undefined) fail('--identities 需要一个 "<name>|<email>|<cname>|<cemail>" 参数');
      const parts = raw.split("|");
      if (parts.length !== 4) {
        fail(`--identities 参数必须是 4 个以 | 分隔的字段，实际 ${parts.length} 个：${JSON.stringify(raw)}`);
      }
      identities = parts;
    } else if (a === "--selftest") {
      selftest = true;
    } else if (a === "-h" || a === "--help") {
      console.log(usage());
      process.exit(EXIT_PASS);
    } else {
      fail(`未知参数 ${JSON.stringify(a)}`);
    }
  }
  return { identities, selftest };
}

function finish() {
  for (const n of notes) console.log("check-legacy-name: " + n);
  if (problems.length) {
    console.error(`check-legacy-name: 命中 ${problems.length} 处旧名或旧前缀标识：`);
    for (const p of problems) console.error("  " + p);
    console.error(
      "\n旧前缀一律改为 tp- / tp_ / TP_ / __tpDebug；产品名只写 TidePage；提交身份用 miipa <miipa@users.noreply.github.com>。",
    );
    console.error("提交信息里出现旧名还会被 .git/hooks/commit-msg 直接拒绝。");
    process.exit(EXIT_HIT);
  }
  console.log(
    `check-legacy-name: 通过（${counts.files} 个文本文件，跳过 ${counts.binary} 个二进制文件，` +
      `${counts.commits} 条提交信息，身份 ${counts.identities} 条，0 命中）`,
  );
  process.exit(EXIT_PASS);
}

const args = parseArgs(process.argv.slice(2));

if (args.selftest) {
  runSelftest();
} else if (args.identities) {
  scanArgIdentities(args.identities);
} else {
  const inside = git(["rev-parse", "--is-inside-work-tree"]);
  if (!inside.ok || inside.out.trim() !== "true") {
    fail("当前目录不是 git 仓库（git rev-parse --is-inside-work-tree 未返回 true）");
  }
  scanFiles();
  scanConfigIdentity();
  scanHistory();
}

finish();
