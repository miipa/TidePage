/** B9：自动同步触发/退避纯函数（不碰 store；供主窗口调度复用）。 */
export type SyncLastResult = "success" | "error" | "skipped" | "";

const BASE = 60_000;
const MAX = 30 * 60_000;

/** 失败后指数退避：60s × 2^n，上限 30min；0 次失败返回 0。 */
export function nextBackoffMs(failCount: number): number {
  if (failCount <= 0) return 0;
  return Math.min(MAX, BASE * 2 ** Math.min(failCount - 1, 10));
}

/** 是否允许自动触发：启用 + 距上次成功/尝试满足间隔 + 退避窗口已过。 */
export function shouldAutoSync(opts: {
  enabled: boolean;
  intervalHours: number;
  lastAt: number;
  now: number;
  failCount: number;
  lastAttemptAt: number;
}): boolean {
  if (!opts.enabled) return false;
  const interval = Math.max(1, opts.intervalHours) * 60 * 60_000;
  if (opts.lastAt > 0 && opts.now - opts.lastAt < interval) return false;
  const backoff = nextBackoffMs(opts.failCount);
  if (backoff > 0 && opts.lastAttemptAt > 0 && opts.now - opts.lastAttemptAt < backoff) return false;
  return true;
}

/** 事件触发防抖：同一窗口内只允许一次（默认 30s）。 */
export function allowEventTrigger(lastEventAt: number, now: number, windowMs = 30_000): boolean {
  return lastEventAt <= 0 || now - lastEventAt >= windowMs;
}

/** 旧配置/异常值归一化为合法结果枚举。 */
export function normalizeLastResult(v: unknown): SyncLastResult {
  return v === "success" || v === "error" || v === "skipped" ? v : "";
}
