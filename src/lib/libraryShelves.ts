/** D3-3：智能书架（纯函数；存储由 settings 键 library.shelves.v1 负责）。 */
export interface Shelf {
  id: string;
  name: string;
  folder: string | null;
  formats: string[];
  states: string[];
  tags: string[];
  query: string;
  createdAt: number;
  /** B9：条目有效更新时间（旧书架回退 createdAt）。 */
  updatedAt?: number;
}

const MAX_NAME = 32;

export function normalizeShelfName(raw: string): string {
  const s = (raw ?? "").trim().replace(/\s+/g, " ");
  return s.slice(0, MAX_NAME);
}

/** 重名策略：自动追加 " (2)"、" (3)"…；大小写/空白归一后比较。 */
export function uniqueShelfName(base: string, existing: Shelf[]): string {
  const want = normalizeShelfName(base) || "书架";
  const used = new Set(existing.map((s) => s.name.trim().toLowerCase()));
  if (!used.has(want.toLowerCase())) return want;
  // 已带 " (n)" 后缀时从 n+1 继续，避免生成 "科幻 (2) (2)"。
  const m = /^(.*) \((\d+)\)$/.exec(want);
  const stem = m ? m[1] : want;
  const start = m ? Number(m[2]) + 1 : 2;
  for (let i = start; i < 1000; i++) {
    // G17：后缀也要计入 32 字上限；先截短 stem 再拼，保证落库后 normalizeShelfName
    // 不会把后缀截掉导致重名。
    const suffix = ` (${i})`;
    const stemMax = Math.max(1, MAX_NAME - suffix.length);
    const cand = stem.slice(0, stemMax) + suffix;
    if (!used.has(cand.toLowerCase())) return cand;
  }
  return want;
}

export function serializeShelves(list: Shelf[]): string {
  return JSON.stringify({ v: 1, shelves: list });
}

function strArr(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && x.trim() !== "") : [];
}

/** 默认 id 工厂：保证与时间相关且带随机尾，避免同一毫秒内重复。 */
function defaultShelfId(n: number): string {
  return `shelf-${Date.now().toString(36)}-${n}-${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * F8-H1：按 id 去重。保留首次出现的 id；后续重复/空 id 用 idFactory 重新分配。
 * 返回新数组（不修改入参），顺序与内容不变 —— D5 合并可能产生重复 id，
 * Library keyed each (s.id) 遇到重复会直接抛错。
 */
export function dedupeShelfIds(
  list: Shelf[],
  idFactory: (n: number) => string = defaultShelfId,
): Shelf[] {
  const seen = new Set<string>();
  let seq = 0;
  return list.map((s) => {
    const id = typeof s.id === "string" ? s.id : "";
    if (id && !seen.has(id)) {
      seen.add(id);
      return s;
    }
    let fresh = "";
    do {
      fresh = idFactory(++seq);
    } while (!fresh || seen.has(fresh));
    seen.add(fresh);
    return { ...s, id: fresh };
  });
}

/** 反序列化 + 字段校验；坏 JSON/坏条目返回空/被过滤，绝不抛错。返回前按 id 去重（F8-H1）。 */
export function deserializeShelves(raw: string | null | undefined): Shelf[] {
  if (!raw) return [];
  try {
    const o = JSON.parse(raw);
    const arr = Array.isArray(o) ? o : Array.isArray(o?.shelves) ? o.shelves : [];
    const out: Shelf[] = [];
    for (const e of arr) {
      if (!e || typeof e !== "object") continue;
      const id = typeof e.id === "string" ? e.id : "";
      const name = normalizeShelfName(typeof e.name === "string" ? e.name : "");
      if (!id || !name) continue;
      out.push({
        id,
        name,
        folder: typeof e.folder === "string" && e.folder.trim() ? e.folder : null,
        formats: strArr(e.formats),
        states: strArr(e.states).filter((s) => s === "read" || s === "unread" || s === "reading"),
        tags: strArr(e.tags),
        query: typeof e.query === "string" ? e.query : "",
        createdAt: typeof e.createdAt === "number" ? e.createdAt : 0,
        updatedAt: typeof e.updatedAt === "number" && e.updatedAt > 0 ? e.updatedAt : (typeof e.createdAt === "number" ? e.createdAt : 0),
      });
    }
    return dedupeShelfIds(out);
  } catch {
    return [];
  }
}

export interface ShelfFilterSnapshot {
  folder: string | null;
  formats: string[];
  states: string[];
  tags: string[];
  query: string;
}

/** B9：按内容变化补 updatedAt（新书架/内容变化→now；未变保留旧值）。 */
export function stampShelfUpdates(next: Shelf[], prev: Shelf[], now: number): Shelf[] {
  const old = new Map(prev.map((s) => [s.id, s]));
  return next.map((s) => {
    const before = old.get(s.id);
    const stamp = s.updatedAt && s.updatedAt > 0 ? s.updatedAt : (s.createdAt || 0);
    if (!before) return { ...s, updatedAt: now };
    const same = before.name === s.name && before.folder === s.folder && before.query === s.query
      && JSON.stringify(before.formats) === JSON.stringify(s.formats)
      && JSON.stringify(before.states) === JSON.stringify(s.states)
      && JSON.stringify(before.tags) === JSON.stringify(s.tags);
    return same ? s : { ...s, updatedAt: now, createdAt: s.createdAt || stamp };
  });
}

/** 从当前筛选构造书架（id/createdAt 由调用方补）。 */
export function shelfFromFilter(name: string, f: ShelfFilterSnapshot, existing: Shelf[]): Shelf {
  return {
    id: `shelf-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    name: uniqueShelfName(name, existing),
    folder: f.folder,
    formats: [...f.formats],
    states: [...f.states],
    tags: [...f.tags],
    query: f.query,
    createdAt: Date.now(),
  };
}

/** 应用书架前校验：非法/空名返回 null；返回可直接套用的筛选快照。 */
export function shelfFilter(s: Shelf | null | undefined): ShelfFilterSnapshot | null {
  if (!s || !s.id) return null;
  return {
    folder: s.folder,
    formats: s.formats.filter((x) => x.trim() !== ""),
    states: s.states.filter((x) => x === "read" || x === "unread" || x === "reading"),
    tags: s.tags.filter((x) => x.trim() !== ""),
    query: s.query,
  };
}
