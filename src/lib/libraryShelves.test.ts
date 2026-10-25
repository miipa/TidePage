import { describe, it, expect } from "vitest";
import {
  deserializeShelves,
  normalizeShelfName,
  serializeShelves,
  shelfFilter,
  shelfFromFilter,
  stampShelfUpdates,
  uniqueShelfName,
} from "./libraryShelves";

const shelf = (patch = {}) => ({ id: "s1", name: "科幻", folder: null, formats: [], states: [], tags: [], query: "", createdAt: 1, updatedAt: 1, ...patch });

describe("libraryShelves", () => {
  it("序列化往返", () => {
    const list = [shelf({ formats: ["epub"], states: ["read"], tags: ["A"], query: "x" })];
    expect(deserializeShelves(serializeShelves(list))).toEqual(list);
  });

  it("updatedAt 归一化与时间戳补齐", () => {
    const old = deserializeShelves(JSON.stringify([{ id: "s1", name: "科幻", folder: null, formats: [], states: [], tags: [], query: "", createdAt: 7 }]))[0];
    expect(old.updatedAt).toBe(7);
    const list = [shelf({ updatedAt: 9 })];
    expect(deserializeShelves(serializeShelves(list))[0].updatedAt).toBe(9);
    const next = stampShelfUpdates([shelf({ name: "改名" })], [shelf()], 500);
    expect(next[0].updatedAt).toBe(500);
    expect(stampShelfUpdates([shelf({ updatedAt: 50 })], [shelf({ updatedAt: 50 })], 600)[0].updatedAt).toBe(50);
    expect(stampShelfUpdates([shelf({ id: "new" })], [], 700)[0].updatedAt).toBe(700);
  });

  it("坏 JSON/坏条目不抛错", () => {
    expect(deserializeShelves("not json")).toEqual([]);
    const raw = JSON.stringify({ shelves: [{ id: "", name: "x" }, { id: "a", name: " " }, { id: "b", name: "ok", formats: "bad", states: ["bad", "read"] }] });
    const out = deserializeShelves(raw);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ id: "b", name: "ok", formats: [], states: ["read"] });
  });

  it("名称归一与自动后缀", () => {
    expect(normalizeShelfName("  a   b ")).toBe("a b");
    expect(uniqueShelfName("科幻", [])).toBe("科幻");
    expect(uniqueShelfName("科幻", [shelf()])).toBe("科幻 (2)");
    expect(uniqueShelfName("科幻 (2)", [shelf(), shelf({ id: "s2", name: "科幻 (2)" })])).toBe("科幻 (3)");
  });

  it("从当前筛选构造并应用", () => {
    const s = shelfFromFilter("我的", { folder: "D:/B", formats: ["epub"], states: ["read"], tags: ["t"], query: "q" }, []);
    expect(s.name).toBe("我的");
    expect(s.folder).toBe("D:/B");
    expect(shelfFilter(s)).toEqual({ folder: "D:/B", formats: ["epub"], states: ["read"], tags: ["t"], query: "q" });
  });

  it("应用时字段校验", () => {
    expect(shelfFilter(null)).toBeNull();
    expect(shelfFilter(shelf({ formats: ["", "epub"], tags: [" ", "x"], states: ["bad", "unread"] }))).toEqual({
      folder: null,
      formats: ["epub"],
      states: ["unread"],
      tags: ["x"],
      query: "",
    });
  });
});
