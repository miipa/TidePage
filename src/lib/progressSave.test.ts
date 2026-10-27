import { describe, it, expect } from "vitest";
import { shouldRetryProgressSave, PROGRESS_RETRY_DELAY_MS } from "./progressSave";

describe("progressSave（F1）", () => {
  it("只在首次失败时重试一次", () => {
    expect(shouldRetryProgressSave(0)).toBe(true);
    expect(shouldRetryProgressSave(1)).toBe(false);
    expect(shouldRetryProgressSave(2)).toBe(false);
    expect(PROGRESS_RETRY_DELAY_MS).toBeGreaterThan(0);
  });
});
