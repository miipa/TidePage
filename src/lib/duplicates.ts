/** D7：重复书纯逻辑（建议保留、默认勾选、排序、忽略过滤）。 */
import type { DuplicateBook, DuplicateGroup } from "./api";

/** 建议保留：read_pct 高优先 → last_read 新 → 路径短 → id 小。 */
export function suggestKeepIndex(books: DuplicateBook[]): number {
  if (!books.length) return -1;
  let best = 0;
  for (let i = 1; i < books.length; i++) {
    const a = books[i];
    const b = books[best];
    if (a.readPct !== b.readPct) {
      if (a.readPct > b.readPct) best = i;
    } else if (a.lastRead !== b.lastRead) {
      if (a.lastRead > b.lastRead) best = i;
    } else if (a.path.length !== b.path.length) {
      if (a.path.length < b.path.length) best = i;
    } else if (a.id < b.id) {
      best = i;
    }
  }
  return best;
}

/** 兼容导出：默认不勾选任何书；扫描/打开/恢复忽略后都不得自动选中。 */
export function defaultDeletion(books: DuplicateBook[], _suspected = false): Record<number, boolean> {
  const out: Record<number, boolean> = {};
  books.forEach((b) => (out[b.id] = false));
  return out;
}

/** 父目录（Windows 路径大小写不敏感）；无分隔符时返回空串。 */
export function parentDirOf(path: string): string {
  const p = String(path ?? "").replace(/[\\/]+$/, "");
  const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  return i > 0 ? p.slice(0, i).replace(/\//g, "\\").toLowerCase() : "";
}

/** 当前结果涉及的全部父目录（去重、稳定排序、大小写不敏感）。 */
export function collectGroupDirs(groups: DuplicateGroup[]): string[] {
  const set = new Set<string>();
  for (const g of groups) for (const b of g.books) {
    const d = parentDirOf(b.path);
    if (d) set.add(d);
  }
  return [...set].sort((a, b) => a.localeCompare(b));
}

function normDir(dir: string): string {
  return dir.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
}

/** 目录筛选：dir 为空=全部；否则只保留该目录副本，不含该目录的组隐藏。 */
export function filterGroupsByDir(groups: DuplicateGroup[], dir: string): DuplicateGroup[] {
  const d = normDir(dir);
  if (!d) return groups;
  return groups
    .map((g) => ({ ...g, books: g.books.filter((b) => parentDirOf(b.path) === d) }))
    .filter((g) => g.books.length > 0);
}

/** 目录内代表书（按 suggestKeepIndex 规则在该目录副本内选）。 */
export function representativeForDir(books: DuplicateBook[], dir: string): DuplicateBook | null {
  const d = normDir(dir);
  const list = d ? books.filter((b) => parentDirOf(b.path) === d) : books;
  if (!list.length) return null;
  return list[suggestKeepIndex(list)];
}

/** C14：置信度文案（百分比整数）。 */
export function confidenceText(confidence: number): string {
  return Math.round(Math.max(0, Math.min(1, confidence)) * 100) + "%";
}

/** 组内保留项与待删除项（只读派生，UI 用）。 */
export function groupSelection(books: DuplicateBook[]): { keep: DuplicateBook | null; candidates: DuplicateBook[] } {
  const keep = suggestKeepIndex(books);
  return { keep: keep >= 0 ? books[keep] : null, candidates: keep >= 0 ? books.filter((_, i) => i !== keep) : [] };
}

/** 组排序：精确组在前 → 书数降序 → 大小降序 → key。 */
export function sortDuplicateGroups(groups: DuplicateGroup[]): DuplicateGroup[] {
  return [...groups].sort(
    (a, b) => Number(a.suspected) - Number(b.suspected) || b.books.length - a.books.length || b.size - a.size || a.key.localeCompare(b.key),
  );
}

/** 过滤忽略组 + 分页（大结果不一次性渲染）。 */
export function visibleDuplicateGroups(
  groups: DuplicateGroup[],
  ignored: Iterable<string>,
  page: number,
  pageSize: number,
): DuplicateGroup[] {
  const skip = new Set(ignored);
  const list = sortDuplicateGroups(groups).filter((g) => !skip.has(g.key));
  const size = Math.max(1, Math.floor(pageSize));
  const start = Math.max(0, Math.floor(page)) * size;
  return list.slice(start, start + size);
}

/** 组内行分页（按需渲染，避免大组一次建大量 DOM）。 */
export function pageGroupRows<T>(rows: T[], limit: number): { visible: T[]; hasMore: boolean; nextLimit: number } {
  const n = Math.max(1, Math.floor(limit));
  return { visible: rows.slice(0, n), hasMore: rows.length > n, nextLimit: Math.min(rows.length, n + 50) };
}

/** 实际待删除书：只统计未忽略组，并尊重每本书的勾选（忽略=不删）。 */
export function selectedForDeletion(
  groups: DuplicateGroup[],
  ignored: Iterable<string>,
  selection: Record<number, boolean>,
): DuplicateBook[] {
  const skip = new Set(ignored);
  return groups
    .filter((g) => !skip.has(g.key))
    .flatMap((g) => g.books.filter((b) => selection[b.id] === true));
}

/** 忽略某组时清掉该组所有勾选（确保不会误删）。 */
export function clearGroupSelection(
  selection: Record<number, boolean>,
  books: DuplicateBook[],
): Record<number, boolean> {
  const next = { ...selection };
  for (const b of books) delete next[b.id];
  return next;
}

/** 恢复被忽略的组：只恢复组、不勾选任何书。 */
export function restoreIgnoredGroups(
  groups: DuplicateGroup[],
  ignored: Iterable<string>,
  selection: Record<number, boolean>,
): { ignored: string[]; selection: Record<number, boolean> } {
  const skip = new Set(ignored);
  const next = { ...selection };
  for (const g of groups) {
    if (!skip.has(g.key)) continue;
    for (const b of g.books) delete next[b.id];
  }
  return { ignored: [], selection: next };
}

/** A/B 动作参数：A=仅移除书库(fromDisk=false)，B=移入回收站(fromDisk=true)。 */
export type DuplicateAction = "remove" | "trash";
export function actionInvocation(books: DuplicateBook[], action: DuplicateAction): { paths: string[]; fromDisk: boolean } {
  return { paths: books.map((b) => b.path), fromDisk: action === "trash" };
}

/** 去掉已被删除的书；剩余 <2 本的组消失。 */
export function pruneDeleted(
  groups: DuplicateGroup[],
  deletedIds: Iterable<number>,
): DuplicateGroup[] {
  const gone = new Set(deletedIds);
  return groups
    .map((g) => ({ ...g, books: g.books.filter((b) => !gone.has(b.id)) }))
    .filter((g) => g.books.length > 1);
}
