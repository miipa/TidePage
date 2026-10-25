import { describe, it, expect } from "vitest";
import {
  choicesFromSelections,
  conflictChoiceToResolve,
  decideByTime,
  normalizeDataSyncOptions,
  syncCountRows,
} from "./davSyncApply";

describe("davSyncApply", () => {
  it("时间决策：本地/远端较新，相同或缺省为冲突", () => {
    expect(decideByTime(200, 100)).toBe("local");
    expect(decideByTime(100, 200)).toBe("remote");
    expect(decideByTime(100, 100)).toBe("conflict");
    expect(decideByTime(0, 100)).toBe("conflict");
    expect(decideByTime(100, 0)).toBe("conflict");
  });

  it("选项归一化：旧配置安全回落，间隔至少 1", () => {
    expect(normalizeDataSyncOptions({})).toEqual({
      syncNotes: true, syncTags: true, syncShelves: true, autoEnabled: false, intervalHours: 24,
    });
    expect(normalizeDataSyncOptions({ syncNotes: false, dataSync: { enabled: true, intervalHours: 0 } })).toEqual({
      syncNotes: false, syncTags: true, syncShelves: true, autoEnabled: true, intervalHours: 24,
    });
    expect(normalizeDataSyncOptions({ dataSync: { intervalHours: 0.2 } }).intervalHours).toBe(1);
  });

  it("结果计数顺序", () => {
    expect(syncCountRows({ added: 1, updated: 2, skipped: 3, conflicts: 4 }).map((r) => r.key)).toEqual([
      "added", "updated", "skipped", "conflicts",
    ]);
  });

  it("逐条冲突选择：未选择默认保留本机", () => {
    const cs = [{ key: "highlight:a" }, { key: "highlight:b" }, { key: "shelves:c" }];
    expect(choicesFromSelections(cs, { "highlight:b": "remote" })).toEqual([
      { key: "highlight:a", choice: "local" },
      { key: "highlight:b", choice: "remote" },
      { key: "shelves:c", choice: "local" },
    ]);
  });

  it("冲突选择映射", () => {
    expect(conflictChoiceToResolve("local")).toBe("local");
    expect(conflictChoiceToResolve("remote")).toBe("remote");
  });
});
