import { describe, it, expect } from "vitest";
import { DEFAULT_MAINTENANCE, normalizeMaintenance } from "./maintenanceConfig";

describe("normalizeMaintenance（F9）", () => {
  it("缺字段补默认", () => {
    expect(normalizeMaintenance(undefined)).toEqual(DEFAULT_MAINTENANCE);
    expect(normalizeMaintenance({})).toEqual(DEFAULT_MAINTENANCE);
    expect(normalizeMaintenance(null, { ...DEFAULT_MAINTENANCE, reminderDismissedUntil: 7 }).reminderDismissedUntil).toBe(7);
  });

  it("非法字段回退（数字为负/非数、枚举越界）", () => {
    const out = normalizeMaintenance({
      reminderDismissedUntil: -1,
      lastAutoVacuumAt: Number.NaN,
      lastAutoVacuumResult: "weird",
    });
    expect(out.reminderDismissedUntil).toBe(0);
    expect(out.lastAutoVacuumAt).toBe(0);
    expect(out.lastAutoVacuumResult).toBe("");
  });

  it("autoVacuum 合并与 idleMinutes 白名单", () => {
    expect(normalizeMaintenance({ autoVacuum: { enabled: true, idleMinutes: 30 } }).autoVacuum).toEqual({
      enabled: true,
      idleMinutes: 30,
    });
    expect(normalizeMaintenance({ autoVacuum: { enabled: false, idleMinutes: 7 } }).autoVacuum).toEqual({
      enabled: false,
      idleMinutes: 15,
    });
    expect(normalizeMaintenance({ autoVacuum: { enabled: "yes", idleMinutes: 10 } }).autoVacuum).toEqual({
      enabled: false,
      idleMinutes: 10,
    });
  });

  it("枚举合法值保留；非负数字保留", () => {
    const out = normalizeMaintenance({
      reminderDismissedUntil: 123,
      lastAutoVacuumAt: 456,
      lastAutoVacuumResult: "success",
    });
    expect(out.reminderDismissedUntil).toBe(123);
    expect(out.lastAutoVacuumAt).toBe(456);
    expect(out.lastAutoVacuumResult).toBe("success");
  });
});
