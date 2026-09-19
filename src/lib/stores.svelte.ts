export type ReadMode = "page" | "scroll";
export type ThemeKey = "paper" | "white" | "green" | "dark" | "night";
export type ZoneKey = "tl" | "tr" | "bl" | "br" | "c";
export type ZoneAction = BindAction | "none";

export const ZONE_DEFS: { id: ZoneKey; name: string }[] = [
  { id: "tl", name: "左上区域" },
  { id: "tr", name: "右上区域" },
  { id: "bl", name: "左下区域" },
  { id: "br", name: "右下区域" },
  { id: "c", name: "中间小区域" },
];

export const DEFAULT_ZONES: Record<ZoneKey, ZoneAction> = {
  tl: "chapterPrev",
  tr: "toc",
  bl: "prev",
  br: "next",
  c: "settings",
};

export type BindAction =
  | "next"
  | "prev"
  | "scrollDown"
  | "scrollUp"
  | "chapterNext"
  | "chapterPrev"
  | "toc"
  | "addBookmark"
  | "bookmarkList"
  | "ai"
  | "bookStats"
  | "settings"
  | "close";

export const BIND_DEFS: { id: BindAction; name: string }[] = [
  { id: "next", name: "下一页 / 下滚一屏" },
  { id: "prev", name: "上一页 / 上滚一屏" },
  { id: "scrollDown", name: "下滚一段" },
  { id: "scrollUp", name: "上滚一段" },
  { id: "chapterNext", name: "下一章" },
  { id: "chapterPrev", name: "上一章" },
  { id: "toc", name: "打开/关闭目录" },
  { id: "addBookmark", name: "添加书签" },
  { id: "bookmarkList", name: "书签列表" },
  { id: "ai", name: "AI 面板" },
  { id: "bookStats", name: "本书统计" },
  { id: "settings", name: "设置" },
  { id: "close", name: "返回书库" },
];

export const DEFAULT_BINDINGS: Record<BindAction, string[]> = {
  next: ["ArrowRight", "Space", "PageDown", "Enter"],
  prev: ["ArrowLeft", "PageUp"],
  scrollDown: ["ArrowDown"],
  scrollUp: ["ArrowUp"],
  chapterNext: [],
  chapterPrev: [],
  toc: ["F3"],
  addBookmark: [],
  bookmarkList: [],
  ai: [],
  bookStats: [],
  settings: [],
  close: ["Esc"],
};

