import { describe, it, expect } from "vitest";
import {
  READ_FIX_BG_SELECTOR,
  READ_FIX_TEXT_SELECTOR,
  buildReadFixCss,
  sanitizeCssText,
} from "./readFix";

// 任务二：荧光笔（mark.tp-hl-background）的底色必须活过 __read_fix 的背景 !important。
// vitest 是 node 环境（无 jsdom），因此只做字符串级断言：真正生效与否由 _test 的 CDP live 脚本
// 用 computed style 验收。

const O = {
  fontFamily: '"Noto Serif SC", serif',
  fontSize: 32,
  lineHeight: 1.75,
  color: "#222",
  background: "#f6f2ea",
};

/** 把 CSS 文本拆成 { selector, body }（本模块输出的规则里没有嵌套块）。 */
function rules(css: string): { selector: string; body: string }[] {
  return Array.from(css.matchAll(/([^{}]+)\{([^{}]*)\}/g), (m) => ({
    selector: m[1].trim(),
    body: m[2],
  }));
}
const hasBackground = (r: { body: string }) => /(^|[;\s])background\s*:/.test(r.body);

describe("buildReadFixCss — 背景规则必须放过划线标记及其后代", () => {
  const css = buildReadFixCss(O);

  it("背景选择器排除 mark.tp-hl / mark.tp-tts 自身与全部后代（内部 <font> 会被 extractContents 搬进来）", () => {
    for (const sel of [
      ":not(mark.tp-hl)",
      ":not(mark.tp-hl *)",
      ":not(mark.tp-tts)",
      ":not(mark.tp-tts *)",
    ]) {
      expect(READ_FIX_BG_SELECTOR).toContain(sel);
      expect(css).toContain(sel);
    }
    expect(READ_FIX_BG_SELECTOR.startsWith("#chapter-content")).toBe(true);
  });

  it("四项覆盖规则仍是不含 :not( 的通用后代选择器", () => {
    const textRule = rules(css).find((r) => r.selector === READ_FIX_TEXT_SELECTOR);
    expect(textRule).toBeDefined();
    expect(READ_FIX_TEXT_SELECTOR).toBe("#chapter-content, #chapter-content *");
    expect(textRule!.selector).not.toContain(":not(");
    for (const decl of ["font-family", "font-size", "line-height", "color"]) {
      expect(textRule!.body).toContain(decl + ":");
    }
    expect(hasBackground(textRule!)).toBe(false);
  });

  it("写 background 的规则只有两条：背景选择器本身 + html/body", () => {
    const bgRules = rules(css).filter(hasBackground);
    expect(bgRules.map((r) => r.selector)).toEqual([
      READ_FIX_BG_SELECTOR,
      "html, body",
    ]);
  });

  it("不存在第二条会给 #chapter-content 后代写 background 的规则", () => {
    const others = rules(css)
      .filter(hasBackground)
      .filter((r) => r.selector.includes("#chapter-content") && r.selector !== READ_FIX_BG_SELECTOR);
    expect(others).toEqual([]);
  });

  it("背景值仍是阅读底色（未被规则拆分改动）", () => {
    for (const r of rules(css).filter(hasBackground)) {
      expect(r.body).toContain(`background: ${O.background} !important;`);
    }
  });
});

