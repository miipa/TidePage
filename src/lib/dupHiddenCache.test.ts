import { describe, it, expect } from "vitest";
import { hiddenIdsFromGroups, parseHiddenCache, serializeHiddenCache } from "./dupHiddenCache";

describe("dupHiddenCache v2", () => {
  it("groups round-trip、memberIds 去重、hiddenIds 派生", () => {
    const raw = serializeHiddenCache([{ repId: 1, copies: 3, memberIds: [1, 2, 2, 3] }], 123);
    const c = parseHiddenCache(raw);
    expect(c?.savedAt).toBe(123);
    expect(c?.groups).toEqual([{ repId: 1, copies: 3, memberIds: [1, 2, 3] }]);
    expect(hiddenIdsFromGroups(c!.groups)).toEqual([2, 3]);
  });
  it("空/非法 JSON/groups 非数组/v1 旧 shape 返回 null", () => {
    expect(parseHiddenCache(null)).toBeNull();
    expect(parseHiddenCache("")).toBeNull();
    expect(parseHiddenCache("not json")).toBeNull();
    expect(parseHiddenCache(JSON.stringify({ groups: "x" }))).toBeNull();
    expect(parseHiddenCache(JSON.stringify({ ids: [1, 2], savedAt: 1 }))).toBeNull();
  });
  it("空 groups 合法；非法成员/rep 被丢弃；空 memberIds 组丢弃", () => {
    expect(parseHiddenCache(JSON.stringify({ groups: [], savedAt: 0 }))).toEqual({ groups: [], savedAt: 0 });
    const c = parseHiddenCache(JSON.stringify({
      groups: [
        { repId: 1, copies: 2, memberIds: [1, 2] },
        { repId: 0, copies: 2, memberIds: [0, 2] },
        { repId: 3, copies: 2, memberIds: [3] },
      ],
      savedAt: "x",
    }));
    expect(c).toEqual({ groups: [{ repId: 1, copies: 2, memberIds: [1, 2] }], savedAt: 0 });
  });
});
