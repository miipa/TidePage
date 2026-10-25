import { describe, it, expect } from "vitest";
import { pageFromLeftPosition, pageFromRatio, ratioFromPage } from "./pagination";

describe("pagination", () => {
  it("ratioFromPage：单页/边界/clamp", () => {
    expect(ratioFromPage(0, 1)).toBe(0);
    expect(ratioFromPage(0, 0)).toBe(0);
    expect(ratioFromPage(0, 10)).toBe(0);
    expect(ratioFromPage(5, 10)).toBeCloseTo(5 / 9, 12);
    expect(ratioFromPage(1, 2)).toBe(1);
    expect(ratioFromPage(9, 10)).toBe(1);
    expect(ratioFromPage(99, 10)).toBe(1);
    expect(ratioFromPage(-3, 10)).toBe(0);
    expect(ratioFromPage(Number.NaN, 10)).toBe(0);
  });

  it("pageFromRatio：四舍五入/clamp/防御", () => {
    expect(pageFromRatio(0.5, 10)).toBe(5);
    expect(pageFromRatio(0.5, 1)).toBe(0);
    expect(pageFromRatio(1, 10)).toBe(9);
    expect(pageFromRatio(2, 10)).toBe(9);
    expect(pageFromRatio(-1, 10)).toBe(0);
    expect(pageFromRatio(Number.NaN, 10)).toBe(0);
    expect(pageFromRatio(0.5, 0)).toBe(0);
    // 与旧公式 Math.round(r * Math.max(1, pageCount - 1)) 结果一致（再夹取后）。
    for (const n of [1, 2, 7, 10]) {
      for (const r of [0, 0.1, 0.33, 0.5, 0.75, 0.99, 1]) {
        const old = Math.max(0, Math.min(Math.round(r * Math.max(1, n - 1)), Math.max(0, n - 1)));
        expect(pageFromRatio(r, n)).toBe(old);
      }
    }
  });

  it("roundtrip：同一页数下 page→ratio→page 不变", () => {
    for (const n of [1, 2, 7, 10, 101]) {
      for (let p = 0; p < n; p++) {
        expect(pageFromRatio(ratioFromPage(p, n), n)).toBe(p);
      }
    }
  });

  it("pageFromLeftPosition：偏移→页并 clamp", () => {
    expect(pageFromLeftPosition(0, 100, 10)).toBe(0);
    expect(pageFromLeftPosition(99, 100, 10)).toBe(0);
    expect(pageFromLeftPosition(100, 100, 10)).toBe(1);
    expect(pageFromLeftPosition(950, 100, 10)).toBe(9);
    expect(pageFromLeftPosition(99999, 100, 10)).toBe(9);
    expect(pageFromLeftPosition(-5, 100, 10)).toBe(0);
    expect(pageFromLeftPosition(50, 0, 10)).toBe(0);
    expect(pageFromLeftPosition(50, 100, 1)).toBe(0);
    expect(pageFromLeftPosition(Number.NaN, 100, 10)).toBe(0);
  });

  it("页数变化时按比例映射仍单调", () => {
    let last = -1;
    for (let p = 0; p < 10; p++) {
      const q = pageFromRatio(ratioFromPage(p, 10), 20);
      expect(q).toBeGreaterThanOrEqual(last);
      last = q;
    }
    expect(pageFromRatio(ratioFromPage(9, 10), 20)).toBe(19);
  });
});
