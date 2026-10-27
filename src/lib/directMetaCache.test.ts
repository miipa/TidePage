import { describe, it, expect } from "vitest";
import { LruCache } from "./directMetaCache";

describe("LruCache（F7）", () => {
  it("不同 book 同 page 不串（key 含 bookId）", () => {
    const c = new LruCache<string, { w: number }>(4);
    c.set("1:0", { w: 100 });
    c.set("2:0", { w: 200 });
    expect(c.get("1:0")).toEqual({ w: 100 });
    expect(c.get("2:0")).toEqual({ w: 200 });
  });

  it("同 key 命中；超上限淘汰最旧；命中改变淘汰顺序", () => {
    const c = new LruCache<string, number>(2);
    c.set("a", 1);
    c.set("b", 2);
    expect(c.get("a")).toBe(1); // a 变最新
    c.set("c", 3); // 淘汰 b
    expect(c.get("b")).toBeUndefined();
    expect(c.get("a")).toBe(1);
    expect(c.get("c")).toBe(3);
    expect(c.size).toBe(2);
  });

  it("重复 set 不增加 size；clear 清空", () => {
    const c = new LruCache<string, number>(2);
    c.set("a", 1);
    c.set("a", 2);
    expect(c.size).toBe(1);
    expect(c.get("a")).toBe(2);
    c.clear();
    expect(c.size).toBe(0);
  });
});
