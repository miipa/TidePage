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
