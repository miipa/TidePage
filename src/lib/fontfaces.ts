import { config } from "./stores.svelte";

const STYLE_ID = "tidepage-fontface-css";

export interface CustomFontItem {
  family: string;
  file: string;
}

/** 生成自定义字体的 @font-face 规则（供正文与独立 AI 窗口共用，唯一样式来源）。 */
export function customFontFaceCss(fonts?: CustomFontItem[]): string {
  const list = fonts ?? config.reader.customFonts ?? [];
  return list
    .filter((f) => f && f.family && f.file)
    .map(
      (f) =>
        // font-display:block：字体未就绪时先不显示回退字体，避免「先别字体再变」的闪烁
        `@font-face { font-family: "${f.family}"; src: url('http://tidepage.localhost/fonts/${f.file}'); font-display: block; }`,
    )
    .join("\n");
}

/** 把 font-family 列表拆成单个字体名（去掉引号）。 */
function splitFamilies(raw: string): string[] {
  return (raw || "")
    .split(",")
    .map((s) => s.trim().replace(/^["']|["']$/g, ""))
    .filter(Boolean);
}

/** 项目自己的字体回退链 —— **必须与 app.css `:root` 的 font-family 逐字一致**
 *  （"Microsoft YaHei", "Segoe UI", system-ui, sans-serif；中文优先：雅黑承担中文字形，
 *  Segoe UI 只兜拉丁/符号，避免中英混排字形割裂）。CSS 变量引用不了 TS 常量，
 *  两处只能靠纪律同步。
 *  ⚠️ 全项目只有这一处实现（Reader 与 AiPanel 都从这里 import）；改回退链时只需要同时改
 *     这里与 app.css :root 两处。 */
const FONT_FALLBACK = '"Microsoft YaHei", "Segoe UI", system-ui, sans-serif';

/**
 * 把配置里的字体设置拼成一条 CSS font-family 栈，**末尾永远接上项目自己的回退链**。
 * 消费点有两条：Reader 的 `--rdr-font`（正文）与 AiPanel 的 `--ai-font`（AI 面板排版）。
 *
 * 为什么必须有回退链：这两处都是给元素**整条**指定 font-family，会顶掉从 app.css 继承来的
 * 那一份。若用户选的字体加载失败而栈里没有退路，浏览器只能落到自己的默认字（通常是衬线），
 * 中文可能整片变豆腐。旧实现是 `if (!latin) return base` —— 直接返回用户填的那一个字族，
 * 零回退；那条硬编码的 fallback 只在 base 为空时才够得着，而实际几乎永远走不到。
 * 另一个连带好处：字体**缺部分字形**时，那些字形现在落到雅黑而不是浏览器默认字。
 */
export function fontStack(raw: string, latinFirst?: string): string {
  const norm = (list: string[]) => list
    .map((s) => s.trim().replace(/^["']|["']$/g, ""))
    .filter(Boolean)
    .map((s) => (/^[A-Za-z][A-Za-z0-9-]*$/.test(s) ? s : `"${s}"`))
    .join(", ");
  const base = norm((raw || "").split(","));
  const latin = norm([latinFirst || ""]);
  // 无论走哪个分支都接上 FONT_FALLBACK：加载失败、缺字、用户只填了西文字体时都有退路。
  // 用户自填的栈里若已有同名项会出现重复 —— CSS 允许重复且先出现者先生效，故不影响解析。
  const stack = [latin, base].filter(Boolean).join(", ");
  return stack ? `${stack}, ${FONT_FALLBACK}` : FONT_FALLBACK;
}

/**
 * 预加载阅读界面常用的字体（正文字体 + 西文字体）。
 * 在渲染章节正文之前 await，可保证首次打开书籍时直接以设定字体呈现，不出现回退字体闪烁。
 * 系统字体没有 @font-face，load() 会立即完成；用户安装的字体则会真正拉取并缓存。
 */
export async function preloadReaderFonts(timeoutMs = 3000): Promise<void> {
  if (typeof document === "undefined" || !document.fonts) return;
  const families = [
    ...splitFamilies(config.reader.fontFamily),
    ...splitFamilies(config.reader.latinFont),
  ];
  if (!families.length) return;
  const size = Math.max(12, Math.min(64, config.reader.fontSize || 16));
  const jobs = families.map((f) =>
    document.fonts.load(`${size}px "${f}"`).catch(() => undefined),
  );
  const timeout = new Promise<void>((r) => setTimeout(r, timeoutMs));
  await Promise.race([Promise.all(jobs).then(() => undefined), timeout]);
}

/**
 * 把当前自定义字体的 @font-face 注入 document.head。
 * 主窗口、独立阅读窗口、独立 AI 窗口（?ai=1）都在 +page.svelte 顶层调用，
 * 保证未挂载 Reader 组件的 AI 窗口也能加载用户安装的字体。
 */
export function syncCustomFontFaces(): void {
  if (typeof document === "undefined") return;
  const css = customFontFaceCss();
  let el = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!css) {
    el?.remove();
    return;
  }
  if (!el) {
    el = document.createElement("style");
    el.id = STYLE_ID;
    document.head.appendChild(el);
  }
  el.textContent = css;
}
