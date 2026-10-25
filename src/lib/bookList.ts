/** R11-2：书库列表总数缓存/是否计数的纯函数（前端只看 key，不做 SQL）。 */

export interface BookListQuery {
  query: string;
  sort: string;
  formats: string[];
  states: string[];
  folder: string | null;
  desc: boolean;
  /** D3：标签名（多选 AND）；空/省略=不约束。 */
  tags?: string[];
}

/** 总数缓存 key：查询/排序/筛选/文件夹/方向任一变化都要重新计数。 */
export function bookListKey(q: BookListQuery): string {
  const formats = [...q.formats].sort().join(",");
  const states = [...q.states].sort().join(",");
  const tags = [...(q.tags ?? [])].map((x) => x.trim().toLowerCase()).sort().join(",");
  return [q.query, q.sort, formats, states, q.folder ?? "", q.desc ? "1" : "0", tags].join("\u0001");
}

/** 当前 key 与已缓存 key 不同（或没有缓存）时需要带 COUNT；滚动窗口复用缓存不计数。 */
export function shouldCountTotal(cachedKey: string, key: string): boolean {
  return cachedKey !== key;
}

/** 后端 total=-1 表示「本次未计数」，此时沿用缓存总数（没有缓存时按 0 处理）。 */
export function resolveListTotal(apiTotal: number, cachedTotal: number): number {
  return apiTotal >= 0 ? apiTotal : Math.max(0, cachedTotal);
}
