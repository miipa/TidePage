import { describe, it, expect } from "vitest";
import { reorderByDrop, type DropPlace } from "$lib/reorder";

const idStr = (s: string) => s;

describe("reorderByDrop", () => {
  it("move forward: target index is computed AFTER removal", () => {
    // 删掉 "a" 后数组为 ["b","c","d"]，在其中 "c" 的 index=1，after → index 2 插入。
    expect(reorderByDrop(["a", "b", "c", "d"], "a", "c", "after", idStr)).toEqual([
      "b",
      "c",
      "a",
      "d",
    ]);
  });

  it("move backward: before target", () => {
    expect(reorderByDrop(["a", "b", "c", "d"], "d", "b", "before", idStr)).toEqual([
      "a",
      "d",
      "b",
      "c",
    ]);
  });

  it("before vs after the same target differ by one slot", () => {
    expect(reorderByDrop(["a", "b", "c"], "a", "c", "before", idStr)).toEqual([
      "b",
      "a",
      "c",
    ]);
    expect(reorderByDrop(["a", "b", "c"], "a", "c", "after", idStr)).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("no-op → null: empty fromId", () => {
    expect(reorderByDrop(["a", "b"], "", "a", "before", idStr)).toBe(null);
  });

  it("no-op → null: fromId === targetId", () => {
    expect(reorderByDrop(["a", "b"], "a", "a", "after", idStr)).toBe(null);
  });

  it("no-op → null: fromId not found", () => {
    expect(reorderByDrop(["a", "b"], "zzz", "a", "before", idStr)).toBe(null);
  });

  it("no-op → null: targetId not found (after fromId removed)", () => {
    expect(reorderByDrop(["a", "b"], "a", "zzz", "after", idStr)).toBe(null);
  });

  it("object array with custom idOf", () => {
    const list = [{ id: "x" }, { id: "y" }];
    const place: DropPlace = "before";
    expect(reorderByDrop(list, "y", "x", place, (o) => o.id)).toEqual([
      { id: "y" },
      { id: "x" },
    ]);
  });

  it("does not mutate the input list", () => {
    const list = ["a", "b", "c"];
    const copy = [...list];
    reorderByDrop(list, "a", "c", "after", idStr);
    expect(list).toEqual(copy);
  });
});
