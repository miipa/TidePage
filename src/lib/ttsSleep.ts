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
