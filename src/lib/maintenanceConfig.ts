/**
 * F9：维护配置归一化（纯函数）。
 *
 * load() 与 applyConfigJson() 各有一份，前者做过默认合并、后者没有；统一到这里，
 * 避免广播/导入来的旧配置缺字段把 UI 状态打回 undefined。
 * defaults 参数由调用方传入（stores 的 DEFAULTS.maintenance），避免纯模块反向依赖 stores。
 */
export interface MaintenanceConfigShape {
  reminderDismissedUntil: number;
  autoVacuum: { enabled: boolean; idleMinutes: number };
  lastAutoVacuumAt: number;
  lastAutoVacuumResult: "success" | "error" | "skipped" | "";
}

export const DEFAULT_MAINTENANCE: MaintenanceConfigShape = {
  reminderDismissedUntil: 0,
  autoVacuum: { enabled: false, idleMinutes: 15 },
  lastAutoVacuumAt: 0,
  lastAutoVacuumResult: "",
};

const IDLE_MINUTES = [10, 15, 30];

/** 非负有限数字，非法回退 fallback。 */
function nonNeg(raw: unknown, fallback: number): number {
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

export function normalizeMaintenance(
  raw: unknown,
  defaults: MaintenanceConfigShape = DEFAULT_MAINTENANCE,
): MaintenanceConfigShape {
  const src = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const avSrc =
    src.autoVacuum && typeof src.autoVacuum === "object"
      ? (src.autoVacuum as Record<string, unknown>)
      : {};
  const avDefaults = defaults.autoVacuum ?? DEFAULT_MAINTENANCE.autoVacuum;
  const idle = Number(avSrc.idleMinutes);
  const result = src.lastAutoVacuumResult;
  return {
    reminderDismissedUntil: nonNeg(src.reminderDismissedUntil, defaults.reminderDismissedUntil),
    autoVacuum: {
      enabled: typeof avSrc.enabled === "boolean" ? avSrc.enabled : avDefaults.enabled === true,
      idleMinutes: IDLE_MINUTES.includes(idle) ? idle : avDefDefaults(avDefaults),
    },
    lastAutoVacuumAt: nonNeg(src.lastAutoVacuumAt, defaults.lastAutoVacuumAt),
    lastAutoVacuumResult:
      result === "success" || result === "error" || result === "skipped"
        ? result
        : defaults.lastAutoVacuumResult,
  };
}

/** idleMinutes 默认值也做白名单校验，保证返回值始终是合法枚举。 */
function avDefDefaults(av: { enabled: boolean; idleMinutes: number }): number {
  return IDLE_MINUTES.includes(Number(av.idleMinutes)) ? Number(av.idleMinutes) : 15;
}