export function comboOf(e: { key: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): string {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  let k = e.key;
  if (k === " ") k = "Space";
  if (k === "Escape") k = "Esc";
  if (k.length === 1) k = k.toUpperCase();
  parts.push(k);
  return parts.join("+");
}

function normBindKey(s: string): string {
  const k = s.trim().toLowerCase();
  return k === "escape" ? "esc" : k;
}

export function bindKeyMatches(stored: string, combo: string): boolean {
  return normBindKey(stored) === normBindKey(combo);
}

export type TextAlign = "justify" | "left";
export type ScrollBounceLevel = "weak" | "medium" | "strong";

/// 滚动模式「章节过渡」三档预设：数值越大，切换章节所需滚动量越大。
export const SCROLL_BOUNCE_PRESETS: Record<
  ScrollBounceLevel,
  { strength: number; threshold: number; label: string }
> = {
  weak: { strength: 0.25, threshold: 180, label: "弱（少量滚动即切换）" },
  medium: { strength: 0.5, threshold: 340, label: "中（推荐）" },
  strong: { strength: 1.2, threshold: 800, label: "强（需滚更多才切换）" },
};

export interface ReaderConfig {
  mode: ReadMode;
  duo: boolean;
  colGap: number;
  anim: boolean;
  fontFamily: string;
  latinFont: string;
  fontSize: number;
  /** 正文字重：#chapter-content p（300 细 / 400 常规 / 500 中） */
  fontWeight: number;
  lineHeight: number;
  paraSpacing: number;
  marginH: number;
  /** >0 时忽略 marginH，按「每行 N 字」自动居中（宽屏更友好） */
  maxLineChars: number;
  marginV: number;
  indent: string;
  textAlign: TextAlign;
  theme: ThemeKey;
  bgColor: string;
  fgColor: string;
  bgImage: string;
  bgOpacity: number;
  cssExtra: string;
  overrideBookCss: boolean;
  pageTurnKeys: boolean;
  wheelTurnPage: boolean;
  bindings: Record<BindAction, string[]>;
  zones: Record<ZoneKey, ZoneAction>;
  zoneHints: boolean;
  customFonts: { family: string; file: string }[];
  aiPanelWidth: number;
  scrollBounce: boolean;
  scrollBounceLevel: ScrollBounceLevel;
  multiWindow: boolean;
  statusPos: "left" | "center" | "right" | "none";
  statusColor: string;
  topbarColor: string;
  /** 划词划线默认样式（见 HIGHLIGHT_STYLES） */
  highlightStyle: string;
  /** 划词划线默认颜色（空=跟随主题强调色） */
  highlightColor: string;
  /** 划词「浏览器搜索」的链接模板，%s 会被替换为选中文字 */
  browserSearchUrl: string;
  conv: { title: string; author: string; css: string; coverPath: string };
}

export const HIGHLIGHT_STYLES: { id: string; name: string }[] = [
  { id: "underline", name: "下划线" },
  { id: "wavy", name: "波浪线" },
  { id: "dotted", name: "点线" },
  { id: "double", name: "双下划线" },
  { id: "background", name: "荧光笔" },
  { id: "strike", name: "删除线" },
  { id: "box", name: "方框" },
];

export interface WebdavConfig {
  enabled: boolean;
  base: string;
  user: string;
  pass: string;
  syncOnOpen: boolean;
  uploadBooks: boolean;
  downloadDir: string;
}

export type AiProvider = "openai" | "ollama" | "anthropic" | "gemini" | "azure-openai";

export interface AiProfile {
  /** 稳定唯一 id（用于渲染 key 与端点识别；旧配置迁移时补齐） */
  id: number;
  name: string;
  provider: AiProvider;
  baseUrl: string;
  apiKey: string;
  model: string;
  /** azure-openai 专用：部署名 */
  deployment: string;
  /** azure-openai 专用：api-version */
  apiVersion: string;
  proxyMode: "auto" | "direct" | "custom";
  proxyUrl: string;
}

export interface AiProviderMeta {
  id: AiProvider;
  label: string;
  baseUrlHint: string;
  modelHint: string;
  needsKey: boolean;
  /** 需要 deployment / api-version 字段 */
  azure?: boolean;
}

// 服务商适配表（前端）：下拉/表单字段/占位提示统一由此驱动，新增 provider 只需在此加一行。
export const AI_PROVIDERS: AiProviderMeta[] = [
  { id: "openai", label: "OpenAI 兼容 API", baseUrlHint: "https://api.deepseek.com", modelHint: "deepseek-chat", needsKey: true },
  { id: "ollama", label: "Ollama 本地", baseUrlHint: "http://localhost:11434", modelHint: "llama3", needsKey: false },
  { id: "anthropic", label: "Anthropic（Claude）", baseUrlHint: "https://api.anthropic.com", modelHint: "claude-3-5-sonnet-latest", needsKey: true },
  { id: "gemini", label: "Google Gemini", baseUrlHint: "https://generativelanguage.googleapis.com", modelHint: "gemini-1.5-flash", needsKey: true },
  { id: "azure-openai", label: "Azure OpenAI", baseUrlHint: "https://<资源名>.openai.azure.com", modelHint: "gpt-4o", needsKey: true, azure: true },
];

export function aiProviderMeta(p: string): AiProviderMeta {
  return AI_PROVIDERS.find((x) => x.id === p) ?? AI_PROVIDERS[0];
}

// 物理清除已废弃的“关闭思考”残留键（不隐藏、不保留）：
// 迁移/同步/导入路径一律调用，确保导出的 profile 中不再出现这些键，
// 发送链也永远读不到它们，行为恒为 auto。
export function stripReasoningArtifacts(ai: any): void {
  if (!ai) return;
  delete ai.reasoningMode;
  for (const p of ai.profiles || []) {
    delete p.reasoningOffModel;
    delete p.reasoningOffStrategy;
    delete p.protocolProbe;
  }
}

export interface PromptItem {
  id: number;
  name: string;
  text: string;
  key: string;
}

export interface AiStyleConfig {
  followReader: boolean;
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  paraSpacing: number;
  marginH: number;
  marginV: number;
  bgColor: string;
  fgColor: string;
  indent: string;
  cssExtra: string;
}

export interface AiConfig {
  enabled: boolean;
  autoOnChapter: boolean;
  system: string;
  profiles: AiProfile[];
  active: number;
  prompts: PromptItem[];
  display: "chat" | "reader";
  style: AiStyleConfig;
  /** 端点失败时自动切换并重跑上一次操作（默认开） */
  failover: boolean;
  /** 划词翻译/释义小窗提示词，{selection} 为选中文字 */
  quickPrompt: string;
}

// “关闭思考”功能已于 v0.1.24 后彻底移除（UI/发送链/存储三层都无残留）：
// AI 请求行为恒为 auto（模型服务端默认）。历史遗留键由 stripReasoningArtifacts
// 在 load/migrate 与 applyConfigJson（跨窗同步/导入）两条入口物理删除。

export const DEFAULT_AI_PROFILES: AiProfile[] = [
  {
    id: 1,
    name: "DeepSeek",
    provider: "openai",
    baseUrl: "https://api.deepseek.com",
    apiKey: "",
    model: "deepseek-chat",
    deployment: "",
    apiVersion: "",
    proxyMode: "auto",
    proxyUrl: "",
  },
];

let profileIdSeq = 1;

/** 补齐缺失/重复的 profile id，保证唯一稳定；返回归一化后的数组本身。 */
export function ensureProfileIds(profiles: AiProfile[]): void {
  let max = 0;
  for (const p of profiles) {
    if (typeof p.id === "number" && Number.isFinite(p.id) && p.id > max) max = p.id;
  }
  const used = new Set<number>();
  for (const p of profiles) {
    let id = p.id;
    if (!Number.isFinite(id) || id <= 0 || used.has(id)) id = ++max;
    used.add(id);
    p.id = id;
  }
  if (max > profileIdSeq) profileIdSeq = max;
}

function nextProfileId(): number {
  return ++profileIdSeq;
}

function sanitizeProvider(v: any): AiProvider {
  const s = typeof v === "string" ? v.toLowerCase() : "";
  if (s === "ollama") return "ollama";
  if (s === "anthropic" || s === "claude") return "anthropic";
  if (s === "gemini" || s === "google") return "gemini";
  if (s === "azure-openai" || s === "azure" || s === "azure_openai") return "azure-openai";
  return "openai";
}

/** 用默认字段模板补齐单个 profile（新增字段缺失时用默认值，不丢用户已填内容）。 */
export function normalizeProfile(p: any): AiProfile {
  const t = structuredClone(DEFAULT_AI_PROFILES[0]);
  const src = p && typeof p === "object" ? p : {};
  const proxyMode =
    src.proxyMode === "custom" || src.proxyMode === "direct" || src.proxyMode === "auto"
      ? src.proxyMode
      : "auto";
  return {
    ...t,
    ...src,
    name: typeof src.name === "string" ? src.name : t.name,
    provider: sanitizeProvider(src.provider),
    baseUrl: typeof src.baseUrl === "string" ? src.baseUrl : "",
    apiKey: typeof src.apiKey === "string" ? src.apiKey : "",
    model: typeof src.model === "string" ? src.model : "",
    deployment: typeof src.deployment === "string" ? src.deployment : "",
    apiVersion: typeof src.apiVersion === "string" ? src.apiVersion : "",
    proxyMode,
    proxyUrl: typeof src.proxyUrl === "string" ? src.proxyUrl : "",
    id: typeof src.id === "number" && Number.isFinite(src.id) && src.id > 0 ? src.id : 0,
  };
}

export function normalizeProfiles(list: any): AiProfile[] {
  const arr = (Array.isArray(list) && list.length ? list : structuredClone(DEFAULT_AI_PROFILES)).map(
    (p: any) => normalizeProfile(p),
  );
  ensureProfileIds(arr);
  return arr;
}

/** 生成不与现有端点重名的名称：优先「新端点」，否则「新端点2/3…」。 */
export function uniqueEndpointName(profiles: AiProfile[]): string {
  const names = new Set(profiles.map((p) => (p.name || "").trim()));
  if (!names.has("新端点")) return "新端点";
  for (let n = 2; n < 100000; n++) {
    const cand = `新端点${n}`;
    if (!names.has(cand)) return cand;
  }
  return `新端点${Date.now()}`;
}

/** 干净的新端点：不复制上一个端点的地址与凭证，provider 默认沿用当前类型。 */
export function newAiProfile(profiles: AiProfile[], provider: AiProvider = "openai"): AiProfile {
  return {
    id: nextProfileId(),
    name: uniqueEndpointName(profiles),
    provider,
    baseUrl: "",
    apiKey: "",
    model: "",
    deployment: "",
    apiVersion: "",
    proxyMode: "auto",
    proxyUrl: "",
  };
}

export const DEFAULT_PROMPTS: PromptItem[] = [
  {
    id: 1,
    name: "剧情总结",
    key: "F1",
    text: "请用简洁的中文总结以下小说章节的主要剧情（120字以内），并猜猜接下来的发展：\n\n{chapter}",
  },
  {
    id: 2,
    name: "翻译为中文",
    key: "F2",
    text: "请将以下内容翻译成流畅的中文，直接输出译文：\n\n{chapter}",
  },
  {
    id: 3,
    name: "精校润色",
    key: "F3",
    text: "请校对并润色以下文本：删除广告与乱码字符，修正错别字与标点，保持原意与结构不要扩写或缩写，直接输出处理后的正文，不要任何解释：\n\n{chapter}",
  },
  {
    id: 4,
    name: "人物情节梳理",
    key: "F4",
    text: "请列出本章出场人物（含身份/立场）和本章主要情节点（条目式，简洁）：\n\n{chapter}",
  },
];

export interface AppConfig {
  reader: ReaderConfig;
  webdav: WebdavConfig;
  ai: AiConfig;
  regex: string;
}

const DEFAULTS: AppConfig = {
  reader: {
    mode: "page",
    duo: false,
    colGap: 48,
    anim: true,
    fontFamily: '"Microsoft YaHei", "Noto Sans CJK SC", "PingFang SC", sans-serif',
    latinFont: "",
    fontSize: 29,
    fontWeight: 300,
    lineHeight: 1.55,
    paraSpacing: 0.5,
    marginH: 468,
    maxLineChars: 0,
    marginV: 51,
    indent: "1em",
    textAlign: "justify",
    theme: "paper",
    bgColor: "#FDF5E6",
    fgColor: "#2c2c2c",
    bgImage: "",
    bgOpacity: 0.35,
    cssExtra: `h2 {
  font-size: 1.6rem;
  font-weight: bold;
  text-align: center;
  margin: 2rem auto 2rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid color-mix(in srgb, currentColor 20%, transparent);
}

@media (max-width: 480px) {
  h2 {
    font-size: 1.4rem;
    margin: 1.5rem auto;
  }
}`,
    overrideBookCss: true,
    pageTurnKeys: true,
    wheelTurnPage: true,
    bindings: structuredClone(DEFAULT_BINDINGS),
    zones: structuredClone(DEFAULT_ZONES),
    zoneHints: false,
    customFonts: [],
    aiPanelWidth: 360,
    scrollBounce: true,
    scrollBounceLevel: "medium",
    multiWindow: false,
    statusPos: "center",
    statusColor: "",
    topbarColor: "",
    highlightStyle: "underline",
    highlightColor: "",
    browserSearchUrl: "https://www.baidu.com/s?wd=%s",
    conv: { title: "", author: "", css: `body {
  max-width: 680px;
  margin: 0 auto;
  padding: 1rem 0.75rem;
  line-height: 1.8;
  text-align: left;
}

h2 {
  color: #4682B4;
  font-size: 1.6rem;
  font-weight: bold;
  text-align: center;
  margin: 2rem auto 2rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid #dddddd;
}

p {
  text-indent: 2em;
  margin-top: 0;
  margin-bottom: 1rem;
}

@media (max-width: 480px) {
  body {
    padding: 1rem 0.5rem;
  }

  h2 {
    font-size: 1.4rem;
    margin: 1.5rem auto;
  }

  p {
    margin-bottom: 0.8rem;
  }
}`, coverPath: "" },
  },
  webdav: {
    enabled: false,
    base: "",
    user: "",
    pass: "",
    syncOnOpen: true,
    uploadBooks: false,
    downloadDir: "",
  },
  ai: {
    enabled: false,
    autoOnChapter: true,
    system: "你是一位专业的阅读助手。回答务必简洁准确，使用中文。",
    profiles: structuredClone(DEFAULT_AI_PROFILES),
    active: 0,
    prompts: structuredClone(DEFAULT_PROMPTS),
    display: "reader",
    style: {
      followReader: true,
      fontFamily: "",
      fontSize: 0,
      lineHeight: 0,
      paraSpacing: 0,
      marginH: 0,
      marginV: 0,
      bgColor: "",
      fgColor: "",
      indent: "1em",
      cssExtra: "",
    },
    failover: true,
    quickPrompt:
      "请解释下列选中的内容，并给出准确的中文翻译/释义（外文译为中文，中文则解释含义与出处语境）。直接输出结果，简洁准确：\n\n{selection}",
  },
  regex: "",
};

/// 旧版默认自定义 CSS（含写死的近黑字色与固定蓝标题色，破坏深色主题）。
/// 仅当存量配置与它逐字相同时才替换为新默认，避免覆盖用户自行修改过的内容。
const LEGACY_DEFAULT_CSS_EXTRA = `#chapter-content p { color: rgba(0,0,0,0.95); font-weight: 300; }

h2 {
  color: #4682B4;
  font-size: 1.6rem;
  font-weight: bold;
  text-align: center;
  margin: 2rem auto 2rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid #dddddd;
}

p {
  text-indent: 2em;
  margin-top: 0;
  margin-bottom: 1rem;
}

@media (max-width: 480px) {
  body {
    padding: 1rem 0.5rem;
  }

  h2 {
    font-size: 1.4rem;
    margin: 1.5rem auto;
  }

  p {
    margin-bottom: 0.8rem;
  }
}`;

const DEPRECATED_READER_KEYS = [
  "fauxBold",
  "fauxItalic",
  "letterSpacing",
  "punctuationCompression",
  "scrollBounceStrength",
  "scrollBounceThreshold",
] as const;

/// 归一化 reader 配置：清理已废弃键、修正非法新字段、
/// 迁移旧默认 cssExtra 与旧滚动缓冲数值。返回是否发生了需要落盘的清理。
function normalizeReaderConfig(r: any, savedRaw?: any): boolean {
  let dirty = false;
  for (const k of DEPRECATED_READER_KEYS) {
    if (Object.prototype.hasOwnProperty.call(r, k)) {
      delete r[k];
      dirty = true;
    }
  }
  const num = (v: any, min: number, max: number, def: number) => {
    const n = typeof v === "number" ? v : Number(v);
    if (!Number.isFinite(n)) return def;
    return Math.max(min, Math.min(max, Math.round(n)));
  };
  const cg = num(r.colGap, 0, 200, DEFAULTS.reader.colGap);
  if (r.colGap !== cg) { r.colGap = cg; dirty = true; }
  const fw = r.fontWeight === 400 || r.fontWeight === 500 || r.fontWeight === 300 ? r.fontWeight : DEFAULTS.reader.fontWeight;
  if (r.fontWeight !== fw) { r.fontWeight = fw; dirty = true; }
  const mlc = num(r.maxLineChars, 0, 120, 0);
  if (r.maxLineChars !== mlc) { r.maxLineChars = mlc; dirty = true; }
  const ta = r.textAlign === "left" ? "left" : "justify";
  if (r.textAlign !== ta) { r.textAlign = ta; dirty = true; }
  // 旧 strength/threshold → 三档 level（仅当旧字段存在且未有 level 时换算）
  if (
    savedRaw &&
    savedRaw.scrollBounceLevel == null &&
    (savedRaw.scrollBounceStrength != null || savedRaw.scrollBounceThreshold != null)
  ) {
    const s = Number(savedRaw.scrollBounceStrength);
    const t = Number(savedRaw.scrollBounceThreshold);
    const score = (Number.isFinite(s) ? s : 0.5) * 1000 + (Number.isFinite(t) ? t : 340);
    r.scrollBounceLevel = score < 1000 ? "weak" : score > 2500 ? "strong" : "medium";
    dirty = true;
  }
  const lvl = r.scrollBounceLevel;
  if (lvl !== "weak" && lvl !== "medium" && lvl !== "strong") {
    r.scrollBounceLevel = "medium";
    dirty = true;
  }
  // 旧默认 cssExtra（写死字色/标题色）→ 新默认，仅逐字匹配时替换
  if (savedRaw && savedRaw.cssExtra === LEGACY_DEFAULT_CSS_EXTRA) {
    r.cssExtra = DEFAULTS.reader.cssExtra;
    dirty = true;
  }
  return dirty;
}

export const THEMES: Record<ThemeKey, { bg: string; fg: string; sel: string }> = {
  paper: { bg: "#f5f1e8", fg: "#3b3630", sel: "#e8e0cd" },
  white: { bg: "#ffffff", fg: "#2b2b2b", sel: "#eef1f4" },
  green: { bg: "#c7e6c9", fg: "#2d3a2d", sel: "#b3d9b5" },
  dark: { bg: "#23262b", fg: "#b8bcc2", sel: "#33373d" },
  night: { bg: "#0f1115", fg: "#8a8f96", sel: "#1d2026" },
};

const KEY = "flowread.config.v1";

function migrateAi(savedAi: any): AiConfig {
  const base = {
    ...structuredClone(DEFAULTS.ai),
    ...(savedAi || {}),
    style: { ...structuredClone(DEFAULTS.ai.style), ...(savedAi?.style || {}) },
  } as AiConfig;
  if (base.display !== "reader") base.display = "chat";
  stripReasoningArtifacts(base);
  const hasNew = Array.isArray(savedAi?.profiles) && savedAi.profiles.length;
  if (hasNew) {
    base.profiles = normalizeProfiles(savedAi.profiles);
    stripReasoningArtifacts(base);
    if (Array.isArray(savedAi.prompts) && savedAi.prompts.length) {
      base.prompts = savedAi.prompts;
    }
    base.active = Math.min(Math.max(savedAi.active | 0, 0), base.profiles.length - 1);
    return base;
  }
  let profiles: AiProfile[];
  const legacyUrl = savedAi?.baseUrl || savedAi?.apiKey || savedAi?.model;
  if (legacyUrl) {
    const pm = savedAi?.proxyMode === "custom" ? "custom" : "auto";
    profiles = normalizeProfiles([
      {
        name: "默认",
        provider: savedAi?.provider ?? "openai",
        baseUrl: savedAi?.baseUrl ?? "",
        apiKey: savedAi?.apiKey ?? "",
        model: savedAi?.model ?? "",
        deployment: savedAi?.deployment ?? "",
        apiVersion: savedAi?.apiVersion ?? "",
        proxyMode: pm,
        proxyUrl: savedAi?.proxyUrl ?? "",
      },
    ]);
  } else {
    profiles = structuredClone(DEFAULT_AI_PROFILES);
  }
  const prompts = structuredClone(DEFAULT_PROMPTS);
  if (savedAi?.promptSummary) prompts[0].text = savedAi.promptSummary;
  if (savedAi?.promptTranslate) prompts[1].text = savedAi.promptTranslate;
  if (savedAi?.action === "custom" && savedAi?.customPrompt) {
    prompts.push({ id: 99, name: "自定义", key: "F9", text: savedAi.customPrompt });
  }
  return {
    enabled: !!savedAi?.enabled,
    autoOnChapter: savedAi?.autoOnChapter ?? true,
    system: savedAi?.system ?? DEFAULTS.ai.system,
    profiles,
    active: 0,
    prompts,
    display: "reader",
    style: structuredClone(DEFAULTS.ai.style),
    failover: savedAi?.failover ?? true,
    quickPrompt: savedAi?.quickPrompt ?? DEFAULTS.ai.quickPrompt,
  };
}

let reasoningMigrationDirty = false;
let readerMigrationDirty = false;
function load(): AppConfig {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return structuredClone(DEFAULTS);
    const saved = JSON.parse(raw);
    // “关闭思考”已彻底移除：迁移时若发现存量旧键，加载后立即物理写回，保证导出的 profile 不含这些键。
    const aiSaved = saved.ai || {};
    const profSaved: any[] = Array.isArray(aiSaved.profiles) ? aiSaved.profiles : [];
    const hasLegacyAiKey = Object.prototype.hasOwnProperty.call(aiSaved, "reasoningMode");
    const hasLegacyProfKey = profSaved.some(
      (p: any) =>
        p &&
        (Object.prototype.hasOwnProperty.call(p, "reasoningOffModel") ||
          Object.prototype.hasOwnProperty.call(p, "reasoningOffStrategy") ||
          Object.prototype.hasOwnProperty.call(p, "protocolProbe")),
    );
    reasoningMigrationDirty = hasLegacyAiKey || hasLegacyProfKey;
    let indent = saved.reader?.indent;
    if (typeof indent === "boolean") indent = indent ? "2em" : "0em";
    if (indent == null) indent = "1em";
    const merged: AppConfig = {
      reader: {
        ...DEFAULTS.reader,
        ...saved.reader,
        indent,
        bindings: {
          ...structuredClone(DEFAULT_BINDINGS),
          ...(saved.reader?.bindings || {}),
        },
        zones: {
          ...structuredClone(DEFAULT_ZONES),
          ...(saved.reader?.zones || {}),
        },
      },
      webdav: { ...DEFAULTS.webdav, ...saved.webdav },
      ai: migrateAi(saved.ai),
      regex: saved.regex ?? "",
    };
    // 迁移：旧默认字体（微软雅黑）替换为书宋衬线字体，以贴近静读天下效果
    if (merged.reader.fontFamily === '"Microsoft YaHei", "Segoe UI", serif') {
      merged.reader.fontFamily = DEFAULTS.reader.fontFamily;
    }
    for (const z of ZONE_DEFS) {
      if (!BIND_DEFS.some((d) => d.id === (merged.reader.zones as Record<string, string>)[z.id]) && (merged.reader.zones as Record<string, string>)[z.id] !== "none") {
        (merged.reader.zones as Record<string, string>)[z.id] = DEFAULT_ZONES[z.id];
      }
    }
    // 迁移：旧版本把 Esc 存成 "Escape"，与 comboOf 产生的 "Esc" 不一致，导致 Esc 失效
    for (const act of Object.keys(merged.reader.bindings) as BindAction[]) {
      const arr = merged.reader.bindings[act];
      if (Array.isArray(arr)) {
        merged.reader.bindings[act] = arr.map((k) => (normBindKey(k) === "esc" ? "Esc" : k));
      }
    }
    // 归一化 reader：清理已废弃键（伪粗体/伪斜体/字距/标点压缩/滚动缓冲数值），
    // 迁移旧默认 cssExtra 与旧滚动缓冲，修正非法新字段。返回 true 时需要落盘。
    if (normalizeReaderConfig(merged.reader, saved.reader)) readerMigrationDirty = true;
    return merged;
  } catch {
    return structuredClone(DEFAULTS);
  }
}

