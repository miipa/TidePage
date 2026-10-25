import { describe, it, expect } from "vitest";
import {
  COVER_CACHE_HINT_BYTES,
  coverReclaimBytes,
  estimateVacuumTemp,
  maintenanceRecordView,
  maintenanceResultKind,
  shouldAutoVacuum,
  shouldRemindMaintenance,
  shouldSuggestCoverCleanup,
} from "./maintenance";

describe("maintenance", () => {
  const MB = 1024 * 1024;
  it("C15 封面缓存提示阈值与可回收口径", () => {
    expect(shouldSuggestCoverCleanup(COVER_CACHE_HINT_BYTES, 0)).toBe(true);
    expect(shouldSuggestCoverCleanup(0, COVER_CACHE_HINT_BYTES)).toBe(true);
    expect(shouldSuggestCoverCleanup(COVER_CACHE_HINT_BYTES - 1, 0)).toBe(false);
    expect(coverReclaimBytes(100, 50)).toBe(150);
    expect(coverReclaimBytes(-1, 10)).toBe(10);
  });

  it("可回收提醒阈值与 7 天关闭", () => {
    expect(shouldRemindMaintenance(201 * MB, 5 * 1024 * MB, 0, Date.now())).toBe(true);
    expect(shouldRemindMaintenance(50 * MB, 100 * MB, 0, Date.now())).toBe(true);
    expect(shouldRemindMaintenance(10 * MB, 500 * MB, 0, Date.now())).toBe(false);
    const now = 1_000_000_000;
    expect(shouldRemindMaintenance(300 * MB, 500 * MB, now - 86_400_000, now)).toBe(false);
    expect(shouldRemindMaintenance(300 * MB, 500 * MB, now - 8 * 86_400_000, now)).toBe(true);
  });
  it("预计临时空间与同盘检查次数", () => {
    expect(estimateVacuumTemp(100 * MB, true)).toEqual({ requiredBytes: 164 * MB, checks: 1 });
    expect(estimateVacuumTemp(100 * MB, false).checks).toBe(2);
  });
  it("自动整理条件", () => {
    const base = { enabled: true, idleMinutes: 15, now: 10_000_000, lastActivity: 0, readerOpen: false, maintenanceRunning: false, scanRunning: false, sessionDone: false, lastAutoVacuumAt: 0, lastAutoVacuumResult: "" };
    expect(shouldAutoVacuum(base)).toBe(true);
    expect(shouldAutoVacuum({ ...base, enabled: false })).toBe(false);
    expect(shouldAutoVacuum({ ...base, readerOpen: true })).toBe(false);
    expect(shouldAutoVacuum({ ...base, maintenanceRunning: true })).toBe(false);
    expect(shouldAutoVacuum({ ...base, scanRunning: true })).toBe(false);
    expect(shouldAutoVacuum({ ...base, sessionDone: true })).toBe(false);
    expect(shouldAutoVacuum({ ...base, lastActivity: 9_940_000 })).toBe(false);
    expect(shouldAutoVacuum({ ...base, lastAutoVacuumResult: "error", lastAutoVacuumAt: 10_000_000 - 3_600_000 })).toBe(false);
    expect(shouldAutoVacuum({ ...base, lastAutoVacuumResult: "error", lastAutoVacuumAt: 10_000_000 - 25 * 3_600_000 })).toBe(true);
  });

  it("历史结果结构化（不硬编码文案）", () => {
    expect(maintenanceResultKind("success")).toBe("success");
    expect(maintenanceResultKind("skipped")).toBe("skipped");
    expect(maintenanceResultKind("error")).toBe("error");
    expect(maintenanceResultKind("unknown")).toBe("error");
    const v = maintenanceRecordView({ at: 0, result: "skipped", error: "备份失败", freed: 0, durationMs: 5.4, backupPath: "b.json" });
    expect(v).toEqual({ at: 0, result: "skipped", freed: 0, durationMs: 5, error: "备份失败", backupPath: "b.json" });
  });
});
