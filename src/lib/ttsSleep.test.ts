import { describe, it, expect } from "vitest";
import {
  formatSleepRemaining,
  initialSleepRemaining,
  shouldAutoNextChapter,
  sleepModeOf,
  tickSleepRemaining,
} from "./ttsSleep";
import { sleepDeadlineFromNow, sleepRemainingSec, shouldArmSleep } from "./ttsSleep";

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

/* ---------------- F5-H1 新增用例 ---------------- */

describe("sleepDeadlineFromNow", () => {
  it("按分钟换算毫秒", () => {
    expect(sleepDeadlineFromNow(5, 1_000)).toBe(1_000 + 5 * 60_000);
    expect(sleepDeadlineFromNow(30, 0)).toBe(30 * 60_000);
  });
  it("0/负数/非数返回 0（未启用）", () => {
    expect(sleepDeadlineFromNow(0, 1_000)).toBe(0);
    expect(sleepDeadlineFromNow(-1, 1_000)).toBe(0);
    expect(sleepDeadlineFromNow(Number.NaN, 1_000)).toBe(0);
  });
});

describe("sleepRemainingSec", () => {
  it("向上取整并夹到 0", () => {
    const now = 10_000;
    expect(sleepRemainingSec(now + 60_000, now)).toBe(60);
    expect(sleepRemainingSec(now + 1_500, now)).toBe(2);
    expect(sleepRemainingSec(now + 1, now)).toBe(1);
    expect(sleepRemainingSec(now - 1, now)).toBe(0);
    expect(sleepRemainingSec(0, now)).toBe(0);
  });
});

describe("shouldArmSleep", () => {
  const now = 100_000;
  it("未启用（0/-1）不起臂", () => {
    expect(shouldArmSleep(0, 0, 0, now)).toBe(false);
    expect(shouldArmSleep(-1, 0, 0, now)).toBe(false);
  });
  it("无截止时间时起臂", () => {
    expect(shouldArmSleep(5, 0, 0, now)).toBe(true);
  });
  it("跨挂载 armed 相同且未过期：不重臂（倒计时不重置）", () => {
    const deadline = sleepDeadlineFromNow(5, now);
    expect(shouldArmSleep(5, 5, deadline, now + 60_000)).toBe(false);
  });
  it("切换分钟数时按当前时刻重新计时", () => {
    const deadline = sleepDeadlineFromNow(5, now);
    expect(shouldArmSleep(10, 5, deadline, now)).toBe(true);
  });
  it("已过期时重新起臂", () => {
    const deadline = sleepDeadlineFromNow(5, now);
    expect(shouldArmSleep(5, 5, deadline, deadline + 1)).toBe(true);
  });
});
