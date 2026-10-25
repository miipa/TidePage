/** R9：翻页模式的「页码 ↔ 章内比例」纯函数。
 *
 * 只抽公共实现，语义与存储完全不变：
 * - 比例 = page / (pageCount - 1)，pageCount<=1 时为 0；
 * - 页码 = round(ratio * (pageCount - 1))，并 clamp 到 [0, pageCount-1]。
 * 进度/书签/划线/TTS/搜索仍存章号+比例/字符偏移，不新增字段。
 */

/** 当前页 → 章内比例（0..1）；单页章节恒为 0。 */
export function ratioFromPage(page: number, pageCount: number): number {
  if (!Number.isFinite(page) || !Number.isFinite(pageCount) || pageCount <= 1) return 0;
  return Math.max(0, Math.min(1, page / (pageCount - 1)));
}

/** 翻页模式下元素相对页容器的水平偏移 → 页号（内部链接锚点定位用；clamp 到 [0, pageCount-1]）。 */
export function pageFromLeftPosition(left: number, stepPx: number, pageCount: number): number {
  const last = Math.max(0, Math.floor(Number.isFinite(pageCount) ? pageCount : 0) - 1);
  if (!Number.isFinite(left) || !Number.isFinite(stepPx) || stepPx <= 0) return 0;
  return Math.max(0, Math.min(last, Math.floor(left / stepPx)));
}

/** 章内比例 → 当前页（四舍五入并 clamp）；单页章节恒为 0。 */
export function pageFromRatio(ratio: number, pageCount: number): number {
  const last = Math.max(0, Math.floor(Number.isFinite(pageCount) ? pageCount : 0) - 1);
  const r = Number.isFinite(ratio) ? Math.max(0, Math.min(1, ratio)) : 0;
  return Math.max(0, Math.min(last, Math.round(r * last)));
}
