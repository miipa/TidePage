// 阅读元素样式：配置、预设与 EPUB 导出样式生成。
// 元素特效的 CSS 主体在 elementStylesCss.ts（由设计稿转换，按 data-el-* 选择器命中阅读器）。
// 这里只放数据与纯函数，避免与 stores 形成循环依赖。

export type ElementStyleKey =
  | "heading" | "subhead" | "lead" | "quote" | "list" | "image"
  | "table" | "link" | "em" | "hr" | "chrome" | "footnote";

export type ElementStyles = Record<ElementStyleKey, string>;
export type ElementCustomCss = Record<ElementStyleKey, string>;

export interface ElementOption { v: string; t: string }
export interface ElementGroup {
  key: ElementStyleKey;
  label: string;
  code: string;
  group: "structure" | "body" | "media" | "inline" | "chrome";
  target: string;
  options: ElementOption[];
}

export const ELEMENT_STYLE_KEYS: ElementStyleKey[] = [
  "heading", "subhead", "lead", "quote", "list", "image",
  "table", "link", "em", "hr", "chrome", "footnote",
];

export const DEFAULT_ELEMENT_STYLES: ElementStyles = {
  heading: "default", subhead: "default", lead: "default", quote: "default",
  list: "default", image: "default", table: "default", link: "default",
  em: "default", hr: "default", chrome: "default", footnote: "default",
};

export const DEFAULT_ELEMENT_CSS: ElementCustomCss = {
  heading: "", subhead: "", lead: "", quote: "", list: "", image: "",
  table: "", link: "", em: "", hr: "", chrome: "", footnote: "",
};

