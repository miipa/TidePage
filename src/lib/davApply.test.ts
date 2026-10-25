import { describe, it, expect } from "vitest";
import { planDavApply, type DavApplyInput } from "./davApply";

// 决策是纯函数：只验证规则与优先级，不涉及 DOM / Tauri。
const base: DavApplyInput = {
  remote: { chapter_index: 3, chapter_ratio: 0.5, updated_at: 2000 },
  localUpdatedAt: 1000,
  chapterCount: 10,
  currentChapter: 3,
  currentRatio: 0.1,
  userMoved: false,
};

const input = (o: Partial<DavApplyInput>): DavApplyInput => ({ ...base, ...o });
const remote = (o: Partial<NonNullable<DavApplyInput["remote"]>>) => ({
  chapter_index: 3,
  chapter_ratio: 0.5,
  updated_at: 2000,
  ...o,
});

describe("planDavApply — 五条规则与优先级", () => {
  it("云端无进度 → none", () => {
    expect(planDavApply(input({ remote: null }))).toBe("none");
  });

  it("云端进度不比本地新（早于或等于）→ none", () => {
    expect(planDavApply(input({ remote: remote({ updated_at: 999 }) }))).toBe("none");
    expect(planDavApply(input({ remote: remote({ updated_at: 1000 }) }))).toBe("none");
  });

  it("章号越界（负数 / 等于或超过目录长度）→ none", () => {
    expect(planDavApply(input({ remote: remote({ chapter_index: -1 }) }))).toBe("none");
    expect(planDavApply(input({ remote: remote({ chapter_index: 10 }) }))).toBe("none");
    expect(planDavApply(input({ remote: remote({ chapter_index: 11 }) }))).toBe("none");
  });

  it("同章且比例差在默认容差 0.002 内 → none（含 0 与近似边界）", () => {
    expect(planDavApply(input({ currentRatio: 0.5 }))).toBe("none");
    expect(planDavApply(input({ currentRatio: 0.499 }))).toBe("none");
    expect(planDavApply(input({ currentRatio: 0.5019 }))).toBe("none");
  });

  it("同章比例差恰为容差 → none（比较是 <=，取可精确表示的 0.25 构造）", () => {
    expect(planDavApply(input({ currentRatio: 0.25, ratioEpsilon: 0.25 }))).toBe("none");
    expect(planDavApply(input({ currentRatio: 0.25, ratioEpsilon: 0.24 }))).toBe("same-chapter");
  });

  it("同章且比例差超容差、用户未移动 → same-chapter", () => {
    expect(planDavApply(input({ currentRatio: 0.49 }))).toBe("same-chapter");
    expect(planDavApply(input({ currentRatio: 0 }))).toBe("same-chapter");
  });

  it("异章（比例无论）且用户未移动 → load-chapter", () => {
    expect(planDavApply(input({ remote: remote({ chapter_index: 4 }), currentRatio: 0.5 }))).toBe("load-chapter");
    expect(planDavApply(input({ remote: remote({ chapter_index: 4 }), currentRatio: 0 }))).toBe("load-chapter");
  });

  it("用户已移动：同章与异章都 → notify-only", () => {
    expect(planDavApply(input({ userMoved: true }))).toBe("notify-only");
    expect(planDavApply(input({ userMoved: true, remote: remote({ chapter_index: 4 }) }))).toBe("notify-only");
  });

  it("用户已移动但同章比例一致：规则 3 先命中 → none（不打扰）", () => {
    expect(planDavApply(input({ userMoved: true, currentRatio: 0.5 }))).toBe("none");
  });

  it("自定义容差生效", () => {
    const eps = 0.05;
    expect(planDavApply(input({ currentRatio: 0.54, ratioEpsilon: eps }))).toBe("none");
    expect(planDavApply(input({ currentRatio: 0.56, ratioEpsilon: eps }))).toBe("same-chapter");
    // 默认容差下同一输入是 same-chapter，说明自定义值确实被采用。
    expect(planDavApply(input({ currentRatio: 0.54 }))).toBe("same-chapter");
  });

  it("规则优先级：章号越界优先于用户已移动（no remote apply）", () => {
    expect(planDavApply(input({ userMoved: true, remote: remote({ chapter_index: 99 }) }))).toBe("none");
  });
});
