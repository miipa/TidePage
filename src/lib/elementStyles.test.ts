import { describe, it, expect } from "vitest";
import {
  DEFAULT_ELEMENT_ACCENT,
  DEFAULT_ELEMENT_CSS,
  DEFAULT_ELEMENT_STYLES,
  ELEMENT_GROUPS,
  ELEMENT_PRESETS,
  ELEMENT_STYLE_KEYS,
  buildEpubCss,
  elementStyleActive,
  normalizeElementCss,
  normalizeElementStyles,
} from "./elementStyles";

describe("element styles", () => {
  it("每个预设的每个变体都在对应元素的可选值里", () => {
    for (const preset of ELEMENT_PRESETS) {
      for (const group of ELEMENT_GROUPS) {
        expect(group.options.map((o) => o.v), preset.id + "." + group.key).toContain(preset.state[group.key]);
      }
    }
  });

  it("归一化：缺失补默认，非法键被过滤，自定义 CSS 只接受字符串", () => {
    expect(normalizeElementStyles(undefined)).toEqual(DEFAULT_ELEMENT_STYLES);
    expect(normalizeElementStyles({ heading: "seal", nope: "x" })).toEqual({ ...DEFAULT_ELEMENT_STYLES, heading: "seal" });
    expect(normalizeElementCss({ heading: "#chapter-content h2{}", hr: 3 })).toEqual({ ...DEFAULT_ELEMENT_CSS, heading: "#chapter-content h2{}" });
  });

  it("预设激活判断逐项比较", () => {
    expect(elementStyleActive(DEFAULT_ELEMENT_STYLES, ELEMENT_PRESETS[0])).toBe(true);
    expect(elementStyleActive({ ...DEFAULT_ELEMENT_STYLES, heading: "double" }, ELEMENT_PRESETS[0])).toBe(false);
  });

  it("EPUB 导出样式包含排版、标题效果，并把 #chapter-content 作用域改为正文", () => {
    const css = buildEpubCss({
      fontFamily: "lxgwwenkai-regular",
      latinFont: "",
      fontSize: 30.5,
      fontWeight: 400,
      lineHeight: 1.55,
      paraSpacing: 0.5,
      indent: "1em",
      textAlign: "justify",
      bgColor: "#f5f1e8",
      fgColor: "#3b3630",
      elementStyles: { ...DEFAULT_ELEMENT_STYLES, heading: "double" },
      cssExtra: "#chapter-content p { color: #111; }",
      elementCss: { ...DEFAULT_ELEMENT_CSS, heading: "#chapter-content h2 { color: #f00; }" },
    });
    expect(css).toContain("font-size: 30.5px");
    expect(css).toContain("border-bottom: 3px double");
    expect(css).toContain("p { color: #111; }");
    expect(css).toContain("h2 { color: #f00; }");
    expect(css).not.toContain("#chapter-content h2");
  });

  it("每个预设都有强调色，极简预设已删除", () => {
    expect(ELEMENT_PRESETS.every((p) => /^#[0-9a-fA-F]{6}$/.test(p.accent))).toBe(true);
    expect(ELEMENT_PRESETS.some((p) => p.id === "minimal")).toBe(false);
    expect(ELEMENT_PRESETS.find((p) => p.id === "soft")?.accent).toBe("#a06848");
    expect(ELEMENT_PRESETS.find((p) => p.id === "default")?.accent).toBe(DEFAULT_ELEMENT_ACCENT);
  });

  it("所有元素的预设/默认键完整", () => {
    expect(ELEMENT_STYLE_KEYS.every((k) => typeof DEFAULT_ELEMENT_STYLES[k] === "string")).toBe(true);
    expect(ELEMENT_STYLE_KEYS.every((k) => typeof DEFAULT_ELEMENT_CSS[k] === "string")).toBe(true);
  });
});
