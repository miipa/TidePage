/** C13：Reader 章节缓存字节预算（HTML 按 JS 字符串 UTF-16 估算 length*2）。 */
export const CHAPTER_CACHE_BYTE_BUDGET = 32 * 1024 * 1024;

export interface ChapterCacheLike {
  html?: string;
}

export function chapterBytes(ch: ChapterCacheLike): number {
  return (ch.html?.length ?? 0) * 2;
}

/** 超预算淘汰最旧章节（保留当前章；单章超预算时允许它单独存在，清掉其它）。 */
export function evictChapterCache(
  cache: Map<number, ChapterCacheLike>,
  maxBytes = CHAPTER_CACHE_BYTE_BUDGET,
  currentIdx = -1,
): void {
  let total = 0;
  for (const ch of cache.values()) total += chapterBytes(ch);
  while (total > maxBytes && cache.size > 1) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    if (oldest === currentIdx) {
      // 当前章不能淘汰：先跳过，淘汰下一项。
      const entries = [...cache.entries()];
      const victim = entries.find(([k]) => k !== currentIdx);
      if (!victim) break;
      total -= chapterBytes(victim[1]);
      cache.delete(victim[0]);
      continue;
    }
    const ch = cache.get(oldest);
    if (!ch) { cache.delete(oldest); continue; }
    total -= chapterBytes(ch);
    cache.delete(oldest);
  }
}
