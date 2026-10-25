/** 主书库去重 hidden 的 localStorage 缓存 v2（tp.dupHidden.v2）。 */
export const HIDDEN_CACHE_KEY = "tp.dupHidden.v2";
export const HIDDEN_CACHE_MAX_IDS = 300_000;
export const HIDDEN_CACHE_MAX_GROUPS = 200_000;

export interface DupDisplayGroup {
  repId: number;
  copies: number;
  memberIds: number[];
}

export interface HiddenCache {
  groups: DupDisplayGroup[];
  savedAt: number;
}

/** 只接受正整数 id、去重、上限保护；其它一律丢弃。 */
export function normalizeHiddenIds(raw: unknown): number[] {
  if (!Array.isArray(raw)) return [];
  const out: number[] = [];
  const seen = new Set<number>();
  for (const v of raw) {
    const n = typeof v === "number" ? v : Number(v);
    if (!Number.isSafeInteger(n) || n <= 0 || seen.has(n)) continue;
    seen.add(n);
    out.push(n);
    if (out.length >= HIDDEN_CACHE_MAX_IDS) break;
  }
  return out;
}

export function hiddenIdsFromGroups(groups: DupDisplayGroup[]): number[] {
  const out: number[] = [];
  for (const g of groups) for (const id of g.memberIds) if (id !== g.repId) out.push(id);
  return normalizeHiddenIds(out);
}

function normalizeGroups(raw: unknown): DupDisplayGroup[] {
  if (!Array.isArray(raw)) return [];
  const out: DupDisplayGroup[] = [];
  for (const row of raw) {
    if (!row || typeof row !== "object") continue;
    const r = row as { repId?: unknown; memberIds?: unknown };
    if (!Number.isSafeInteger(r.repId) || (r.repId as number) <= 0) continue;
    const members = normalizeHiddenIds(r.memberIds);
    if (members.length < 2) continue;
    out.push({ repId: r.repId as number, copies: members.length, memberIds: members });
    if (out.length >= HIDDEN_CACHE_MAX_GROUPS) break;
  }
  return out;
}

/** 非法 JSON / 非对象 / groups 非数组 → null；空 groups 合法。 */
export function parseHiddenCache(raw: string | null): HiddenCache | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as { groups?: unknown; savedAt?: unknown };
    if (!v || typeof v !== "object" || !Array.isArray(v.groups)) return null;
    const groups = normalizeGroups(v.groups);
    const savedAt = typeof v.savedAt === "number" && Number.isFinite(v.savedAt) ? v.savedAt : 0;
    return { groups, savedAt };
  } catch {
    return null;
  }
}

export function serializeHiddenCache(groups: DupDisplayGroup[], savedAt: number): string {
  return JSON.stringify({ groups: normalizeGroups(groups), savedAt: Math.max(0, Math.floor(savedAt)) });
}