export const config = $state<AppConfig>(load());

/** 返回一份 reader 默认配置（深拷贝，供设置页回退字体/恢复默认使用）。 */
export function defaultReaderConfig(): ReaderConfig {
  return structuredClone(DEFAULTS.reader);
}

/** 仅重置「阅读」设置段（不动 AI / WebDAV / 分章正则）。 */
export function resetReaderConfig(): void {
  config.reader = structuredClone(DEFAULTS.reader);
  saveConfig();
}

// 迁移落盘：检测到已废弃的旧键/需迁移的旧值（“关闭思考”、reader 废弃字段与旧默认）时，
// 启动即把清理后的配置写回 localStorage。延后到模块初始化完成，避免 TDZ。
if (reasoningMigrationDirty || readerMigrationDirty) setTimeout(saveConfig, 0);

export function applyConfigJson(json: string) {
  try {
    if (json === JSON.stringify(config)) return;
    const obj = JSON.parse(json);
    if (!obj || typeof obj !== "object" || !obj.reader || !obj.ai) return;
    const rawReader = obj.reader;
    obj.reader = {
      ...structuredClone(DEFAULTS.reader),
      ...obj.reader,
      bindings: {
        ...structuredClone(DEFAULT_BINDINGS),
        ...(obj.reader.bindings || {}),
      },
      zones: {
        ...structuredClone(DEFAULT_ZONES),
        ...(obj.reader.zones || {}),
      },
    };
    // 跨窗同步/导入同样清理 reader 废弃键并迁移旧默认值
    normalizeReaderConfig(obj.reader, rawReader);
    obj.ai = { ...structuredClone(DEFAULTS.ai), ...obj.ai };
    obj.ai.profiles = normalizeProfiles(obj.ai.profiles);
    obj.ai.style = { ...structuredClone(DEFAULTS.ai.style), ...(obj.ai.style || {}) };
    if (obj.ai.failover !== false) obj.ai.failover = true;
    obj.ai.active = Math.min(Math.max((obj.ai.active as number) | 0, 0), obj.ai.profiles.length - 1);
    stripReasoningArtifacts(obj.ai);
    if (obj.ai.display !== "reader") obj.ai.display = "chat";
    for (const k of Object.keys(config) as (keyof AppConfig)[]) {
      (config as any)[k] = obj[k];
    }
    // 记录为“已保存”，避免收到自身广播回声时重复 apply / 再次广播
    lastSavedJson = JSON.stringify(config);
    // 跨窗同步/导入同样物理清除废弃的“关闭思考”键（stripReasoningArtifacts），
    // 保证两端内存与 localStorage 均不再出现这些键，行为恒为 auto。
  } catch {
    /* ignore */
  }
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;
let lastSavedJson = "";

/** 立即写盘并广播（端点切换等需要“即时生效”的场景）。 */
export function saveConfigNow() {
  clearTimeout(saveTimer);
  try {
    const json = JSON.stringify(config);
    localStorage.setItem(KEY, json);
    if (json !== lastSavedJson) {
      lastSavedJson = json;
      void import("@tauri-apps/api/event")
        .then((m) => m.emit("flowread-config", json))
        .catch(() => {});
    }
  } catch {
    /* ignore */
  }
}

export function saveConfig() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveConfigNow, 300);
}

