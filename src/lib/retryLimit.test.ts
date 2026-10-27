import { describe, it, expect } from "vitest";
import { shouldRetryMangaMeasure, MANGA_MEASURE_RETRY_LIMIT } from "./retryLimit";

describe("shouldRetryMangaMeasure", () => {
  it("未到上限时允许继续，到限/超限停止", () => {
    expect(shouldRetryMangaMeasure(0, 3)).toBe(true);
    expect(shouldRetryMangaMeasure(2, 3)).toBe(true);
    expect(shouldRetryMangaMeasure(3, 3)).toBe(false);
    expect(shouldRetryMangaMeasure(4, 3)).toBe(false);
  });

  it("默认上限为 60 帧，恰好到限停止", () => {
    expect(MANGA_MEASURE_RETRY_LIMIT).toBe(60);
    expect(shouldRetryMangaMeasure(MANGA_MEASURE_RETRY_LIMIT - 1)).toBe(true);
    expect(shouldRetryMangaMeasure(MANGA_MEASURE_RETRY_LIMIT)).toBe(false);
  });

  it("非法/负值保守停止", () => {
    // 负数按 0 处理，仍允许尝试
    expect(shouldRetryMangaMeasure(-1, 3)).toBe(true);
    expect(shouldRetryMangaMeasure(NaN, 3)).toBe(false);
    expect(shouldRetryMangaMeasure(1, NaN)).toBe(false);
    expect(shouldRetryMangaMeasure(1, 0)).toBe(false);
  });
});
