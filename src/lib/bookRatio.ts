/**
 * A4：窗口化滚动条（PDF/漫画瀑布流）的拇指比例，全书量程。
 * 窗口起始章 + 窗口内滚动比例 × 窗口章数，再除以总章数；与底部进度条同源。
 * 单章窗口时退化为 (start + ratio) / total，与既有行为一致。
 */
export function windowedBookRatio(
  startChapter: number,
  windowChapters: number,
  scrollRatio: number,
  totalChapters: number,
): number {
  const total = Math.max(1, Math.floor(totalChapters) || 1);
  const start = Math.max(0, Math.min(total - 1, Math.floor(startChapter) || 0));
  const span = Math.max(0, windowChapters || 0);
  const within = Math.max(0, Math.min(1, Number.isFinite(scrollRatio) ? scrollRatio : 0));
  const raw = (start + within * span) / total;
  return Math.max(0, Math.min(1, raw));
}