export const ELEMENT_GROUPS: ElementGroup[] = [
  { key: "heading", label: "章节标题", code: "h2", group: "structure", target: "h2", options: [
    { v: "default", t: "默认" }, { v: "rule", t: "单线" }, { v: "double", t: "双线" }, { v: "numbered", t: "编号" },
    { v: "gradient", t: "渐变" }, { v: "seal", t: "朱印框" }, { v: "tape", t: "胶带贴" }, { v: "invert", t: "反白块" }, { v: "plain", t: "无装饰" },
  ]},
  { key: "subhead", label: "小节标题", code: "h3", group: "structure", target: "h3", options: [
    { v: "default", t: "默认" }, { v: "bar", t: "左竖标" }, { v: "pill", t: "胶囊徽章" }, { v: "centerline", t: "双翼短线" }, { v: "plain", t: "无装饰" },
  ]},
  { key: "lead", label: "正文首段", code: "p:first", group: "body", target: "p:first-of-type", options: [
    { v: "default", t: "默认" }, { v: "dropcap", t: "首字下沉" }, { v: "stamp", t: "朱印首字" }, { v: "symbol", t: "段首符号" },
    { v: "leadline", t: "首行提亮" }, { v: "none", t: "无处理" },
  ]},
  { key: "quote", label: "引用段落", code: "blockquote", group: "body", target: "blockquote", options: [
    { v: "default", t: "默认" }, { v: "leftline", t: "左边线" }, { v: "bigquote", t: "大引号" }, { v: "card", t: "浮雕卡片" },
    { v: "center", t: "居中诗体" }, { v: "manuscript", t: "朱批框" }, { v: "note", t: "便利贴" }, { v: "brutal", t: "硬阴影" },
  ]},
  { key: "list", label: "列表样式", code: "ul / li", group: "body", target: "ul", options: [
    { v: "default", t: "默认" }, { v: "dot", t: "圆点" }, { v: "dash", t: "短横" }, { v: "rule", t: "左竖线" },
    { v: "numbered", t: "序号" }, { v: "check", t: "勾选框" }, { v: "ornament", t: "菱花" },
  ]},
  { key: "image", label: "插图画框", code: "figure", group: "media", target: "figure", options: [
    { v: "default", t: "默认" }, { v: "rounded", t: "圆角" }, { v: "paper", t: "白卡衬底" }, { v: "full", t: "通栏" },
    { v: "fold", t: "内嵌框" }, { v: "polaroid", t: "拍立得" }, { v: "film", t: "复古胶片" },
  ]},
  { key: "table", label: "数据表格", code: "table", group: "media", target: "table", options: [
    { v: "default", t: "默认" }, { v: "line", t: "极简横线" }, { v: "threeline", t: "三线学术" }, { v: "zebra", t: "斑马纹" }, { v: "grid", t: "乌丝方格" },
  ]},
  { key: "link", label: "行内链接", code: "a", group: "inline", target: "a", options: [
    { v: "default", t: "默认" }, { v: "underline", t: "细下划线" }, { v: "arrow", t: "外链箭头" }, { v: "highlight", t: "荧光底" },
    { v: "dashed", t: "虚线批注" }, { v: "badge", t: "胶囊签" }, { v: "color", t: "纯色" },
  ]},
  { key: "em", label: "重点强调", code: "mark.fx", group: "inline", target: "mark.fx", options: [
    { v: "default", t: "默认" }, { v: "mark", t: "半高荧光" }, { v: "underline", t: "粗底线" }, { v: "color", t: "变色加粗" },
    { v: "bracket", t: "直角引号" }, { v: "dot", t: "着重圆点" }, { v: "circle", t: "朱笔圈选" },
  ]},
  { key: "hr", label: "章节分隔", code: "hr", group: "inline", target: "hr", options: [
    { v: "default", t: "默认" }, { v: "line", t: "细线" }, { v: "dots", t: "三点" }, { v: "diamond", t: "单菱形" },
    { v: "scroll", t: "卷轴花纹" }, { v: "cut", t: "裁切线" }, { v: "space", t: "纯留白" },
  ]},
  { key: "chrome", label: "页眉页脚", code: ".topbar / .footer", group: "chrome", target: ".topbar", options: [
    { v: "default", t: "默认" }, { v: "classic", t: "书卷双线" }, { v: "magazine", t: "杂志粗线" }, { v: "minimal", t: "极简隐匿" },
    { v: "guji", t: "线装书耳" }, { v: "journal", t: "手账网格" },
  ]},
  { key: "footnote", label: "脚注弹窗", code: ".tp-fn-pop", group: "chrome", target: ".tp-fn-pop", options: [
    { v: "default", t: "默认" }, { v: "paper", t: "纸卡" }, { v: "seal", t: "朱印" },
    { v: "ink", t: "墨色" }, { v: "minimal", t: "极简" },
  ]},
];

/** 元素样式的默认强调色（= 网页原型默认/书卷预设的 accent）。 */
export const DEFAULT_ELEMENT_ACCENT = "#8a5a3b";

export interface ElementPreset {
  id: string;
  name: string;
  sub: string;
  /** 整套方案的元素强调色；写入 reader.elementAccent，阅读器以 --fx-accent 注入。 */
  accent: string;
  state: ElementStyles;
}

function all(over: Partial<ElementStyles>): ElementStyles {
  return Object.assign({}, DEFAULT_ELEMENT_STYLES, over);
}

