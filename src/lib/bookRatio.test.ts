import { describe, it, expect } from "vitest";
import { windowedBookRatio } from "./bookRatio";

describe("windowedBookRatio（A4）", () => {
  it("单章窗口：起始章 + 章内比例", () => {
    expect(windowedBookRatio(0, 1, 0, 10)).toBe(0);
    expect(windowedBookRatio(0, 1, 1, 10)).toBeCloseTo(0.1);
    expect(windowedBookRatio(4, 1, 0.5, 10)).toBeCloseTo(0.45);
  });
  it("多章窗口：比例按窗口章数铺开", () => {
    expect(windowedBookRatio(0, 2, 1, 10)).toBeCloseTo(0.2);
    expect(windowedBookRatio(2, 2, 0.5, 10)).toBeCloseTo(0.3);
  });
  it("边界：比例 0/1、单章书、非法值夹取", () => {
    expect(windowedBookRatio(0, 1, 0, 1)).toBe(0);
    expect(windowedBookRatio(0, 1, 1, 1)).toBe(1);
    expect(windowedBookRatio(9, 1, 1, 10)).toBe(1);
    expect(windowedBookRatio(-3, 1, -1, 10)).toBe(0);
    expect(windowedBookRatio(0, 1, Number.NaN, 0)).toBe(0);
    expect(windowedBookRatio(0, 1, 2, 10)).toBeCloseTo(0.1);
  });
});