// 跨窗双向同步：主窗口与独立 AI 窗口都监听同一广播事件。
// 用 lastSavedJson 去重，避免自身回声导致的循环（applyConfigJson 不再触发广播）。
void import("@tauri-apps/api/event")
  .then((m) =>
    m.listen<string>("flowread-config", (e) => {
      if (e.payload && e.payload !== lastSavedJson) applyConfigJson(e.payload);
    }),
  )
  .catch(() => {});

export interface Toast {
  id: number;
  msg: string;
  kind: "info" | "error";
  sticky: boolean;
}

export const toasts = $state<{ items: Toast[] }>({ items: [] });
let toastId = 0;

export function toast(msg: string, kind: "info" | "error" = "info", sticky?: boolean) {
  const id = ++toastId;
  const t: Toast = { id, msg, kind, sticky: sticky ?? kind === "error" };
  toasts.items.push(t);
  if (!t.sticky) {
    setTimeout(() => dismissToast(id), 5200);
  }
}

export function dismissToast(id: number) {
  const i = toasts.items.findIndex((t) => t.id === id);
  if (i >= 0) toasts.items.splice(i, 1);
}

// 去除图片/SVG/样式等非文本标签：带插图（尤其 base64 内嵌）的章节
// 若直接按 HTML 截断传给 AI，会把正文挤出截断范围导致“没有文字”
export function stripNonTextHtml(html: string): string {
  if (!html) return "";
  return html
    .replace(/<img\b[^>]*>/gi, "")
    .replace(/<picture\b[\s\S]*?<\/picture>/gi, "")
    .replace(/<source\b[^>]*>/gi, "")
    .replace(/<svg\b[\s\S]*?<\/svg>/gi, "")
    .replace(/<style\b[\s\S]*?<\/style>/gi, "")
    .replace(/<script\b[\s\S]*?<\/script>/gi, "")
    .replace(/<video\b[\s\S]*?(<\/video>)?/gi, "")
    .replace(/<audio\b[\s\S]*?(<\/audio>)?/gi, "")
    .replace(/<iframe\b[\s\S]*?(<\/iframe>)?/gi, "");
}

