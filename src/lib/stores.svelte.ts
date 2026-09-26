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
  | "ttsPlayPause"
  | "ttsStop"
  | "ttsNext"
  | "ttsPrev"
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
  { id: "ttsPlayPause", name: "朗读：播放/暂停" },
  { id: "ttsStop", name: "朗读：停止" },
  { id: "ttsNext", name: "朗读：下一句" },
  { id: "ttsPrev", name: "朗读：上一句" },
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
  ttsPlayPause: [],
  ttsStop: [],
  ttsNext: [],
  ttsPrev: [],
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

/** 顶栏/底栏可显示的状态项。 */
export type StatusItemId =
  | "bookTitle"
  | "chapterName"
  | "chapterIndex"
  | "chapterCount"
  | "chapterPages"
  | "bookPages"
  | "pagePercent"
  | "bookProgress"
  | "chars"
  | "clock"
  | "speed"
  | "mode";

export interface StatusItem {
  id: StatusItemId;
  enabled: boolean;
}

/** 状态项清单（设置页勾选；顶栏顺序即可在阅读界面拖动排序）。 */
export const STATUS_ITEM_DEFS: { id: StatusItemId; name: string }[] = [
  { id: "bookTitle", name: "书名" },
  { id: "chapterName", name: "章节名" },
  { id: "chapterIndex", name: "当前章号（第X章）" },
  { id: "chapterCount", name: "总章数（共Y章）" },
  { id: "chapterPages", name: "本节页数 / 百分比" },
  { id: "bookPages", name: "全书总页数（估算）" },
  { id: "pagePercent", name: "本节进度" },
  { id: "bookProgress", name: "全书进度" },
  { id: "chars", name: "本章字数" },
  { id: "clock", name: "时间" },
  { id: "speed", name: "阅读速度（字/分）" },
  { id: "mode", name: "阅读模式" },
];

export const STATUS_ITEM_IDS: Set<string> = new Set(STATUS_ITEM_DEFS.map((d) => d.id));

/// 滚动模式「章节过渡」三档预设：数值越大，切换章节所需滚动量越大。
export const SCROLL_BOUNCE_PRESETS: Record<
  ScrollBounceLevel,
  { strength: number; threshold: number; label: string }
> = {
  weak: { strength: 0.25, threshold: 180, label: "弱（少量滚动即切换）" },
  medium: { strength: 0.5, threshold: 340, label: "中（推荐）" },
  strong: { strength: 1.2, threshold: 800, label: "强（需滚更多才切换）" },
};

/** 阅读界面顶栏右侧可拖拽排序/可隐藏的按钮 id（默认顺序）。 */
export const READER_TOPBAR_IDS = [
  "webdav", "convert", "bookmarkAdd", "bookmarkList", "ttsPlay", "ttsStop",
  "highlight", "ai", "stats", "toc", "manga", "manager", "images", "css", "settings",
] as const;

/** 漫画显示方式：瀑布流（纵向连续） / 横向（横向连续阅读）。 */
export type MangaLayout = "flow" | "horizontal";
/** 漫画阅读方向：从左到右 / 从右到左（日漫）。 */
export type MangaDirection = "ltr" | "rtl";

/** 漫画模式独立设置（与普通阅读排版分开；仅漫画模式/手动漫画模式下生效）。 */
export interface MangaConfig {
  /** 默认图片大小倍率（阅读界面右下角可再按单本覆盖） */
  imgScale: number;
  /** 默认图片适应阅读器宽度 */
  fitWidth: boolean;
  /** 漫画模式左右边距（px） */
  marginH: number;
  /** 漫画模式上下边距（px） */
  marginV: number;
  /** 图片之间的间距（px，支持 0=无缝紧密连接） */
  imgGap: number;
  /** 显示方式：瀑布流（纵向连续）/ 横向（横向连续阅读） */
  layout: MangaLayout;
  /** 阅读方向：从左到右 / 从右到左 */
  direction: MangaDirection;
  /** 仅显示图片（隐藏正文文字） */
  hideText: boolean;
  /** 漫画背景色（空=跟随普通主题背景） */
  bgColor: string;
}
export type ReaderTopbarId = (typeof READER_TOPBAR_IDS)[number];

/** 书库顶栏可拖拽排序的图标按钮 id（默认顺序）。 */
export const LIBRARY_TOPBAR_IDS = ["webdav", "random", "search", "notes", "stats", "settings"] as const;
export type LibraryTopbarId = (typeof LIBRARY_TOPBAR_IDS)[number];

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
  /** 沉浸阅读：无操作一段时间后自动淡出顶栏/底栏，移动鼠标或按键恢复。 */
  immersive: boolean;
  customFonts: { family: string; file: string }[];
  aiPanelWidth: number;
  scrollBounce: boolean;
  scrollBounceLevel: ScrollBounceLevel;
  multiWindow: boolean;
  statusPos: "left" | "center" | "right" | "none";
  statusColor: string;
  topbarColor: string;
  /** 阅读界面顶栏右侧按钮顺序（含隐藏项；拖动排序后持久化） */
  topbarOrder: string[];
  /** 阅读界面顶栏隐藏的按钮 id（重新显示时回到 topbarOrder 中的位置） */
  topbarHidden: string[];
  /** 漫画书籍图片大小倍率（旧字段，已迁移到 manga.imgScale，保留仅用于迁移） */
  mangaImageScale?: number;
  /** 漫画模式独立设置 */
  manga: MangaConfig;
  /** 是否已展示过漫画首次引导（全局，仅第一次打开漫画时提示） */
  mangaGuideDone: boolean;
  /** 顶栏状态项（顺序=显示顺序；可在阅读界面拖动排序） */
  statusItems: StatusItem[];
  /** 底栏进度条左侧显示项（顺序=显示顺序） */
  footerLeft: StatusItemId[];
  /** 底栏进度条右侧显示项（顺序=显示顺序） */
  footerRight: StatusItemId[];
  /** 划词划线默认样式（见 HIGHLIGHT_STYLES） */
  highlightStyle: string;
  /** 划词划线默认颜色（空=跟随主题强调色） */
  highlightColor: string;
  /** 划词菜单可用的颜色调色板（可增删） */
  highlightColors: string[];
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

