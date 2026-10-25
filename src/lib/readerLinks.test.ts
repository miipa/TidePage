import { describe, it, expect } from "vitest";
import { parseInternalHref, pickSameDocAnchorTarget } from "./readerLinks";

describe("readerLinks", () => {
  it("同文档锚点", () => {
    expect(parseInternalHref("#top")).toEqual({ fileBase: "", fragment: "top", sameDoc: true });
    expect(parseInternalHref("#filepos0000001234")).toEqual({
      fileBase: "",
      fragment: "filepos0000001234",
      sameDoc: true,
    });
    expect(parseInternalHref("#sec%202")).toEqual({ fileBase: "", fragment: "sec 2", sameDoc: true });
  });

  it("跨文档锚点（相对路径/大小写/query）", () => {
    expect(parseInternalHref("text00001.html#filepos1")).toEqual({
      fileBase: "text00001.html",
      fragment: "filepos1",
      sameDoc: false,
    });
    expect(parseInternalHref("Text00001.XHTML#sec2")).toEqual({
      fileBase: "text00001.xhtml",
      fragment: "sec2",
      sameDoc: false,
    });
    expect(parseInternalHref("OEBPS/ch1.xhtml?x=1#sec2")).toEqual({
      fileBase: "ch1.xhtml",
      fragment: "sec2",
      sameDoc: false,
    });
    expect(parseInternalHref("../dir/Text%20001.html#a%23b")).toEqual({
      fileBase: "text 001.html",
      fragment: "a#b",
      sameDoc: false,
    });
  });

  it("无 fragment / 空 / 畸形编码", () => {
    expect(parseInternalHref("ch1.xhtml")).toEqual({
      fileBase: "ch1.xhtml",
      fragment: "",
      sameDoc: false,
    });
    expect(parseInternalHref("")).toBeNull();
    expect(parseInternalHref("   ")).toBeNull();
    expect(parseInternalHref("#")).toBeNull();
    expect(parseInternalHref("#bad%zz")).toEqual({ fileBase: "", fragment: "bad%zz", sameDoc: true });
  });
});
describe("pickSameDocAnchorTarget", () => {
  it("有效章号 → 用定位章号（含当前章）", () => {
    expect(pickSameDocAnchorTarget(3, 7)).toBe(7);
    expect(pickSameDocAnchorTarget(3, 3)).toBe(3);
    expect(pickSameDocAnchorTarget(3, 0)).toBe(0);
  });

  it("null/undefined/负数/非整数 → 回当前章", () => {
    expect(pickSameDocAnchorTarget(3, null)).toBe(3);
    expect(pickSameDocAnchorTarget(3, undefined)).toBe(3);
    expect(pickSameDocAnchorTarget(3, -1)).toBe(3);
    expect(pickSameDocAnchorTarget(3, 2.5)).toBe(3);
  });
});
