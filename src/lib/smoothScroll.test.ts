import { describe, it, expect } from "vitest";
import { accumulateOverscroll, type OverscrollState } from "./smoothScroll";

const base = (): OverscrollState => ({ over: 0, pending: 0, threshold: 100, strength: 1, bounce: true });

describe("accumulateOverscroll（F7）", () => {
  it("未达阈值：只累计 pending 与视觉 over，不跨章", () => {
    const a = accumulateOverscroll(base(), 40);
    expect(a.edge).toBe(0);
    expect(a.carry).toBe(0);
    expect(a.pending).toBe(40);
    expect(a.over).toBeGreaterThan(0);
    expect(a.over).toBeLessThan(40);
  });

  it("连续越界跨阈值：carry 等于原始越界总量，over 清零（同一位移只计一次）", () => {
    let s = base();
    let a = accumulateOverscroll(s, 40);
    s = { ...s, over: a.over, pending: a.pending };
    a = accumulateOverscroll(s, 40);
    s = { ...s, over: a.over, pending: a.pending };
    a = accumulateOverscroll(s, 40);
    expect(a.edge).toBe(1);
    expect(a.carry).toBe(120);
    expect(a.over).toBe(0);
    expect(a.over + Math.abs(a.carry)).toBe(120);
  });

  it("反向越界重置累计，不回带旧方向", () => {
    let s = base();
    let a = accumulateOverscroll(s, 60);
    s = { ...s, over: a.over, pending: a.pending };
    a = accumulateOverscroll(s, -70);
    expect(a.edge).toBe(0);
    expect(a.pending).toBe(-70);
    a = accumulateOverscroll({ ...s, over: a.over, pending: a.pending }, -40);
    expect(a.edge).toBe(-1);
    expect(a.carry).toBe(-110);
  });

  it("非回弹分支：一次越界直接跨章，carry 为原始量", () => {
    const a = accumulateOverscroll({ ...base(), bounce: false }, -25);
    expect(a.edge).toBe(-1);
    expect(a.carry).toBe(-25);
    expect(a.over).toBe(0);
    expect(a.pending).toBe(0);
  });

  it("delta=0 清 pending 不产生 edge", () => {
    const a = accumulateOverscroll({ ...base(), pending: 50, over: 10 }, 0);
    expect(a).toEqual({ over: 10, pending: 0, edge: 0, carry: 0 });
  });
});