export const ELEMENT_PRESETS: ElementPreset[] = [
  { id: "default", name: "默认", sub: "不额外装饰", accent: "#8a5a3b", state: all({}) },
  { id: "classic", name: "书卷", sub: "双线 · 首字 · 大引号", accent: "#8a5a3b", state: all({
    heading: "double", subhead: "plain", lead: "dropcap", quote: "bigquote", list: "rule",
    image: "paper", table: "line", link: "underline", hr: "diamond", em: "underline", chrome: "classic",
  })},
  { id: "guji", name: "古籍", sub: "朱印 · 批注框 · 乌丝表", accent: "#b03a2e", state: all({
    heading: "seal", subhead: "centerline", lead: "stamp", quote: "manuscript", list: "ornament",
    image: "fold", table: "grid", link: "dashed", hr: "scroll", em: "dot", chrome: "guji",
  })},
  { id: "magazine", name: "杂志", sub: "大编号 · 卡片 · 通栏", accent: "#2e6b62", state: all({
    heading: "numbered", subhead: "bar", lead: "leadline", quote: "card", list: "numbered",
    image: "full", table: "zebra", link: "arrow", hr: "line", em: "mark", chrome: "magazine",
  })},
  { id: "journal", name: "手账", sub: "胶带 · 拍立得 · 便利贴", accent: "#587246", state: all({
    heading: "tape", subhead: "pill", lead: "symbol", quote: "note", list: "check",
    image: "polaroid", table: "zebra", link: "badge", hr: "cut", em: "circle", chrome: "journal",
  })},
  { id: "academic", name: "学术", sub: "竖标 · 三线表 · 序号", accent: "#2b4c7e", state: all({
    heading: "rule", subhead: "bar", lead: "none", quote: "leftline", list: "numbered",
    image: "paper", table: "threeline", link: "underline", hr: "line", em: "underline", chrome: "classic",
  })},
  { id: "avant", name: "画报", sub: "反白 · 硬阴影 · 胶片", accent: "#c84b31", state: all({
    heading: "invert", subhead: "pill", lead: "stamp", quote: "brutal", list: "numbered",
    image: "film", table: "threeline", link: "arrow", hr: "cut", em: "mark", chrome: "magazine",
  })},
  { id: "soft", name: "柔和", sub: "渐变 · 符号 · 三点", accent: "#a06848", state: all({
    heading: "gradient", subhead: "centerline", lead: "symbol", quote: "leftline", list: "dot",
    image: "rounded", table: "line", link: "highlight", hr: "dots", em: "bracket", chrome: "classic",
  })},
];

/** 每种元素的自定义 CSS 模板（完整规则；注入时原样拼到动态 CSS 末尾）。 */
export const ELEMENT_CODE_TEMPLATES: ElementCustomCss = {
  heading: "#chapter-content :is(.tp-el-heading, h2) {\n  \n}",
  subhead: "#chapter-content :is(.tp-el-subhead, h3) {\n  \n}",
  lead: "#chapter-content .tp-el-lead {\n  \n}",
  quote: "#chapter-content blockquote {\n  \n}",
  list: "#chapter-content ul {\n  \n}\n#chapter-content li {\n  \n}",
  image: "#chapter-content img,\n#chapter-content figure {\n  \n}",
  table: "#chapter-content table {\n  \n}",
  link: "#chapter-content a {\n  \n}",
  em: "#chapter-content :is(em, strong, b, i, mark.fx) {\n  \n}",
  hr: "#chapter-content :is(.tp-el-hr, hr) {\n  \n}",
  chrome: ".reader .topbar,\n.reader .footer {\n  \n}",
  footnote: ".tp-fn-pop {\n  \n}",
};

export function normalizeElementStyles(v: unknown): ElementStyles {
  const out = structuredClone(DEFAULT_ELEMENT_STYLES);
  if (!v || typeof v !== "object") return out;
  for (const key of ELEMENT_STYLE_KEYS) {
    const val = (v as Record<string, unknown>)[key];
    if (typeof val === "string" && val) out[key] = val;
  }
  return out;
}

export function normalizeElementCss(v: unknown): ElementCustomCss {
  const out = structuredClone(DEFAULT_ELEMENT_CSS);
  if (!v || typeof v !== "object") return out;
  for (const key of ELEMENT_STYLE_KEYS) {
    const val = (v as Record<string, unknown>)[key];
    if (typeof val === "string") out[key] = val;
  }
  return out;
}

export function elementStyleActive(styles: ElementStyles, preset: ElementPreset): boolean {
  return ELEMENT_STYLE_KEYS.every((k) => styles[k] === preset.state[k]);
}

