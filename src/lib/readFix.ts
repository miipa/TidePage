// 阅读器排版硬修复：旧式 MOBI 的 <font face/size/color>（presentational hint）会压过阅读器
// 的字体/字号/行高/颜色/背景设置，表现为「字体设置失效」。仅加在 #chapter-content 上的字体
// 会被后代元素自身的内联样式/属性盖掉，因此这里做两件事，缺一不可：
//
//   1) 用通用后代选择器 `#chapter-content *`（!important）全量覆盖五项：
//      font-family / font-size / line-height / color / background；
//      背景额外覆盖 html / body / #chapter-content / #chapter-content * 共四层。
//      ——刻意不用标签白名单（p/div/span/li/...）：白名单漏掉任一标签就会穿透。
//      唯一的例外是划线标记（mark.tp-hl / mark.tp-tts）**及其后代**：背景单独成一条规则并
//      排除它们（见 READ_FIX_BG_SELECTOR）。否则 "荧光笔" 样式的底色会被这条 !important
//      涂成页面底色；且 paintHighlights() 会把原书的 <font>/<span> 搬进 mark 内部，
//      只排除 mark 自身盖不住它们。四项覆盖不排除任何元素。
//   2) DOM 就绪后直接 removeAttribute 剥离 <font> 的 face / size / color：
//      属性不存在 → presentational hint 不存在，比再加样式更彻底。
//
// 幂等：固定 id="__read_fix"，重复调用只替换 textContent，绝不 append。
// 注入点不依赖 </head>：用 document.getElementById("chapter-content") 判空，
// 为空时返回 null（仅 DEV 下打一行 [fixfont] 警示）——生产构建不残留排版日志。

export interface ReadFixOptions {
  /** 正文字体栈（CSS font-family 值） */
  fontFamily: string;
  /** 等宽字体栈（代码块） */
  monoFont?: string;
  /** 字号 px */
  fontSize: number;
  /** 行高（无单位倍数） */
  lineHeight: number | string;
  /** 文字颜色 */
  color: string;
  /** 背景颜色。设了阅读背景图时传 `transparent`：这条规则带 !important，
   *  不透明底色会把图片挡在正文后面（边距区却能看到图 → 正文内外不同源）。 */
  background: string;
}

export const READ_FIX_STYLE_ID = "__read_fix";

/** 四项覆盖（字体/字号/行高/字色）的选择器：#chapter-content 及其全部后代，无例外。 */
export const READ_FIX_TEXT_SELECTOR = "#chapter-content, #chapter-content *";

/**
 * 背景覆盖的选择器：排除划线标记（mark.tp-hl / mark.tp-tts）**及其全部后代**。
 *
 * 为什么必须排除后代：paintHighlights()（src/lib/highlights.ts）用 range.extractContents()
 * 把原书的 <font>/<span> 搬进 mark 内部，它们仍会命中 `:not(mark.tp-hl)` 之后的 `*` 规则，
 * 把荧光笔的底色整块盖回页面底色 —— 只排除 mark 自身等于没排除。
 * 也不采用「给 mark.tp-hl-background 补 !important」：盖不住内部元素的背景，
 * 还会搅乱 box/note 圆点那几层的层叠。
 */
export const READ_FIX_BG_SELECTOR =
  "#chapter-content, #chapter-content *:not(mark.tp-hl):not(mark.tp-hl *):not(mark.tp-tts):not(mark.tp-tts *)";

const DEFAULT_MONO = "Consolas, \"Courier New\", monospace";
/** F7：非法值回退用的安全常量（不随配置变化）。 */
const DEFAULT_FONT_FAMILY = "serif";
const DEFAULT_COLOR = "#1a1a1a";
const DEFAULT_SIZE = 32;
const DEFAULT_LINE_HEIGHT = 1.5;

/** F7：通用 CSS 文本净化：去掉 NUL、把 CR/LF/TAB 变空格、去掉 { } ; < > 结构字符。
 *  刻意**不改写成对引号**，合法字体栈/颜色值必须逐字节保持不变。 */
export function sanitizeCssText(raw: string): string {
  if (typeof raw !== "string") return "";
  let out = "";
  for (const c of raw) {
    if (c === "\u0000") continue;
    if (c === "\r" || c === "\n" || c === "\t") {
      out += " ";
      continue;
    }
    if (c === "{" || c === "}" || c === ";" || c === "<" || c === ">") continue;
    out += c;
  }
  return out;
}

