import { describe, it, expect } from "vitest";
import {
  READ_FIX_BG_SELECTOR,
  READ_FIX_TEXT_SELECTOR,
  buildReadFixCss,
} from "./readFix";

// 任务二：荧光笔（mark.fr-hl-background）的底色必须活过 __read_fix 的背景 !important。
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

  it("背景选择器排除 mark.fr-hl / mark.fr-tts 自身与全部后代（内部 <font> 会被 extractContents 搬进来）", () => {
    for (const sel of [
      ":not(mark.fr-hl)",
      ":not(mark.fr-hl *)",
      ":not(mark.fr-tts)",
      ":not(mark.fr-tts *)",
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