/** 划词菜单与划线设置里的预设颜色。 */
export const HIGHLIGHT_COLORS: string[] = [
  "#c0392b",
  "#e67e22",
  "#f1c40f",
  "#27ae60",
  "#2980b9",
  "#8e44ad",
  "#d3549e",
  "#7f8c8d",
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

export type TtsEngine = "system" | "online";

export interface TtsConfig {
  /** 朗读引擎：system=系统离线语音；online=在线神经语音 */
  engine: TtsEngine;
  /** 在线语音来源：profile=复用 AI 端点；custom=自定义本地/远程 TTS 服务地址 */
  onlineSource: "profile" | "custom";
  /** 自定义 TTS 服务的接口协议：openai=OpenAI 兼容 /v1/audio/speech；dashscope=通义千问 Qwen-TTS */
  customProvider: string;
  /** 自定义 TTS 服务地址（OpenAI 兼容 /v1/audio/speech） */
  customBaseUrl: string;
  /** 自定义 TTS 服务密钥（本地服务可留空） */
  customApiKey: string;
  /** 在线语音使用的 AI 端点 id（0=使用当前端点） */
  onlineProfileId: number;
  /** 在线语音模型（如 tts-1 / gpt-4o-mini-tts） */
  onlineModel: string;
  /** 在线语音音色（固定音色 id；留空=服务端默认） */
  onlineVoice: string;
  /** 在线语音音频格式 */
  onlineFormat: string;
  /** 指定语音（voice.name 或 voiceURI），空=按语言自动选择 */
  voice: string;
  /** 自动选择语音时的语言 */
  lang: string;
  rate: number;
  pitch: number;
  volume: number;
  /** 一章读完自动续读下一章 */
  autoNextChapter: boolean;
  /** 朗读时高亮当前句并自动滚动跟随 */
  highlight: boolean;
  /** 定时停止（分钟，0=关闭） */
  sleepMinutes: number;
}

export interface LibraryConfig {
  /** 书库顶栏图标按钮顺序（拖动排序后持久化） */
  topbarOrder: string[];
}

/** 应用外壳界面配色（仅影响书库/设置/面板等外壳，与阅读正文完全独立）。 */
export type UiTheme = "light" | "dark" | "system";

/** 外壳配色方案（卡片点选）。 */
export interface UiSkin {
  id: string;
  name: string;
  /** 明暗基调，用于跟随系统时挑选对应肤色。 */
  mode: "light" | "dark";
  bg: string;
  panel: string;
  panel2: string;
  fg: string;
  fg2: string;
  line: string;
  accent: string;
  /** 主按钮实底文字色（对比 accent）。 */
  accentFg: string;
  /** 语义色（用于状态/危险动作）。 */
  success: string;
  warning: string;
  danger: string;
  /** 卡片圆角（部分皮肤走硬朗/柔和两种气质）。 */
  radiusCard: string;
  /** 卡片阴影：柔 or 细线（部分皮肤几乎不用阴影）。 */
  shadow: string;
}

/**
 * 外壳可选皮肤：3 浅 + 1 深 + 1 近黑（共 5 套）。
 * - Cream Island / Nocturne：取自用户提供的设计稿（仅提取配色与"细线/平板"气质，不引入衬线标题）。
 * - 麻布 / 冷雾 / 石墨：在保留 4px 栅格与对比度规范下做性格化扩展。
 */
export const UI_SKINS: UiSkin[] = [
  // ===== 浅色 =====
  {
    id: "cream",
    name: "奶油",
    mode: "light",
    bg: "#FFF0DF", panel: "#FFFFFF", panel2: "#FBEBDA",
    fg: "#292524", fg2: "#78716C", line: "rgba(41,37,36,0.10)",
    accent: "#1C1917", accentFg: "#FFF0DF",
    success: "#2D6A4F", warning: "#FF6900", danger: "#9F1239",
    radiusCard: "16px", shadow: "0 1px 2px rgba(41,37,36,0.06)",
  },
  {
    id: "linen",
    name: "麻布",
    mode: "light",
    bg: "#F5F1E8", panel: "#FFFDF9", panel2: "#ECE6D9",
    fg: "#2E2A24", fg2: "#7A7367", line: "rgba(46,42,36,0.11)",
    accent: "#8C5A2B", accentFg: "#FFFDF9",
    success: "#3F7A4E", warning: "#B07A1E", danger: "#A8453C",
    radiusCard: "10px", shadow: "0 1px 3px rgba(46,42,36,0.08), 0 1px 2px rgba(46,42,36,0.05)",
  },
  {
    id: "mist",
    name: "冷雾",
    mode: "light",
    bg: "#F3F6FA", panel: "#FFFFFF", panel2: "#E8EEF6",
    fg: "#20242C", fg2: "#5F6975", line: "rgba(32,36,44,0.09)",
    accent: "#3B6FB0", accentFg: "#FFFFFF",
    success: "#2F7D5B", warning: "#A9761B", danger: "#B4433C",
    radiusCard: "12px", shadow: "0 1px 2px rgba(32,36,44,0.05), 0 6px 20px rgba(32,36,44,0.06)",
  },
  // ===== 深色 =====
  {
    id: "nocturne",
    name: "夜行",
    mode: "dark",
    bg: "#13111C", panel: "#1C1A28", panel2: "#0B0B0F",
    fg: "#F7F7F8", fg2: "#A1A0AB", line: "#33323E",
    accent: "#59497A", accentFg: "#F7F7F8",
    success: "#428A72", warning: "#B08A3E", danger: "#F9A8D4",
    radiusCard: "12px", shadow: "none",
  },
  {
    id: "graphite",
    name: "石墨",
    mode: "dark",
    bg: "#16181D", panel: "#1E2126", panel2: "#252932",
    fg: "#E6E8EC", fg2: "#9AA0A8", line: "rgba(255,255,255,0.10)",
    accent: "#4FAE95", accentFg: "#0F1115",
    success: "#46B47C", warning: "#D4A72C", danger: "#E06B62",
    radiusCard: "12px", shadow: "0 1px 2px rgba(0,0,0,0.35)",
  },
];

/** 旧皮肤 id → 新皮肤 id（升级时迁移，避免用户所选失效）。 */
export const UI_SKIN_ALIAS: Record<string, string> = {
  mist: "mist",
  cool: "mist",
  eye: "linen",
  graphite: "graphite",
  ink: "nocturne",
};

/** 解析皮肤 id（含旧 id 迁移），找不到时按明暗返回该组第一套。 */
export function resolveSkinId(id: string | undefined, mode: "light" | "dark"): string {
  const mapped = id ? (UI_SKIN_ALIAS[id] ?? id) : "";
  if (UI_SKINS.some((s) => s.id === mapped)) return mapped;
  return UI_SKINS.find((s) => s.mode === mode)?.id ?? UI_SKINS[0].id;
}

/** 阅读配色方案（完整排版快照，作为"存档点"）。 */
export interface ReaderScheme {
  id: string;
  name: string;
  /** 内置方案标记（可改名/删除，删除后不找回）。 */
  builtin?: boolean;
  /** 内置方案预设版本：用于升级时把内置方案刷新到新预设，用户自建方案不受影响。 */
  rev?: number;
  /** 快照的阅读排版配置。 */
  cfg: ReaderConfig;
}

/** 内置阅读方案当前预设版本；递增即可让存量安装的内置方案刷新为新预设。 */
const BUILTIN_SCHEME_REV = 2;

export interface AppConfig {
  reader: ReaderConfig;
  library: LibraryConfig;
  webdav: WebdavConfig;
  ai: AiConfig;
  tts: TtsConfig;
  regex: string;
  /** 外壳配色：light/dark 为指定皮肤 id；system 跟随系统明暗（用系统调色？否，用对应肤色的 system 皮肤）。 */
  uiTheme: UiTheme;
  /** 系统模式下浅色/深色各自使用的皮肤 id。 */
  uiSkinLight: string;
  uiSkinDark: string;
  /** 阅读配色方案库（全局共享）。 */
  readerSchemes: ReaderScheme[];
}

const DEFAULTS: AppConfig = {
  reader: {
    mode: "page",
    duo: false,
    colGap: 48,
    anim: true,
    fontFamily: "lxgwwenkai-regular",
    latinFont: "",
    fontSize: 32,
    fontWeight: 400,
    lineHeight: 1.55,
    paraSpacing: 0.5,
    marginH: 320,
    maxLineChars: 0,
    marginV: 80,
    indent: "1em",
    textAlign: "justify",
    theme: "paper",
    bgColor: "#FDF5E6",
    fgColor: "#2c2c2c",
    bgImage: "",
    bgOpacity: 0.35,
    cssExtra: `
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
}`,
    overrideBookCss: true,
    pageTurnKeys: true,
    wheelTurnPage: true,
    bindings: structuredClone(DEFAULT_BINDINGS),
    zones: { tl: "prev", tr: "next", bl: "prev", br: "next", c: "none" },
    zoneHints: false,
    immersive: false,
    customFonts: [
      { family: "仓耳云黑-w03", file: "仓耳云黑-w03.ttf" },
      { family: "方正兰亭圆_gbk", file: "方正兰亭圆_gbk.ttf" },
      { family: "lxgwneoxiheiscreen", file: "lxgwneoxiheiscreen.ttf" },
      { family: "lxgwwenkai-regular", file: "lxgwwenkai-regular.ttf" },
    ],
    aiPanelWidth: 360,
    scrollBounce: true,
    scrollBounceLevel: "weak",
    multiWindow: false,
    statusPos: "center",
    statusColor: "",
    topbarColor: "",
    topbarOrder: [...READER_TOPBAR_IDS],
    topbarHidden: [],
    manga: { imgScale: 1, fitWidth: false, marginH: 8, marginV: 8, imgGap: 6, layout: "flow", direction: "ltr", hideText: false, bgColor: "" },
    mangaGuideDone: false,
    statusItems: [
      { id: "chapterName", enabled: true },
      { id: "chapterPages", enabled: false },
      { id: "bookProgress", enabled: false },
      { id: "bookTitle", enabled: false },
      { id: "mode", enabled: false },
      { id: "speed", enabled: false },
      { id: "clock", enabled: false },
    ],
    footerLeft: ["clock", "bookPages"],
    footerRight: ["bookProgress", "chapterPages"],
    highlightStyle: "underline",
    highlightColor: "",
    highlightColors: structuredClone(HIGHLIGHT_COLORS),
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
  library: {
    topbarOrder: [...LIBRARY_TOPBAR_IDS],
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
    enabled: true,
    autoOnChapter: true,
    system: "你是一位专业的阅读助手。回答务必简洁准确，使用中文。",
    profiles: structuredClone(DEFAULT_AI_PROFILES),
    active: 0,
    prompts: [
      {
        id: 1,
        name: "剧情总结",
        key: "F1",
        text: "直观说出文本写了什么，不要做阅读理解，不要脑补，不要分段，不要分点，200字内。\n{chapter}",
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
    ],
    display: "reader",
    style: {
      followReader: false,
      fontFamily: "仓耳云黑-w03",
      fontSize: 28,
      lineHeight: 1.65,
      paraSpacing: 0,
      marginH: 98,
      marginV: 2,
      bgColor: "#FDF5E6",
      fgColor: "",
      indent: "1em",
      cssExtra: "",
    },
    failover: true,
    quickPrompt:
      "请解释下列选中的内容，并给出准确的中文翻译/释义（外文译为中文，中文则解释含义与出处语境）。直接输出结果，简洁准确：\n\n{selection}",
  },
  tts: {
    engine: "online",
    onlineSource: "custom",
    customProvider: "openai",
    customBaseUrl: "http://127.0.0.1:3900",
    customApiKey: "",
    onlineProfileId: 0,
    onlineModel: "omnivoice",
    onlineVoice: "33aa3b77",
    onlineFormat: "wav",
    voice: "Microsoft Kangkang - Chinese (Simplified, PRC)",
    lang: "zh-CN",
    rate: 1.4,
    pitch: 1.05,
    volume: 0.95,
    autoNextChapter: true,
    highlight: true,
    sleepMinutes: 0,
  },
  regex: "",
  uiTheme: "system",
  uiSkinLight: "cream",
  uiSkinDark: "nocturne",
  readerSchemes: [],
};
// 注：readerSchemes 默认空，由 load() 用内置方案补齐（见 builtinSchemes）。

/// 内置阅读方案（首次启动/未迁移时写入方案库；可改名、可删除）。
/// 每套方案把「排版 + 间距节奏 + 配色」整体打包，而不仅换背景色：
///  - 衬线（lxgwwenkai 楷体）走首行缩进 + 两端对齐的纸书版式；
///  - 无衬线（雅黑/苹方）走无缩进 + 左对齐的屏幕版式，并用每行字数上限控制行宽。
/// 字号/行高/段距/边距均按「长文阅读」调过，避免默认值在宽屏上把行拉得过长。
function builtinSchemes(): ReaderScheme[] {
  const base = () => structuredClone(DEFAULTS.reader);
  const mk = (id: string, name: string, patch: Partial<ReaderConfig>): ReaderScheme => ({
    id,
    name,
    builtin: true,
    rev: BUILTIN_SCHEME_REV,
    cfg: { ...base(), ...patch },
  });
  const SERIF = "lxgwwenkai-regular";
  const SANS = '"Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", sans-serif';
  return [
    mk("s-paper", "纸质", {
      theme: "paper", bgColor: "#FDF5E6", fgColor: "#2c2c2c",
      fontFamily: SERIF, latinFont: "Georgia",
      fontSize: 32, lineHeight: 1.75, paraSpacing: 0.6,
      marginH: 320, maxLineChars: 0, marginV: 80,
      indent: "1em", textAlign: "justify",
    }),
    mk("s-modern", "现代", {
      theme: "white", bgColor: "#FAFAF9", fgColor: "#22252A",
      fontFamily: SANS, latinFont: "Segoe UI",
      fontSize: 30, lineHeight: 1.7, paraSpacing: 0.9,
      marginH: 0, maxLineChars: 38, marginV: 72,
      indent: "0em", textAlign: "left",
    }),
    mk("s-green", "护眼绿", {
      theme: "green", bgColor: "#c7e6c9", fgColor: "#2d3a2d",
      fontFamily: SERIF, latinFont: "Georgia",
      fontSize: 32, lineHeight: 1.8, paraSpacing: 0.6,
      marginH: 320, maxLineChars: 0, marginV: 80,
      indent: "1em", textAlign: "justify",
    }),
    mk("s-night", "夜间", {
      theme: "dark", bgColor: "#23262b", fgColor: "#b8bcc2",
      fontFamily: SANS, latinFont: "Segoe UI",
      fontSize: 30, lineHeight: 1.7, paraSpacing: 0.8,
      marginH: 0, maxLineChars: 40, marginV: 72,
      indent: "0em", textAlign: "left",
    }),
    mk("s-ink", "纯黑", {
      theme: "night", bgColor: "#0f1115", fgColor: "#8a8f96",
      fontFamily: SANS, latinFont: "Segoe UI",
      fontSize: 30, lineHeight: 1.7, paraSpacing: 0.8,
      marginH: 0, maxLineChars: 40, marginV: 72,
      indent: "0em", textAlign: "left",
    }),
    mk("s-wide", "宽屏舒适", {
      theme: "paper", bgColor: "#F7F3EA", fgColor: "#2b2b2b",
      fontFamily: SERIF, latinFont: "Georgia",
      fontSize: 34, lineHeight: 1.8, paraSpacing: 0.6,
      marginH: 0, maxLineChars: 36, marginV: 96,
      indent: "1em", textAlign: "justify",
    }),
  ];
}

/// 内置方案升级：把「未被用户覆盖过」的内置方案刷新到当前预设（保留用户改过的名字），
/// 并补上新增的内置方案；用户自建方案原样保留。
/// 判定「未被覆盖」的方式：内置方案缺 rev 或 rev 落后于当前版本。
function upgradeBuiltinSchemes(list: ReaderScheme[]): { list: ReaderScheme[]; dirty: boolean } {
  const fresh = builtinSchemes();
  const freshById = new Map(fresh.map((s) => [s.id, s]));
  const seen = new Set<string>();
  let dirty = false;
  const out: ReaderScheme[] = [];
  for (const s of list) {
    seen.add(s.id);
    const f = s.builtin ? freshById.get(s.id) : undefined;
    if (f && s.rev !== BUILTIN_SCHEME_REV) {
      out.push({ ...f, name: s.name || f.name });
      dirty = true;
    } else {
      out.push(s);
    }
  }
  for (const f of fresh) {
    if (!seen.has(f.id)) {
      out.push(f);
      dirty = true;
    }
  }
  return { list: out, dirty };
}

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
  // 顶栏/底栏状态项：过滤非法 id、去重、保证非空
  if (!Array.isArray(r.statusItems)) {
    r.statusItems = structuredClone(DEFAULTS.reader.statusItems);
    dirty = true;
  } else {
    const seen = new Set<string>();
    const items: StatusItem[] = [];
    for (const it of r.statusItems) {
      const id = it && typeof it.id === "string" ? it.id : "";
      if (!STATUS_ITEM_IDS.has(id) || seen.has(id)) {
        dirty = true;
        continue;
      }
      seen.add(id);
      items.push({ id: id as StatusItemId, enabled: it.enabled !== false });
    }
    if (!items.length) {
      items.push(...structuredClone(DEFAULTS.reader.statusItems));
      dirty = true;
    }
    if (items.length !== r.statusItems.length) dirty = true;
    r.statusItems = items;
  }
  const normFooter = (v: any, def: StatusItemId[]): StatusItemId[] => {
    if (!Array.isArray(v)) return [...def];
    const out: StatusItemId[] = [];
    for (const x of v) {
      if (typeof x === "string" && STATUS_ITEM_IDS.has(x) && !out.includes(x as StatusItemId)) {
        out.push(x as StatusItemId);
      }
    }
    return out;
  };
  const fl = normFooter(r.footerLeft, DEFAULTS.reader.footerLeft);
  if (JSON.stringify(fl) !== JSON.stringify(r.footerLeft)) {
    r.footerLeft = fl;
    dirty = true;
  }
  const fr = normFooter(r.footerRight, DEFAULTS.reader.footerRight);
  if (JSON.stringify(fr) !== JSON.stringify(r.footerRight)) {
    r.footerRight = fr;
    dirty = true;
  }
  // 顶栏按钮顺序：过滤非法 id、去重，并把新增按钮补到末尾（保证旧配置升级后新按钮出现）。
  const normOrder = (v: any, all: readonly string[]): string[] => {
    const out: string[] = [];
    if (Array.isArray(v)) {
      for (const x of v) {
        if (typeof x === "string" && all.includes(x) && !out.includes(x)) out.push(x);
      }
    }
    for (const id of all) if (!out.includes(id)) out.push(id);
    return out;
  };
  const order = normOrder(r.topbarOrder, READER_TOPBAR_IDS);
  if (JSON.stringify(order) !== JSON.stringify(r.topbarOrder)) {
    r.topbarOrder = order;
    dirty = true;
  }
  const hidden = Array.isArray(r.topbarHidden)
    ? r.topbarHidden.filter((x: any) => typeof x === "string" && READER_TOPBAR_IDS.includes(x as ReaderTopbarId))
    : [];
  const hiddenUniq = [...new Set<string>(hidden)];
  if (JSON.stringify(hiddenUniq) !== JSON.stringify(r.topbarHidden)) {
    r.topbarHidden = hiddenUniq;
    dirty = true;
  }
  // 漫画模式设置：校验各字段；旧字段 mangaImageScale 迁移到 manga.imgScale 后删除。
  const clampF = (v: any, min: number, max: number, def: number) => {
    const n = typeof v === "number" ? v : Number(v);
    return Number.isFinite(n) ? Math.max(min, Math.min(max, Math.round(n * 20) / 20)) : def;
  };
  const clampI = (v: any, min: number, max: number, def: number) => {
    const n = typeof v === "number" ? v : Number(v);
    return Number.isFinite(n) ? Math.max(min, Math.min(max, Math.round(n))) : def;
  };
  if (!r.manga || typeof r.manga !== "object") {
    r.manga = structuredClone(DEFAULTS.reader.manga);
    dirty = true;
  }
  const legacyMs = typeof r.mangaImageScale === "number" ? r.mangaImageScale : Number(r.mangaImageScale);
  if (Number.isFinite(legacyMs)) {
    if (r.manga.imgScale == null) r.manga.imgScale = legacyMs;
    dirty = true;
  }
  if ("mangaImageScale" in r) {
    delete r.mangaImageScale;
    dirty = true;
  }
  const mi = clampF(r.manga.imgScale, 0.3, 3, DEFAULTS.reader.manga.imgScale);
  if (r.manga.imgScale !== mi) { r.manga.imgScale = mi; dirty = true; }
  const mfw = r.manga.fitWidth === true;
  if (r.manga.fitWidth !== mfw) { r.manga.fitWidth = mfw; dirty = true; }
  const mmh = clampI(r.manga.marginH, 0, 400, DEFAULTS.reader.manga.marginH);
  if (r.manga.marginH !== mmh) { r.manga.marginH = mmh; dirty = true; }
  const mmv = clampI(r.manga.marginV, 0, 400, DEFAULTS.reader.manga.marginV);
  if (r.manga.marginV !== mmv) { r.manga.marginV = mmv; dirty = true; }
  const mg = clampI(r.manga.imgGap, 0, 80, DEFAULTS.reader.manga.imgGap);
  if (r.manga.imgGap !== mg) { r.manga.imgGap = mg; dirty = true; }
  // 迁移旧值：single → flow、double → horizontal。
  let mlayout = r.manga.layout;
  if (mlayout === "single") mlayout = "flow";
  else if (mlayout === "double") mlayout = "horizontal";
  else if (mlayout !== "flow" && mlayout !== "horizontal") mlayout = DEFAULTS.reader.manga.layout;
  if (r.manga.layout !== mlayout) { r.manga.layout = mlayout; dirty = true; }
  const mdir = r.manga.direction === "rtl" ? "rtl" : "ltr";
  if (r.manga.direction !== mdir) { r.manga.direction = mdir; dirty = true; }
  const mhide = r.manga.hideText === true;
  if (r.manga.hideText !== mhide) { r.manga.hideText = mhide; dirty = true; }
  const mbg = typeof r.manga.bgColor === "string" ? r.manga.bgColor : "";
  if (r.manga.bgColor !== mbg) { r.manga.bgColor = mbg; dirty = true; }
  const mgd = r.mangaGuideDone === true;
  if (r.mangaGuideDone !== mgd) { r.mangaGuideDone = mgd; dirty = true; }
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

/** 归一化方案库：补齐 id/name，快照用默认配置兜底；空库时注入内置方案。 */
function normalizeSchemes(list: any): ReaderScheme[] {
  const out: ReaderScheme[] = [];
  const seen = new Set<string>();
  if (Array.isArray(list)) {
    for (const s of list) {
      if (!s || typeof s !== "object") continue;
      let id = typeof s.id === "string" && s.id ? s.id : `s-${Math.random().toString(36).slice(2, 9)}`;
      if (seen.has(id)) id = `s-${Math.random().toString(36).slice(2, 9)}`;
      seen.add(id);
      const name = typeof s.name === "string" && s.name.trim() ? s.name.trim().slice(0, 40) : "未命名方案";
      const cfgSrc = s.cfg && typeof s.cfg === "object" ? s.cfg : {};
      const cfg: ReaderConfig = {
        ...structuredClone(DEFAULTS.reader),
        ...cfgSrc,
        bindings: { ...structuredClone(DEFAULT_BINDINGS), ...(cfgSrc.bindings || {}) },
        zones: { ...structuredClone(DEFAULT_ZONES), ...(cfgSrc.zones || {}) },
      };
      normalizeReaderConfig(cfg, cfgSrc);
      out.push({ id, name, builtin: s.builtin === true, rev: typeof s.rev === "number" ? s.rev : undefined, cfg });
    }
  }
  if (!out.length) return builtinSchemes();
  return out;
}

function load(): AppConfig {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) {
      // 全新安装：注入内置阅读配色方案，并落盘一次。
      const fresh = structuredClone(DEFAULTS);
      fresh.readerSchemes = builtinSchemes();
      readerMigrationDirty = true;
      return fresh;
    }
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
        highlightColors:
          Array.isArray(saved.reader?.highlightColors) && saved.reader.highlightColors.length
            ? [...saved.reader.highlightColors]
            : structuredClone(HIGHLIGHT_COLORS),
      },
      library: { ...DEFAULTS.library, ...(saved.library || {}) },
      webdav: { ...DEFAULTS.webdav, ...saved.webdav },
      ai: migrateAi(saved.ai),
      tts: { ...DEFAULTS.tts, ...(saved.tts || {}) },
      regex: saved.regex ?? "",
      uiTheme: saved.uiTheme === "light" || saved.uiTheme === "dark" ? saved.uiTheme : "system",
      uiSkinLight: resolveSkinId(saved.uiSkinLight, "light"),
      uiSkinDark: resolveSkinId(saved.uiSkinDark, "dark"),
      readerSchemes: normalizeSchemes(saved.readerSchemes),
    };
    // 迁移：旧默认字体（微软雅黑）替换为书宋衬线字体，以贴近静读天下效果
    if (merged.reader.fontFamily === '"Microsoft YaHei", "Segoe UI", serif') {
      merged.reader.fontFamily = DEFAULTS.reader.fontFamily;
    }
    // 迁移：把存量安装的内置阅读方案刷新到当前预设（新增「现代 / 宽屏舒适」等）。
    const schemeUpgrade = upgradeBuiltinSchemes(merged.readerSchemes);
    merged.readerSchemes = schemeUpgrade.list;
    if (schemeUpgrade.dirty) readerMigrationDirty = true;
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
    // 书库顶栏顺序归一化：
    //   1) 旧配置里的「search」原地迁移为「notes」（顶栏搜索按钮已改为笔记入口），保持它原来的位置；
    //   2) 过滤非法 id、去重；3) 把新增按钮（含 notes）补到末尾，保证旧配置升级后按钮一定出现。
    // 不做这步的话，存量用户的 topbarOrder 仍含已删除的 search、且缺 notes，导致笔记入口整个消失。
    {
      const libAll = LIBRARY_TOPBAR_IDS as readonly string[];
      const src = Array.isArray(merged.library.topbarOrder) ? merged.library.topbarOrder : [];
      const out: string[] = [];
      for (const raw of src) {
        const id = raw === "search" ? "notes" : raw; // 原地迁移 search → notes
        if (typeof id === "string" && libAll.includes(id) && !out.includes(id)) out.push(id);
      }
      for (const id of libAll) if (!out.includes(id)) out.push(id); // 补齐缺失（新增按钮到末尾）
      if (JSON.stringify(out) !== JSON.stringify(merged.library.topbarOrder)) {
        merged.library.topbarOrder = out;
        readerMigrationDirty = true; // 复用同一落盘标记，确保迁移结果持久化
      }
    }
    // 方案库首次为空时会注入内置方案：需要落盘一次，否则刷新前 localStorage 仍为空。
    if (!Array.isArray(saved.readerSchemes) || !saved.readerSchemes.length) readerMigrationDirty = true;
    return merged;
  } catch {
    return structuredClone(DEFAULTS);
  }
}

