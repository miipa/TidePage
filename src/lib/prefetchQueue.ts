/**
 * F1：章节预取队列规划（纯函数）。
 *
 * 旧实现只 push 不裁剪：跳章/换中心后旧条目仍排队，队列只增不减。
 * 这里每次按「距当前中心由近到远」直接重建期望窗口，已缓存/在途的章节排除，
 * 再截断到 max，保证队列长度有界且靠近新中心的章节优先。
 */
export function planPrefetchQueue(
  center: number,
  total: number,
  span: number,
  cached: ReadonlySet<number>,
  inflight: ReadonlySet<number>,
  _currentQueue: readonly number[],
  max: number,
): number[] {
  if (!Number.isFinite(center) || !Number.isFinite(total) || total <= 0) return [];
  const out: number[] = [];
  const seen = new Set<number>();
  const push = (n: number) => {
    if (out.length >= max) return;
    if (n < 0 || n >= total || n === center) return;
    if (seen.has(n) || cached.has(n) || inflight.has(n)) return;
    seen.add(n);
    out.push(n);
  };
  // 由近到远交替取两侧（先后一后前一），与旧实现顺序一致但受 max 约束。
  for (let d = 1; d <= span && out.length < max; d++) {
    push(center + d);
    push(center - d);
  }
  return out;
}