export async function copyText(text: string) {  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand("copy"); } catch { /* ignore */ }
    ta.remove();
  }
}

import { WebviewWindow, getAllWebviewWindows } from "@tauri-apps/api/webviewWindow";
import { api } from "./api";

export const APP_BUILD = "2026.09.13a";

// 与 Rust 主窗口本次启动实际使用的 WebView2 参数保持一致：
// Windows 上同一 user-data-dir 的 WebView2 环境要求所有窗口的
// additionalBrowserArguments 完全相同，否则新建 webview 静默失败（幽灵窗口）
let appliedArgs: Promise<string> | null = null;
function mainWindowArgs(): Promise<string> {
  if (!appliedArgs) {
    appliedArgs = api
      .getSetting("render.webviewArgs.applied")
      .then((v) => (v || "").trim())
      .catch(() => "");
  }
  return appliedArgs;
}

async function winExtraOptions(): Promise<{ additionalBrowserArgs?: string }> {
  const a = await mainWindowArgs();
  return a ? { additionalBrowserArgs: a } : {};
}

export const ui = $state({
  view: "library" as "library" | "reader",
  settingsOpen: false,
  searchOpen: false,
  aiOpen: false,
  statsOpen: false,
  webdavOpen: false,
});

export const convertState = $state({
  open: false,
  book: null as { id: number; title: string; path: string; format: string } | null,
});

