/** R7-3：书内搜索首次建立全文缓存的进度状态（纯函数，便于单测）。
 *
 * 后端在首次书内搜索分批写 chapter_text 时发 "book-text-progress" 事件
 * （payload: book_id/done/total）。本模块只负责把事件换算成弹窗要显示的
 * 百分比状态；请求结束/失败时由调用方 reset 清空。
 */

export interface SearchProgress {
  done: number;
  total: number;
  /** 0..100 的整数百分比。 */
  pct: number;
}

export interface SearchProgressEvent {
  bookId: number;
  done: number;
  total: number;
}

/** 把一次进度事件并入当前状态；返回 null 表示隐藏进度行。
 *
 * 口径（与单测一致）：
 * - 事件 bookId 与当前书不一致：保持原状态（其它书的进度不串台）；
 * - total <= 0：隐藏（无法算百分比）；
 * - done 越界 clamp 到 [0, total]；
 * - done >= total：显示 100%（由请求结束后清空）。
 */
export function applySearchProgress(
  prev: SearchProgress | null,
  ev: SearchProgressEvent,
  currentBookId: number,
): SearchProgress | null {
  if (ev.bookId !== currentBookId) return prev;
  const total = Math.floor(ev.total);
  if (!Number.isFinite(total) || total <= 0) return null;
  const done = Math.max(0, Math.min(Math.floor(ev.done), total));
  const pct = Math.round((done / total) * 100);
  return { done, total, pct };
}

/** 请求结束/失败时清空进度行。 */
export function resetSearchProgress(): null {
  return null;
}
