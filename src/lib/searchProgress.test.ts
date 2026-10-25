import { describe, it, expect } from "vitest";
import {
  applySearchProgress,
  resetSearchProgress,
  type SearchProgress,
} from "./searchProgress";

const ev = (bookId: number, done: number, total: number) => ({ bookId, done, total });

describe("searchProgress", () => {
  it("其它书的进度不串台：保持原状态", () => {
    const prev: SearchProgress = { done: 1, total: 4, pct: 25 };
    expect(applySearchProgress(prev, ev(2, 3, 4), 1)).toBe(prev);
    expect(applySearchProgress(null, ev(2, 3, 4), 1)).toBeNull();
  });

  it("total<=0 隐藏进度", () => {
    expect(applySearchProgress(null, ev(1, 0, 0), 1)).toBeNull();
    expect(applySearchProgress({ done: 1, total: 2, pct: 50 }, ev(1, 1, -3), 1)).toBeNull();
  });

  it("正常换算百分比并四舍五入", () => {
    expect(applySearchProgress(null, ev(1, 1, 3), 1)).toEqual({ done: 1, total: 3, pct: 33 });
    expect(applySearchProgress(null, ev(1, 2, 3), 1)).toEqual({ done: 2, total: 3, pct: 67 });
  });

  it("done 越界 clamp 到 [0,total]，完成后为 100%", () => {
    expect(applySearchProgress(null, ev(1, 9, 4), 1)).toEqual({ done: 4, total: 4, pct: 100 });
    expect(applySearchProgress(null, ev(1, -2, 4), 1)).toEqual({ done: 0, total: 4, pct: 0 });
  });

  it("完成事件显示 100%，由请求结束统一清空", () => {
    const p = applySearchProgress(null, ev(1, 4, 4), 1);
    expect(p?.pct).toBe(100);
    expect(resetSearchProgress()).toBeNull();
  });

  it("小数输入向下取整，避免出现 100% 提前跳变", () => {
    expect(applySearchProgress(null, ev(1, 1.9, 4.9), 1)).toEqual({ done: 1, total: 4, pct: 25 });
  });
});
