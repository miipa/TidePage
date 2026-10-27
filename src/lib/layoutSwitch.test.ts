import { describe, it, expect } from "vitest";
import { layoutSwitchRatio } from "./layoutSwitch";

describe("layoutSwitchRatio（B5）", () => {
  it("同布局不触发", () => {
    expect(layoutSwitchRatio("page", "page", 0.3, 0.7)).toBeNull();
    expect(layoutSwitchRatio("scroll", "scroll", 0.3, 0.7)).toBeNull();
  });
  it("page→scroll 用翻页比例；scroll→page 用滚动比例", () => {
    expect(layoutSwitchRatio("page", "scroll", 0.42, 0.9)).toBe(0.42);
    expect(layoutSwitchRatio("scroll", "page", 0.42, 0.9)).toBe(0.9);
  });
  it("比例夹取 0..1，NaN 归零", () => {
    expect(layoutSwitchRatio("page", "scroll", 1.5, 0)).toBe(1);
    expect(layoutSwitchRatio("page", "scroll", -1, 0)).toBe(0);
    expect(layoutSwitchRatio("scroll", "page", 0, Number.NaN)).toBe(0);
  });
});
