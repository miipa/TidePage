import { describe, it, expect } from "vitest";
import {
  findNodeIndexByOffset,
  findPdfMatches,
  groupPdfMatches,
  locateOccurrence,
  nodeRangeForOffsets,
  normalizePdfQuery,
  searchProgress,
  snippetFor,
  stepHit,
} from "./pdfSearch";

describe("pdfSearch", () => {
  it("B11 跨页合并：跨行/跨页一条、单页/多次/空页/边界", () => {
    const pages = [
      { page: 0, text: "hello world" },
      { page: 1, text: "foo bar" },
      { page: 2, text: "HELLO   again hello" },
      { page: 3, text: "" },
    ];
    const cross = groupPdfMatches([{ page: 0, text: "alpha be" }, { page: 1, text: "ta gamma" }], "beta");
    expect(cross).toHaveLength(1);
    expect(cross[0].startPage).toBe(0);
    expect(cross[0].endPage).toBe(1);
    expect(cross[0].segments.map((s) => s.page)).toEqual([0, 1]);
    expect(groupPdfMatches(pages, "hello")).toHaveLength(3);
    expect(groupPdfMatches(pages, "world foo bar")).toHaveLength(1);
    const samePage = groupPdfMatches(pages, "hello");
    expect(samePage[2].segments[0].occurrence).toBe(1);
    expect(groupPdfMatches([{ page: 0, text: "x" }, { page: 1, text: "y" }], "x")[0].startPage).toBe(0);
    // 跨页 segment 必须是页内偏移（非起始页不能是整本全局偏移）。
    const p0 = "alpha CROSS", p1 = "PAGEMARK beta";
    const g = groupPdfMatches([{ page: 0, text: p0 }, { page: 1, text: p1 }], "CROSS PAGEMARK");
    expect(g).toHaveLength(1);
    expect(g[0].segments).toHaveLength(2);
    for (const s of g[0].segments) {
      const t = s.page === 0 ? p0 : p1;
      expect(s.start).toBeGreaterThanOrEqual(0);
      expect(s.end).toBeGreaterThan(s.start);
      expect(s.end).toBeLessThanOrEqual(t.length);
    }
    expect(p0.slice(g[0].segments[0].start, g[0].segments[0].end)).toBe("CROSS");
    expect(p1.slice(g[0].segments[1].start, g[0].segments[1].end)).toBe("PAGEMARK");
    // 跨行 + 页首页尾边界：查询从页尾开始、下一页页首结束，段不越界。
    const b0 = "tail CROS", b1 = "S head";
    const gb = groupPdfMatches([{ page: 0, text: b0 }, { page: 1, text: b1 }], "CROSS");
    expect(b0.slice(gb[0].segments[0].start, gb[0].segments[0].end)).toBe("CROS");
    expect(b1.slice(gb[0].segments[1].start, gb[0].segments[1].end)).toBe("S");
    expect(groupPdfMatches(pages, "zzz")).toEqual([]);
    expect(groupPdfMatches([{ page: 0, text: "" }], "x")).toEqual([]);
  });

  it("B11 文本层二分定位：边界/跨节点/找不到", () => {
    const starts = [0, 5, 8];
    const lens = [5, 3, 4];
    expect(findNodeIndexByOffset(starts, 0)).toBe(0);
    expect(findNodeIndexByOffset(starts, 7)).toBe(1);
    expect(findNodeIndexByOffset(starts, 99)).toBe(2);
    expect(findNodeIndexByOffset([], 1)).toBe(-1);
    expect(nodeRangeForOffsets(starts, lens, 4, 9)).toEqual({ startIndex: 0, startOffset: 4, endIndex: 2, endOffset: 1 });
    expect(nodeRangeForOffsets(starts, lens, 8, 12)).toEqual({ startIndex: 2, startOffset: 0, endIndex: 2, endOffset: 4 });
    expect(nodeRangeForOffsets(starts, lens, 12, 13)).toBeNull();
    expect(nodeRangeForOffsets([0, 5], [2, 2], 0, 10)).toEqual({ startIndex: 0, startOffset: 0, endIndex: 1, endOffset: 2 });
    expect(nodeRangeForOffsets(starts, lens, 5, 5)).toBeNull();
  });

  it("查询归一与大小写/连续空白匹配", () => {
    expect(normalizePdfQuery("  hello   world  ")).toBe("hello world");
    const hits = findPdfMatches("Hello   World! hello world", "hello world", 3);
    expect(hits).toHaveLength(2);
    expect(hits[0]).toMatchObject({ page: 3, start: 0 });
    expect(hits[0].snippet).toContain("Hello");
  });

  it("多次出现带序号，文本层按第 N 次定位（大小写/空白差异兼容）", () => {
    const hits = findPdfMatches("Hello world. HELLO   world. hello world", "hello world", 2);
    expect(hits).toHaveLength(3);
    expect(hits.map((h) => h.occurrence)).toEqual([0, 1, 2]);
    // 文本层可能是「HelloworldHELLOworld」这类无空格拼接，仍按第 N 次定位。
    const layer = "Hello world. HELLO   world. hello world";
    expect(locateOccurrence(layer, "hello world", 1)).toEqual({ start: 13, end: 26 });
    expect(locateOccurrence(layer, "hello world", 3)).toBeNull();
    expect(locateOccurrence("Helloworld HELLOworld", "hello world", 1)).toEqual({ start: 11, end: 21 });
  });

  it("空查询/空文本无命中", () => {
    expect(findPdfMatches("abc", "  ", 0)).toEqual([]);
    expect(findPdfMatches("", "abc", 0)).toEqual([]);
  });

  it("片段与首尾省略", () => {
    const text = "x".repeat(100) + "needle" + "y".repeat(100);
    const snip = snippetFor(text, 100, 106, 5);
    expect(snip).toContain("needle");
    expect(snip.startsWith("…")).toBe(true);
    expect(snip.endsWith("…")).toBe(true);
  });

  it("导航循环/边界", () => {
    expect(stepHit(0, 3, 1)).toBe(1);
    expect(stepHit(2, 3, 1)).toBe(0);
    expect(stepHit(0, 3, -1)).toBe(2);
    expect(stepHit(0, 0, 1)).toBe(0);
    expect(searchProgress(5, 0)).toBe(0);
    expect(searchProgress(5, 10)).toBe(50);
  });
});

describe("groupPdfMatches 页缺口（F16）", () => {
  it("页号不连续（0,2）不跨缺口拼接：查询只命中单页内内容", () => {
    const pages = [
      { page: 0, text: "跨页开头" },
      { page: 2, text: "另一页内容" },
    ];
    // 缺口处拼出「开头另一页」若仍合并会产生一条假命中；按 run 分组后不能命中。
    const groups = groupPdfMatches(pages, "开头另一页");
    expect(groups).toHaveLength(0);
    // 单页内命中正常。
    const hit = groupPdfMatches(pages, "另一页");
    expect(hit).toHaveLength(1);
    expect(hit[0].segments.map((s) => s.page)).toEqual([2]);
  });
  it("连续页（0,1）正常跨页合并成一条", () => {
    const pages = [
      { page: 0, text: "上半句在" },
      { page: 1, text: "这里" },
    ];
    const groups = groupPdfMatches(pages, "在这里");
    expect(groups).toHaveLength(1);
    expect(groups[0].segments.map((s) => s.page)).toEqual([0, 1]);
  });
  it("单页与空输入", () => {
    expect(groupPdfMatches([{ page: 5, text: "命中" }], "命中")).toHaveLength(1);
    expect(groupPdfMatches([], "命中")).toHaveLength(0);
  });
});
