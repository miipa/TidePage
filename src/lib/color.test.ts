import { describe, it, expect } from "vitest";
import { parseColor, normalizeColor, toHex, toRgbString, safeColor } from "./color";

// 特征化测试（characterization）：固定 color.ts 当前行为，而非理想行为。
// 期望值均对照现有实现实际输出推导。

describe("parseColor — hex", () => {
  it("#rgb 展开为 #rrggbb", () => {
    expect(parseColor("#abc")).toEqual({ r: 170, g: 187, b: 204, a: 1 });
  });
  it("#rrggbb", () => {
    expect(parseColor("#1a2b3c")).toEqual({ r: 26, g: 43, b: 60, a: 1 });
  });
  it("#rgba：第 4 位十六进制展开并归一到 0..1", () => {
    const c = parseColor("#abcd")!;
    expect(c.r).toBe(170);
    expect(c.g).toBe(187);
    expect(c.b).toBe(204);
    expect(c.a).toBeCloseTo(221 / 255, 6); // ≈0.8667
  });
  it("#rrggbbaa：alpha = AA/255", () => {
    const c = parseColor("#1a2b3c80")!;
    expect(c).toMatchObject({ r: 26, g: 43, b: 60 });
    expect(c.a).toBeCloseTo(128 / 255, 6); // ≈0.50196
  });
  it("大小写不敏感 + 两端空白", () => {
    expect(parseColor("  #ABCDEF  ")).toEqual({ r: 171, g: 205, b: 239, a: 1 });
  });
  it("非法 hex → null", () => {
    expect(parseColor("#12")).toBeNull(); // 位数 < 3
    expect(parseColor("#12345")).toBeNull(); // 位数非 3/4/6/8
    expect(parseColor("#1234567")).toBeNull();
  });
});

describe("parseColor — rgb()/rgba()", () => {
  it("逗号分隔整数", () => {
    expect(parseColor("rgb(10, 20, 30)")).toEqual({ r: 10, g: 20, b: 30, a: 1 });
  });
  it("百分比通道 ×2.55 后四舍五入并钳制（50% 因浮点为 127.4999… 四舍五入得 127）", () => {
    expect(parseColor("rgb(50%, 0%, 100%)")).toEqual({ r: 127, g: 0, b: 255, a: 1 });
  });
  it("rgba 带 alpha", () => {
    expect(parseColor("rgba(0, 0, 0, 0.5)")).toEqual({ r: 0, g: 0, b: 0, a: 0.5 });
  });
  it("空白/斜杠分隔（现代语法）", () => {
    expect(parseColor("rgba(10 20 30 / 0.25)")).toEqual({ r: 10, g: 20, b: 30, a: 0.25 });
  });
  it("alpha 百分比 → /100", () => {
    expect(parseColor("rgba(0,0,0,50%)")).toEqual({ r: 0, g: 0, b: 0, a: 0.5 });
  });
  it("通道越界钳制到 0..255", () => {
    expect(parseColor("rgb(300, -5, 20)")).toEqual({ r: 255, g: 0, b: 20, a: 1 });
  });
  it("alpha 越界钳制到 0..1", () => {
    expect(parseColor("rgba(0,0,0,2)")).toEqual({ r: 0, g: 0, b: 0, a: 1 });
    expect(parseColor("rgba(0,0,0,-1)")).toEqual({ r: 0, g: 0, b: 0, a: 0 });
  });
  it("参数不足 → null", () => {
    expect(parseColor("rgb(1, 2)")).toBeNull();
    expect(parseColor("rgb()")).toBeNull();
  });
});

describe("parseColor — 其它非法输入", () => {
  it("空串/未知字符串 → null", () => {
    expect(parseColor("")).toBeNull();
    expect(parseColor("   ")).toBeNull();
    expect(parseColor("nope")).toBeNull();
  });
});

describe("toHex", () => {
  it("通道两位十六进制，忽略 alpha", () => {
    expect(toHex({ r: 255, g: 0, b: 16, a: 1 })).toBe("#ff0010");
    expect(toHex({ r: 170, g: 187, b: 204, a: 0.5 })).toBe("#aabbcc");
  });
  it("越界通道在输出时钳制", () => {
    expect(toHex({ r: 300, g: -5, b: 16, a: 1 })).toBe("#ff0010");
  });
});

describe("toRgbString", () => {
  it("a>=1 → rgb()", () => {
    expect(toRgbString({ r: 10, g: 20, b: 30, a: 1 })).toBe("rgb(10, 20, 30)");
  });
  it("a<1 → rgba()，alpha 两位四舍五入", () => {
    expect(toRgbString({ r: 10, g: 20, b: 30, a: 0.5 })).toBe("rgba(10, 20, 30, 0.5)");
    expect(toRgbString({ r: 0, g: 0, b: 0, a: 221 / 255 })).toBe("rgba(0, 0, 0, 0.87)");
    expect(toRgbString({ r: 1, g: 2, b: 3, a: 0.125 })).toBe("rgba(1, 2, 3, 0.13)");
  });
});

describe("normalizeColor", () => {
  it("不透明 → #rrggbb", () => {
    expect(normalizeColor("#abc")).toBe("#aabbcc");
    expect(normalizeColor("rgb(10,20,30)")).toBe("#0a141e");
  });
  it("半透明 → rgba() 字符串", () => {
    expect(normalizeColor("#00000080")).toBe("rgba(0, 0, 0, 0.5)");
    expect(normalizeColor("rgba(10,20,30,0.5)")).toBe("rgba(10, 20, 30, 0.5)");
  });
  it("非法 → null", () => {
    expect(normalizeColor("nope")).toBeNull();
  });
});

describe("safeColor", () => {
  it("合法 → 归一化值", () => {
    expect(safeColor("#abc", "#000000")).toBe("#aabbcc");
  });
  it("非法 → 回退 fallback（原样返回）", () => {
    expect(safeColor("nope", "#123456")).toBe("#123456");
    expect(safeColor("", "transparent")).toBe("transparent");
  });
});
