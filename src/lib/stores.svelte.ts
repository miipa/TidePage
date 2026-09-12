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

export interface ReaderConfig {
  mode: ReadMode;
  duo: boolean;
  anim: boolean;
  fontFamily: string;
  latinFont: string;
  fontSize: number;
  lineHeight: number;
  paraSpacing: number;
  marginH: number;
  marginV: number;
  indent: string;
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
  fauxBold: boolean;
  fauxItalic: boolean;
  letterSpacing: number;
  scrollBounce: boolean;
  scrollBounceStrength: number;
  scrollBounceThreshold: number;
  punctuationCompression: boolean;
  multiWindow: boolean;
  statusPos: "left" | "center" | "right" | "none";
  statusColor: string;
  topbarColor: string;
  conv: { title: string; author: string; css: string; coverPath: string };
}

export interface WebdavConfig {
  enabled: boolean;
  base: string;
  user: string;
  pass: string;
  syncOnOpen: boolean;
  uploadBooks: boolean;
  downloadDir: string;
}

export interface AiProfile {
  name: string;
  provider: "openai" | "ollama";
  baseUrl: string;
  apiKey: string;
  model: string;
  proxyMode: "auto" | "direct" | "custom";
  proxyUrl: string;
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
  reasoningMode: AiReasoningMode;
}

export type AiReasoningMode = "auto" | "off";

// 仅按协议判定的单一事实源：Ollama /api/chat 明确记录 think 参数；
// 通用 OpenAI-compatible 端点没有统一明确的“关闭思考”字段（reasoning_effort
// 仅部分 OpenAI 模型支持，第三方端点可能以未知字段拒绝），不得盲塞。
export function providerSupportsReasoningOff(provider: string): boolean {
  return provider === "ollama";
}

// 单一能力闸门：当激活端点不支持明确关闭思考时，把持久化的 off 归一为 auto，
// 保证不会“保存 off 后每次发送都报错”。在 load/applyConfigJson/端点切换处统一调用。
export function normalizeAiCapability(ai: AiConfig): AiConfig {
  const active = ai.profiles[Math.min(ai.active | 0, ai.profiles.length - 1)] ?? ai.profiles[0];
  if (ai.reasoningMode === "off" && (!active || !providerSupportsReasoningOff(active.provider))) {
    ai.reasoningMode = "auto";
  }
  return ai;
}

