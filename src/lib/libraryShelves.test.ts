import { describe, it, expect } from "vitest";
import {
  dedupeShelfIds,
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

  it("重复 id 保留第一条并重新分配其余（F8-H1）", () => {
    const list = [
      shelf({ id: "dup", name: "A" }),
      shelf({ id: "dup", name: "B" }),
      shelf({ id: "x", name: "C" }),
      shelf({ id: "dup", name: "D" }),
    ];
    const out = dedupeShelfIds(list, (n) => `new-${n}`);
    expect(out.map((s) => s.id)).toEqual(["dup", "new-1", "x", "new-2"]);
    // 顺序与内容不变
    expect(out.map((s) => s.name)).toEqual(["A", "B", "C", "D"]);
    // 入参未被修改
    expect(list[1].id).toBe("dup");
  });

  it("反序列化返回前按 id 去重（注入工厂便于确定性断言）", () => {
    const raw = JSON.stringify([
      { id: "same", name: "A" },
      { id: "same", name: "B" },
      { id: "same", name: "C" },
    ]);
    const out = deserializeShelves(raw);
    expect(out).toHaveLength(3);
    const ids = out.map((s) => s.id);
    expect(new Set(ids).size).toBe(3);
    expect(ids[0]).toBe("same");
    expect(ids.slice(1).every((id) => id.startsWith("shelf-"))).toBe(true);
  });

  it("空 id 也会重新分配（F8-H1 边界）", () => {
    const out = dedupeShelfIds([shelf({ id: "", name: "A" })], (n) => `gen-${n}`);
    expect(out[0].id).toBe("gen-1");
    expect(out[0].name).toBe("A");
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

describe("uniqueShelfName 32 字上限（G17）", () => {
  it("长基名追加后缀后总长仍 ≤32，normalize 后后缀保留且唯一", () => {
    const base = "这是一本非常非常长的书架名称用来测试截断行为总共三十二个字符";
    const s1 = shelf({ id: "a", name: uniqueShelfName(base, []) });
    const s2 = shelf({ id: "b", name: uniqueShelfName(base, [s1]) });
    const s3 = shelf({ id: "c", name: uniqueShelfName(base, [s1, s2]) });
    for (const s of [s1, s2, s3]) {
      expect(s.name.length).toBeLessThanOrEqual(32);
      expect(normalizeShelfName(s.name)).toBe(s.name);
    }
    const names = new Set([s1.name, s2.name, s3.name].map((n) => n.toLowerCase()));
    expect(names.size).toBe(3);
    expect(s2.name.endsWith("(2)")).toBe(true);
    expect(s3.name.endsWith("(3)")).toBe(true);
  });
  it("已有 (2) 时从 (3) 继续", () => {
    const base = "科幻";
    const used = [shelf({ id: "a", name: "科幻" }), shelf({ id: "b", name: "科幻 (2)" })];
    expect(uniqueShelfName(base, used)).toBe("科幻 (3)");
  });
});
