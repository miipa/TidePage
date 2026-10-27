/**
 * F1：进度保存失败的重试决策（纯函数）。
 * 首次失败（attempt=0）重试一次；重试仍失败时由调用方提示一次并停止本轮。
 */
export function shouldRetryProgressSave(attempt: number): boolean {
  return attempt === 0;
}

/** 重试前的等待（毫秒）：只重试一次，2s 足够避开瞬时锁/IO 抖动。 */
export const PROGRESS_RETRY_DELAY_MS = 2000;