describe("buildReadFixCss — F7 CSS 值白名单", () => {
  const fontOf = (css: string) => /font-family: ([^;]+) !important;/.exec(css)?.[1];
  const colorOf = (css: string) => /color: ([^;]+) !important;/.exec(css)?.[1];
  const sizeOf = (css: string) => /font-size: ([^;]+) !important;/.exec(css)?.[1];
  const lhOf = (css: string) => /line-height: ([^;]+) !important;/.exec(css)?.[1];
  const bgOf = (css: string) => /html, body { background: ([^;]+) !important; }/.exec(css)?.[1];

  it("合法字体栈逐字节不变（默认值不回退）", () => {
    const css = buildReadFixCss(O);
    expect(fontOf(css)).toBe('"Noto Serif SC", serif');
    expect(css).toContain('font-family: Consolas, "Courier New", monospace !important;');
  });

  it("自定义等宽字体栈合法时保留", () => {
    const css = buildReadFixCss({ ...O, monoFont: 'Consolas, "Courier New", monospace' });
    expect(css).toContain('font-family: Consolas, "Courier New", monospace !important;');
  });

  it("合法颜色值逐字节保留", () => {
    for (const v of ["#abc", "#abcd", "#aabbcc", "#aabbccdd", "transparent", "rgb(0, 0, 0)", "rgba(1, 2, 3, 0.5)", "hsl(0, 0%, 0%)", "hsla(0, 0%, 0%, 0.3)", "red", "RED"]) {
      const css = buildReadFixCss({ ...O, color: v, background: v });
      expect(colorOf(css), v).toBe(v);
      expect(bgOf(css), v).toBe(v);
    }
  });

  it("注入型颜色/背景回退默认（不做转义改写）", () => {
    for (const bad of [
      "red; } body { background: url(https://evil.example/x) }",
      "url(x)",
      "rgb(0,0,0);",
      "expression(alert(1))",
      "var(--x)",
      "#12 #34",
    ]) {
      const css = buildReadFixCss({ ...O, color: bad, background: bad });
      expect(colorOf(css), bad).toBe("#1a1a1a");
      expect(bgOf(css), bad).toBe("transparent");
    }
  });

  it("字体栈含 ;{}() 或 url 子串或反斜杠时回退 serif", () => {
    for (const bad of [
      'Arial; } body { background: red',
      "Arial(url)",
      "url(x)",
      "Arial{}",
      "Arial()",
      "Arial\\\\Backslash",
      "Arial: x",
    ]) {
      const css = buildReadFixCss({ ...O, fontFamily: bad, monoFont: bad });
      expect(fontOf(css), bad).toBe("serif");
      expect(css).toContain("font-family: Consolas");
    }
    // 空字体栈也回退。
    expect(fontOf(buildReadFixCss({ ...O, fontFamily: "" }))).toBe("serif");
  });

  it("Unicode 自定义字体族名逐字节通过（第 5 轮回归）", () => {
    const cases = [
      '"思源宋体", serif',
      '"源ノ明朝", serif',
      '"Noto Serif SC", 思源宋体, serif',
      "思源宋体",
    ];
    for (const v of cases) {
      const css = buildReadFixCss({ ...O, fontFamily: v, monoFont: v });
      expect(fontOf(css), v).toBe(v);
      expect(css, v).toContain('font-family: ' + v + ' !important;');
    }
  });

  it("Unicode 字体名中的危险字符仍回退默认", () => {
    for (const bad of ['思源宋体; } body {', '思源宋体(url)', '思源宋体{}', '思源宋体\\x', '思源宋体()', '思源 宋体\u00a0x']) {
      const css = buildReadFixCss({ ...O, fontFamily: bad });
      expect(fontOf(css), bad).toBe("serif");
    }
  });

  it("fontSize / lineHeight 非法回退", () => {
    for (const bad of [Number.NaN, -1, 0, Number.POSITIVE_INFINITY]) {
      expect(sizeOf(buildReadFixCss({ ...O, fontSize: bad })), String(bad)).toBe("32px");
    }
    expect(lhOf(buildReadFixCss({ ...O, lineHeight: Number.NaN }))).toBe("1.5");
    expect(lhOf(buildReadFixCss({ ...O, lineHeight: 0 }))).toBe("1.5");
    expect(lhOf(buildReadFixCss({ ...O, lineHeight: -0.5 }))).toBe("1.5");
    expect(lhOf(buildReadFixCss({ ...O, lineHeight: "1.75" }))).toBe("1.75");
    expect(lhOf(buildReadFixCss({ ...O, lineHeight: "1.75abc" }))).toBe("1.5");
    expect(lhOf(buildReadFixCss({ ...O, lineHeight: "0" }))).toBe("1.5");
  });

  it("sanitizeCssText 去控制字符与结构字符，成对引号不被改写", () => {
    expect(sanitizeCssText('"Noto Serif SC"')).toBe('"Noto Serif SC"');
    expect(sanitizeCssText("a\tb\nc")).toBe("a b c");
    expect(sanitizeCssText("x{};<>y")).toBe("xy");
    expect(sanitizeCssText("A\u0000B")).toBe("AB");
  });
});
