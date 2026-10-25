import { describe, it, expect } from "vitest";
import {
  clearQueue,
  createPdfCoverQueue,
  enqueueVisible,
  pickIdleBatch,
  queueSize,
  takeNext,
} from "./pdfCoverQueue";

describe("pdfCoverQueue", () => {
  it("可见项插队首、去重、排除 tried", () => {
    const q = createPdfCoverQueue();
    const tried = new Set<number>([9]);
    expect(enqueueVisible(q, [1, 2, 9, 2], tried)).toEqual([1, 2]);
    expect(enqueueVisible(q, [3, 1], tried)).toEqual([3]);
    expect(q.visible).toEqual([3, 1, 2]);
    expect(takeNext(q)).toBe(3);
    expect(takeNext(q)).toBe(1);
    expect(takeNext(q)).toBe(2);
    expect(takeNext(q)).toBeNull();
  });

  it("清空后可重新入队", () => {
    const q = createPdfCoverQueue();
    enqueueVisible(q, [5], new Set());
    clearQueue(q);
    expect(queueSize(q)).toBe(0);
    expect(enqueueVisible(q, [5], new Set())).toEqual([5]);
  });

  it("空闲批次跳过 tried/已入队，且限制数量", () => {
    const tried = new Set([2]);
    const seen = new Set([3]);
    expect(pickIdleBatch([1, 2, 3, 4, 5, 6], tried, seen, 3)).toEqual([1, 4, 5]);
    expect(pickIdleBatch([], tried, seen, 10)).toEqual([]);
  });

  it("非法 id 被忽略", () => {
    const q = createPdfCoverQueue();
    expect(enqueueVisible(q, [0, -1, Number.NaN, 7], new Set())).toEqual([7]);
  });
});
