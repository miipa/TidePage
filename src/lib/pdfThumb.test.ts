import { describe, it, expect } from "vitest";
import {
  clampPage,
  normalizeThumbsOpen,
  rowOfPage,
  thumbLruStep,
  visibleThumbRange,
  visibleThumbRows,
} from "./pdfThumb";

describe("pdfThumb", () => {
  it("可见范围夹取", () => {
    expect(visibleThumbRange(0, 10, 100, 5)).toEqual({ start: 0, end: 15 });
    expect(visibleThumbRange(50, 10, 100, 5)).toEqual({ start: 45, end: 65 });
    expect(visibleThumbRange(98, 10, 100, 5)).toEqual({ start: 93, end: 100 });
    expect(visibleThumbRange(0, 0, 0)).toEqual({ start: 0, end: 0 });
  });

  it("LRU 淘汰与命中刷新", () => {
    expect(thumbLruStep(["a", "b", "c"], "d", 3)).toEqual({ order: ["b", "c", "d"], evicted: ["a"] });
    expect(thumbLruStep(["a", "b", "c"], "a", 3)).toEqual({ order: ["b", "c", "a"], evicted: [] });
    expect(thumbLruStep([], "a", 1).order).toEqual(["a"]);
  });

  it("网格虚拟滚动按行计算", () => {
    expect(visibleThumbRows(0, 600, 150, 2, 100, 1)).toEqual({ startRow: 0, endRow: 5, rows: 50 });
    expect(visibleThumbRows(1500, 600, 150, 2, 100, 1)).toMatchObject({ startRow: 9, endRow: 15 });
    expect(visibleThumbRows(0, 600, 150, 2, 0).rows).toBe(0);
    expect(rowOfPage(5, 2)).toBe(2);
  });

  it("打开态与页码归一", () => {
    expect(normalizeThumbsOpen("1")).toBe(true);
    expect(normalizeThumbsOpen("0")).toBe(false);
    expect(normalizeThumbsOpen(null)).toBe(false);
    expect(clampPage(-5, 10)).toBe(0);
    expect(clampPage(99, 10)).toBe(9);
    expect(clampPage(3, 0)).toBe(0);
  });
});