function stripContentScope(s: string): string {
  return s
    .replace(/#chapter-content\s*>\s*/g, "")
    .replace(/#chapter-content\s+/g, "")
    .replace(/#chapter-content/g, "body");
}

/** EPUB 导出用样式：排版基础 + 标题/首段效果 + 用户自定义 CSS。 */
export function buildEpubCss(r: {
  fontFamily: string; latinFont: string; fontSize: number; fontWeight: number;
  lineHeight: number; paraSpacing: number; indent: string; textAlign: string;
  bgColor: string; fgColor: string; elementStyles?: Partial<ElementStyles>;
  cssExtra?: string; elementCss?: Partial<ElementCustomCss>;
}): string {
  const styles = Object.assign({}, DEFAULT_ELEMENT_STYLES, r.elementStyles || {});
  const accent = "#8a5a3b";
  const fg = r.fgColor || "#2c2c2c";
  const bg = r.bgColor || "#FDF5E6";
  const family = (r.fontFamily || "").trim() || "serif";
  const lines: string[] = [];
  const indent = r.textAlign === "left" ? "0" : r.indent || "1em";
  lines.push("body { margin: 0 auto; padding: 1em 0.8em; max-width: 42em; font-family: " + family + ", serif; font-size: " + r.fontSize + "px; font-weight: " + r.fontWeight + "; line-height: " + r.lineHeight + "; color: " + fg + "; background: " + bg + "; text-align: " + r.textAlign + "; }");
  lines.push("p { margin: 0 0 " + r.paraSpacing + "em; text-indent: " + indent + "; line-height: inherit; }");
  lines.push("p:first-of-type { text-indent: 0; }");
  const h = styles.heading;
  if (h === "plain") {
    lines.push("h2 { font-size: 1.2em; font-weight: 600; text-align: center; margin: 0.4em 0 0.8em; border: 0; }");
  } else if (h === "rule" || h === "numbered") {
    lines.push("h2 { font-size: 1.2em; font-weight: 600; text-align: left; margin: 1em 0 0.8em; padding-bottom: .4em; border-bottom: 1px solid " + accent + "; }");
  } else if (h === "double") {
    lines.push("h2 { font-size: 1.2em; font-weight: 600; text-align: center; margin: 1em 0 0.8em; padding-bottom: .4em; border-bottom: 3px double " + accent + "; }");
  } else if (h === "gradient") {
    lines.push("h2 { font-size: 1.2em; font-weight: 600; text-align: center; margin: 1em 0 1.1em; color: " + accent + "; }");
  } else if (h === "seal") {
    lines.push("h2 { display: table; margin: 1em auto; padding: .6em 1em; border: 2px solid " + accent + "; font-size: 1.2em; font-weight: 600; text-align: center; }");
  } else if (h === "tape" || h === "invert") {
    lines.push("h2 { margin: 1em 0; padding: .7em 1em; background: " + (h === "invert" ? fg : "color-mix(in srgb, " + accent + " 12%, " + bg + ")") + "; color: " + (h === "invert" ? bg : fg) + "; font-size: 1.2em; font-weight: 600; text-align: left; }");
  } else {
    lines.push("h2 { font-size: 1.2em; font-weight: 600; text-align: center; margin: 0.4em 0 0.8em; }");
  }
  const lead = styles.lead;
  if (lead === "dropcap" || lead === "stamp") {
    lines.push("p:first-of-type::first-letter { float: left; font-size: 2.9em; line-height: .86; padding: .05em .12em 0 0; color: " + accent + "; font-weight: 600; }");
  } else if (lead === "symbol") {
    lines.push("p:first-of-type::before { content: \"❖\"; display: block; color: " + accent + "; letter-spacing: .35em; }");
  } else if (lead === "leadline") {
    lines.push("p:first-of-type::first-line { color: " + accent + "; font-weight: 600; }");
  }
  if (r.cssExtra && r.cssExtra.trim()) lines.push(stripContentScope(r.cssExtra));
  for (const key of ELEMENT_STYLE_KEYS) {
    const code = r.elementCss && r.elementCss[key];
    if (code && code.trim()) lines.push(stripContentScope(code));
  }
  return lines.join("\n");
}
