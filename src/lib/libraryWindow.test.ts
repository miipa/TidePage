import { describe, it, expect } from "vitest";
import { windowCovers } from "./libraryWindow";

describe("windowCovers（F3）", () => {
  it("完整覆盖/部分覆盖/未覆盖", () => {
    expect(windowCovers(100, 50, 110, 140)).toBe(true);
    expect(windowCovers(100, 50, 100, 150)).toBe(true);
    expect(windowCovers(100, 50, 90, 120)).toBe(false);
    expect(windowCovers(100, 50, 140, 200)).toBe(false);
  });
  it("空窗口一律不覆盖", () => {
    expect(windowCovers(0, 0, 0, 1)).toBe(false);
    expect(windowCovers(100, 0, 100, 100)).toBe(false);
  });
  it("跨段边界", () => {
    expect(windowCovers(0, 1, 0, 1)).toBe(true);
    expect(windowCovers(0, 1, 0, 2)).toBe(false);
  });
});
