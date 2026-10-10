// 阅读器排版硬修复：旧式 MOBI 的 <font face/size/color>（presentational hint）会压过阅读器
// 的字体/字号/行高/颜色/背景设置，表现为「字体设置失效」。仅加在 #chapter-content 上的字体
// 会被后代元素自身的内联样式/属性盖掉，因此这里做两件事，缺一不可：
//
//   1) 用通用后代选择器 `#chapter-content *`（!important）全量覆盖五项：
//      font-family / font-size / line-height / color / background；
//      背景额外覆盖 html / body / #chapter-content / #chapter-content * 共四层。
//      ——刻意不用标签白名单（p/div/span/li/...）：白名单漏掉任一标签就会穿透。
//   2) DOM 就绪后直接 removeAttribute 剥离 <font> 的 face / size / color：
//      属性不存在 → presentational hint 不存在，比再加样式更彻底。
//
// 幂等：固定 id="__read_fix"，重复调用只替换 textContent，绝不 append。
// 注入点不依赖 </head>：用 document.getElementById("chapter-content") 判空，
// 为空时 console.warn("[fixfont] NO #chapter-content") 并返回，不静默。

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
  /** 背景颜色 */
  background: string;
}

export const READ_FIX_STYLE_ID = "__read_fix";

const DEFAULT_MONO = "Consolas, \"Courier New\", monospace";

/** 拼出最终下发到页面的 CSS。font-family 放在首行，便于 READ_CSS 的 head 直接核对新值。 */
export function buildReadFixCss(o: ReadFixOptions): string {
  const FONT_FAMILY = o.fontFamily;
  const MONO_FONT = o.monoFont || DEFAULT_MONO;
  const SIZE = o.fontSize;
  const LH = o.lineHeight;
  const BG = o.background;
  const TEXT = o.color;
  return [
    // 五项全量覆盖：命中 #chapter-content 及其所有后代
    "#chapter-content, #chapter-content * {",
    `  font-family: ${FONT_FAMILY} !important;`,
    `  font-size: ${SIZE}px !important;`,
    `  line-height: ${LH} !important;`,
    `  color: ${TEXT} !important;`,
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

/** 移除已注入的修复样式（例如关闭「覆盖书籍排版」时）。 */
export function clearReadFix(): void {
  document.getElementById(READ_FIX_STYLE_ID)?.remove();
}

/**
 * 应用排版修复。必须在 #chapter-content 已进入 DOM 之后调用。
 * 幂等：重复调用只替换 __read_fix 的 textContent。
 * @returns 成功返回 { stripped, html }；找不到 #chapter-content 返回 null。
 */
export function applyReadFix(o: ReadFixOptions): { stripped: number; html: number } | null {
  const root = document.getElementById("chapter-content");
  if (!root) {
    console.warn("[fixfont] NO #chapter-content");
    return null;
  }
  const stripped = stripLegacyFontAttrs(root);
  const css = buildReadFixCss(o);
  // 配置下发链路日志：最终 CSS 长度 + 前 120 字符（font-family 在首行，便于核对新值）
  console.log("READ_CSS", `len=${css.length} head=${css.slice(0, 120)}`);
  let tag = document.getElementById(READ_FIX_STYLE_ID) as HTMLStyleElement | null;
  if (!tag) {
    tag = document.createElement("style");
    tag.id = READ_FIX_STYLE_ID;
    document.head.appendChild(tag);
  }
  tag.textContent = css;
  const html = root.innerHTML.length;
  console.log(`[fixfont] injected, stripped=${stripped}, html=${html}`);
  return { stripped, html };
}