export const AI_WIN_LABEL = "flowread-ai";

export const aiState = $state({
  lastMode: (typeof localStorage !== "undefined"
    ? (localStorage.getItem("fr.ai.mode") as "drawer" | "window" | null)
    : null) ?? "drawer",
});

export function saveAiMode(m: "drawer" | "window") {
  aiState.lastMode = m;
  try {
    localStorage.setItem("fr.ai.mode", m);
  } catch { /* ignore */ }
}

export async function openAiWindow() {
  saveAiMode("window");
  const w = Number(localStorage.getItem("fr.ai.winw")) || 430;
  const h = Number(localStorage.getItem("fr.ai.winh")) || 700;
  try {
    const win = new WebviewWindow(AI_WIN_LABEL, {
      url: "/?ai=1",
      title: "FlowRead AI",
      width: w,
      height: h,
      minWidth: 300,
      minHeight: 260,
      center: true,
      ...(await winExtraOptions()),
    });
    win.once("tauri://created", () => {
      ui.aiOpen = false;
      localStorage.setItem("fr.ai.open", "1");
      try {
        emit("ai-window-status", { open: true });
      } catch { /* ignore */ }
    });
    win.once("tauri://error", () => {
      saveAiMode("drawer");
      ui.aiOpen = true;
    });
  } catch {
    saveAiMode("drawer");
    ui.aiOpen = true;
  }
}

