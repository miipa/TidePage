import { describe, it, expect } from "vitest";
import { TYPESETTING_READER_KEYS, pickSchemeTypesetting } from "./readerScheme";

type Reader = Record<string, unknown>;

const defaults: Reader = {
  fontFamily: "default-font",
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
  cssExtra: "default-css",
  overrideBookCss: true,
  multiWindow: false,
  browserSearchUrl: "https://example.com/s?q=%s",
  aiPanelWidth: 360,
  conv: { title: "", author: "", css: "", coverPath: "" },
};

const current: Reader = {
  ...defaults,
  fontSize: 28,
  theme: "night",
  multiWindow: true,
  browserSearchUrl: "https://my.search/?q=%s",
  aiPanelWidth: 480,
  conv: { title: "t", author: "a", css: "c", coverPath: "p" },
};

describe("pickSchemeTypesetting", () => {
  it("非排版字段保持不变，排版字段被方案更新", () => {
    const scheme: Reader = {
      ...defaults,
      fontFamily: "scheme-font",
      fontSize: 36,
      theme: "sepia",
      bgColor: "#eeddcc",
    };
    const out = pickSchemeTypesetting(scheme, current, defaults);
    expect(out.fontFamily).toBe("scheme-font");
    expect(out.fontSize).toBe(36);
    expect(out.theme).toBe("sepia");
    expect(out.bgColor).toBe("#eeddcc");
    // 非排版字段全部保留当前值
    expect(out.multiWindow).toBe(true);
    expect(out.browserSearchUrl).toBe("https://my.search/?q=%s");
    expect(out.aiPanelWidth).toBe(480);
    expect(out.conv).toEqual(current.conv);
    // 白名单集合与交付说明一致
    expect(TYPESETTING_READER_KEYS).toContain("overrideBookCss");
    expect(TYPESETTING_READER_KEYS).not.toContain("multiWindow");
    expect(TYPESETTING_READER_KEYS).not.toContain("conv");
    expect(TYPESETTING_READER_KEYS).not.toContain("aiPanelWidth");
  });

  it("方案缺字段用默认值补齐，不写 undefined", () => {
    const out = pickSchemeTypesetting({ fontSize: 40 }, current, defaults);
    expect(out.fontSize).toBe(40);
    expect(out.fontFamily).toBe(defaults.fontFamily);
    expect(out.margins).toEqual(defaults.margins);
    expect(out.overrideBookCss).toBe(true);
    expect(Object.values(out).some((v) => v === undefined)).toBe(false);
  });

  it("不修改入参；对象字段深拷贝不共享引用", () => {
    const scheme = { ...defaults, margins: { scroll: { h: 1, v: 2 } } };
    const beforeScheme = JSON.stringify(scheme);
    const beforeCurrent = JSON.stringify(current);
    const out = pickSchemeTypesetting(scheme, current, defaults);
    expect(JSON.stringify(scheme)).toBe(beforeScheme);
    expect(JSON.stringify(current)).toBe(beforeCurrent);
    (out.margins as Record<string, unknown>).scroll = { h: 9, v: 9 };
    expect(JSON.stringify(scheme)).toBe(beforeScheme);
    expect((current.margins as Record<string, unknown>).scroll).toEqual({ h: 320, v: 80 });
  });
});
