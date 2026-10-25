import { describe, it, expect } from "vitest";
import {
  displayChapterTitle,
  isNumericTitle,
  normalizeChapterTitle,
  stripChapterFilenameSuffix,
} from "./chapterTitle";

const sec = (n: number) => `第${n}节`;

describe("chapterTitle", () => {
  it("去掉文件名后缀（Rust 同款样例）", () => {
    expect(stripChapterFilenameSuffix("春秋左传注 修订本 (全四册) - text00001.html")).toBe(
      "春秋左传注 修订本 (全四册)",
    );
    expect(stripChapterFilenameSuffix("书名 text00001.xhtml")).toBe("书名");
    expect(stripChapterFilenameSuffix("text00001.html")).toBe("");
    expect(stripChapterFilenameSuffix("书名 - text00001.htm")).toBe("书名");
    expect(stripChapterFilenameSuffix("书名 text00001.HTML")).toBe("书名");
    // 不匹配：普通标题里的合法分隔/句中 .html 字样。
    expect(stripChapterFilenameSuffix("JavaScript - 权威指南")).toBe("JavaScript - 权威指南");
    expect(stripChapterFilenameSuffix("书名 - text00001.html - 第2章")).toBe(
      "书名 - text00001.html - 第2章",
    );
    expect(stripChapterFilenameSuffix("关于 index.html 的讨论")).toBe("关于 index.html 的讨论");
  });

  it("去后缀后为空/纯数字 → 空标题", () => {
    expect(normalizeChapterTitle("text00001.html")).toBe("");
    expect(normalizeChapterTitle("第2章.html")).toBe("");
    expect(normalizeChapterTitle("1")).toBe("");
    expect(normalizeChapterTitle("１２")).toBe("");
    expect(normalizeChapterTitle("003")).toBe("");
    expect(normalizeChapterTitle("   ")).toBe("");
  });

  it("正常标题保留；多空格/全角数字/大小写扩展名", () => {
    expect(normalizeChapterTitle("  第一章 绪论  ")).toBe("第一章 绪论");
    expect(normalizeChapterTitle("春秋左传注 修订本 (全四册) - text00001.html")).toBe(
      "春秋左传注 修订本 (全四册)",
    );
    expect(normalizeChapterTitle("书名  text00001.xhtml")).toBe("书名");
    expect(normalizeChapterTitle("Chap 1 - intro.xhtml")).toBe("Chap 1");
  });

  it("displayChapterTitle：空/纯数字用第 N 节，N=chapter_index+1", () => {
    expect(displayChapterTitle("", 0, sec)).toBe("第1节");
    expect(displayChapterTitle("1", 4, sec)).toBe("第5节");
    expect(displayChapterTitle("１２", 11, sec)).toBe("第12节");
    expect(displayChapterTitle("text00001.html", 2, sec)).toBe("第3节");
    expect(displayChapterTitle("第一章", 3, sec)).toBe("第一章");
    expect(displayChapterTitle("书名 - c0.xhtml", 7, sec)).toBe("书名");
  });

  it("isNumericTitle 边界", () => {
    expect(isNumericTitle("")).toBe(true);
    expect(isNumericTitle("  ")).toBe(true);
    expect(isNumericTitle("0")).toBe(true);
    expect(isNumericTitle("０")).toBe(true);
    expect(isNumericTitle("第1章")).toBe(false);
    expect(isNumericTitle("1.2")).toBe(false);
  });
});
