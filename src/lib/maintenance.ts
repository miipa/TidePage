/** B10：维护提醒/预检/历史纯函数。 */
export const FREELIST_ALERT_BYTES = 200 * 1024 * 1024;
export const FREELIST_ALERT_RATIO = 0.1;
export const DISMISS_DAYS = 7;

export function formatBytes(n: number): string {
  const v = Math.max(0, n);
  if (v >= 1024 ** 3) return (v / 1024 ** 3).toFixed(v >= 10 * 1024 ** 3 ? 0 : 1) + " GB";
  if (v >= 1024 ** 2) return Math.round(v / 1024 ** 2) + " MB";
  if (v >= 1024) return Math.round(v / 1024) + " KB";
  return v + " B";
}


/** C15：封面/缓存治理建议阈值（只提示，不自动清理）。 */
export const COVER_CACHE_HINT_BYTES = 8 * 1024 ** 3;

export function shouldSuggestCoverCleanup(cacheBytes: number, coversBytes: number): boolean {
  return cacheBytes >= COVER_CACHE_HINT_BYTES || coversBytes >= COVER_CACHE_HINT_BYTES;
}

/** 审计可回收口径文案用：孤儿字节 + 同 stem 冗余字节。 */
export function coverReclaimBytes(orphanBytes: number, sameStemBytes: number): number {
  return Math.max(0, orphanBytes) + Math.max(0, sameStemBytes);
}
/** 可回收提醒：freelist > 200MB 或占 DB > 10%；关闭后 7 天内不再提示。 */
export function shouldRemindMaintenance(freelistBytes: number, dbBytes: number, dismissedAt: number, now: number): boolean {
  if (dismissedAt > 0 && now - dismissedAt < DISMISS_DAYS * 86_400_000) return false;
  if (freelistBytes >= FREELIST_ALERT_BYTES) return true;
  return dbBytes > 0 && freelistBytes / dbBytes >= FREELIST_ALERT_RATIO;
}

/** 预计临时空间 ≈ DB + 64MB；同盘只计一次。 */
export function estimateVacuumTemp(dbBytes: number, sameDisk: boolean): { requiredBytes: number; checks: number } {
  const required = dbBytes + 64 * 1024 * 1024;
  return { requiredBytes: required, checks: sameDisk ? 1 : 2 };
}

/** B10：自动整理条件（frontend 调度与测试共用）。 */
export function shouldAutoVacuum(opts: {
  enabled: boolean;
  idleMinutes: number;
  now: number;
  lastActivity: number;
  readerOpen: boolean;
  maintenanceRunning: boolean;
  scanRunning: boolean;
  sessionDone: boolean;
  lastAutoVacuumAt: number;
  lastAutoVacuumResult: string;
}): boolean {
  if (!opts.enabled || opts.sessionDone) return false;
  if (opts.readerOpen || opts.maintenanceRunning || opts.scanRunning) return false;
  const idleMs = Math.max(1, opts.idleMinutes) * 60_000;
  if (opts.now - opts.lastActivity < idleMs) return false;
  if (opts.lastAutoVacuumResult !== "success" && opts.lastAutoVacuumAt > 0 && opts.now - opts.lastAutoVacuumAt < 24 * 60 * 60_000) return false;
  return true;
}

export type MaintenanceResultKind = "success" | "error" | "skipped";
/** 历史结果归一：success/error/skipped（未知按 error）。 */
export function maintenanceResultKind(result: string): MaintenanceResultKind {
  return result === "success" || result === "skipped" ? result : "error";
}

export type MaintenanceRecordLike = {
  at: number; result: string; error: string; freed: number; durationMs: number; backupPath?: string;
};
/** 历史行结构化数据；文案由组件用 i18n key 组装，不含硬编码中文。 */
export function maintenanceRecordView(r: MaintenanceRecordLike) {
  return {
    at: r.at,
    result: maintenanceResultKind(r.result),
    freed: r.freed,
    durationMs: Math.round(r.durationMs),
    error: r.error,
    backupPath: r.backupPath || "",
  };
}
