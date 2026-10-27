/**
 * F7：有界 LRU 缓存（纯类，泛型）。
 *
 * pdf.ts 的直读页元信息按页缓存，跨书会串页且无上限；这里以 bookId+page 为 key，
 * 容量固定，命中即移到末尾，超限淘汰最旧。
 */
export class LruCache<K, V> {
  private map = new Map<K, V>();
  private readonly max: number;

  constructor(max: number) {
    this.max = Math.max(1, Math.floor(max));
  }

  get size(): number {
    return this.map.size;
  }

  get(key: K): V | undefined {
    const v = this.map.get(key);
    if (v === undefined) return undefined;
    // 命中移到末尾，保证 Map 插入顺序 = LRU 顺序。
    this.map.delete(key);
    this.map.set(key, v);
    return v;
  }

  set(key: K, value: V): void {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.max) {
      const oldest = this.map.keys().next();
      if (oldest.done) break;
      this.map.delete(oldest.value);
    }
  }

  clear(): void {
    this.map.clear();
  }
}
