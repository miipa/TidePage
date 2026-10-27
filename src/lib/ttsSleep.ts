/** D8：朗读睡眠/章末停止的纯状态逻辑（TtsBar/Reader 只做展示与调用）。
 * sleepMinutes 语义：>0 = 分钟倒计时；-1 = 章末停止；0/undefined = 不启用。
 */
export type SleepMode = "off" | "minutes" | "chapter";

export function sleepModeOf(sleepMinutes: number | null | undefined): SleepMode {
  const m = sleepMinutes ?? 0;
  if (m < 0) return "chapter";
  if (m > 0) return "minutes";
  return "off";
}

/** 倒计时展示文本：分钟模式 mm:ss；章末模式固定提示；关闭为空串。 */
export function formatSleepRemaining(remainingMs: number, mode: SleepMode): string {
  if (mode === "chapter") return "章末停止";
  if (mode !== "minutes") return "";
  const total = Math.max(0, Math.ceil(remainingMs / 1000));
  const mm = Math.floor(total / 60);
  const ss = total % 60;
  return `${mm}:${String(ss).padStart(2, "0")}`;
}

/** 章末时是否允许自动进入下一章：只有未启用章末停止且全局允许续章。 */
export function shouldAutoNextChapter(
  sleepMinutes: number | null | undefined,
  autoNextChapter: boolean,
): boolean {
  return autoNextChapter && sleepModeOf(sleepMinutes) !== "chapter";
}

/** 分钟倒计时状态机：tick 返回新的剩余毫秒（<=0 表示到点）。 */
export function tickSleepRemaining(remainingMs: number, elapsedMs: number): number {
  if (!Number.isFinite(remainingMs) || !Number.isFinite(elapsedMs)) return 0;
  return Math.max(0, remainingMs - Math.max(0, elapsedMs));
}

/** 起读时初始化倒计时：分钟模式 minutes*60000，章末/关闭返回 null。 */
export function initialSleepRemaining(sleepMinutes: number | null | undefined): number | null {
  const m = sleepMinutes ?? 0;
  return m > 0 ? m * 60_000 : null;
}

/* ---------------- F5-H1：跨 TtsBar 挂载的睡眠定时 ---------------- */
// 为什么保留上面那组纯逻辑并用新增函数而不是替换：它们是既有测试覆盖的公共函数，
// 本轮只补「截止时间在 ttsState 里跨挂载保持」所需的新纯函数（TtsBar 卸载重挂不重置倒计时）。

/** 从 now 起 n 分钟后的截止时间（毫秒）；n<=0 返回 0（未启用）。 */
export function sleepDeadlineFromNow(minutes: number, now: number): number {
  if (!Number.isFinite(minutes) || minutes <= 0) return 0;
  return now + minutes * 60_000;
}

/** 距离截止的剩余秒数（向上取整，<=0 返回 0；未启用返回 0）。 */
export function sleepRemainingSec(deadline: number, now: number): number {
  if (!deadline || deadline <= 0) return 0;
  const left = Math.ceil((deadline - now) / 1000);
  return left > 0 ? left : 0;
}

/**
 * 是否需要（重新）起臂：
 * - minutes<=0：否（未启用/章末停止由 Reader 处理）；
 * - 没有有效截止时间：是；
 * - 已经过期：是（上一次朗读残留的截止时间不能把新一次朗读立刻停掉）；
 * - 设置分钟数与已起臂分钟数不同：是（用户改时长按当前时刻重新计时）；
 * - 否则：否（跨挂载保持同一截止时间，不重置）。
 */
export function shouldArmSleep(
  minutes: number,
  armedMinutes: number,
  deadline: number,
  now: number = Date.now(),
): boolean {
  if (!Number.isFinite(minutes) || minutes <= 0) return false;
  if (!deadline || deadline <= 0) return true;
  if (deadline <= now) return true;
  if (armedMinutes !== minutes) return true;
  return false;
}
