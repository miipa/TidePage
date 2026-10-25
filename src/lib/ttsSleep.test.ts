import { describe, it, expect } from "vitest";
import {
  formatSleepRemaining,
  initialSleepRemaining,
  shouldAutoNextChapter,
  sleepModeOf,
  tickSleepRemaining,
} from "./ttsSleep";

describe("ttsSleep", () => {
  it("模式识别：0/空=关闭，正数=分钟，-1=章末", () => {
    expect(sleepModeOf(0)).toBe("off");
    expect(sleepModeOf(undefined)).toBe("off");
    expect(sleepModeOf(15)).toBe("minutes");
    expect(sleepModeOf(-1)).toBe("chapter");
  });

  it("倒计时展示", () => {
    expect(formatSleepRemaining(90_000, "minutes")).toBe("1:30");
    expect(formatSleepRemaining(5_000, "minutes")).toBe("0:05");
    expect(formatSleepRemaining(0, "chapter")).toBe("章末停止");
    expect(formatSleepRemaining(1000, "off")).toBe("");
  });

  it("章末停止时不自动续章", () => {
    expect(shouldAutoNextChapter(-1, true)).toBe(false);
    expect(shouldAutoNextChapter(15, true)).toBe(true);
    expect(shouldAutoNextChapter(0, false)).toBe(false);
  });

  it("倒计时递减与初始化", () => {
    expect(tickSleepRemaining(1000, 400)).toBe(600);
    expect(tickSleepRemaining(500, 900)).toBe(0);
    expect(initialSleepRemaining(15)).toBe(15 * 60_000);
    expect(initialSleepRemaining(-1)).toBeNull();
    expect(initialSleepRemaining(0)).toBeNull();
  });
});
