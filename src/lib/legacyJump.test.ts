import { describe, it, expect } from "vitest";
import { resolveLegacyChapterIndex, resolveProgressByPct } from "./legacyJump";

const toc = [
  { title: "第一章 起点", chars: 100 },
  { title: "第二章 发展", chars: 300 },
  { title: "第三章 结局", chars: 600 },
];
const norm = (s: string) => s.trim().toLowerCase().replace(/\s+/g, " ");

describe("resolveLegacyChapterIndex", () => {
  it("范围内且标题匹配 → 原索引", () => {
    expect(resolveLegacyChapterIndex(1, "第二章 发展", toc, norm)).toEqual({ index: 1, corrected: true });
    expect(resolveLegacyChapterIndex(2, "第三章 结局", toc, norm)).toEqual({ index: 2, corrected: true });
  });

  it("旧标题空/纯数字 → 原索引", () => {
    expect(resolveLegacyChapterIndex(1, "", toc, norm)).toEqual({ index: 1, corrected: true });
    expect(resolveLegacyChapterIndex(1, "12", toc, norm)).toEqual({ index: 1, corrected: true });
  });

  it("标题唯一匹配 → 校正索引", () => {
    expect(resolveLegacyChapterIndex(0, "第三章 结局", toc, norm)).toEqual({ index: 2, corrected: true });
    expect(resolveLegacyChapterIndex(2, "第一章 起点", toc, norm)).toEqual({ index: 0, corrected: true });
  });

  it("标题重复 → 不影响「索引范围内且当章标题匹配」；跨章重复时不校正", () => {
    const dup = [{ title: "序" }, { title: "序" }, { title: "正文" }];
    // 索引 0 的章标题本身就是「序」→ 原索引可信。
    expect(resolveLegacyChapterIndex(0, "序", dup, norm)).toEqual({ index: 0, corrected: true });
    // 索引 2（正文）却存了「序」：重复标题无法唯一确定 → 不校正。
    expect(resolveLegacyChapterIndex(2, "序", dup, norm)).toEqual({ index: 2, corrected: false });
  });

  it("无匹配 → 原索引并标记未校正", () => {
    expect(resolveLegacyChapterIndex(1, "不存在的章", toc, norm)).toEqual({ index: 1, corrected: false });
  });

  it("越界：唯一标题可校正；无匹配 → 夹到 0 且未校正", () => {
    expect(resolveLegacyChapterIndex(99, "第三章 结局", toc, norm)).toEqual({ index: 2, corrected: true });
    expect(resolveLegacyChapterIndex(-3, "第二章 发展", toc, norm)).toEqual({ index: 1, corrected: true });
    expect(resolveLegacyChapterIndex(99, "不存在的章", toc, norm)).toEqual({ index: 0, corrected: false });
  });
});

describe("resolveProgressByPct", () => {
  it("按 chars 累计定位（总 chars=1000）", () => {
    expect(resolveProgressByPct(0, toc)).toBe(0);
    expect(resolveProgressByPct(10, toc)).toBe(0);
    expect(resolveProgressByPct(20, toc)).toBe(1);
    expect(resolveProgressByPct(50, toc)).toBe(2);
    expect(resolveProgressByPct(99, toc)).toBe(2);
    expect(resolveProgressByPct(100, toc)).toBe(2);
  });

  it("无效输入/空 toc/字符全 0 → null", () => {
    expect(resolveProgressByPct(50, [])).toBeNull();
    expect(resolveProgressByPct(Number.NaN, toc)).toBeNull();
    expect(resolveProgressByPct(50, [{ title: "a", chars: 0 }])).toBeNull();
  });
});
