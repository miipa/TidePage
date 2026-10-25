/** D8：单本覆盖合并（纯函数，供 bookcfg.setBookOverride 使用）。
 * 值为 undefined 的键被移除（回退全局）；其它键原样保留。 */
export function mergeBookOverride<T extends object>(cur: T, patch: Partial<T>): T {
  const next = { ...cur, ...patch } as Record<string, unknown>;
  for (const k of Object.keys(next)) {
    if (next[k] === undefined) delete next[k];
  }
  return next as T;
}
