import { describe, it, expect } from "vitest";
import { PAGE_HEIGHT_EPSILON_PX, detectPagedStyleConflict, tallBlockAction } from "./pagedBreak";

describe("pagedBreak", () => {
  it("不超过一页 → keep", () => {
    expect(tallBlockAction(100, 800, false)).toBe("keep");
    expect(tallBlockAction(800 - PAGE_HEIGHT_EPSILON_PX, 800, false)).toBe("keep");
    expect(tallBlockAction(800 - PAGE_HEIGHT_EPSILON_PX, 800, true)).toBe("keep");
  });

  it("略高于一页 → 非媒体 fragment / 媒体 fit", () => {
    expect(tallBlockAction(800 - PAGE_HEIGHT_EPSILON_PX + 1, 800, false)).toBe("fragment");
    expect(tallBlockAction(2000, 800, false)).toBe("fragment");
    expect(tallBlockAction(2000, 800, true)).toBe("fit");
  });

  it("pageHeight<=0 或非法值 → keep（首帧防御）", () => {
    expect(tallBlockAction(1000, 0, false)).toBe("keep");
    expect(tallBlockAction(1000, -5, true)).toBe("keep");
    expect(tallBlockAction(Number.NaN, 800, false)).toBe("keep");
    expect(tallBlockAction(1000, Number.NaN, true)).toBe("keep");
    expect(tallBlockAction(Number.POSITIVE_INFINITY, 800, false)).toBe("keep");
  });
});
describe("detectPagedStyleConflict", () => {
  const p = (breakInside: string, breakAfter = "auto", display = "block", float = "none") => ({
    breakInside,
    breakAfter,
    display,
    float,
  });

  it("正常 avoid 不产生兜底", () => {
    expect(detectPagedStyleConflict(p("avoid", "avoid"), true)).toBeNull();
    expect(detectPagedStyleConflict(p("avoid", "avoid"), false)).toBeNull();
  });

  it("书内 auto / legacy page-break-inside 顶掉 avoid 时补 break-inside", () => {
    expect(detectPagedStyleConflict(p("auto"), false)?.breakInside).toBe(true);
    expect(detectPagedStyleConflict(p("page-break-inside"), false)?.breakInside).toBe(true);
  });

  it("标题 break-after 非 avoid 时补 break-after；非标题不要求", () => {
    expect(detectPagedStyleConflict(p("avoid", "auto"), true)?.breakAfter).toBe(true);
    expect(detectPagedStyleConflict(p("avoid", "auto"), false)).toBeNull();
  });

  it("display inline / float 非 none 时补 display/float", () => {
    expect(detectPagedStyleConflict(p("avoid", "avoid", "inline-block"), false)?.displayBlock).toBe(true);
    expect(detectPagedStyleConflict(p("avoid", "avoid", "block", "left"), false)?.floatNone).toBe(true);
  });

  it("多个冲突同时返回", () => {
    const f = detectPagedStyleConflict(p("auto", "auto", "inline", "right"), true);
    expect(f).toEqual({ breakInside: true, breakAfter: true, displayBlock: true, floatNone: true });
  });
});