export const config = $state<AppConfig>(load());

/** 解析当前生效的界面主题（system 时跟随操作系统）。 */
export function resolvedUiTheme(): "light" | "dark" {
  const t = config.uiTheme;
  if (t === "light" || t === "dark") return t;
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch {
    return "light";
  }
}

/** 取当前生效的外壳皮肤。 */
export function activeUiSkin(): UiSkin {
  const mode = resolvedUiTheme();
  const id = resolveSkinId(mode === "dark" ? config.uiSkinDark : config.uiSkinLight, mode);
  return UI_SKINS.find((s) => s.id === id) ?? UI_SKINS.find((s) => s.mode === mode) ?? UI_SKINS[0];
}

/** 把外壳皮肤写进 <html> 的 CSS 变量，供全局样式生效。 */
export function applyUiTheme(): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const mode = resolvedUiTheme();
  const s = activeUiSkin();
  root.dataset.theme = mode;
  root.style.setProperty("--bg", s.bg);
  root.style.setProperty("--bg-elev", s.panel);
  root.style.setProperty("--panel", s.panel);
  root.style.setProperty("--panel2", s.panel2);
  root.style.setProperty("--fg", s.fg);
  root.style.setProperty("--fg2", s.fg2);
  root.style.setProperty("--line", s.line);
  root.style.setProperty("--line-strong", s.line);
  root.style.setProperty("--accent", s.accent);
  root.style.setProperty("--accent-fg", s.accentFg);
  root.style.setProperty("--success", s.success);
  root.style.setProperty("--warning", s.warning);
  root.style.setProperty("--danger", s.danger);
  root.style.setProperty("--radius-card", s.radiusCard);
  root.style.setProperty("--shadow-card", s.shadow);
  // 阴影按皮肤前景色「着色」而非纯黑：更融于背景、不显脏（深色皮肤用更低透明度）。
  const tint = s.fg;
  const a = (v: number) => (mode === "dark" ? v * 1.0 : v * 0.55);
  root.style.setProperty("--shadow-sm", `0 1px 2px color-mix(in srgb, ${tint} ${Math.round(a(10))}%, transparent)`);
  root.style.setProperty("--shadow-md", `0 6px 20px color-mix(in srgb, ${tint} ${Math.round(a(16))}%, transparent)`);
  root.style.setProperty("--shadow-lg", `0 16px 48px color-mix(in srgb, ${tint} ${Math.round(a(28))}%, transparent)`);
}

