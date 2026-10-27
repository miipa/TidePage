import { describe, it, expect } from "vitest";
import { dragReleaseCommits } from "./dragSort";

describe("dragReleaseCommits（G21）", () => {
  it("pointerup 正常提交", () => {
    expect(dragReleaseCommits("pointerup")).toBe(true);
  });
  it("pointercancel 不提交（取消路径）", () => {
    expect(dragReleaseCommits("pointercancel")).toBe(false);
    expect(dragReleaseCommits("pointerleave")).toBe(false);
  });
});
