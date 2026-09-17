import { config } from "./stores.svelte";

const STYLE_ID = "flowread-fontface-css";

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
        `@font-face { font-family: "${f.family}"; src: url('http://flowread.localhost/fonts/${f.file}'); }`,
    )
    .join("\n");
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
