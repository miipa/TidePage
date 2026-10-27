import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { TimerGate } from "./timerGate";

describe("TimerGate（F7）", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("schedule 到点执行", () => {
    const g = new TimerGate();
    const fn = vi.fn();
    g.schedule(fn, 100);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("cancel 后不执行；重复 schedule 只保留最新一次", () => {
    const g = new TimerGate();
    const a = vi.fn();
    const b = vi.fn();
    g.schedule(a, 100);
    g.schedule(b, 100);
    vi.advanceTimersByTime(100);
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledTimes(1);
    g.schedule(a, 100);
    g.cancel();
    vi.advanceTimersByTime(100);
    expect(a).not.toHaveBeenCalled();
  });

  it("stop 后不再执行且 stopped=true、重复 stop 幂等", () => {
    const g = new TimerGate();
    const fn = vi.fn();
    g.schedule(fn, 100);
    g.stop();
    expect(g.stopped).toBe(true);
    g.stop();
    vi.advanceTimersByTime(100);
    expect(fn).not.toHaveBeenCalled();
    g.schedule(fn, 100);
    vi.advanceTimersByTime(100);
    expect(fn).not.toHaveBeenCalled();
  });
});
