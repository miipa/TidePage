import { describe, it, expect } from "vitest";
import { MIN_AVAIL, MIN_COL_CHARS, MIN_COL_W, clampMarginH, minColWidth } from "./readLayout";

// 默认配置（stores.svelte.ts DEFAULTS.reader）：字号 32、列间距 48、边距每侧 320。
const P = { duo: false, colGap: 48, fontSize: 32 };

describe("minColWidth — 一屏排版所需的最小可用宽度", () => {
  it("单栏：字号 × 最少字符数（不小于列宽下限 40）", () => {
    expect(minColWidth(P)).toBe(32 * MIN_COL_CHARS);
  });
  it("双栏：两列各自的下限 + 一个列间距", () => {
    expect(minColWidth({ ...P, duo: true })).toBe(32 * MIN_COL_CHARS * 2 + 48);
  });
  it("字号极小时取绝对下限 160", () => {
    expect(minColWidth({ duo: false, colGap: 48, fontSize: 10 })).toBe(MIN_AVAIL);
  });
  it("字号/间距非法值不产生 NaN", () => {
    const v = minColWidth({ duo: false, colGap: Number.NaN, fontSize: Number.NaN });
    expect(Number.isFinite(v)).toBe(true);
    expect(v).toBe(MIN_AVAIL);
  });
});

describe("clampMarginH — 有效左右内缩", () => {
  it("宽窗口：不改动用户设置（320 仍是 320）", () => {
    expect(clampMarginH({ contentW: 1600, marginH: 320, ...P })).toBe(320);
    expect(clampMarginH({ contentW: 1200, marginH: 320, ...P })).toBe(320);
  });
  it("窄窗口：夹到「内容区仍留得下 minColWidth」的上限", () => {
    // 700 - 320*2 = 60 < 256 → 每侧只能留 (700-256)/2 = 222
    const mh = clampMarginH({ contentW: 700, marginH: 320, ...P });
    expect(mh).toBeCloseTo((700 - minColWidth(P)) / 2, 6);
    expect(mh).toBeLessThan(320);
  });
  it("不变式：留得下就满足 contentW - 2*mh ≥ minColWidth，留不下则夹到 0（不产生负宽）", () => {
    for (const W of [320, 500, 700, 900, 1200, 2000]) {
      for (const duo of [false, true]) {
        const o = { ...P, duo };
        const mh = clampMarginH({ contentW: W, marginH: 500, ...o });
        expect(mh).toBeGreaterThanOrEqual(0);
        if (W >= minColWidth(o)) {
          expect(W - mh * 2).toBeGreaterThanOrEqual(minColWidth(o) - 1e-9);
        } else {
          // 窗口本身比最小排版宽度还窄：边距全让出，正文铺满可视区（唯一可行解）
          expect(mh).toBe(0);
        }
      }
    }
  });
  it("永远不会放大用户设置（只收窄）", () => {
    expect(clampMarginH({ contentW: 2000, marginH: 12, ...P })).toBe(12);
    expect(clampMarginH({ contentW: 2000, marginH: 0, ...P })).toBe(0);
  });
  it("内容区比最小宽度还窄：夹到 0（宁可正文铺满，也不留负宽）", () => {
    expect(clampMarginH({ contentW: 200, marginH: 320, ...P })).toBe(0);
    expect(clampMarginH({ contentW: 0, marginH: 320, ...P })).toBe(0);
  });
  it("负边距/非法输入归一为 0", () => {
    expect(clampMarginH({ contentW: 900, marginH: -40, ...P })).toBe(0);
    expect(clampMarginH({ contentW: Number.NaN, marginH: 320, ...P })).toBe(0);
  });
  it("列宽下限 MIN_COL_W 生效：字号 1px 时仍按 40px 列宽夹取", () => {
    const o = { duo: false, colGap: 0, fontSize: 1 };
    expect(minColWidth(o)).toBe(MIN_AVAIL); // max(160, 40) = 160
    const mh = clampMarginH({ contentW: 400, marginH: 320, ...o });
    expect(mh).toBe((400 - MIN_AVAIL) / 2);
  });
});
