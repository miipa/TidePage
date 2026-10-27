import { describe, it, expect, beforeEach } from "vitest";
import { nextUniqueLabelId, __resetUniqueLabelSeqForTests } from "./uniqueLabelId";

describe("nextUniqueLabelId", () => {
  beforeEach(() => __resetUniqueLabelSeqForTests());

  it("无冲突时返回连续唯一 id", () => {
    const a = nextUniqueLabelId("set-f", () => false);
    const b = nextUniqueLabelId("set-f", () => false);
    expect(a).toMatch(/^set-f\d+$/);
    expect(b).not.toBe(a);
    expect(Number(b.slice(5))).toBe(Number(a.slice(5)) + 1);
  });

  it("taken 命中时跳过已占用 id", () => {
    const taken = new Set<string>();
    const first = nextUniqueLabelId("set-l", (id) => taken.has(id));
    taken.add(first);
    const second = nextUniqueLabelId("set-l", (id) => taken.has(id));
    expect(second).not.toBe(first);
    expect(taken.has(second)).toBe(false);
  });

  it("前缀不同互不影响，序号继续递增", () => {
    const a = nextUniqueLabelId("set-f", () => false);
    const b = nextUniqueLabelId("set-l", () => false);
    expect(a.startsWith("set-f")).toBe(true);
    expect(b.startsWith("set-l")).toBe(true);
    expect(Number(b.slice(5))).toBeGreaterThan(Number(a.slice(5)));
  });
});
