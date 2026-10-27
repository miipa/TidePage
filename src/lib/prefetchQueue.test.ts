import { describe, it, expect } from "vitest";
import { planPrefetchQueue } from "./prefetchQueue";

const empty = new Set<number>();
const plan = (center: number, total = 100, current: number[] = []) =>
  planPrefetchQueue(center, total, 6, empty, empty, current, 40);

describe("planPrefetchQueue（F1）", () => {
  it("按距中心由近到远生成窗口，排除 center 自身", () => {
    expect(plan(10)).toEqual([11, 9, 12, 8, 13, 7, 14, 6, 15, 5, 16, 4]);
  });

  it("已缓存/在途章节排除", () => {
    const q = planPrefetchQueue(10, 100, 6, new Set([11, 9]), new Set([12]), [], 40);
    expect(q).not.toContain(11);
    expect(q).not.toContain(9);
    expect(q).not.toContain(12);
    expect(q[0]).toBe(8);
  });

  it("重复调用不重复、队列有界（max 截断）", () => {
    const q = planPrefetchQueue(50, 100, 20, empty, empty, [51, 49], 5);
    expect(q).toHaveLength(5);
    expect(new Set(q).size).toBe(5);
  });

  it("center 变化后靠近新中心的项优先，旧窗口外项丢弃", () => {
    const old = [90, 91, 92];
    const q = planPrefetchQueue(10, 100, 20, empty, empty, old, 40);
    expect(q[0]).toBe(11);
    expect(q).not.toContain(90);
    // 同一次重建不重复
    expect(new Set(q).size).toBe(q.length);
  });

  it("total 边界：空/1/末尾章", () => {
    expect(planPrefetchQueue(0, 0, 6, empty, empty, [], 40)).toEqual([]);
    expect(plan(0, 1)).toEqual([]);
    // 末尾章只有前侧
    expect(plan(99, 100)).toEqual([98, 97, 96, 95, 94, 93]);
    expect(plan(0, 100)).toEqual([1, 2, 3, 4, 5, 6]);
  });
});