// 界面主题响应式生效：uiTheme / 皮肤变化即更新；system 时监听系统切换。
if (typeof window !== "undefined") {
  $effect.root(() => {
    $effect(() => {
      void config.uiTheme;
      void config.uiSkinLight;
      void config.uiSkinDark;
      applyUiTheme();
    });
  });
  try {
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      if (config.uiTheme === "system") applyUiTheme();
    });
  } catch { /* ignore */ }
  applyUiTheme();
}

/** 返回一份 reader 默认配置（深拷贝，供设置页回退字体/恢复默认使用）。 */
export function defaultReaderConfig(): ReaderConfig {
  return structuredClone(DEFAULTS.reader);
}

/** 仅重置「阅读」设置段（不动 AI / WebDAV / 分章正则）。 */
export function resetReaderConfig(): void {
  config.reader = structuredClone(DEFAULTS.reader);
  saveConfig();
}

/* ===================== 阅读配色方案（存档点） ===================== */

/** 深拷贝阅读配置：config.reader 是 Svelte $state 代理，structuredClone 会抛 DataCloneError，
 *  因此走 JSON 序列化（纯数据、无函数/循环引用）。 */
function snapshotReader(): ReaderConfig {
  return JSON.parse(JSON.stringify(config.reader)) as ReaderConfig;
}

