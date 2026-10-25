/** R12-1：PDF 封面补全队列（纯逻辑，Library 只负责取数与渲染）。
 *
 * 目标：可见区 PDF 优先、串行、去重、失败不重挑；可见队列清空后才允许按小批量空闲补充。
 * 队列只存 id，不持有任何行对象 / DOM / pdf.js 资源。
 */

export interface PdfCoverQueue {
  /** 可见区优先队列（FIFO；新可见项插到队首）。 */
  visible: number[];
  /** 已在 queue/已处理集合（tried 或已入队），避免重复。 */
  seen: Set<number>;
}

export function createPdfCoverQueue(): PdfCoverQueue {
  return { visible: [], seen: new Set() };
}

/** 可见区 PDF id 插到队首（保持传入顺序在前、去重、排除 tried/已入队）；返回新增 id。 */
export function enqueueVisible(
  q: PdfCoverQueue,
  ids: Iterable<number>,
  tried: ReadonlySet<number>,
): number[] {
  const added: number[] = [];
  for (const id of ids) {
    if (!Number.isFinite(id) || id <= 0) continue;
    if (tried.has(id) || q.seen.has(id)) continue;
    q.seen.add(id);
    added.push(id);
  }
  if (added.length) q.visible = [...added, ...q.visible];
  return added;
}

/** 空闲补充批次追加到队尾（去重、排除 tried/已入队）。 */
export function enqueueIdle(q: PdfCoverQueue, ids: Iterable<number>): number[] {
  const added: number[] = [];
  for (const id of ids) {
    if (!Number.isFinite(id) || id <= 0) continue;
    if (q.seen.has(id)) continue;
    q.seen.add(id);
    q.visible.push(id);
    added.push(id);
  }
  return added;
}

/** 取下一个待处理 id（可见队列优先）；空返回 null。 */
export function takeNext(q: PdfCoverQueue): number | null {
  const id = q.visible.shift();
  return id === undefined ? null : id;
}

export function queueSize(q: PdfCoverQueue): number {
  return q.visible.length;
}

export function clearQueue(q: PdfCoverQueue): void {
  q.visible = [];
  q.seen.clear();
}

/** 从「全库缺封面」id 列表中挑空闲补充批次：跳过 tried/已入队，最多 max 个。 */
export function pickIdleBatch(
  ids: Iterable<number>,
  tried: ReadonlySet<number>,
  seen: ReadonlySet<number>,
  max: number,
): number[] {
  const out: number[] = [];
  for (const id of ids) {
    if (out.length >= max) break;
    if (!Number.isFinite(id) || id <= 0) continue;
    if (tried.has(id) || seen.has(id)) continue;
    out.push(id);
  }
  return out;
}