export const DEFAULT_AI_PROFILES: AiProfile[] = [
  {
    name: "DeepSeek",
    provider: "openai",
    baseUrl: "https://api.deepseek.com",
    apiKey: "",
    model: "deepseek-chat",
    proxyMode: "auto",
    proxyUrl: "",
  },
];

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
    anim: true,
    fontFamily: '"Microsoft YaHei", "Noto Sans CJK SC", "PingFang SC", sans-serif',
    latinFont: "",
    fontSize: 29,
    lineHeight: 1.55,
    paraSpacing: 0.5,
    marginH: 468,
    marginV: 51,
    indent: "1em",
    theme: "paper",
    bgColor: "#FDF5E6",
    fgColor: "#2c2c2c",
    bgImage: "",
    bgOpacity: 0.35,
    cssExtra: `#chapter-content p { color: rgba(0,0,0,0.95); font-weight: 300; }

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
    zones: structuredClone(DEFAULT_ZONES),
    zoneHints: false,
    customFonts: [],
    aiPanelWidth: 360,
    fauxBold: false,
    fauxItalic: false,
    letterSpacing: 0.05,
    scrollBounce: true,
    scrollBounceStrength: 0.5,
    scrollBounceThreshold: 340,
    punctuationCompression: false,
    multiWindow: false,
    statusPos: "center",
    statusColor: "",
    topbarColor: "",
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
    reasoningMode: "auto",
  },
  regex: "",
};

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
  if (base.reasoningMode !== "off") base.reasoningMode = "auto";
  const hasNew = Array.isArray(savedAi?.profiles) && savedAi.profiles.length;
  if (hasNew) {
    base.profiles = savedAi.profiles.map((p: any) => ({
      ...structuredClone(DEFAULT_AI_PROFILES[0]),
      ...p,
    }));
    if (Array.isArray(savedAi.prompts) && savedAi.prompts.length) {
      base.prompts = savedAi.prompts;
    }
    base.active = Math.min(Math.max(savedAi.active | 0, 0), base.profiles.length - 1);
    return normalizeAiCapability(base);
  }
  let profiles: AiProfile[];
  const legacyUrl = savedAi?.baseUrl || savedAi?.apiKey || savedAi?.model;
  if (legacyUrl) {
    const pm = savedAi?.proxyMode === "custom" ? "custom" : "auto";
    profiles = [
      {
        name: "默认",
        provider: savedAi?.provider ?? "openai",
        baseUrl: savedAi?.baseUrl ?? "",
        apiKey: savedAi?.apiKey ?? "",
        model: savedAi?.model ?? "",
        proxyMode: pm,
        proxyUrl: savedAi?.proxyUrl ?? "",
      },
    ];
  } else {
    profiles = structuredClone(DEFAULT_AI_PROFILES);
  }
  const prompts = structuredClone(DEFAULT_PROMPTS);
  if (savedAi?.promptSummary) prompts[0].text = savedAi.promptSummary;
  if (savedAi?.promptTranslate) prompts[1].text = savedAi.promptTranslate;
  if (savedAi?.action === "custom" && savedAi?.customPrompt) {
    prompts.push({ id: 99, name: "自定义", key: "F9", text: savedAi.customPrompt });
  }
  return normalizeAiCapability({
    enabled: !!savedAi?.enabled,
    autoOnChapter: savedAi?.autoOnChapter ?? true,
    system: savedAi?.system ?? DEFAULTS.ai.system,
    profiles,
    active: 0,
    prompts,
    display: "reader",
    style: structuredClone(DEFAULTS.ai.style),
    reasoningMode: savedAi?.reasoningMode === "off" ? "off" : "auto",
  });
}

function load(): AppConfig {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return structuredClone(DEFAULTS);
    const saved = JSON.parse(raw);
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
    // 一次性迁移：字距旧默认 0.045 → 0.05（v0.1.20 起 0.05 为确定默认值）。
    // 仅当存量值【恰好等于旧默认】时上抬；用户手动调过的其他值（含 0.044/0.046）不动。
    // flag 防重复执行。
    try {
      if (!localStorage.getItem("flowread.lsmig.v1")) {
        if (merged.reader.letterSpacing === 0.045) merged.reader.letterSpacing = 0.05;
        localStorage.setItem("flowread.lsmig.v1", "1");
      }
    } catch { /* ignore */ }
    return merged;
  } catch {
    return structuredClone(DEFAULTS);
  }
}

export const config = $state<AppConfig>(load());

export function applyConfigJson(json: string) {
  try {
    if (json === JSON.stringify(config)) return;
    const obj = JSON.parse(json);
    if (!obj || typeof obj !== "object" || !obj.reader || !obj.ai) return;
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
    obj.ai = { ...structuredClone(DEFAULTS.ai), ...obj.ai };
    obj.ai.style = { ...structuredClone(DEFAULTS.ai.style), ...(obj.ai.style || {}) };
    if (obj.ai.reasoningMode !== "off") obj.ai.reasoningMode = "auto";
    if (obj.ai.display !== "reader") obj.ai.display = "chat";
    for (const k of Object.keys(config) as (keyof AppConfig)[]) {
      (config as any)[k] = obj[k];
    }
    normalizeAiCapability(config.ai);
  } catch {
    /* ignore */
  }
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;
let lastSavedJson = "";
export function saveConfig() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      const json = JSON.stringify(config);
      localStorage.setItem(KEY, json);
      if (json !== lastSavedJson) {
        lastSavedJson = json;
        void import("@tauri-apps/api/event")
          .then((m) => m.emitTo(AI_WIN_LABEL, "flowread-config", json))
          .catch(() => {});
      }
    } catch {
      /* ignore */
    }
  }, 300);
}

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

export const APP_BUILD = "2026.09.11d";

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
