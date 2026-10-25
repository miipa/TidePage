import { describe, it, expect } from "vitest";
import { chapterBytes, evictChapterCache } from "./chapterCache";

const MB = 1024 * 1024;

describe("chapterCache", () => {
  it("未超预算不淘汰", () => {
    const c = new Map([[1, { html: "a".repeat(1000) }], [2, { html: "b".repeat(1000) }]]);
    evictChapterCache(c, 1 * MB);
    expect(c.size).toBe(2);
  });
  it("超预算按最旧淘汰并保留当前章", () => {
    const c = new Map<number, { html: string }>([
      [1, { html: "a".repeat(MB / 2) }],
      [2, { html: "b".repeat(MB / 2) }],
      [3, { html: "c".repeat(MB / 2) }],
    ]);
    evictChapterCache(c, 1 * MB, 2);
    expect(c.has(2)).toBe(true);
    expect(c.size).toBeLessThan(3);
  });
  it("单章超大允许单独存在", () => {
    const c = new Map<number, { html: string }>([
      [1, { html: "a".repeat(MB) }],
      [2, { html: "b".repeat(10) }],
    ]);
    evictChapterCache(c, 1 * MB, 1);
    expect(c.size).toBe(1);
    expect(c.has(1)).toBe(true);
  });
  it("UTF-16 估算", () => {
    expect(chapterBytes({ html: "abc" })).toBe(6);
  });
});
