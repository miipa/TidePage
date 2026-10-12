import { describe, it, expect } from "vitest";
import { BOOK_EXTENSIONS, stripBookExt } from "./bookExt";

describe("BOOK_EXTENSIONS", () => {
  it("精确等于当前集合与顺序", () => {
    expect(BOOK_EXTENSIONS).toEqual(["txt", "epub", "mobi", "azw3", "prc"]);
  });
});

describe("stripBookExt — 固定当前行为", () => {
  it("剥离结尾书籍扩展名", () => {
    expect(stripBookExt("a.epub")).toBe("a");
    expect(stripBookExt("archive.prc")).toBe("archive");
  });
  it("大小写不敏感", () => {
    expect(stripBookExt("My Book.MOBI")).toBe("My Book");
  });
  it("非 ASCII 文件名", () => {
    expect(stripBookExt("报告.txt")).toBe("报告");
  });
  it("仅剥离最后一段扩展名", () => {
    expect(stripBookExt("a.b.azw3")).toBe("a.b");
  });
  it("非书籍扩展名/无扩展名原样返回", () => {
    expect(stripBookExt("note.pdf")).toBe("note.pdf");
    expect(stripBookExt("noext")).toBe("noext");
  });
});