/** 字体栈白名单：Unicode 字母/数字（支持中文、日文等自定义字体族名）+ 空格、逗号、点、
 *  单/双引号、下划线、连字符。含括号/斜杠/反斜杠/冒号/等号/控制字符/非 ASCII 空格或 url 子串
 *  一律回退 —— 防止注入额外 CSS 规则；合法值不做任何转义改写。 */
function safeFontStack(raw: string | undefined, fallback: string): string {
  if (typeof raw !== "string" || !raw) return fallback;
  for (const c of raw) {
    // JS 字符串没有 isAlphanumeric；用 Unicode 属性字母/数字（中文、日文等均属 p{L}）。
    if (/[\p{L}\p{N}]/u.test(c)) continue;
    if (" ,.'\"-".includes(c)) continue;
    return fallback;
  }
  if (/url/i.test(raw)) return fallback;
  // 白名单已保证无控制字符/结构字符；再过一遍通用净化（合法值逐字节不变）。
  return sanitizeCssText(raw);
}

/** 常见 CSS 颜色名白名单（写死；来源：CSS Color Module Level 4 的 16 基本色 + 常用灰阶）。
 *  只用于判断「输入是否是安全的颜色字面量」，不参与渲染。 */
const CSS_COLOR_NAMES = new Set([
  "transparent", "currentcolor", "white", "black", "red", "green", "blue", "gray", "grey",
  "silver", "maroon", "olive", "lime", "aqua", "teal", "navy", "fuchsia", "purple", "orange",
  "yellow", "pink", "brown", "gold", "cyan", "magenta", "violet", "indigo", "beige", "ivory",
  "khaki", "lavender", "salmon", "tan", "turquoise", "crimson", "orchid", "plum", "coral",
  "darkred", "darkgreen", "darkblue", "darkgray", "darkgrey", "lightgray", "lightgrey",
]);

/** 颜色值白名单：transparent/命名色/#RGB(A)/#RRGGBB(AA)/rgb()/rgba()/hsl()/hsla()。
 *  rgb 类函数内部只允许数字/逗号/百分号/空格/点；其它字符（含 ; } url( expression(）拒绝。 */
function safeColor(raw: string | undefined, fallback: string): string {
  if (typeof raw !== "string") return fallback;
  const v = raw.trim();
  if (!v) return fallback;
  const lower = v.toLowerCase();
  if (CSS_COLOR_NAMES.has(lower)) return v;
  if (/^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(v)) return v;
  const fn = /^(rgb|rgba|hsl|hsla)\(([0-9.,%\s]+)\)$/.exec(v);
  if (fn) return sanitizeCssText(v);
  return fallback;
}

function safeFontSize(raw: number): number {
  return Number.isFinite(raw) && raw > 0 ? raw : DEFAULT_SIZE;
}

function safeLineHeight(raw: number | string): number | string {
  if (typeof raw === "number") {
    return Number.isFinite(raw) && raw > 0 ? raw : DEFAULT_LINE_HEIGHT;
  }
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (/^\d*\.?\d+$/.test(trimmed)) {
      const n = Number.parseFloat(trimmed);
      if (Number.isFinite(n) && n > 0) return trimmed;
    }
  }
  return DEFAULT_LINE_HEIGHT;
}

