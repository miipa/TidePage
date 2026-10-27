/**
 * F3：回落档窗口覆盖判定（纯函数）。
 * 用于确保「当前窗口不覆盖请求区间才补发请求」，并用 rowsLen（而非过滤后长度）
 * 判断，避免重复视图过滤改变长度导致的取数→赋值→effect 重跑循环。
 */
export function windowCovers(
  rangeStart: number,
  rowsLen: number,
  start: number,
  end: number,
): boolean {
  if (rowsLen <= 0) return false;
  return rangeStart <= start && rangeStart + rowsLen >= end;
}
