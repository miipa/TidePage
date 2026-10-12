export type DropPlace = "before" | "after";

/**
 * 拖拽重排：在 list 中把 id==fromId 的元素移动到 id==targetId 之前/之后，返回新数组。
 * 语义严格复刻现有 6 处处理器：先按 fromId 删除，再在**已删短的数组**上定位 targetId。
 * 任一无效条件返回 null，调用方据此跳过重赋值与持久化（保持 no-op 不触发 save）。
 */
export function reorderByDrop<T>(
  list: readonly T[],
  fromId: string,
  targetId: string,
  place: DropPlace,
  idOf: (item: T) => string,
): T[] | null {
  if (!fromId || fromId === targetId) return null;
  const arr = list.slice();
  const fi = arr.findIndex((x) => idOf(x) === fromId);
  if (fi < 0) return null;
  const [moved] = arr.splice(fi, 1);
  const ti = arr.findIndex((x) => idOf(x) === targetId);
  if (ti < 0) return null;
  arr.splice(place === "after" ? ti + 1 : ti, 0, moved);
  return arr;
}
