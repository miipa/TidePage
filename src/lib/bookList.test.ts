import { describe, it, expect } from "vitest";
import { bookListKey, resolveListTotal, shouldCountTotal } from "./bookList";

const base = {
  query: "",
  sort: "added",
  formats: [] as string[],
  states: [] as string[],
  folder: null as string | null,
  desc: true,
  tags: [] as string[],
};

describe("bookList", () => {
  it("key 覆盖查询/排序/筛选/文件夹/方向；筛选顺序无关", () => {
    const k = bookListKey(base);
    expect(bookListKey({ ...base })).toBe(k);
    expect(bookListKey({ ...base, query: "abc" })).not.toBe(k);
    expect(bookListKey({ ...base, sort: "title" })).not.toBe(k);
    expect(bookListKey({ ...base, desc: false })).not.toBe(k);
    expect(bookListKey({ ...base, folder: "D:/Books" })).not.toBe(k);
    const f1 = bookListKey({ ...base, formats: ["epub", "mobi"] });
    const f2 = bookListKey({ ...base, formats: ["mobi", "epub"] });
    expect(f1).toBe(f2);
    expect(f1).not.toBe(k);
    expect(bookListKey({ ...base, states: ["read"] })).not.toBe(k);
    expect(bookListKey({ ...base, tags: ["A", "b"] })).toBe(
      bookListKey({ ...base, tags: ["b", "a"] }),
    );
    expect(bookListKey({ ...base, tags: ["a"] })).not.toBe(k);
  });

  it("仅 key 变化时需要计数（滚动窗口复用缓存）", () => {
    const k = bookListKey(base);
    expect(shouldCountTotal("", k)).toBe(true);
    expect(shouldCountTotal(k, k)).toBe(false);
    expect(shouldCountTotal(k, bookListKey({ ...base, query: "x" }))).toBe(true);
  });

  it("未计数（-1）沿用缓存总数；计数结果覆盖缓存", () => {
    expect(resolveListTotal(-1, 176194)).toBe(176194);
    expect(resolveListTotal(-1, -1)).toBe(0);
    expect(resolveListTotal(12, 176194)).toBe(12);
  });
});