/** 以当前阅读配置为快照，新建一个方案；返回新方案 id。 */
export function createReaderScheme(name: string): string {
  const id = `s-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  config.readerSchemes = [
    ...config.readerSchemes,
    { id, name: name.trim().slice(0, 40) || "未命名方案", cfg: snapshotReader() },
  ];
  saveConfig();
  return id;
}

/** 用当前阅读配置覆盖已有方案（存档）。 */
export function overwriteReaderScheme(id: string): void {
  config.readerSchemes = config.readerSchemes.map((s) =>
    s.id === id ? { ...s, cfg: snapshotReader() } : s,
  );
  saveConfig();
}

/** 重命名方案。 */
export function renameReaderScheme(id: string, name: string): void {
  const n = name.trim().slice(0, 40);
  if (!n) return;
  config.readerSchemes = config.readerSchemes.map((s) => (s.id === id ? { ...s, name: n } : s));
  saveConfig();
}

/** 删除方案（内置的也可删，删除后不找回）。 */
export function deleteReaderScheme(id: string): void {
  config.readerSchemes = config.readerSchemes.filter((s) => s.id !== id);
  saveConfig();
}

/** 把方案应用为全局阅读设置（写进 config.reader）。 */
export function applyReaderSchemeGlobal(id: string): void {
  const s = config.readerSchemes.find((x) => x.id === id);
  if (!s) return;
  config.reader = JSON.parse(JSON.stringify(s.cfg)) as ReaderConfig;
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
      highlightColors:
        Array.isArray(obj.reader.highlightColors) && obj.reader.highlightColors.length
          ? [...obj.reader.highlightColors]
          : structuredClone(HIGHLIGHT_COLORS),
    };
    // 跨窗同步/导入同样清理 reader 废弃键并迁移旧默认值
    normalizeReaderConfig(obj.reader, rawReader);
    obj.library = { ...structuredClone(DEFAULTS.library), ...(obj.library || {}) };
    obj.ai = { ...structuredClone(DEFAULTS.ai), ...obj.ai };
    obj.ai.profiles = normalizeProfiles(obj.ai.profiles);
    obj.ai.style = { ...structuredClone(DEFAULTS.ai.style), ...(obj.ai.style || {}) };
    if (obj.ai.failover !== false) obj.ai.failover = true;
    obj.ai.active = Math.min(Math.max((obj.ai.active as number) | 0, 0), obj.ai.profiles.length - 1);
    stripReasoningArtifacts(obj.ai);
    if (obj.ai.display !== "reader") obj.ai.display = "chat";
    obj.tts = { ...structuredClone(DEFAULTS.tts), ...(obj.tts || {}) };
    obj.uiTheme = obj.uiTheme === "light" || obj.uiTheme === "dark" ? obj.uiTheme : "system";
    obj.uiSkinLight = resolveSkinId(obj.uiSkinLight, "light");
    obj.uiSkinDark = resolveSkinId(obj.uiSkinDark, "dark");
    obj.readerSchemes = normalizeSchemes(obj.readerSchemes);
    for (const k of Object.keys(config) as (keyof AppConfig)[]) {
      (config as any)[k] = obj[k];
    }
    applyUiTheme();
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
  /** 保留字段以兼容旧签名；所有提示现在都会自动消失。 */
  sticky: boolean;
}

export const toasts = $state<{ items: Toast[] }>({ items: [] });
let toastId = 0;

/** 提示展示时长：普通提示 5.2s，错误提示稍长（8s），但一律自动消失，不再常驻。 */
const TOAST_TTL_MS = 5200;
const TOAST_TTL_ERROR_MS = 8000;

export function toast(msg: string, kind: "info" | "error" = "info", _sticky?: boolean) {
  const id = ++toastId;
  // 忽略调用方传入的 sticky：所有提示统一自动消失，避免红色错误提示无法关闭地常驻。
  const t: Toast = { id, msg, kind, sticky: false };
  toasts.items.push(t);
  setTimeout(() => dismissToast(id), kind === "error" ? TOAST_TTL_ERROR_MS : TOAST_TTL_MS);
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

// 版本号单一数据源：构建期由 vite.config.js 从 package.json 注入 __APP_VERSION__
// （与 src-tauri/tauri.conf.json 的 version 保持一致），避免运行时读版本所需的权限问题。
export const APP_VERSION = `v${typeof __APP_VERSION__ === "string" && __APP_VERSION__ ? __APP_VERSION__ : "0.0.0"}`;

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
  /** 笔记界面（按书分组显示带感想的划线）。 */
  notesOpen: false,
  /** 阅读界面当前打开的书 id（0=未打开）；供设置页把阅读方案"仅应用到本书"。 */
  currentBookId: 0,
  /** 封面刷新信号：后端 `cover-updated` 事件在 App 根组件（+page）监听后递增此值，
   *  书库组件响应式重拉当前窗口，令扫描/打开插队扫出的新封面即时显示。 */
  coverTick: 0,
  /** 最近一次 cover-updated 的 book id（取自事件 payload.id）。 */
  lastCoverId: 0,
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

// ===== 多动作确认框（delete-dialog）：让用户在若干明确动作间选择，不用「确定/取消」藏语义。 =====
export interface ConfirmAction {
  id: string;
  label: string;
  /** primary=主按钮 / danger=危险动作 / plain=普通。 */
  kind?: "primary" | "danger" | "plain";
}
export const choiceBox = $state<{
  msg: string;
  actions: ConfirmAction[];
  active: boolean;
}>({ msg: "", actions: [], active: false });
let choiceResolve: ((id: string | null) => void) | null = null;

/** 弹出多动作确认框；返回被选动作 id，取消返回 null。 */
export function appChoose(msg: string, actions: ConfirmAction[]): Promise<string | null> {
  choiceBox.msg = msg;
  choiceBox.actions = actions;
  choiceBox.active = true;
  return new Promise((res) => {
    choiceResolve = res;
  });
}

export function choiceAnswer(id: string | null) {
  choiceBox.active = false;
  choiceResolve?.(id);
  choiceResolve = null;
}