export function aiWindowOpenMark(): string | null {
  try {
    return localStorage.getItem("fr.ai.open");
  } catch {
    return null;
  }
}

export async function emit(_event: string, _payload: unknown) {
  try {
    await (await import("@tauri-apps/api/event")).emit(_event, _payload);
  } catch { /* ignore */ }
}

export async function toggleAi() {
  let mode = aiState.lastMode;
  try {
    const m = localStorage.getItem("fr.ai.mode");
    if (m === "drawer" || m === "window") mode = m as "drawer" | "window";
  } catch { /* ignore */ }
  if (mode === "window") {
    try {
      const w = await WebviewWindow.getByLabel(AI_WIN_LABEL);
      if (w) {
        await w.unminimize().catch(() => {});
        await w.show().catch(() => {});
        await w.setFocus().catch(() => {});
        localStorage.setItem("fr.ai.open", "1");
        return;
      }
    } catch { /* ignore */ }
    try {
      const wins = await getAllWebviewWindows();
      const existing = wins.find((x) => x.label === AI_WIN_LABEL);
      if (existing) {
        existing.unminimize();
        existing.show();
        existing.setFocus();
        localStorage.setItem("fr.ai.open", "1");
        return;
      }
    } catch { /* ignore */ }
    localStorage.setItem("fr.ai.open", "0");
    openAiWindow();
    return;
  }
  ui.aiOpen = !ui.aiOpen;
}

