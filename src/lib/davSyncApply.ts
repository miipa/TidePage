/** D5：WebDAV 数据同步的前端纯决策/归一化（后端已完成合并，前端只做冲突展示与重跑）。 */
export type SyncTimeDecision = "local" | "remote" | "conflict";

/** 时间戳决策：本地较新→local，远端较新→remote，相同/缺失→conflict（无法自动判定）。 */
export function decideByTime(localTime: number, remoteTime: number): SyncTimeDecision {
  if (localTime > 0 && remoteTime > 0 && localTime !== remoteTime) {
    return localTime > remoteTime ? "local" : "remote";
  }
  return "conflict";
}

/** 同步选项归一化：旧配置缺字段时安全回落 false，间隔至少 1 小时。 */
export function normalizeDataSyncOptions(wd: {
  syncNotes?: boolean;
  syncTags?: boolean;
  syncShelves?: boolean;
  dataSync?: { enabled?: boolean; intervalHours?: number };
}): { syncNotes: boolean; syncTags: boolean; syncShelves: boolean; autoEnabled: boolean; intervalHours: number } {
  return {
    syncNotes: wd.syncNotes !== false,
    syncTags: wd.syncTags !== false,
    syncShelves: wd.syncShelves !== false,
    autoEnabled: wd.dataSync?.enabled === true,
    intervalHours: Math.max(1, Number(wd.dataSync?.intervalHours) || 24),
  };
}

export interface SyncCounts {
  added: number;
  updated: number;
  skipped: number;
  conflicts: number;
}

/** 结果计数行（固定顺序，UI/测试共用）。 */
export function syncCountRows(s: SyncCounts): { key: keyof SyncCounts; value: number }[] {
  return [
    { key: "added", value: s.added },
    { key: "updated", value: s.updated },
    { key: "skipped", value: s.skipped },
    { key: "conflicts", value: s.conflicts },
  ];
}

/** 逐条冲突选择：未选择的条目默认保留本机；返回后端 resolveChoices。 */
export function choicesFromSelections(
  conflicts: { key: string }[],
  selections: Record<string, "local" | "remote">,
): { key: string; choice: "local" | "remote" }[] {
  return conflicts.map((c) => ({ key: c.key, choice: selections[c.key] ?? "local" }));
}

/** 冲突选择：保留本机 = local；采用云端 = remote（由前端重跑同步并指定 resolve）。 */
export function conflictChoiceToResolve(choice: "local" | "remote"): "local" | "remote" {
  return choice;
}
