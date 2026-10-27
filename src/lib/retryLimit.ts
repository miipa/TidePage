// 按帧重试上限的纯判定：漫画测量与分页测量共用同一范式，
// 只把「是否还能再试」独立成纯函数，便于 vitest 覆盖边界而不依赖 rAF/DOM。
// 为什么单独成模块：组件里直接写 `attempt >= limit` 无法断言边界，回归时容易把上限删掉。

/** 漫画测量：容器尺寸为 0 时的按帧重试上限（60 帧 ≈ 1 秒）。 */
export const MANGA_MEASURE_RETRY_LIMIT = 60;

/**
 * attempt 是「已经重试过的帧数」（从 0 开始）。
 * 返回 true 表示还允许再排一帧；false 表示本次重试链到此为止。
 * 上限本身不含在允许范围内：attempt === limit 时停止。
 */
export function shouldRetryMangaMeasure(attempt: number, limit: number = MANGA_MEASURE_RETRY_LIMIT): boolean {
  if (!Number.isFinite(attempt) || !Number.isFinite(limit)) return false;
  const a = Math.max(0, Math.floor(attempt));
  const l = Math.max(0, Math.floor(limit));
  return a < l;
}