export async function listWindowLabels(): Promise<string[]> {
  try {
    const wins = await getAllWebviewWindows();
    return wins.map((x) => x.label);
  } catch {
    return [];
  }
}

export const pendingOpen = $state({ bookId: 0, nonce: 0 });

export function requestOpenBook(bookId: number) {
  pendingOpen.bookId = bookId;
  pendingOpen.nonce++;
  ui.view = "reader";
}

export function openBook(
  book: { id: number; title: string },
) {
  if (config.reader.multiWindow) void openBookInNewWindow(book);
  else requestOpenBook(book.id);
}

export async function openBookInNewWindow(
  book: { id: number; title: string },
) {
  const label = `reader-${book.id}`;
  const extra = await winExtraOptions();
  try {
    const ww = new WebviewWindow(label, {
      url: `/?book=${book.id}&title=${encodeURIComponent(book.title)}`,
      title: book.title,
      width: 1100,
      height: 780,
      minWidth: 640,
      minHeight: 480,
      center: true,
      visible: true,
      ...extra,
    });
    ww.once("tauri://error", () => {
      requestOpenBook(book.id);
    });
  } catch {
    requestOpenBook(book.id);
  }
}

export const confirmBox = $state({ msg: "", active: false });
let confirmResolve: ((v: boolean) => void) | null = null;

export function appConfirm(msg: string): Promise<boolean> {
  confirmBox.msg = msg;
  confirmBox.active = true;
  return new Promise((res) => {
    confirmResolve = res;
  });
}

export function confirmAnswer(v: boolean) {
  confirmBox.active = false;
  confirmResolve?.(v);
  confirmResolve = null;
}
