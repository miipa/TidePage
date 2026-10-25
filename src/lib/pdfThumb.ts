/** D6-2：PDF 缩略图纯逻辑（可见范围、LRU、打开态）；渲染在 pdf.ts。 */

/** 固定窗口可见范围（start 为窗口内首个可见项，overscan 两侧预取）。 */
export function visibleThumbRange(
  start: number,
  count: number,
  total: number,
  overscan = 6,
): { start: number; end: number } {
  if (total <= 0 || count <= 0) return { start: 0, end: 0 };
  const s = Math.max(0, Math.floor(start) - overscan);
  const e = Math.min(total, Math.ceil(start + count) + overscan);
  return { start: s, end: Math.max(s, e) };
}

/** 固定网格虚拟滚动：按行计算可见范围（含 overscan 行）。total=0 返回 0 行。 */
export function visibleThumbRows(
  scrollTop: number,
  viewportH: number,
  rowH: number,
  cols: number,
  total: number,
  overscanRows = 2,
): { startRow: number; endRow: number; rows: number } {
  const rows = Math.max(0, Math.ceil(total / Math.max(1, cols)));
  if (!rows || rowH <= 0) return { startRow: 0, endRow: 0, rows };
  const first = Math.max(0, Math.floor(scrollTop / rowH) - overscanRows);
  const last = Math.min(rows, Math.ceil((scrollTop + Math.max(0, viewportH)) / rowH) + overscanRows);
  return { startRow: first, endRow: Math.max(first, last), rows };
}

/** 当前页所在行（用于打开面板时定位）。 */
export function rowOfPage(page: number, cols: number): number {
  return Math.floor(Math.max(0, page) / Math.max(1, cols));
}

/** 缩略图 LRU：命中刷新新鲜度；超容淘汰最旧。 */
export function thumbLruStep(
  order: string[],
  token: string,
  max: number,
): { order: string[]; evicted: string[] } {
  const next = order.filter((t) => t !== token);
  next.push(token);
  const evicted: string[] = [];
  while (next.length > Math.max(1, max)) {
    const old = next.shift();
    if (old) evicted.push(old);
  }
  return { order: next, evicted };
}

/** 打开态归一化（localStorage）：只认 "1"/"0" 字符串，未知回落 false。 */
export function normalizeThumbsOpen(raw: string | null | undefined): boolean {
  return raw === "1" || raw === "true";
}

/** 页码归一：入参可能越界/非数字 → 落到 [0,total-1]；空文档返回 0。 */
export function clampPage(page: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(total - 1, Math.max(0, Math.floor(page || 0)));
}
