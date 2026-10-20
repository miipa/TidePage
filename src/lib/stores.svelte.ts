export type ReadMode = "page" | "scroll";
export type ThemeKey = "paper" | "white" | "green" | "dark" | "night";
export type ZoneKey = "tl" | "tr" | "bl" | "br" | "c";
export type ZoneAction = BindAction | "none";

export const ZONE_DEFS: { id: ZoneKey; name: string }[] = [
  { id: "tl", name: "左上区域" }, // i18n-allow:标识符：ZONE_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("zone", id)，中文不上屏。
  { id: "tr", name: "右上区域" }, // i18n-allow:标识符：ZONE_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("zone", id)，中文不上屏。
  { id: "bl", name: "左下区域" }, // i18n-allow:标识符：ZONE_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("zone", id)，中文不上屏。
  { id: "br", name: "右下区域" }, // i18n-allow:标识符：ZONE_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("zone", id)，中文不上屏。
  { id: "c", name: "中间小区域" }, // i18n-allow:标识符：ZONE_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("zone", id)，中文不上屏。
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
  { id: "next", name: "下一页 / 下滚一屏" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "prev", name: "上一页 / 上滚一屏" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "scrollDown", name: "下滚一段" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "scrollUp", name: "上滚一段" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "chapterNext", name: "下一章" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "chapterPrev", name: "上一章" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "toc", name: "打开/关闭目录" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "addBookmark", name: "添加书签" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "bookmarkList", name: "书签列表" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "ai", name: "AI 面板" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "bookStats", name: "本书统计" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "ttsPlayPause", name: "朗读：播放/暂停" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "ttsStop", name: "朗读：停止" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "ttsNext", name: "朗读：下一句" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "ttsPrev", name: "朗读：上一句" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "settings", name: "设置" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
  { id: "close", name: "返回书库" }, // i18n-allow:标识符：BIND_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("bind", id)，中文不上屏。
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

import type { ChapterTransition, PageTransition } from "./chapterTx";

export type TextAlign = "justify" | "left";
/** 滚动模式的章节过渡档位（定义在 lib/chapterTx.ts，与过渡几何/解锁判据同源）。 */
export type { ChapterTransition };
/** 翻页模式（单栏/双栏）的章节过渡档位。 */
export type { PageTransition };

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
  { id: "bookTitle", name: "书名" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "chapterName", name: "章节名" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "chapterIndex", name: "当前章号（第X章）" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "chapterCount", name: "总章数（共Y章）" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "chapterPages", name: "本节页数 / 百分比" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "bookPages", name: "全书总页数（估算）" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "pagePercent", name: "本节进度" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "bookProgress", name: "全书进度" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "chars", name: "本章字数" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "clock", name: "时间" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "speed", name: "阅读速度（字/分）" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
  { id: "mode", name: "阅读模式" }, // i18n-allow:标识符：STATUS_ITEM_DEFS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("statusItem", id)，中文不上屏。
];

export const STATUS_ITEM_IDS: Set<string> = new Set(STATUS_ITEM_DEFS.map((d) => d.id));

/** 阅读界面顶栏右侧可拖拽排序/可隐藏的按钮 id（默认顺序）。 */
export const READER_TOPBAR_IDS = [
  "webdav", "webdavDownload", "convert", "bookmarkAdd", "bookmarkList", "ttsPlay", "ttsStop",
  "highlight", "ai", "stats", "toc", "manga", "manager", "images", "css", "settings",
] as const;

/** 漫画显示方式：瀑布流（纵向连续） / 横向（横向连续阅读）。 */
export type MangaLayout = "flow" | "horizontal";
/** 漫画阅读方向：从左到右 / 从右到左（日漫）。 */
export type MangaDirection = "ltr" | "rtl";

/** 漫画模式与 PDF 共用的独立设置（与普通阅读排版分开）。 */
export interface MangaConfig {
  /** 默认图片大小倍率（阅读界面右下角可再按单本覆盖） */
  imgScale: number;
  /** 默认图片适应阅读器宽度 */
  fitWidth: boolean;
  /** 默认图片适应阅读器高度（与 fitWidth 互斥） */
  fitHeight: boolean;
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
  /** 自动识别漫画：关掉后不再把「图片多」的书判为漫画（仍可顶栏手动切换） */
  autoDetect: boolean;
}
export type ReaderTopbarId = (typeof READER_TOPBAR_IDS)[number];

/** 书库顶栏可拖拽排序的图标按钮 id（默认顺序）。 */
export const LIBRARY_TOPBAR_IDS = ["webdav", "random", "search", "notes", "stats", "settings"] as const;
export type LibraryTopbarId = (typeof LIBRARY_TOPBAR_IDS)[number];

/** 阅读区边距作用域：scroll=上下滚动，page=左右翻页单栏，duo=左右翻页双栏。 */
export type MarginScope = "scroll" | "page" | "duo";
export interface MarginSides { h: number; v: number }

export interface ReaderConfig {
  mode: ReadMode;
  duo: boolean;
  colGap: number;
  fontFamily: string;
  latinFont: string;
  fontSize: number;
  /** 正文字重：#chapter-content p（300 细 / 400 常规 / 500 中） */
  fontWeight: number;
  lineHeight: number;
  paraSpacing: number;
  /** 阅读区边距：滚动/单栏/双栏各自独立保存，切换模式即时生效 */
  margins: Record<MarginScope, MarginSides>;
  indent: string;
  textAlign: TextAlign;
  theme: ThemeKey;
  bgColor: string;
  fgColor: string;
  bgImage: string;
  bgOpacity: number;
  cssExtra: string;
  overrideBookCss: boolean;
  wheelTurnPage: boolean;
  bindings: Record<BindAction, string[]>;
  zones: Record<ZoneKey, ZoneAction>;
  zoneHints: boolean;
  /** 沉浸阅读：无操作一段时间后自动淡出顶栏/底栏，移动鼠标或按键恢复。 */
  immersive: boolean;
  customFonts: { family: string; file: string }[];
  aiPanelWidth: number;
  /** 滚动模式的章节过渡档位（cover/spring/none）。 */
  chapterTransition: ChapterTransition;
  /** 翻页模式（单栏/双栏）的章节过渡档位（h-spring/h-slide/none）。 */
  pageTransition: PageTransition;
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
  /** 漫画模式与 PDF 共用的独立设置（见 MangaConfig） */
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

/** 阅读区当前生效的边距作用域：滚动模式用 scroll，否则按栏数取 duo/page。
 *  这是该规则的**唯一来源**（Reader / AiPanel / 设置页都调它）——
 *  抄成多份时，改动一处就会让三处的边距认到不同作用域。 */
export function marginScopeOf(r: ReaderConfig): MarginScope {
  return r.mode === "scroll" ? "scroll" : r.duo ? "duo" : "page";
}

export const HIGHLIGHT_STYLES: { id: string; name: string }[] = [
  { id: "underline", name: "下划线" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
  { id: "wavy", name: "波浪线" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
  { id: "dotted", name: "点线" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
  { id: "double", name: "双下划线" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
  { id: "background", name: "荧光笔" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
  { id: "strike", name: "删除线" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
  { id: "box", name: "方框" }, // i18n-allow:标识符：HIGHLIGHT_STYLES 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("hlStyle", id)，中文不上屏。
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

export interface WebdavBackupConfig {
  /** 是否启用改动备份 */
  enabled: boolean;
  /** 检测周期（小时）：6/12/24/48 */
  intervalHours: number;
  /** 上次备份时间（毫秒时间戳；0=从未） */
  lastAt: number;
  /** 上次备份时的配置签名（配置/凭据有实质变化才改变） */
  lastSig?: string;
  /** 上次检测时间（毫秒时间戳） */
  lastCheckAt?: number;
  /** 改动备份最近一次失败原因（可读，不弹硬错误） */
  lastError?: string;
}

export interface WebdavConfig {
  enabled: boolean;
  base: string;
  user: string;
  pass: string;
  syncOnOpen: boolean;
  uploadBooks: boolean;
  downloadDir: string;
  /** 设置备份（手动 + 定期） */
  backup: WebdavBackupConfig;
}

export type AiProvider = "openai" | "ollama" | "anthropic" | "gemini" | "azure-openai" | "dashscope";

/** 全局网络代理：所有 AI 端点与在线语音合成共用一份（不再按端点各存一份）。 */
export interface AiProxyConfig {
  /** direct=强制直连；system=环境变量/Windows 系统代理；manual=手动填地址 */
  mode: "direct" | "system" | "manual";
  /** manual 模式下裸 host:port 的 scheme */
  type: "http" | "https" | "socks5";
  /** 手动代理地址（敏感字段：仅加密落盘，localStorage 恒为空串） */
  url: string;
}

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
  /** 端点网络代理模式：inherit=沿用全局默认；其余同全局（direct/system/manual）。缺省 inherit。 */
  proxyMode: "inherit" | "direct" | "system" | "manual";
  /** manual 模式下裸 host:port 的 scheme（仅 manual 有意义）。 */
  proxyType: "http" | "https" | "socks5";
  /** 手动代理地址（敏感字段：仅加密落盘，localStorage 恒为空串；仅 manual 有意义）。 */
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
  { id: "openai", label: "OpenAI 兼容 API", baseUrlHint: "https://api.deepseek.com", modelHint: "deepseek-chat", needsKey: true }, // i18n-allow:标识符：AI_PROVIDERS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("aiProvider", id)，中文不上屏。
  { id: "ollama", label: "Ollama 本地", baseUrlHint: "http://localhost:11434", modelHint: "llama3", needsKey: false }, // i18n-allow:标识符：AI_PROVIDERS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("aiProvider", id)，中文不上屏。
  { id: "anthropic", label: "Anthropic（Claude）", baseUrlHint: "https://api.anthropic.com", modelHint: "claude-3-5-sonnet-latest", needsKey: true }, // i18n-allow:标识符：AI_PROVIDERS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("aiProvider", id)，中文不上屏。
  { id: "gemini", label: "Google Gemini", baseUrlHint: "https://generativelanguage.googleapis.com", modelHint: "gemini-1.5-flash", needsKey: true },
  { id: "azure-openai", label: "Azure OpenAI", baseUrlHint: "https://<资源名>.openai.azure.com", modelHint: "gpt-4o", needsKey: true, azure: true }, // i18n-allow:端点占位符：Azure 服务地址模板里的 <资源名> 是地址的一段，地址属标识符不翻译。
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
  /** AI 设置页三个分类的显示顺序（可在设置里拖拽排序）。id 取 AI_GROUPS。 */
  groupOrder: string[];
  /** 全局网络代理（一份，作用于所有端点） */
  proxy: AiProxyConfig;
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
    proxyMode: "inherit",
    proxyType: "http",
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
  // dashscope（通义千问）只出现在配置文件里（下拉列表不提供该选项），但后端 Provider::parse
  // 认这个 id 且走 OpenAI 兼容协议；这里必须放行，否则读一次配置就被改写成 openai（数据丢失）。
  if (s === "dashscope" || s === "qwen") return "dashscope";
  if (s === "azure-openai" || s === "azure" || s === "azure_openai") return "azure-openai";
  return "openai";
}

/** 端点代理模式归一化：仅认 inherit/direct/system/manual；旧值（custom/auto）与非法值回落 inherit。 */
function sanitizeEndpointProxyMode(v: any): "inherit" | "direct" | "system" | "manual" {
  return v === "direct" || v === "system" || v === "manual" ? v : "inherit";
}

/** 端点代理 scheme 归一化：仅认 http/https/socks5，非法回落 http。 */
function sanitizeProxyType(v: any): "http" | "https" | "socks5" {
  return v === "https" || v === "socks5" ? v : "http";
}

/** 用默认字段模板补齐单个 profile（新增字段缺失时用默认值，不丢用户已填内容）。 */
export function normalizeProfile(p: any): AiProfile {
  const t = structuredClone(DEFAULT_AI_PROFILES[0]);
  const src = p && typeof p === "object" ? p : {};
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
    // 端点代理：旧版「每端点代理」的 proxyMode/proxyUrl 字段已在 migrate 的 absorbLegacyProfileProxy
    // 阶段被物理删除（旧 schema 无 proxyType），到这里只会是新 schema 或缺省，故可直接归一化。
    proxyMode: sanitizeEndpointProxyMode(src.proxyMode),
    proxyType: sanitizeProxyType(src.proxyType),
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
  if (!names.has("新端点")) return "新端点"; // i18n-allow:持久化值：落盘值恒为中文 —— uniqueEndpointName() 的去重判断（names.has / 新端点{n} 递增）依赖它，存量配置里也已是中文；显示侧经 seedEndpointLabel() 查 dict.seedEndpoint.* ，不写回 config。
  for (let n = 2; n < 100000; n++) {
    const cand = `新端点${n}`; // i18n-allow:持久化值：落盘值恒为中文 —— uniqueEndpointName() 的去重判断（names.has / 新端点{n} 递增）依赖它，存量配置里也已是中文；显示侧经 seedEndpointLabel() 查 dict.seedEndpoint.numbered ，不写回 config。
    if (!names.has(cand)) return cand;
  }
  return `新端点${Date.now()}`; // i18n-allow:持久化值：10 万个序号都用尽后的兜底，运行期唯一值，故不进静态表；显示侧由 seedEndpointLabel() 的「新端点+数字」正则按 dict.seedEndpoint.numbered 拼，不写回 config。
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
    proxyMode: "inherit",
    proxyType: "http",
    proxyUrl: "",
  };
}

export const DEFAULT_PROMPTS: PromptItem[] = [
  {
    id: 1,
    name: "剧情总结", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
    key: "F1",
    text: "请用简洁的中文总结以下小说章节的主要剧情（120字以内），并猜猜接下来的发展：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
  },
  {
    id: 2,
    name: "翻译为中文", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
    key: "F2",
    text: "请将以下内容翻译成流畅的中文，直接输出译文：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
  },
  {
    id: 3,
    name: "精校润色", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
    key: "F3",
    text: "请校对并润色以下文本：删除广告与乱码字符，修正错别字与标点，保持原意与结构不要扩写或缩写，直接输出处理后的正文，不要任何解释：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
  },
  {
    id: 4,
    name: "人物情节梳理", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
    key: "F4",
    text: "请列出本章出场人物（含身份/立场）和本章主要情节点（条目式，简洁）：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
  },
];

export type TtsEngine = "system" | "online";

/** 一条「语音服务配置档案」：显式保存接口协议与地址/密钥/音色等，可随时切换。 */
export interface TtsService {
  /** 稳定唯一 id（用于渲染 key 与端点识别；旧配置迁移时补齐） */
  id: number;
  name: string;
  /** 接口协议：openai/voicestudio/azure/dashscope/minimax/fish/gptsovits/breeze/volcano/baidu/edge */
  provider: string;
  /** 服务地址（edge 固定接口可留空；火山/百度亦为固定接口） */
  baseUrl: string;
  /** 密钥 / API Key（本地服务可留空） */
  apiKey: string;
  /** 模型（如 tts-1 / omnivoice；留空由后端取默认） */
  model: string;
  /** 音色 / 参考音频路径（留空=服务端默认） */
  voice: string;
  /** 音频格式（wav/mp3/opus/aac/flac） */
  format: string;
  /** 百度语音专用：Secret Key */
  secretKey: string;
  /** 火山引擎专用：AppID */
  appId: string;
  /** 火山引擎专用：Cluster（可留空，默认 volcano_tts） */
  cluster: string;
  /** 端点网络代理模式：inherit=沿用全局默认；其余同全局（direct/system/manual）。缺省 inherit。 */
  proxyMode: "inherit" | "direct" | "system" | "manual";
  /** manual 模式下裸 host:port 的 scheme（仅 manual 有意义）。 */
  proxyType: "http" | "https" | "socks5";
  /** 手动代理地址（敏感字段：仅加密落盘，localStorage 恒为空串；仅 manual 有意义）。 */
  proxyUrl: string;
}

/** 全部 TTS 接口协议 id（前端下拉与迁移校验共用）。 */
export const TTS_PROTOCOLS = [
  "openai",
  "voicestudio",
  "azure",
  "dashscope",
  "minimax",
  "fish",
  "gptsovits",
  "breeze",
  "volcano",
  "baidu",
  "edge",
] as const;

export interface TtsConfig {
  /** 朗读引擎：system=系统离线语音；online=在线神经语音 */
  engine: TtsEngine;
  /** 在线语音服务配置档案列表（至少一条） */
  services: TtsService[];
  /** 当前使用的服务档案下标 */
  activeService: number;
  /** 指定语音（voice.name 或 voiceURI），空=按语言自动选择（系统语音用） */
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

let ttsServiceIdSeq = 1;

/** 补齐缺失/重复的服务 id，保证唯一稳定。 */
export function ensureTtsServiceIds(list: TtsService[]): void {
  let max = 0;
  for (const s of list) {
    if (typeof s.id === "number" && Number.isFinite(s.id) && s.id > max) max = s.id;
  }
  const used = new Set<number>();
  for (const s of list) {
    let id = s.id;
    if (!Number.isFinite(id) || id <= 0 || used.has(id)) id = ++max;
    used.add(id);
    s.id = id;
  }
  if (max > ttsServiceIdSeq) ttsServiceIdSeq = max;
}

function nextTtsServiceId(): number {
  return ++ttsServiceIdSeq;
}

/** 归一化 TTS 接口协议：未知/缺失一律回退 OpenAI 兼容。 */
export function sanitizeTtsProvider(v: any): string {
  const s = typeof v === "string" ? v.toLowerCase().trim() : "";
  return (TTS_PROTOCOLS as readonly string[]).includes(s) ? s : "openai";
}

/** 用默认字段模板补齐单条服务（新增字段缺失时用默认值，不丢用户已填内容）。 */
export function normalizeTtsService(s: any): TtsService {
  const src = s && typeof s === "object" ? s : {};
  return {
    id: typeof src.id === "number" && Number.isFinite(src.id) && src.id > 0 ? src.id : 0,
    name: typeof src.name === "string" ? src.name : "",
    provider: sanitizeTtsProvider(src.provider),
    baseUrl: typeof src.baseUrl === "string" ? src.baseUrl : "",
    apiKey: typeof src.apiKey === "string" ? src.apiKey : "",
    model: typeof src.model === "string" ? src.model : "",
    voice: typeof src.voice === "string" ? src.voice : "",
    format: typeof src.format === "string" && src.format ? src.format : "wav",
    secretKey: typeof src.secretKey === "string" ? src.secretKey : "",
    appId: typeof src.appId === "string" ? src.appId : "",
    cluster: typeof src.cluster === "string" ? src.cluster : "",
    proxyMode: sanitizeEndpointProxyMode(src.proxyMode),
    proxyType: sanitizeProxyType(src.proxyType),
    proxyUrl: typeof src.proxyUrl === "string" ? src.proxyUrl : "",
  };
}

/** 生成不与现有服务重名的名称：优先「新服务」，否则「新服务2/3…」。 */
export function uniqueTtsServiceName(list: TtsService[]): string {
  const names = new Set(list.map((s) => (s.name || "").trim()));
  if (!names.has("新服务")) return "新服务"; // i18n-allow:持久化值：落盘值恒为中文 —— uniqueTtsServiceName() 的去重判断（names.has / 新服务{n} 递增）依赖它，存量配置里也已是中文；显示侧经 seedTtsLabel() 查 dict.seedTts.* ，不写回 config。
  for (let n = 2; n < 100000; n++) {
    const cand = `新服务${n}`; // i18n-allow:持久化值：落盘值恒为中文 —— uniqueTtsServiceName() 的去重判断依赖它；显示侧经 seedTtsLabel() 查 dict.seedTts.numbered ，不写回 config。
    if (!names.has(cand)) return cand;
  }
  return `新服务${Date.now()}`; // i18n-allow:持久化值：10 万个序号都用尽后的兜底，运行期唯一值，故不进静态表；显示侧由 seedTtsLabel() 的「新服务+数字」正则按 dict.seedTts.numbered 拼，不写回 config。
}

/** 干净的新服务档案：不复制上一条的地址与凭证。 */
export function newTtsService(list: TtsService[]): TtsService {
  return {
    id: nextTtsServiceId(),
    name: uniqueTtsServiceName(list),
    provider: "openai",
    baseUrl: "",
    apiKey: "",
    model: "",
    voice: "",
    format: "wav",
    secretKey: "",
    appId: "",
    cluster: "",
    proxyMode: "inherit",
    proxyType: "http",
    proxyUrl: "",
  };
}

/** 当前生效的语音服务档案（越界时夹取到有效范围）。 */
export function activeTtsService(): TtsService | null {
  const list = config.tts.services;
  if (!list || !list.length) return null;
  return list[Math.min(Math.max(config.tts.activeService, 0), list.length - 1)] ?? list[0];
}

export interface LibraryConfig {
  /** 书库顶栏图标按钮顺序（拖动排序后持久化） */
  topbarOrder: string[];
  /** 是否在封面上显示阅读进度环角标（默认开启，可在「首选项」关闭） */
  showCoverProgress: boolean;
  /** 封面扫描：auto = 后台为全部书籍生成封面；manual = 打开书籍时生成该本封面。 */
  coverScan: "auto" | "manual";
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
  /** 卡片圆角（部分皮肤走硬朗/柔和两种气质）。运行时映射到 --radius-lg。 */
  radiusCard: string;
}

/**
 * 外壳可选皮肤：5 浅 + 5 深（共 10 套）。仅影响外壳（书库/设置/阅读器工具栏/面板/弹窗），
 * 与阅读正文配色（THEMES / reader.*）完全解耦——切换皮肤不改动任何阅读界面颜色设置。
 *
 * 选色约束（依据用户需求）：
 *  1) 易读：正文 fg 与 bg 对比 ≥ 12:1；次要 fg2 ≥ 4.5:1；accent 作彩色文字/主按钮均过 AA。
 *  2) 区分度：十套主色分布在色谱不同区段——
 *     浅色 = 中性咖 / 赤陶橙 / 松林绿 / 湖光蓝 / 紫藤紫；
 *     深色 = 石墨青 / 琥珀金 / 深海蓝 / 绯夜红 / 夜行紫。
 *  3) 全窗口：所有令牌经 applyUiTheme() 注入 :root，books/reader/面板/弹窗共享同一套变量。
 */
export const UI_SKINS: UiSkin[] = [
  // ===================== 浅色 5 套 =====================
  // 奶油（保留 · 微调）：暖奶油纸底 + 中性深咖主色，作为「中性暖」一极，与其余彩色皮肤区分。
  {
    id: "cream",
    name: "奶油", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "light",
    bg: "#FFF1E2", panel: "#FFFFFF", panel2: "#FBEADA",
    fg: "#292320", fg2: "#7A6E64", line: "rgba(41,35,32,0.10)",
    accent: "#3A2A20", accentFg: "#FFF1E2",
    success: "#2E7D5B", warning: "#B5791C", danger: "#B23A3A",
    radiusCard: "16px"
  },
  // 赤陶：暖陶土橙（~20°），温润手作气质。
  {
    id: "clay",
    name: "赤陶", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "light",
    bg: "#FBF3EC", panel: "#FFFFFF", panel2: "#F3E6DB",
    fg: "#2C2622", fg2: "#756759", line: "rgba(44,38,34,0.10)",
    accent: "#A9482A", accentFg: "#FFF7F2",
    success: "#2E7D5B", warning: "#B5791C", danger: "#B23A3A",
    radiusCard: "12px"
  },
  // 松林：沉静森绿（~155°），清新自然。
  {
    id: "pine",
    name: "松林", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "light",
    bg: "#EFF5F0", panel: "#FFFFFF", panel2: "#E2EDE5",
    fg: "#1F2622", fg2: "#63726A", line: "rgba(31,38,34,0.10)",
    accent: "#1E7A55", accentFg: "#F3FBF6",
    success: "#2E7D5B", warning: "#B5791C", danger: "#B23A3A",
    radiusCard: "12px"
  },
  // 湖光：清爽湖蓝（~205°），冷静专注。
  {
    id: "lagoon",
    name: "湖光", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "light",
    bg: "#EDF3F9", panel: "#FFFFFF", panel2: "#E0E9F3",
    fg: "#1E252E", fg2: "#5E6975", line: "rgba(30,37,46,0.09)",
    accent: "#2E6CB0", accentFg: "#F2F7FC",
    success: "#2E7D5B", warning: "#B5791C", danger: "#B23A3A",
    radiusCard: "12px"
  },
  // 紫藤：柔雅紫红（~315°），优雅有格调。
  {
    id: "lilac",
    name: "紫藤", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "light",
    bg: "#F7F1F6", panel: "#FFFFFF", panel2: "#EEE2EC",
    fg: "#2A2430", fg2: "#726A78", line: "rgba(42,36,48,0.10)",
    accent: "#9C3A80", accentFg: "#FBF3F8",
    success: "#2E7D5B", warning: "#B5791C", danger: "#B23A3A",
    radiusCard: "12px"
  },
  // ===================== 深色 5 套 =====================
  // 石墨（保留）：中性冷灰底 + 薄荷青主色（~168°）。
  {
    id: "graphite",
    name: "石墨", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "dark",
    bg: "#16181D", panel: "#1E2126", panel2: "#252932",
    fg: "#E6E8EC", fg2: "#9AA0A8", line: "rgba(255,255,255,0.10)",
    accent: "#4FAE95", accentFg: "#0F1115",
    success: "#4FB985", warning: "#D6A73C", danger: "#E5695F",
    radiusCard: "12px"
  },
  // 琥珀：暖褐底 + 琥珀金主色（~42°），温暖夜读。
  {
    id: "amber",
    name: "琥珀", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "dark",
    bg: "#1A1712", panel: "#221E17", panel2: "#2B261D",
    fg: "#ECE6DA", fg2: "#ADA491", line: "rgba(255,246,230,0.10)",
    accent: "#D6A845", accentFg: "#201A0C",
    success: "#4FB985", warning: "#D6A73C", danger: "#E5695F",
    radiusCard: "12px"
  },
  // 深海：冷蓝黑底 + 亮蓝主色（~215°），深邃沉浸。
  {
    id: "abyss",
    name: "深海", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "dark",
    bg: "#121620", panel: "#1A1F2B", panel2: "#222839",
    fg: "#E4E8F0", fg2: "#949DB0", line: "rgba(210,224,255,0.10)",
    accent: "#5B93E6", accentFg: "#0A1020",
    success: "#4FB985", warning: "#D6A73C", danger: "#E5695F",
    radiusCard: "12px"
  },
  // 绯夜：暖黑底 + 绯红主色（~353°），醒目而不刺眼。
  {
    id: "crimson",
    name: "绯夜", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "dark",
    bg: "#1C1418", panel: "#241A1F", panel2: "#2E2129",
    fg: "#EEE2E6", fg2: "#AE9BA2", line: "rgba(255,230,238,0.10)",
    accent: "#E77080", accentFg: "#2A0E12",
    success: "#4FB985", warning: "#D6A73C", danger: "#E5695F",
    radiusCard: "12px"
  },
  // 夜行（保留 id · 主色提亮）：紫黑底 + 紫水晶主色（~272°）。
  {
    id: "nocturne",
    name: "夜行", // i18n-allow:标识符：UI_SKINS 里 id 的中文别名（源码可读性用）。落盘只存 ASCII id，界面渲染走 labelOfId("skin", id)，中文不上屏。
    mode: "dark",
    bg: "#16121F", panel: "#1E1A2A", panel2: "#262138",
    fg: "#ECE8F5", fg2: "#A29BB4", line: "rgba(228,220,255,0.10)",
    accent: "#A98BE0", accentFg: "#17102A",
    success: "#4FB985", warning: "#D6A73C", danger: "#E5695F",
    radiusCard: "12px"
  },
];

/** 旧皮肤 id → 新皮肤 id（升级时迁移，避免用户所选失效）。 */
export const UI_SKIN_ALIAS: Record<string, string> = {
  cream: "cream",
  linen: "clay",   // 旧「麻布」暖棕 → 赤陶
  eye: "clay",
  mist: "lagoon",  // 旧「冷雾」冷蓝 → 湖光
  cool: "lagoon",
  graphite: "graphite",
  ink: "nocturne",
  nocturne: "nocturne",
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
const BUILTIN_SCHEME_REV = 3;

/** 界面语言取值的唯一归一化点：auto（跟随系统）/ zh-CN / en。
 *  刻意不在此 import src/lib/i18n —— stores 是 i18n 的数据源，反向依赖会成环。 */
export function normLanguageSetting(v: unknown): "auto" | "zh-CN" | "en" {
  return v === "zh-CN" || v === "en" ? v : "auto";
}

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
  /** 界面语言：auto=跟随系统 / zh-CN=简体中文 / en=English。
   *  接入 i18n 后由 src/lib/i18n 消费；非法值一律归一为 "auto"（见 load/applyConfigJson）。 */
  language: string;
  /** 设置面板分区（扁平列表）显示顺序；可在设置里拖拽排序。id 取 SETTINGS_TABS。 */
  settingsOrder: string[];
}

/** 设置面板全部分区 id（扁平列表默认顺序，去分组后的唯一顺序来源）。 */
export const SETTINGS_TABS = [
  "reader",
  "appearance",
  "manga",
  "tts",
  "split",
  "ai",
  "keys",
  "webdav",
  "prefs",
  "other",
] as const;

/** 归一化设置分区顺序：过滤失效 id、去重，把新增分区补到末尾（增删分区不崩）。 */
export function normSettingsOrder(v: any): string[] {
  const all = SETTINGS_TABS as readonly string[];
  const out: string[] = [];
  if (Array.isArray(v)) {
    for (const x of v) {
      if (typeof x === "string" && all.includes(x) && !out.includes(x)) out.push(x);
    }
  }
  for (const id of all) if (!out.includes(id)) out.push(id);
  return out;
}

/** AI 设置页三个分类 id（默认顺序：节点 → 窗口 → 提示词）。 */
export const AI_GROUPS = ["node", "window", "prompt"] as const;
export type AiGroup = (typeof AI_GROUPS)[number];

/** 归一化 AI 分类顺序：过滤非法 id、去重，把缺失分类补到末尾（旧配置无 groupOrder 也不崩）。 */
export function normAiGroupOrder(v: any): string[] {
  const all = AI_GROUPS as readonly string[];
  const out: string[] = [];
  if (Array.isArray(v)) {
    for (const x of v) {
      if (typeof x === "string" && all.includes(x) && !out.includes(x)) out.push(x);
    }
  }
  for (const id of all) if (!out.includes(id)) out.push(id);
  return out;
}

const DEFAULTS: AppConfig = {
  reader: {
    mode: "page",
    duo: false,
    colGap: 48,
    fontFamily: "lxgwwenkai-regular",
    latinFont: "",
    fontSize: 32,
    fontWeight: 400,
    lineHeight: 1.75,
    paraSpacing: 0.6,
    margins: { scroll: { h: 320, v: 80 }, page: { h: 320, v: 80 }, duo: { h: 320, v: 80 } },
    indent: "1em",
    textAlign: "justify",
    theme: "paper",
    bgColor: "#FDF5E6",
    fgColor: "#2c2c2c",
    bgImage: "",
    bgOpacity: 0.35,
    cssExtra: `
#chapter-content :is(h1, h2, h3) {
  color: #4682B4 !important;
  font-size: 1.6rem !important;
  font-weight: bold !important;
  text-align: center !important;
  margin: 2rem auto !important;
  padding-bottom: 0.5rem !important;
  border-bottom: 1px solid #ddd !important;
}
#chapter-content p {
  text-indent: 1em !important;
  margin: 0 0 1rem !important;
}`,
    overrideBookCss: true,
    wheelTurnPage: true,
    bindings: structuredClone(DEFAULT_BINDINGS),
    zones: { tl: "prev", tr: "next", bl: "prev", br: "next", c: "none" },
    zoneHints: false,
    immersive: false,
    customFonts: [
      { family: "仓耳云黑-w03", file: "仓耳云黑-w03.ttf" }, // i18n-allow:字体族名：必须与字体文件 / 系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
      { family: "方正兰亭圆_gbk", file: "方正兰亭圆_gbk.ttf" }, // i18n-allow:字体族名：必须与字体文件 / 系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
      { family: "lxgwneoxiheiscreen", file: "lxgwneoxiheiscreen.ttf" },
      { family: "lxgwwenkai-regular", file: "lxgwwenkai-regular.ttf" },
    ],
    aiPanelWidth: 360,
    chapterTransition: "cover",
    pageTransition: "h-slide",
    multiWindow: false,
    statusPos: "center",
    statusColor: "",
    topbarColor: "",
    topbarOrder: [...READER_TOPBAR_IDS],
    topbarHidden: [],
    manga: { imgScale: 1.5, fitWidth: true, fitHeight: false, marginH: 8, marginV: 8, imgGap: 6, layout: "flow", direction: "ltr", hideText: false, bgColor: "", autoDetect: true },
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
    showCoverProgress: true,
    coverScan: "auto",
  },
  webdav: {
    enabled: false,
    base: "",
    user: "",
    pass: "",
    syncOnOpen: true,
    uploadBooks: false,
    downloadDir: "",
    backup: { enabled: false, intervalHours: 24, lastAt: 0, lastSig: "", lastCheckAt: 0 },
  },
  ai: {
    enabled: true,
    autoOnChapter: true,
    system: "你是一位专业的阅读助手。回答务必简洁准确，使用中文。", // i18n-allow:用户内容：AI 系统提示词正文是种子数据（用户可编辑），规则要求永不翻译。
    profiles: structuredClone(DEFAULT_AI_PROFILES),
    active: 0,
    prompts: [
      {
        id: 1,
        name: "剧情总结", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
        key: "F1",
        text: "直观说出文本写了什么，不要做阅读理解，不要脑补，不要分段，不要分点，200字内。\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
      },
      {
        id: 2,
        name: "翻译为中文", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
        key: "F2",
        text: "请将以下内容翻译成流畅的中文，直接输出译文：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
      },
      {
        id: 3,
        name: "精校润色", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
        key: "F3",
        text: "请校对并润色以下文本：删除广告与乱码字符，修正错别字与标点，保持原意与结构不要扩写或缩写，直接输出处理后的正文，不要任何解释：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
      },
      {
        id: 4,
        name: "人物情节梳理", // i18n-allow:用户内容：默认提示词档名（用户可编辑的种子数据），与提示词正文一并保留。
        key: "F4",
        text: "请列出本章出场人物（含身份/立场）和本章主要情节点（条目式，简洁）：\n\n{chapter}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
      },
    ],
    display: "reader",
    style: {
      followReader: false,
      fontFamily: "仓耳云黑-w03", // i18n-allow:字体族名：默认字体族名，必须与字体文件 / 系统字体名逐字一致（否则 CSS font-family 匹配不上）。
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
      "请解释下列选中的内容，并给出准确的中文翻译/释义（外文译为中文，中文则解释含义与出处语境）。直接输出结果，简洁准确：\n\n{selection}", // i18n-allow:用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
    groupOrder: [...AI_GROUPS],
    // 默认「系统代理」：与旧版每端点默认 auto 行为一致，老用户升级后不变。
    proxy: { mode: "system", type: "http", url: "" },
  },
  tts: {
    engine: "online",
    services: [
      {
        id: 1,
        name: "默认语音服务", // i18n-allow:持久化值：TTS 服务档名原样落进 config.tts.services（用户可自行改名）；种子名显示侧经 seedTtsLabel() 查 dict.seedTts.default 。
        provider: "openai",
        baseUrl: "http://127.0.0.1:3900",
        apiKey: "",
        model: "omnivoice",
        voice: "33aa3b77",
        format: "wav",
        secretKey: "",
        appId: "",
        cluster: "",
        proxyMode: "inherit",
        proxyType: "http",
        proxyUrl: "",
      },
    ],
    activeService: 0,
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
  language: "auto",
  settingsOrder: [...SETTINGS_TABS],
};
// 注：readerSchemes 默认空，由 load() 用内置方案补齐（见 builtinSchemes）。

/// 内置阅读方案（首次启动/未迁移时写入方案库；可改名、可删除）。
/// 每套方案把「排版 + 间距节奏 + 配色」整体打包，而不仅换背景色：
///  - 衬线（lxgwwenkai 楷体）走首行缩进 + 两端对齐的纸书版式；
///  - 无衬线（雅黑/苹方）走无缩进 + 左对齐的屏幕版式，行宽由左右边距控制。
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
    mk("s-paper", "纸质", { // i18n-allow:持久化值：内置配色方案的 name 会落进 config.readerSchemes，是 builtinSchemeNames() 的默认名来源；界面显示走 dict.scheme.<id>，这里的中文不落地上屏。
      theme: "paper", bgColor: "#FDF5E6", fgColor: "#2c2c2c",
      fontFamily: SERIF, latinFont: "Georgia",
      fontSize: 32, lineHeight: 1.75, paraSpacing: 0.6,
      margins: { scroll: { h: 320, v: 80 }, page: { h: 320, v: 80 }, duo: { h: 320, v: 80 } },
      indent: "1em", textAlign: "justify",
    }),
    mk("s-modern", "现代", { // i18n-allow:持久化值：内置配色方案的 name 会落进 config.readerSchemes，是 builtinSchemeNames() 的默认名来源；界面显示走 dict.scheme.<id>，这里的中文不落地上屏。
      theme: "white", bgColor: "#FAFAF9", fgColor: "#22252A",
      fontFamily: SANS, latinFont: "Segoe UI",
      fontSize: 30, lineHeight: 1.7, paraSpacing: 0.9,
      margins: { scroll: { h: 320, v: 72 }, page: { h: 320, v: 72 }, duo: { h: 320, v: 72 } },
      indent: "0em", textAlign: "left",
    }),
    mk("s-green", "护眼绿", { // i18n-allow:持久化值：内置配色方案的 name 会落进 config.readerSchemes，是 builtinSchemeNames() 的默认名来源；界面显示走 dict.scheme.<id>，这里的中文不落地上屏。
      theme: "green", bgColor: "#c7e6c9", fgColor: "#2d3a2d",
      fontFamily: SERIF, latinFont: "Georgia",
      fontSize: 32, lineHeight: 1.8, paraSpacing: 0.6,
      margins: { scroll: { h: 320, v: 80 }, page: { h: 320, v: 80 }, duo: { h: 320, v: 80 } },
      indent: "1em", textAlign: "justify",
    }),
    mk("s-night", "夜间", { // i18n-allow:持久化值：内置配色方案的 name 会落进 config.readerSchemes，是 builtinSchemeNames() 的默认名来源；界面显示走 dict.scheme.<id>，这里的中文不落地上屏。
      theme: "dark", bgColor: "#23262b", fgColor: "#b8bcc2",
      fontFamily: SANS, latinFont: "Segoe UI",
      fontSize: 30, lineHeight: 1.7, paraSpacing: 0.8,
      margins: { scroll: { h: 320, v: 72 }, page: { h: 320, v: 72 }, duo: { h: 320, v: 72 } },
      indent: "0em", textAlign: "left",
    }),
    mk("s-ink", "纯黑", { // i18n-allow:持久化值：内置配色方案的 name 会落进 config.readerSchemes，是 builtinSchemeNames() 的默认名来源；界面显示走 dict.scheme.<id>，这里的中文不落地上屏。
      theme: "night", bgColor: "#0f1115", fgColor: "#8a8f96",
      fontFamily: SANS, latinFont: "Segoe UI",
      fontSize: 30, lineHeight: 1.7, paraSpacing: 0.8,
      margins: { scroll: { h: 320, v: 72 }, page: { h: 320, v: 72 }, duo: { h: 320, v: 72 } },
      indent: "0em", textAlign: "left",
    }),
  ];
}

/// 内置方案 id → 默认显示名。**唯一来源就是 builtinSchemes() 本身**，不另抄一份常量，
/// 否则改了内置方案名这里就会漂移。懒求值：避免模块初始化时就 structuredClone 五份默认配置。
/// 用途仅限「用户有没有改过内置方案的名字」这一判定；显示文案由界面经 labelOfId("scheme", id) 取。
let builtinNameMap: Record<string, string> | null = null;
export function builtinSchemeNames(): Record<string, string> {
  if (!builtinNameMap) builtinNameMap = Object.fromEntries(builtinSchemes().map((s) => [s.id, s.name]));
  return builtinNameMap;
}

/** 早期版本写进方案库的种子名 → 文案表 dict.scheme.* 的键后缀。
 *  这些名字是我们的文案而非用户命名，故显示时接查表；用户一旦改名即不再命中，回到「原样显示」。 */
export const SEED_SCHEME_NAME_KEYS: Record<string, string> = {
  默认: "s-default",
  未命名方案: "s-untitled",
};

/// 内置方案升级：把「未被用户覆盖过」的内置方案刷新到当前预设（保留用户改过的名字）；
/// 用户自建方案与已被改过的内置方案原样保留；已退役的内置方案（如 s-wide）从存量中移除。
/// 判定「未被覆盖」的方式：内置方案缺 rev 或 rev 落后于当前版本。
/// 注意：不再把「列表里缺失的内置方案」补回——用户删除后不再复活。
function upgradeBuiltinSchemes(list: ReaderScheme[]): { list: ReaderScheme[]; dirty: boolean } {
  const fresh = builtinSchemes();
  const freshById = new Map(fresh.map((s) => [s.id, s]));
  let dirty = false;
  const out: ReaderScheme[] = [];
  for (const s of list) {
    // 已退役的内置方案：从存量安装中清除，不再找回。
    if (s.id === "s-wide") {
      dirty = true;
      continue;
    }
    const f = s.builtin ? freshById.get(s.id) : undefined;
    if (f && s.rev !== BUILTIN_SCHEME_REV) {
      out.push({ ...f, name: s.name || f.name });
      dirty = true;
    } else {
      out.push(s);
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
  // 翻页动效开关已移除（动画恒开、系统「减少动效」优先）：把存量配置里的 anim 一并清掉，
  // 否则 spread(saved.reader) 会让这个死字段永远写回 localStorage。
  "anim",
  "fauxBold",
  "fauxItalic",
  "letterSpacing",
  "punctuationCompression",
  // 「章节过渡」旧三键已由 chapterTransition 取代（层叠覆盖/阻尼弹簧/关闭）：物理清掉，
  // 否则 spread(saved.reader) 会让这两个死字段永远写回 localStorage。
  "scrollBounce",
  "scrollBounceLevel",
  "scrollBounceStrength",
  "scrollBounceThreshold",
  // 边距由标量 marginH/marginV 改为三作用域 margins（任务 3）：旧键要在被清掉前读出来播种，
  // 故 normalizeMargins() 必须排在下面这段清理之前。
  "marginH",
  "marginV",
  // 「每行字数上限」已移除（任务 4）：行宽一律由左右边距决定，残留键物理清掉。
  "maxLineChars",
  // 「键盘翻页」开关已移除：翻页键本来就绑在「按键与手势」里，闸门纯属多余，残留键物理清掉。
  "pageTurnKeys",
] as const;

/** 字号归一化：夹到 [10,64]，保留 1 位小数（支持手输 32.5）。
 *  刻意不复用 normalizeReaderConfig 里的 num()：那个会 Math.round，
 *  会把存量/导入的小数字号当场四舍五入掉，表现为「设置里输了小数没反应」。 */
export function normFontSize(v: any): number {
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isFinite(n)) return DEFAULTS.reader.fontSize;
  return Math.min(64, Math.max(10, Math.round(n * 10) / 10));
}

/** 边距归一化：把旧标量 marginH/marginV 播种到三个作用域，保证 margins 永远是完整对象，
 *  然后删除旧字段。方案快照/导入 JSON/存量 localStorage 三种来源都必须过这里，
 *  否则 applyReaderSchemeGlobal 之后的 config.reader.margins 会是 undefined（读 .h 直接炸）。
 *
 *  src 是「原始快照」：调用方常先把 DEFAULTS 铺进 r（load / applyReaderSchemeGlobal），
 *  那样 r.margins 恒为完整对象，会把「老配置只存了标量」误判成「用户存过 margins」。
 *  有 src 时以 src 里到底有没有 margins 判断；无 src 时按 r 自己判断。
 *  返回是否改动了 r（供 dirty 落盘标记）。 */
function normalizeMargins(r: any, src?: any): boolean {
  const clampM = (v: any, min: number, max: number, def: number) => {
    const n = typeof v === "number" ? v : Number(v);
    return Number.isFinite(n) ? Math.max(min, Math.min(max, n)) : def;
  };
  const h0 = clampM(r.marginH, 0, 560, DEFAULTS.reader.margins.page.h);
  const v0 = clampM(r.marginV, 0, 400, DEFAULTS.reader.margins.page.v);
  const srcObj = src && typeof src === "object" ? src : r;
  const m = srcObj.margins;
  const has = !!m && typeof m === "object" && !Array.isArray(m);
  let dirty = false;
  if (!has) {
    // 老配置：三个作用域全部等于原来那一份边距，老用户观感不变。
    const seed = { h: h0, v: v0 };
    r.margins = { scroll: { ...seed }, page: { ...seed }, duo: { ...seed } };
    dirty = true;
  } else {
    const out = {} as Record<MarginScope, MarginSides>;
    for (const s of ["scroll", "page", "duo"] as MarginScope[]) {
      const sm = m[s] && typeof m[s] === "object" ? m[s] : {};
      // 缺失/非法的分项用旧标量兜底（不是固定 320/80）：否则存量方案的边距会被悄悄改掉。
      const h = clampM(sm.h, 0, 560, h0);
      const v = clampM(sm.v, 0, 400, v0);
      out[s] = { h, v };
      const cur = r.margins && r.margins[s];
      if (!cur || cur.h !== h || cur.v !== v) dirty = true;
    }
    r.margins = out;
  }
  if (Object.prototype.hasOwnProperty.call(r, "marginH")) { delete r.marginH; dirty = true; }
  if (Object.prototype.hasOwnProperty.call(r, "marginV")) { delete r.marginV; dirty = true; }
  return dirty;
}

/// 归一化 reader 配置：清理已废弃键、修正非法新字段、
/// 迁移旧默认 cssExtra 与旧滚动缓冲数值。返回是否发生了需要落盘的清理。
function normalizeReaderConfig(r: any, savedRaw?: any): boolean {
  let dirty = false;
  // 边距迁移必须先于废弃键清理：旧标量 marginH/marginV 的取值要在被 delete 前读出来播种三作用域。
  if (normalizeMargins(r, savedRaw)) dirty = true;
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
  // 字号允许小数（手输 32.5）：夹到 [10,64] 并保留 1 位小数。
  // 不能复用上面的 num()（它会 Math.round），否则存量/导入的小数字号会被吃回整数。
  const fsz = normFontSize(r.fontSize);
  if (r.fontSize !== fsz) { r.fontSize = fsz; dirty = true; }
  const ta = r.textAlign === "left" ? "left" : "justify";
  if (r.textAlign !== ta) { r.textAlign = ta; dirty = true; }
  // 「章节过渡」档位：非法/缺失一律取 cover（旧配置只有 scrollBounce* 三键，已被上面物理清掉）。
  const ct = r.chapterTransition;
  if (ct !== "cover" && ct !== "spring" && ct !== "none") {
    r.chapterTransition = "cover";
    dirty = true;
  }
  // 翻页模式的「章节过渡」档位：非法/缺失一律取 h-slide（与普通翻页观感一致的缺省档）；
  // 已删除的视差侧滑档也走这里 → 可预期地回落 h-slide（显式枚举，不靠默认分支吞旧值）。
  const pt = r.pageTransition;
  if (pt !== "h-spring" && pt !== "h-slide" && pt !== "none") {
    r.pageTransition = "h-slide";
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
  // 适宽 / 适高互斥：两者同时为真时以适宽优先（历史配置兜底）。
  let mfh = r.manga.fitHeight === true;
  if (mfw && mfh) mfh = false;
  if (r.manga.fitHeight !== mfh) { r.manga.fitHeight = mfh; dirty = true; }
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
  // 自动识别漫画：缺失/非布尔一律补 true，保证老配置升级后行为与升级前一致（默认自动识别）。
  const mad = r.manga.autoDetect !== false;
  if (r.manga.autoDetect !== mad) { r.manga.autoDetect = mad; dirty = true; }
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

const KEY = "tidepage.config.v1";

/** 从代理地址 scheme 推类型（迁移旧配置用）。 */
function proxyTypeFromUrl(url: string): AiProxyConfig["type"] {
  const u = url.trim().toLowerCase();
  if (u.startsWith("socks5://") || u.startsWith("socks5h://")) return "socks5";
  if (u.startsWith("https://")) return "https";
  return "http";
}

/** 全局代理归一化：非法/缺失字段回落默认值；返回归一化结果与是否需要落盘。 */
function normalizeAiProxy(p: any): { proxy: AiProxyConfig; dirty: boolean } {
  const d = DEFAULTS.ai.proxy;
  const mode = p?.mode === "direct" || p?.mode === "system" || p?.mode === "manual" ? p.mode : d.mode;
  const type = p?.type === "http" || p?.type === "https" || p?.type === "socks5" ? p.type : d.type;
  const url = typeof p?.url === "string" ? p.url : d.url;
  const dirty = !p || p.mode !== mode || p.type !== type || p.url !== url;
  return { proxy: { mode, type, url }, dirty };
}

/**
 * 旧版「每端点代理」（profile.proxyMode / profile.proxyUrl）→ 全局代理。
 * 取当前 active 端点那份，并在**所有** profile 上物理删除这两个字段：
 * 否则 normalizeProfile 的 {...src} 会把死字段一直写回配置，且 stripSecrets 已不再清它，
 * 旧导入配置里的明文 proxyUrl 可能重新落进 localStorage。
 * 无旧字段返回 null（调用方保留原全局代理）。
 */
/**
 * 识别**旧版**「每端点代理」schema：profile 上带 proxyMode/proxyUrl 但**没有** proxyType。
 * 新版每端点代理恒带 proxyType（且 proxyMode ∈ inherit/direct/system/manual），据此区分，
 * 绝不把新版配置当旧版搬进全局。命中旧版时把 active 端点那份搬到全局，并物理删除所有端点上的旧字段。
 */
export function absorbLegacyProfileProxy(ai: any): AiProxyConfig | null {
  const list: any[] = Array.isArray(ai?.profiles) ? ai.profiles : [];
  const isLegacy = (p: any) =>
    p &&
    typeof p === "object" &&
    (Object.prototype.hasOwnProperty.call(p, "proxyMode") ||
      Object.prototype.hasOwnProperty.call(p, "proxyUrl")) &&
    !Object.prototype.hasOwnProperty.call(p, "proxyType");
  if (!list.some(isLegacy)) return null;
  const active = list[Math.min(Math.max(ai?.active | 0, 0), Math.max(list.length - 1, 0))] || {};
  // active 端点可能已是新版（混合配置的极端情形）：取它的旧值，没有则按 system 兜底。
  const mode: AiProxyConfig["mode"] =
    active.proxyMode === "custom" ? "manual" : active.proxyMode === "direct" ? "direct" : "system";
  const url = typeof active.proxyUrl === "string" ? active.proxyUrl : "";
  for (const p of list) {
    if (isLegacy(p)) {
      delete p.proxyMode;
      delete p.proxyUrl;
    }
  }
  return { mode, type: proxyTypeFromUrl(url), url };
}

function migrateAi(savedAi: any): AiConfig {
  const base = {
    ...structuredClone(DEFAULTS.ai),
    ...(savedAi || {}),
    style: { ...structuredClone(DEFAULTS.ai.style), ...(savedAi?.style || {}) },
  } as AiConfig;
  const proxyNorm = normalizeAiProxy(savedAi?.proxy);
  base.proxy = proxyNorm.proxy;
  // 字段被修正/补齐也要落盘，否则每次启动都重复归一化。
  if (proxyNorm.dirty) aiMigrationDirty = true;
  // 旧版「每端点代理」→ 全局：搬 active 端点那份，并删除 profile 上的死字段。
  // 地址可能只在加密存储里（localStorage 的 profile.proxyUrl 已被 stripSecrets 清空）：
  // 此时得到 manual + 空地址，由 applySecrets 按旧密文键 ai:{id}:proxyUrl 补齐。
  const legacyProxy = absorbLegacyProfileProxy(savedAi);
  if (legacyProxy) {
    base.proxy = legacyProxy;
    // stripSecrets 已不再清 profile.proxyUrl：不置位就会出现「旧字段长期残留、每次启动重复迁移」。
    aiMigrationDirty = true;
    // 告诉 applySecrets：本次是旧 schema 升级，旧密文键 ai:{id}:proxyUrl 应搬进全局而非回填端点。
    legacyProxyPending = true;
  }
  base.groupOrder = normAiGroupOrder(base.groupOrder);
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
    // 旧版单端点：代理字段同样搬到全局（profile 上不再保留）。
    const mode: AiProxyConfig["mode"] =
      savedAi?.proxyMode === "custom" ? "manual" : savedAi?.proxyMode === "direct" ? "direct" : "system";
    const u = typeof savedAi?.proxyUrl === "string" ? savedAi.proxyUrl : "";
    base.proxy = { mode, type: proxyTypeFromUrl(u), url: u };
    profiles = normalizeProfiles([
      {
        name: "默认", // i18n-allow:持久化值：旧版单端点配置迁移出的 AI 端点档名，原样落进 config.ai.profiles（用户可自行改名）；显示侧经 seedEndpointLabel() 查 dict.seedEndpoint.legacy 。
        provider: savedAi?.provider ?? "openai",
        baseUrl: savedAi?.baseUrl ?? "",
        apiKey: savedAi?.apiKey ?? "",
        model: savedAi?.model ?? "",
        deployment: savedAi?.deployment ?? "",
        apiVersion: savedAi?.apiVersion ?? "",
      },
    ]);
  } else {
    profiles = structuredClone(DEFAULT_AI_PROFILES);
  }
  const prompts = structuredClone(DEFAULT_PROMPTS);
  if (savedAi?.promptSummary) prompts[0].text = savedAi.promptSummary;
  if (savedAi?.promptTranslate) prompts[1].text = savedAi.promptTranslate;
  if (savedAi?.action === "custom" && savedAi?.customPrompt) {
    prompts.push({ id: 99, name: "自定义", key: "F9", text: savedAi.customPrompt }); // i18n-allow:用户内容：旧版 customPrompt 迁移出的提示词档名（用户可编辑的种子数据）。
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
    groupOrder: normAiGroupOrder(savedAi?.groupOrder),
    proxy: base.proxy,
  };
}

let reasoningMigrationDirty = false;
let readerMigrationDirty = false;
let ttsMigrationDirty = false;
// 升级迁移：旧版 localStorage 里含明文凭据，需落盘一次（加密入库 + 脱敏 localStorage）。
let secretsMigrationDirty = false;
// 旧版「每端点代理」密文刚被采纳进全局时置位，用于立刻落盘一次（新键取代旧密文键）。
let legacyProxyAdopted = false;
// migrate 检测到旧 schema 的「每端点代理」时置位；applySecrets 据此把旧密文键一次性搬进全局，
// 而非回填到端点（新旧共用 ai:{id}:proxyUrl 键名，故迁移这一趟必须跳过端点回填）。
let legacyProxyPending = false;
// AI 配置做了结构性迁移（旧每端点代理字段、proxy 归一化修正）时置位，启动落盘一次。
let aiMigrationDirty = false;

/** 语音服务配置档案的遗留扁平键（迁移后物理删除，避免导出的配置残留协议概念）。 */
const LEGACY_TTS_KEYS = [
  "onlineSource",
  "onlineProfileId",
  "customProvider",
  "customBaseUrl",
  "customApiKey",
  "onlineModel",
  "onlineVoice",
  "onlineFormat",
] as const;

/**
 * 迁移语音朗读配置到「服务配置档案」模型。
 *  - 已含 services：逐条归一化并补齐 id。
 *  - 仅含旧扁平字段：由旧 customProvider/customBaseUrl/customApiKey/onlineModel/
 *    onlineVoice/onlineFormat 构造一条服务；provider 缺省 openai。
 *  - 一律删除遗留扁平键；对用户不丢地址/密钥/音色。
 */
function migrateTts(savedTts: any): TtsConfig {
  const base: any = { ...structuredClone(DEFAULTS.tts), ...(savedTts || {}) };
  const hasServices = Array.isArray(savedTts?.services) && savedTts.services.length > 0;
  if (hasServices) {
    const list = savedTts.services.map((s: any) => normalizeTtsService(s));
    ensureTtsServiceIds(list);
    base.services = list;
  } else {
    const src = savedTts && typeof savedTts === "object" ? savedTts : {};
    const svc = normalizeTtsService({
      id: 1,
      name: "默认语音服务", // i18n-allow:持久化值：TTS 服务档名原样落进 config.tts.services（用户可自行改名）；种子名显示侧经 seedTtsLabel() 查 dict.seedTts.default 。
      provider:
        typeof src.customProvider === "string" && src.customProvider
          ? src.customProvider
          : "openai",
      baseUrl: typeof src.customBaseUrl === "string" ? src.customBaseUrl : "",
      apiKey: typeof src.customApiKey === "string" ? src.customApiKey : "",
      model: typeof src.onlineModel === "string" ? src.onlineModel : "",
      voice: typeof src.onlineVoice === "string" ? src.onlineVoice : "",
      format: typeof src.onlineFormat === "string" ? src.onlineFormat : "",
    });
    // 仅 OpenAI 兼容在缺省地址时补默认本地服务地址；其它协议地址语义不同，不臆造。
    if (svc.provider === "openai" && !svc.baseUrl.trim()) {
      svc.baseUrl = DEFAULTS.tts.services[0].baseUrl;
    }
    base.services = [svc];
    base.activeService = 0;
  }
  if (!base.services.length) {
    base.services = [normalizeTtsService(structuredClone(DEFAULTS.tts.services[0]))];
  }
  base.activeService = Math.min(Math.max((base.activeService as number) | 0, 0), base.services.length - 1);
  for (const k of LEGACY_TTS_KEYS) delete base[k];
  return base as TtsConfig;
}

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
      const name = typeof s.name === "string" && s.name.trim() ? s.name.trim().slice(0, 40) : "未命名方案"; // i18n-allow:持久化值：方案名兜底值原样落进 config.readerSchemes（用户可自行改名）；种子名显示侧由 schemeLabel() 经 SEED_SCHEME_NAME_KEYS 查 dict.scheme.s-untitled 。
      const cfgSrc = s.cfg && typeof s.cfg === "object" ? s.cfg : {};
      // 存量方案的 cfg 先升级成三作用域边距（并删掉其中的旧标量键），
      // 这样下面 spread 进 cfg 的 margins 才是完整结构。
      normalizeMargins(cfgSrc);
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
    const migratedAi = migrateAi(saved.ai);
    // 迁移：语音朗读由「扁平协议字段」改为「服务配置档案」；无 services 时需落盘一次。
    if (!Array.isArray(saved.tts?.services) || !saved.tts.services.length) ttsMigrationDirty = true;
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
      webdav: {
        ...DEFAULTS.webdav,
        ...saved.webdav,
        backup: { ...DEFAULTS.webdav.backup, ...(saved.webdav?.backup || {}) },
      },
      ai: migratedAi,
      tts: migrateTts(saved.tts),
      regex: saved.regex ?? "",
      uiTheme: saved.uiTheme === "light" || saved.uiTheme === "dark" ? saved.uiTheme : "system",
      uiSkinLight: resolveSkinId(saved.uiSkinLight, "light"),
      uiSkinDark: resolveSkinId(saved.uiSkinDark, "dark"),
      readerSchemes: normalizeSchemes(saved.readerSchemes),
      // i18n 接线：只认 auto / zh-CN / en，旧配置里的任意字符串（含早期空串）一律回落 auto。
      language: normLanguageSetting(saved.language),
      settingsOrder: normSettingsOrder(saved.settingsOrder),
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
    // 存量用户自建方案的 cfg 里可能还留着旧标量边距/每行字数上限：normalizeSchemes 已在内存里把它们
    // 升级成三作用域 margins，这里补一个落盘标记，保证升级结果真正写回 localStorage。
    if (
      Array.isArray(saved.readerSchemes) &&
      saved.readerSchemes.some(
        (s: any) =>
          s?.cfg &&
          (Object.prototype.hasOwnProperty.call(s.cfg, "marginH") ||
            Object.prototype.hasOwnProperty.call(s.cfg, "marginV") ||
            Object.prototype.hasOwnProperty.call(s.cfg, "maxLineChars")),
      )
    ) {
      readerMigrationDirty = true;
    }
    // 升级迁移：旧版 localStorage 明文存有凭据 → 标记落盘一次（saveConfigNow 会
    // 加密写入 secret.config 并生成脱敏 localStorage）。
    if (Object.values(collectSecrets(merged)).some((v) => v !== "")) secretsMigrationDirty = true;
    return merged;
  } catch {
    return structuredClone(DEFAULTS);
  }
}

export const config = $state<AppConfig>(load());

/* ===================== 凭据抽取 / 回填 / 脱敏 ===================== */

/** 敏感字段清单（唯一来源）：localStorage、跨窗广播、导出、云端备份一律不得含其明文。
 *  - webdav.pass
 *  - ai.proxy.url（全局代理地址，可能含 账号:密码@host）
 *  - ai.profiles[*].apiKey
 *  - tts.services[*].apiKey / secretKey / appId
 *  key 采用稳定标识：`webdav:pass`、`ai:proxyUrl`、`tts:{id}:secretKey` 等。 */

/** 从配置中抽取全部敏感字段（含空串，保证「清空」也能同步）；纯函数，不改动入参。 */
export function collectSecrets(cfg: AppConfig): Record<string, string> {
  const out: Record<string, string> = {};
  const wd = (cfg as any)?.webdav;
  if (wd) out["webdav:pass"] = typeof wd.pass === "string" ? wd.pass : "";
  // 全局代理地址是敏感字段（可能含 账号:密码@host），与密钥同级单独加密。
  const ai = (cfg as any)?.ai;
  if (ai) out["ai:proxyUrl"] = typeof ai.proxy?.url === "string" ? ai.proxy.url : "";
  for (const p of ((cfg as any)?.ai?.profiles ?? []) as any[]) {
    if (!p) continue;
    out[`ai:${p.id}:apiKey`] = typeof p.apiKey === "string" ? p.apiKey : "";
    // 端点手动代理地址同样是敏感字段（可能含 账号:密码@host）。
    out[`ai:${p.id}:proxyUrl`] = typeof p.proxyUrl === "string" ? p.proxyUrl : "";
  }
  for (const s of ((cfg as any)?.tts?.services ?? []) as any[]) {
    if (!s) continue;
    out[`tts:${s.id}:apiKey`] = typeof s.apiKey === "string" ? s.apiKey : "";
    out[`tts:${s.id}:secretKey`] = typeof s.secretKey === "string" ? s.secretKey : "";
    out[`tts:${s.id}:appId`] = typeof s.appId === "string" ? s.appId : "";
    out[`tts:${s.id}:proxyUrl`] = typeof s.proxyUrl === "string" ? s.proxyUrl : "";
  }
  return out;
}

/** 把抽取结果写回配置（仅覆盖 map 中显式出现的键）；纯副作用函数，不改动 map。 */
export function applySecrets(cfg: AppConfig, map: Record<string, unknown> | null | undefined): void {
  if (!map || typeof map !== "object") return;
  const wd = (cfg as any)?.webdav;
  if (wd && typeof map["webdav:pass"] === "string") wd.pass = map["webdav:pass"];
  const ai = (cfg as any)?.ai;
  if (ai && typeof map["ai:proxyUrl"] === "string") {
    ai.proxy.url = map["ai:proxyUrl"];
    // 存量不一致修复：地址带 scheme 时以地址为准（与后端 normalize_proxy_url 的语义一致）。
    // migrateAi 阶段的 normalizeAiProxy 跑在 url 已被 stripSecrets 清空之后，无从推断 scheme，
    // 只有这里（密文回填拿到真实地址）能调和；地址不带 scheme 则保留用户所选类型。
    if (ai.proxy.url.includes("://")) ai.proxy.type = proxyTypeFromUrl(ai.proxy.url);
  }
  // 一次性采纳旧版「每端点代理」密文键 ai:{id}:proxyUrl → 全局默认：仅在 migrate 检测到旧 schema
  // （legacyProxyPending）时执行，避免把新版某端点的手动代理地址误当旧值搬进全局。旧键 ai:{id}:proxyUrl
  // 与新版「端点手动代理」复用同名键，故这一次迁移里绝不把它再回填到端点（端点此时一律 inherit）：
  // 下一次 saveConfigNow 以新键整体覆盖，旧值被清为空串。
  const adoptingLegacy = !!(ai && legacyProxyPending);
  if (ai && legacyProxyPending) {
    legacyProxyPending = false;
    if (!ai.proxy.url) {
      const list = ((ai.profiles ?? []) as any[]).filter(Boolean);
      const active = list[Math.min(Math.max(ai.active | 0, 0), Math.max(list.length - 1, 0))];
      const legacy = active ? map[`ai:${active.id}:proxyUrl`] : undefined;
      if (typeof legacy === "string" && legacy) {
        // 地址非空说明旧配置就是 custom 代理；若全局模式还是默认的 system，则一并切成 manual。
        const mode = ai.proxy.mode === "system" ? "manual" : ai.proxy.mode;
        ai.proxy = { mode, type: proxyTypeFromUrl(legacy), url: legacy };
        legacyProxyAdopted = true;
      }
    }
  }
  for (const p of ((cfg as any)?.ai?.profiles ?? []) as any[]) {
    if (!p) continue;
    const ka = `ai:${p.id}:apiKey`;
    if (typeof map[ka] === "string") p.apiKey = map[ka];
    // 新版端点手动代理地址回填（旧 schema 一次性迁移这一趟跳过，见上）。
    const kp = `ai:${p.id}:proxyUrl`;
    if (!adoptingLegacy && typeof map[kp] === "string") {
      p.proxyUrl = map[kp];
      // 地址带 scheme 时以地址为准同步 type（与全局 ai.proxy 的调和逻辑一致）。
      if (p.proxyUrl.includes("://")) p.proxyType = proxyTypeFromUrl(p.proxyUrl);
    }
  }
  for (const s of ((cfg as any)?.tts?.services ?? []) as any[]) {
    if (!s) continue;
    for (const f of ["apiKey", "secretKey", "appId"] as const) {
      const k = `tts:${s.id}:${f}`;
      if (typeof map[k] === "string") (s as any)[f] = map[k];
    }
    const kp = `tts:${s.id}:proxyUrl`;
    if (typeof map[kp] === "string") {
      s.proxyUrl = map[kp];
      if (s.proxyUrl.includes("://")) s.proxyType = proxyTypeFromUrl(s.proxyUrl);
    }
  }
}

/** 返回脱敏后的配置深拷贝：敏感字段置空（保留字段形状，便于类型稳定与回填）。 */
export function stripSecrets(cfg: AppConfig): AppConfig {
  const copy = JSON.parse(JSON.stringify(cfg)) as AppConfig;
  const wd = (copy as any)?.webdav;
  if (wd) wd.pass = "";
  const ai = (copy as any)?.ai;
  if (ai?.proxy) ai.proxy.url = "";
  for (const p of ((copy as any)?.ai?.profiles ?? []) as any[]) {
    p.apiKey = "";
    p.proxyUrl = "";
  }
  for (const s of ((copy as any)?.tts?.services ?? []) as any[]) {
    s.apiKey = "";
    s.secretKey = "";
    s.appId = "";
    s.proxyUrl = "";
  }
  return copy;
}

/** 传给后端的全局默认代理参数（端点选择「跟随全局」时解析为此值）。 */
export function aiProxyArgs(): { proxyMode: string; proxyUrl: string; proxyType: string } {
  return { proxyMode: config.ai.proxy.mode, proxyUrl: config.ai.proxy.url, proxyType: config.ai.proxy.type };
}

/**
 * 解析单个端点（AI profile / TTS service）实际要发给后端的代理参数：
 *  - inherit（或缺省）→ 全局默认 config.ai.proxy；
 *  - direct / system  → 对应模式，地址置空（后端只认 direct/system/manual）；
 *  - manual           → 端点自带的 type/url。
 * **绝不**把 inherit 透传给后端：后端 resolve_proxy 不认识该模式。
 */
export function resolveEndpointProxy(
  ep: { proxyMode?: string; proxyType?: string; proxyUrl?: string } | null | undefined,
): { proxyMode: string; proxyUrl: string; proxyType: string } {
  const mode = ep?.proxyMode;
  if (mode === "direct") return { proxyMode: "direct", proxyUrl: "", proxyType: "http" };
  if (mode === "system") return { proxyMode: "system", proxyUrl: "", proxyType: "http" };
  if (mode === "manual") {
    return {
      proxyMode: "manual",
      proxyUrl: typeof ep?.proxyUrl === "string" ? ep.proxyUrl : "",
      proxyType: ep?.proxyType === "https" || ep?.proxyType === "socks5" ? ep.proxyType : "http",
    };
  }
  // inherit / 缺省 / 非法 → 全局默认
  return aiProxyArgs();
}

/** 启动后异步从加密存储读回凭据并回填；各窗 api 调用依赖凭据前需 await 此 Promise。 */
let resolveSecretsReady: () => void = () => {};
export const secretsReady: Promise<void> = new Promise<void>((r) => {
  resolveSecretsReady = r;
});

// 仅做一次性初始化：读 secret.config → 解析 → 回填到当前 config。
// 用 setTimeout 延后到所有模块求值完成，规避与 api.ts 的循环导入在求值期触发 TDZ。
setTimeout(() => {
  void (async () => {
    let typeFixed = false;
    try {
      const raw = await api.secretRead();
      if (raw) {
        const map = JSON.parse(raw) as Record<string, unknown>;
        const typeBefore = config.ai.proxy.type;
        applySecrets(config, map);
        // applySecrets 刚修好了存量「类型 ↔ 地址 scheme」不一致：置 dirty 并立即落盘，
        // 否则下次启动密文回填又把它改回旧值（applySecrets 是纯副作用，不会触发保存）。
        if (config.ai.proxy.type !== typeBefore) {
          aiMigrationDirty = true;
          typeFixed = true;
        }
      }
    } catch {
      /* 无加密设施 / 尚未写入时忽略，配置保持脱敏状态 */
    } finally {
      // 采纳了旧密文键、或修正了 type/scheme 不一致：立刻落盘一次
      // （前者让新键 ai:proxyUrl 取代 ai:{id}:proxyUrl；后者把修正后的 type 写回）。
      if (legacyProxyAdopted || typeFixed) saveConfigNow();
      resolveSecretsReady();
    }
  })();
}, 0);

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

/** 默认封面配色（随外壳主题变化）：g1/g2/g3=渐变三档，tc=书名字色。
 *  仅用于「无内嵌封面」书籍的默认封面（有封面的书不受影响）；coverUrl 读取它拼进 URL，
 *  故切换主题会即时重算这些字段 → 默认封面 URL 变化 → 自动重取、颜色随主题更新。 */
export const coverTheme = $state({ g1: "#36485c", g2: "#212d3c", g3: "#141c26", tc: "#f4ecdc" });

/** 默认封面底图缓存破坏版本（对象包裹以保证跨模块响应式，照搬 coverTheme 模式）。
 *  仅用于「无内嵌封面」书籍的 /cover-default/ 端点：换/恢复默认底图后自增，
 *  coverUrl 在模板中读取 defaultCoverBust.n 建立响应式依赖 → 无内嵌封面的 <img> 立即重取新底图；
 *  有内嵌封面的书走 /book-cover/ 端点、不读此值，故完全不受影响、不闪白。 */
export const defaultCoverBust = $state({ n: 0 });
export function bumpDefaultCoverBust(): void {
  defaultCoverBust.n++;
}

/** sRGB 线性插值混合两个 #rrggbb（t=0 取 a，t=1 取 b）。 */
function mixHex(a: string, b: string, t: number): string {
  const p = (h: string) => {
    const x = h.replace("#", "");
    return [parseInt(x.slice(0, 2), 16), parseInt(x.slice(2, 4), 16), parseInt(x.slice(4, 6), 16)];
  };
  const [r1, g1, b1] = p(a);
  const [r2, g2, b2] = p(b);
  const m = (u: number, v: number) => Math.round(u + (v - u) * t).toString(16).padStart(2, "0");
  return `#${m(r1, r2)}${m(g1, g2)}${m(b1, b2)}`;
}

/** 由外壳皮肤推导默认封面配色。
 *  设计：默认封面是「与外壳同源的柔和浅色卡片」，不是高饱和色块——
 *  以主题表面色（panel2/bg）为底，只掺入极少量主色作为暖调，气质轻淡、与浅色主题相衬。
 *  书名字色统一：浅色主题=柔和深色、深色主题=柔和浅色（非纯黑纯白，护眼易读），不随主色变化。 */
function computeCoverTheme(s: UiSkin): { g1: string; g2: string; g3: string; tc: string } {
  let g1: string, g2: string, g3: string;
  if (s.mode === "dark") {
    // 深色：柔和深卡片，顶部透一点主色微光，底部略沉。
    g1 = mixHex(s.panel2, s.accent, 0.14);
    g2 = s.panel2;
    g3 = mixHex(s.bg, "#000000", 0.22);
  } else {
    // 浅色：清浅纸感卡片，仅 10–22% 主色着色，避免厚重。
    g1 = mixHex(s.panel2, "#ffffff", 0.4);
    g2 = mixHex(s.panel2, s.accent, 0.1);
    g3 = mixHex(s.panel2, s.accent, 0.22);
  }
  // 书名字色：浅色主题柔和深色，深色主题柔和浅色（避免纯黑/纯白刺眼）。
  const tc = s.mode === "dark" ? "#EFEBE4" : "#2A2723";
  return { g1, g2, g3, tc };
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
  // 更清晰的强调描边（按钮/控件 hover、分隔）：由前景色按模式派生，保证各皮肤下都有足够可见度。
  root.style.setProperty("--line-strong", `color-mix(in srgb, ${s.fg} ${mode === "dark" ? 22 : 18}%, transparent)`);
  root.style.setProperty("--accent", s.accent);
  root.style.setProperty("--accent-fg", s.accentFg);
  root.style.setProperty("--success", s.success);
  root.style.setProperty("--warning", s.warning);
  root.style.setProperty("--danger", s.danger);
  // 皮肤圆角：直接落到 --radius-lg（弹窗 / 卡片 / 皮肤卡共用这一档），
  // 让每套皮肤的圆角气质真正生效，而不是写一个没人读的 --radius-card。
  root.style.setProperty("--radius-lg", s.radiusCard);
  // 分层柔和阴影（Vercel 的「堆叠式」配方：近距细影 + 远距扩散影），并按皮肤前景色
  // 「着色」而非纯黑：更融于背景、不显脏。深色皮肤整体加深（乘 1.0），浅色减淡（乘 0.55）。
  const tint = s.fg;
  const a = (v: number) => Math.round(mode === "dark" ? v : v * 0.55);
  const mix = (pct: number) => `color-mix(in srgb, ${tint} ${pct}%, transparent)`;
  root.style.setProperty("--shadow-sm", `0 1px 2px ${mix(a(6))}, 0 1px 1px ${mix(a(4))}`);
  root.style.setProperty("--shadow-md", `0 2px 2px ${mix(a(5))}, 0 8px 8px -4px ${mix(a(7))}`);
  root.style.setProperty("--shadow-drawer", `0 2px 2px ${mix(a(5))}, 0 16px 32px -8px ${mix(a(11))}`);
  root.style.setProperty(
    "--shadow-lg",
    `0 1px 1px ${mix(a(4))}, 0 8px 16px -4px ${mix(a(7))}, 0 24px 32px -8px ${mix(a(9))}`,
  );

  // 默认封面配色随主题更新（供 coverUrl 拼进默认封面 URL；仅影响无内嵌封面书）。
  const ct = computeCoverTheme(s);
  coverTheme.g1 = ct.g1;
  coverTheme.g2 = ct.g2;
  coverTheme.g3 = ct.g3;
  coverTheme.tc = ct.tc;
}

/**
 * 把**全局**阅读配色写进 <html> 的 --rdr-bg / --rdr-fg（阅读区表面变量的根）。
 *
 * 为什么需要：阅读区表面变量（--rdr-surface/-line/-shadow-*）在 app.css 里由
 * --rdr-bg/--rdr-fg 派生。正常阅读时它们写在 .reader 上（见 Reader.svelte 的
 * readerVars()，含「仅本书方案」覆盖），但有两类地方拿不到 .reader：
 *   ① 独立阅读窗 / 独立面板窗（StandaloneBook / StandalonePanel）——整窗就是阅读内容；
 *   ② portal 到 body 的阅读区浮层（书内搜索）——脱离 .reader 子树。
 * 这两处都需要一个「阅读主题」而不是「外壳皮肤」的值，否则会出现深色外壳 + 纸色书页
 * 时浮层渲染成深色小窗的割裂感。
 *
 * 取值与 +page.svelte 的 aiRootStyle（独立 AI 窗）同源同法：本书方案拿不到（那属于
 * Reader 的职责，它会用更精确的值覆盖这里的全局值），故这里只取全局 config.reader。
 * 派生量（--rdr-surface 等）刻意不在这里写：留在 app.css 由这两个基色推导，改一处即可。
 */
function applyReaderSurfaceVars(): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const t = THEMES[config.reader.theme];
  const bg = config.reader.bgColor || t.bg;
  const fg = config.reader.fgColor || t.fg;
  root.style.setProperty("--rdr-bg", bg);
  root.style.setProperty("--rdr-fg", fg);
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
  // 阅读配色变化即更新全局 --rdr-bg/-fg（供无 .reader 的阅读窗与 portal 浮层）。
  // 节流：设置面板里这两个色值是 bind:value（逐 input 事件写），而写 CSS 变量会让
  // 整个文档的样式失效重算。逐字符重算既无必要（六位色值输入时中间态本来也不是合法颜色），
  // 又正好卡在用户连续输入的路径上，故合并到停手后。theme 是按钮切换，保持即时。
  let surfaceVarTimer: ReturnType<typeof setTimeout> | undefined;
  $effect.root(() => {
    $effect(() => {
      void config.reader.theme;
      void config.reader.bgColor;
      void config.reader.fgColor;
      if (surfaceVarTimer) clearTimeout(surfaceVarTimer);
      // applyReaderSurfaceVars 内部读的是「调用那一刻」的当前值，故节流后自动落到最新值。
      surfaceVarTimer = setTimeout(() => {
        surfaceVarTimer = undefined;
        applyReaderSurfaceVars();
      }, 150);
    });
  });
  try {
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      if (config.uiTheme === "system") applyUiTheme();
    });
  } catch { /* ignore */ }
  applyUiTheme();
  applyReaderSurfaceVars();
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
    { id, name: name.trim().slice(0, 40) || "未命名方案", cfg: snapshotReader() }, // i18n-allow:持久化值：方案名兜底值会原样落进 config.readerSchemes，用户可自行改名。
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
  // 方案快照可能是旧结构（没有 margins / 只有标量 marginH/V）：先合并默认值再走 load 同一条归一化路径，
  // 保证 config.reader.margins 一定完整（否则读 .h 直接炸）；播种取方案自己的旧边距，方案仍还原它原本的值。
  const merged = { ...structuredClone(DEFAULTS.reader), ...JSON.parse(JSON.stringify(s.cfg)) };
  normalizeReaderConfig(merged, s.cfg);
  config.reader = merged as ReaderConfig;
  saveConfig();
}

// 迁移落盘：检测到已废弃的旧键/需迁移的旧值（“关闭思考”、reader 废弃字段与旧默认）时，
// 启动即把清理后的配置写回 localStorage。延后到模块初始化完成，避免 TDZ。
//
// 必须等 secretsReady：secret_write 是**整体替换**密文，若在凭据回填之前就落盘，
// 旧版每端点的代理地址（密文键 ai:{id}:proxyUrl）会在迁移读取前被新键 ai:proxyUrl 覆盖掉。
if (
  reasoningMigrationDirty ||
  readerMigrationDirty ||
  ttsMigrationDirty ||
  secretsMigrationDirty ||
  aiMigrationDirty
) {
  void secretsReady.then(() => saveConfig());
}

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
    obj.webdav = { ...structuredClone(DEFAULTS.webdav), ...(obj.webdav || {}) };
    obj.webdav.backup = {
      ...structuredClone(DEFAULTS.webdav.backup),
      ...((obj.webdav && obj.webdav.backup) || {}),
    };
    obj.ai = { ...structuredClone(DEFAULTS.ai), ...obj.ai };
    obj.ai.proxy = normalizeAiProxy(obj.ai.proxy).proxy;
    // 跨窗同步/导入可能带来旧版每端点代理字段：须在 normalizeProfiles **之前**于原始 profile 上识别并
    // 搬到全局（normalize 会给每个端点补上新版 proxyType，之后就无法再判定是否旧 schema 了）。
    const legacyProxy = absorbLegacyProfileProxy(obj.ai);
    if (legacyProxy) obj.ai.proxy = legacyProxy;
    obj.ai.profiles = normalizeProfiles(obj.ai.profiles);
    obj.ai.style = { ...structuredClone(DEFAULTS.ai.style), ...(obj.ai.style || {}) };
    obj.ai.groupOrder = normAiGroupOrder(obj.ai.groupOrder);
    if (obj.ai.failover !== false) obj.ai.failover = true;
    obj.ai.active = Math.min(Math.max((obj.ai.active as number) | 0, 0), obj.ai.profiles.length - 1);
    stripReasoningArtifacts(obj.ai);
    if (obj.ai.display !== "reader") obj.ai.display = "chat";
    obj.tts = migrateTts(obj.tts);
    obj.uiTheme = obj.uiTheme === "light" || obj.uiTheme === "dark" ? obj.uiTheme : "system";
    obj.uiSkinLight = resolveSkinId(obj.uiSkinLight, "light");
    obj.uiSkinDark = resolveSkinId(obj.uiSkinDark, "dark");
    obj.readerSchemes = normalizeSchemes(obj.readerSchemes);
    obj.language = normLanguageSetting(obj.language);
    obj.settingsOrder = normSettingsOrder(obj.settingsOrder);
    // 跨窗广播/导出的都是「已脱敏 config」，敏感字段均为空。先记下本窗当前密钥，
    // 应用后对传入为空者回填，避免用空值覆盖已回填/待回填的凭据；
    // 真正的「清空密钥」由 tidepage-secrets-changed 信号（重读加密存储）兜底。
    const preservedSecrets = collectSecrets(config);
    for (const k of Object.keys(config) as (keyof AppConfig)[]) {
      (config as any)[k] = obj[k];
    }
    const incoming = collectSecrets(config);
    const keep: Record<string, string> = {};
    for (const k of Object.keys(preservedSecrets)) {
      if (preservedSecrets[k] !== "" && (incoming[k] ?? "") === "") keep[k] = preservedSecrets[k];
    }
    applySecrets(config, keep);
    applyUiTheme();
    // 记录为“已保存”（脱敏形态，与广播/落盘一致），避免收到自身广播回声时重复 apply / 再次广播
    lastSavedJson = JSON.stringify(stripSecrets(config));
    // 跨窗同步/导入同样物理清除废弃的“关闭思考”键（stripReasoningArtifacts），
    // 保证两端内存与 localStorage 均不再出现这些键，行为恒为 auto。
  } catch {
    /* ignore */
  }
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;
let lastSavedJson = "";
// 上次已写入加密存储的凭据快照：去重，避免每次保存都重复加密/广播敏感信号。
let lastSavedSecretsJson: string | null = null;

/** 立即写盘并广播（端点切换等需要“即时生效”的场景）。 */
export function saveConfigNow() {
  clearTimeout(saveTimer);
  try {
    // 凭据单独加密落盘；localStorage 与跨窗广播只保留脱敏配置（绝不含明文密钥）。
    const secretsJson = JSON.stringify(collectSecrets(config));
    const json = JSON.stringify(stripSecrets(config));
    localStorage.setItem(KEY, json);
    // 先广播脱敏 config（含新增/删除的端点列表），再发敏感信号：
    // 保证其它窗口先拿到新的 profiles/services，随后才能按 id 正确回填其密钥。
    if (json !== lastSavedJson) {
      lastSavedJson = json;
      void import("@tauri-apps/api/event")
        .then((m) => m.emit("tidepage-config", json))
        .catch(() => {});
    }
    if (secretsJson !== lastSavedSecretsJson) {
      lastSavedSecretsJson = secretsJson;
      void api.secretWrite(secretsJson).catch(() => {});
      // 无载荷信号：各窗（含自身）收到后重读加密存储并回填。
      void import("@tauri-apps/api/event")
        .then((m) => m.emit("tidepage-secrets-changed", null))
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
    m.listen<string>("tidepage-config", (e) => {
      if (e.payload && e.payload !== lastSavedJson) applyConfigJson(e.payload);
    }),
  )
  .catch(() => {});

// 敏感变更信号（无载荷）：任一窗口改动了凭据后，各窗重读加密存储并回填。
// 单独走信号而非随 config 广播，确保密钥不经广播明文、且空值不会误清已回填的密钥。
void import("@tauri-apps/api/event")
  .then((m) =>
    m.listen<null>("tidepage-secrets-changed", () => {
      void (async () => {
        try {
          const raw = await api.secretRead();
          if (raw) applySecrets(config, JSON.parse(raw) as Record<string, unknown>);
        } catch {
          /* 重读失败则保持现状 */
        }
      })();
    }),
  )
  .catch(() => {});

/* ===================== WebDAV 设置备份（手动 + 定期） ===================== */

/** 防重入：手动与定期共用，进行中则拒绝再次触发。 */
let settingsBackupRunning = false;

/** 确保 backup 子配置存在（旧内存/旧广播可能缺字段）。 */
function webdavBackupCfg(): WebdavBackupConfig {
  const wd = config.webdav as any;
  if (!wd.backup || typeof wd.backup !== "object") {
    wd.backup = { enabled: false, intervalHours: 24, lastAt: 0, lastSig: "", lastCheckAt: 0 };
  }
  return wd.backup as WebdavBackupConfig;
}

/**
 * 配置签名：以 stripSecrets(config) 剔除 webdav.backup 子对象后，连同 collectSecrets(config)
 * 做 FNV-1a 32 位哈希（十六进制）。只有「配置或凭据有实质变化」才改变签名；备份动作自身
 * 改 lastAt/lastSig/lastCheckAt 不改变签名，避免自我触发。
 */
function settingsSig(): string {
  const base = stripSecrets(config) as any;
  if (base?.webdav) delete base.webdav.backup;
  const s = JSON.stringify(base) + "\u0000" + JSON.stringify(collectSecrets(config));
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

/** 本地时间戳 `yyyyMMdd-HHmmss`（用于备份文件名）。 */
function backupStamp(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}` +
    `-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
  );
}

/**
 * 组装并上传设置备份（手动与改动检测共用同一分支）。
 * 文件名 `config-<设备名>-<本地时间>.json`；凭据用 WebDAV 账号派生密钥加密（跨设备可解），
 * 非密文配置脱敏。备份不限量，不再清理历史。返回写入的远端路径。
 */
export async function runSettingsBackup(): Promise<string> {
  // 抛 ASCII 错误码：stores 不能反向 import i18n（会与 config 形成初始化环），
  // 故这里只给稳定 code + 英文兜底 message，由调用方（SettingsModal）按 ERR_CODES 本地化。
  if (settingsBackupRunning) throw Object.assign(new Error("backup_running"), { code: "backup_running" });
  const wd = config.webdav;
  if (!wd.base.trim() || !wd.user.trim() || !wd.pass) {
    throw Object.assign(new Error("backup_need_creds"), { code: "backup_need_creds" });
  }
  settingsBackupRunning = true;
  try {
    const passphrase = `${wd.user}\u0000${wd.pass}`;
    const plaintext = JSON.stringify(collectSecrets(config));
    const enc = await api.backupSeal(plaintext, passphrase);
    const device = await api.hostName();
    const name = `config-${device}-${backupStamp(new Date())}.json`;
    const payload = { v: 2, at: Date.now(), device, config: stripSecrets(config), enc };
    const path = await api.webdavBackupSettings(
      wd.base,
      wd.user,
      wd.pass,
      name,
      JSON.stringify(payload),
    );
    const b = webdavBackupCfg();
    b.lastAt = Date.now();
    b.lastSig = settingsSig();
    b.lastError = undefined;
    saveConfigNow();
    return path;
  } finally {
    settingsBackupRunning = false;
  }
}

/** 改动检测：基础 tick 每 5 分钟；到检测周期后比对签名（剔除 backup 自身），
 *  配置或凭据有变化才真正上传。失败仅记 lastError，不弹硬错误。 */
function webdavBackupTick() {
  const wd = config.webdav;
  const b = webdavBackupCfg();
  if (!b.enabled) return;
  if (!wd.base.trim() || !wd.user.trim() || !wd.pass) return;
  const intervalMs = Math.max(1, Number(b.intervalHours) || 24) * 3_600_000;
  if (Date.now() - (b.lastCheckAt || 0) < intervalMs) return; // 未到检测周期
  b.lastCheckAt = Date.now();
  saveConfig();
  if (settingsSig() !== (b.lastSig || "")) {
    void runSettingsBackup().catch((e) => {
      b.lastError = String(e);
      saveConfig();
    });
  }
}

if (typeof window !== "undefined") {
  // 仅主窗口跑调度，避免多个窗口同时触发重复备份。
  void import("@tauri-apps/api/webviewWindow")
    .then((m) => m.getCurrentWebviewWindow().label)
    .then((label) => {
      if (label === "main") setInterval(webdavBackupTick, 5 * 60_000);
    })
    .catch(() => {});
}

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
import { AI_MODE_KEY, AI_OPEN_KEY, AI_WIN_H_KEY, AI_WIN_W_KEY } from "./aiPanelKeys";

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

/** 书库进出书后保留的会话级状态：搜索词、离开时的顶部锚点与滚动位置。
 *  书库与阅读界面是条件渲染，切进阅读会销毁书库组件，这些值放模块级才不会丢。
 *  只活在本次运行内，不写 localStorage（便携模式下各窗口的 localStorage 不共享）。 */
export const libSession = $state({
  queryInput: "",
  query: "",
  anchor: null as { id: number; rank: number } | null,
  scrollTop: 0,
});

export const convertState = $state({
  open: false,
  book: null as { id: number; title: string; path: string; format: string } | null,
});

export const AI_WIN_LABEL = "tidepage-ai";

export const aiState = $state({
  lastMode: (typeof localStorage !== "undefined"
    ? (localStorage.getItem(AI_MODE_KEY) as "drawer" | "window" | null)
    : null) ?? "drawer",
});

export function saveAiMode(m: "drawer" | "window") {
  aiState.lastMode = m;
  try {
    localStorage.setItem(AI_MODE_KEY, m);
  } catch { /* ignore */ }
}

/** 打开独立 AI 窗口。返回是否成功建窗（true=已建/建成，false=建窗失败需回退抽屉）。
 *  不再读写 ui.aiOpen：抽屉显隐一律由使用方（Reader.rightPanel）掌控。 */
export async function openAiWindow(): Promise<boolean> {
  saveAiMode("window");
  const w = Number(localStorage.getItem(AI_WIN_W_KEY)) || 430;
  const h = Number(localStorage.getItem(AI_WIN_H_KEY)) || 700;
  const extra = await winExtraOptions();
  return await new Promise<boolean>((resolve) => {
    try {
      const win = new WebviewWindow(AI_WIN_LABEL, {
        url: "/?ai=1",
        title: "TidePage AI",
        width: w,
        height: h,
        minWidth: 300,
        minHeight: 260,
        center: true,
        decorations: false, // 无原生标题栏：与主窗口一致，改用前端自绘窗口控制
        ...extra,
      });
      win.once("tauri://created", () => {
        localStorage.setItem(AI_OPEN_KEY, "1");
        try {
          emit("ai-window-status", { open: true });
        } catch { /* ignore */ }
        resolve(true);
      });
      win.once("tauri://error", () => {
        saveAiMode("drawer");
        resolve(false);
      });
    } catch {
      saveAiMode("drawer");
      resolve(false);
    }
  });
}

export function aiWindowOpenMark(): string | null {
  try {
    return localStorage.getItem(AI_OPEN_KEY);
  } catch {
    return null;
  }
}

/** 任务 5：把阅读界面面板拉出为独立窗口。复用 AI 窗创建范式（同 additional_browser_args）。
 *  URL 走 ?panel=xxx&book=<id>，由 +page.svelte standalone 挂载对应组件并按 book id 自拉数据。
 *  返回是否成功建窗（true=聚焦已有/建成，false=失败）。 */
export type PanelKind = "toc" | "bookmarks" | "highlights" | "stats" | "notes";

export async function openPanelWindow(panel: PanelKind, bookId: number, title: string): Promise<boolean> {
  const label = `panel-${panel}`;
  try {
    const ex = await WebviewWindow.getByLabel(label);
    if (ex) {
      // 窗口 label 与书无关（panel-<kind>），URL 里的 book 只在首次加载时生效：
      // 复用已有窗口必须显式广播「当前是哪本书」，否则那扇窗永远停在建窗时的那本。
      await emit("tidepage-book-changed", { bookId, title });
      await ex.unminimize().catch(() => {});
      await ex.show().catch(() => {});
      await ex.setFocus().catch(() => {});
      return true;
    }
  } catch { /* ignore */ }
  const extra = await winExtraOptions(); // 与 AI 窗一致：带主窗相同 additional_browser_args
  return await new Promise<boolean>((resolve) => {
    try {
      const win = new WebviewWindow(label, {
        url: `/?panel=${panel}&book=${bookId}`,
        title,
        width: 440,
        height: 660,
        minWidth: 300,
        minHeight: 260,
        center: true,
        decorations: false, // 无原生标题栏：与主窗口一致，改用前端自绘窗口控制
        ...extra,
      });
      win.once("tauri://created", () => resolve(true));
      win.once("tauri://error", () => resolve(false));
      // 新窗口的监听要等它 onMount 之后才存在：由面板发 tidepage-panel-ready，这里回一次当前书。
      void (async () => {
        try {
          const { listen } = await import("@tauri-apps/api/event");
          const un = await listen("tidepage-panel-ready", async () => {
            await emit("tidepage-book-changed", { bookId, title });
            un();
          });
        } catch { /* ignore */ }
      })();
    } catch {
      resolve(false);
    }
  });
}

export async function emit(_event: string, _payload: unknown) {
  try {
    await (await import("@tauri-apps/api/event")).emit(_event, _payload);
  } catch { /* ignore */ }
}

/** 切换 AI：返回 true=已以独立窗口呈现（聚焦已有/建窗成功）；false=需由调用方回退抽屉。
 *  不再改写 ui.aiOpen；抽屉态完全交由 Reader.rightPanel 管理。 */
export async function toggleAi(): Promise<boolean> {
  let mode = aiState.lastMode;
  try {
    const m = localStorage.getItem(AI_MODE_KEY);
    if (m === "drawer" || m === "window") mode = m as "drawer" | "window";
  } catch { /* ignore */ }
  if (mode === "window") {
    try {
      const w = await WebviewWindow.getByLabel(AI_WIN_LABEL);
      if (w) {
        await w.unminimize().catch(() => {});
        await w.show().catch(() => {});
        await w.setFocus().catch(() => {});
        localStorage.setItem(AI_OPEN_KEY, "1");
        return true;
      }
    } catch { /* ignore */ }
    try {
      const wins = await getAllWebviewWindows();
      const existing = wins.find((x) => x.label === AI_WIN_LABEL);
      if (existing) {
        existing.unminimize();
        existing.show();
        existing.setFocus();
        localStorage.setItem(AI_OPEN_KEY, "1");
        return true;
      }
    } catch { /* ignore */ }
    localStorage.setItem(AI_OPEN_KEY, "0");
    return await openAiWindow();
  }
  // drawer 模式：抽屉显隐由 Reader.rightPanel 掌控，这里不再改 ui.aiOpen；返回 false 让调用方开抽屉。
  return false;
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
      decorations: false, // 无原生标题栏：与主窗口一致，改用前端自绘窗口控制
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