/** 拼出最终下发到页面的 CSS。font-family 放在首行，便于 READ_CSS 的 head 直接核对新值。 */
export function buildReadFixCss(o: ReadFixOptions): string {
  // F7：配置可经导入/备份恢复进入运行态，所有插值先按字段白名单校验；非法值回退默认，
  // 合法值不做引号/反斜杠改写（逐字节保留）。这里不做「整体转义」——那会改变合法字体栈。
  const FONT_FAMILY = safeFontStack(o.fontFamily, DEFAULT_FONT_FAMILY);
  const MONO_FONT = safeFontStack(o.monoFont || DEFAULT_MONO, DEFAULT_MONO);
  const SIZE = safeFontSize(o.fontSize);
  const LH = safeLineHeight(o.lineHeight);
  const BG = safeColor(o.background, "transparent");
  const TEXT = safeColor(o.color, DEFAULT_COLOR);
  return [
    // 四项全量覆盖：命中 #chapter-content 及其所有后代，不排除任何元素
    `${READ_FIX_TEXT_SELECTOR} {`,
    `  font-family: ${FONT_FAMILY} !important;`,
    `  font-size: ${SIZE}px !important;`,
    `  line-height: ${LH} !important;`,
    `  color: ${TEXT} !important;`,
    "}",
    // 背景单独一条并排除划线标记及其后代：否则 "荧光笔" 的底色会被涂成页面底色
    `${READ_FIX_BG_SELECTOR} {`,
    `  background: ${BG} !important;`,
    "}",
    // 背景四层：html / body / #chapter-content（上面已含）/ #chapter-content *（上面已含）
    `html, body { background: ${BG} !important; }`,
    // 代码块保留等宽（可选增强，不影响五项覆盖）
    `#chapter-content :is(pre, code, kbd, samp, tt) { font-family: ${MONO_FONT} !important; }`,
  ].join("\n");
}

/** 剥离 root 内所有 <font> 的 face/size/color 属性；返回被剥离的属性个数。 */
export function stripLegacyFontAttrs(root: ParentNode): number {
  let stripped = 0;
  for (const el of Array.from(root.querySelectorAll("font"))) {
    for (const attr of ["face", "size", "color"]) {
      if (el.hasAttribute(attr)) {
        el.removeAttribute(attr);
        stripped++;
      }
    }
  }
  return stripped;
}

/** P2-6：上次注入 CSS 对应的配置签名；签名不变时跳过重建（换章不再重复拼 CSS/写 textContent）。 */
let lastCssKey: string | null = null;

/** 计算影响最终 CSS 的配置签名（顺序固定；非法值按 buildReadFixCss 同款回退后再参与）。 */
export function readFixOptionsKey(o: ReadFixOptions): string {
  return JSON.stringify([
    safeFontStack(o.fontFamily, DEFAULT_FONT_FAMILY),
    safeFontStack(o.monoFont || DEFAULT_MONO, DEFAULT_MONO),
    safeFontSize(o.fontSize),
    safeLineHeight(o.lineHeight),
    safeColor(o.color, DEFAULT_COLOR),
    safeColor(o.background, "transparent"),
  ]);
}

/** 移除已注入的修复样式（例如关闭「覆盖书籍排版」时）。 */
export function clearReadFix(): void {
  document.getElementById(READ_FIX_STYLE_ID)?.remove();
  lastCssKey = null;
}

/**
 * 应用排版修复。必须在 #chapter-content 已进入 DOM 之后调用。
 * 幂等：重复调用只替换 __read_fix 的 textContent。
 * @returns 成功返回 { stripped, html }；找不到 #chapter-content 返回 null。
 */
export function applyReadFix(o: ReadFixOptions): { stripped: number; html: number } | null {
  const root = document.getElementById("chapter-content");
  if (!root) {
    if (import.meta.env.DEV) console.warn("[fixfont] NO #chapter-content");
    return null;
  }
  // 每章都必须剥离 <font> 属性（新章 DOM 未清理）；CSS 只在首次或配置变化时重建。
  const stripped = stripLegacyFontAttrs(root);
  const key = readFixOptionsKey(o);
  let tag = document.getElementById(READ_FIX_STYLE_ID) as HTMLStyleElement | null;
  if (!tag || lastCssKey !== key) {
    const css = buildReadFixCss(o);
    // 配置下发链路日志：最终 CSS 长度 + 前 120 字符（font-family 在首行，便于核对新值）；仅 DEV。
    if (import.meta.env.DEV) console.log("READ_CSS", `len=${css.length} head=${css.slice(0, 120)}`);
    if (!tag) {
      tag = document.createElement("style");
      tag.id = READ_FIX_STYLE_ID;
      document.head.appendChild(tag);
    }
    tag.textContent = css;
    lastCssKey = key;
  }
  // P2-6：生产构建不做整章 innerHTML 序列化（仅 DEV 日志需要）；Vite 会消除该分支。
  let html = 0;
  if (import.meta.env.DEV) {
    html = root.innerHTML.length;
    console.log(`[fixfont] injected, stripped=${stripped}, html=${html}`);
  }
  return { stripped, html };
}
