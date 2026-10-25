import { describe, it, expect } from "vitest";
import { allowEventTrigger, nextBackoffMs, normalizeLastResult, shouldAutoSync } from "./autoSync";

describe("autoSync", () => {
  it("指数退避与上限", () => {
    expect(nextBackoffMs(0)).toBe(0);
    expect(nextBackoffMs(1)).toBe(60_000);
    expect(nextBackoffMs(2)).toBe(120_000);
    expect(nextBackoffMs(10)).toBe(30 * 60_000);
  });
  it("触发条件：启用/间隔/退避", () => {
    const base = { enabled: true, intervalHours: 1, lastAt: 0, now: 10 * 60_000, failCount: 0, lastAttemptAt: 0 };
    expect(shouldAutoSync(base)).toBe(true);
    expect(shouldAutoSync({ ...base, enabled: false })).toBe(false);
    expect(shouldAutoSync({ ...base, lastAt: 0, now: 30 * 60_000 })).toBe(true);
    expect(shouldAutoSync({ ...base, lastAt: 9 * 60_000, failCount: 0 })).toBe(false);
    expect(shouldAutoSync({ ...base, failCount: 1, lastAttemptAt: 0 })).toBe(true);
    expect(shouldAutoSync({ ...base, failCount: 1, lastAttemptAt: 10 * 60_000 - 30_000 })).toBe(false);
  });
  it("事件防抖与旧值归一化", () => {
    expect(allowEventTrigger(0, 1000)).toBe(true);
    expect(allowEventTrigger(1000, 20_000)).toBe(false);
    expect(allowEventTrigger(1000, 40_000)).toBe(true);
    expect(normalizeLastResult("success")).toBe("success");
    expect(normalizeLastResult("oops")).toBe("");
  });
});
