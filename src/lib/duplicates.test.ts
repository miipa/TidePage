import { describe, it, expect } from "vitest";
import {
  actionInvocation,
  clearGroupSelection,
  collectGroupDirs,
  confidenceText,
  defaultDeletion,
  filterGroupsByDir,
  parentDirOf,
  representativeForDir,
  groupSelection,
  pageGroupRows,
  pruneDeleted,
  restoreIgnoredGroups,
  selectedForDeletion,
  sortDuplicateGroups,
  suggestKeepIndex,
  visibleDuplicateGroups,
} from "./duplicates";
import type { DuplicateBook, DuplicateGroup } from "./api";

const book = (patch: Partial<DuplicateBook> = {}): DuplicateBook => ({
  id: 1, path: "C:/a.epub", title: "T", author: "", format: "epub", size: 10,
  addedAt: 1, lastRead: 0, readPct: 0, ...patch,
});
const group = (key: string, books: DuplicateBook[], suspected = false): DuplicateGroup => ({
  key, size: books[0]?.size ?? 0, suspected,
  reason: suspected ? "title-author-equal" : "exact",
  confidence: suspected ? 0.92 : 1,
  books,
});

describe("duplicates", () => {
  it("建议保留：阅读进度高 → 最近读 → 路径短 → id 小", () => {
    const bs = [book({ id: 1, path: "C:/long/path/x.epub" }), book({ id: 2, path: "C:/b.epub", readPct: 50 })];
    expect(suggestKeepIndex(bs)).toBe(1);
    const same = [book({ id: 1, path: "C:/lo.epub" }), book({ id: 2, path: "C:/short.epub" })];
    expect(suggestKeepIndex(same)).toBe(0);
    expect(suggestKeepIndex([book({ id: 2 }), book({ id: 1 })])).toBe(1);
    expect(suggestKeepIndex([])).toBe(-1);
  });

  it("默认选中为空，保留建议仍按阅读规则", () => {
    const bs = [book({ id: 1, readPct: 10 }), book({ id: 2, readPct: 90 }), book({ id: 3 })];
    expect(defaultDeletion(bs)).toEqual({ 1: false, 2: false, 3: false });
    expect(defaultDeletion(bs, true)).toEqual({ 1: false, 2: false, 3: false });
    expect(groupSelection(bs).keep?.id).toBe(2);
    expect(groupSelection(bs).candidates.map((b) => b.id)).toEqual([1, 3]);
  });

  it("C14 疑似组排序精确在前、置信度文案", () => {
    const bs = [book({ id: 1 }), book({ id: 2 })];
    expect(defaultDeletion(bs)).toEqual({ 1: false, 2: false });
    const exact = group("a", [book({ id: 1 }), book({ id: 2 })]);
    const sus = group("b", [book({ id: 3 }), book({ id: 4 })], true);
    expect(sortDuplicateGroups([sus, exact]).map((g) => g.key)).toEqual(["a", "b"]);
    expect(confidenceText(0.92)).toBe("92%");
    expect(confidenceText(1)).toBe("100%");
  });

  it("组排序、忽略过滤与分页", () => {
    const gs = [group("a", [book(), book()]), group("b", [book(), book(), book()])];
    expect(sortDuplicateGroups(gs)[0].key).toBe("b");
    expect(visibleDuplicateGroups(gs, ["b"], 0, 10).map((g) => g.key)).toEqual(["a"]);
    expect(visibleDuplicateGroups(gs, [], 0, 1).map((g) => g.key)).toEqual(["b"]);
    expect(visibleDuplicateGroups(gs, [], 1, 1).map((g) => g.key)).toEqual(["a"]);
  });

  it("忽略组不进入待删除；恢复忽略只恢复组不勾选", () => {
    const bs = [book({ id: 1 }), book({ id: 2 }), book({ id: 3 })];
    const gs = [group("a", bs)];
    const sel = { 1: true, 2: false, 3: true };
    expect(selectedForDeletion(gs, [], sel).map((b) => b.id)).toEqual([1, 3]);
    expect(selectedForDeletion(gs, ["a"], sel)).toEqual([]);
    const cleared = clearGroupSelection(sel, bs);
    expect(cleared).toEqual({});
    const restored = restoreIgnoredGroups(gs, ["a"], cleared);
    expect(restored.ignored).toEqual([]);
    expect(restored.selection).toEqual({});
    expect(selectedForDeletion(gs, restored.ignored, restored.selection)).toEqual([]);
  });

  it("目录去重/大小写/筛选与目录内代表书", () => {
    const books = [
      book({ id: 1, path: "C:/Books/A/x.epub", readPct: 10 }),
      book({ id: 2, path: "C:/Books/A/y.epub", readPct: 90 }),
      book({ id: 3, path: "C:/Books/B/z.epub", readPct: 50 }),
      book({ id: 4, path: "C:/BOOKS/a/w.epub" }),
    ];
    expect(parentDirOf("C:/Books/A/x.epub")).toBe("c:\\books\\a");
    expect(collectGroupDirs([group("g", books)])).toEqual(["c:\\books\\a", "c:\\books\\b"]);
    const onlyA = filterGroupsByDir([group("g", books)], "C:/BOOKS/A");
    expect(onlyA).toHaveLength(1);
    expect(onlyA[0].books.map((b) => b.id).sort()).toEqual([1, 2, 4]);
    expect(collectGroupDirs(onlyA)).toEqual(["c:\\books\\a"]);
    expect(onlyA[0].books.length).toBe(3);
    expect(representativeForDir(books, "c:\\books\\a")?.id).toBe(2);
    expect(representativeForDir(books, "c:\\books\\b")?.id).toBe(3);
    expect(filterGroupsByDir([group("g", books)], "c:\\nope")).toEqual([]);
  });

  it("A/B 动作路径与 fromDisk 参数", () => {
    const bs = [book({ id: 1, path: "C:/x/a.epub" }), book({ id: 2, path: "C:/x/b.epub" })];
    const sel = { 1: true, 2: false };
    const picked = selectedForDeletion([group("g", bs)], [], sel);
    expect(picked.map((b) => b.id)).toEqual([1]);
    expect(actionInvocation(picked, "remove")).toEqual({ paths: ["C:/x/a.epub"], fromDisk: false });
    expect(actionInvocation(picked, "trash")).toEqual({ paths: ["C:/x/a.epub"], fromDisk: true });
  });

  it("组内行分页", () => {
    const rows = Array.from({ length: 120 }, (_, i) => i);
    const first = pageGroupRows(rows, 20);
    expect(first.visible).toHaveLength(20);
    expect(first.hasMore).toBe(true);
    expect(first.nextLimit).toBe(70);
    expect(pageGroupRows(rows, 200)).toEqual({ visible: rows, hasMore: false, nextLimit: 120 });
  });

  it("删除后剪枝，剩余不足两本消失", () => {
    const gs = [group("a", [book({ id: 1 }), book({ id: 2 })]), group("b", [book({ id: 3 }), book({ id: 4 }), book({ id: 5 })])];
    const after = pruneDeleted(gs, [1]);
    expect(after.map((g) => g.key)).toEqual(["b"]);
    expect(after[0].books.map((b) => b.id)).toEqual([3, 4, 5]);
    const after2 = pruneDeleted(gs, [3, 4]);
    expect(after2.map((g) => g.key)).toEqual(["a"]);
    expect(after2[0].books.map((b) => b.id)).toEqual([1, 2]);
  });
});
