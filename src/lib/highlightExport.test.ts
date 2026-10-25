import { describe, it, expect } from "vitest";
import {
  EXPORT_FIELDS,
  formatExportTime,
  previewText,
  sortExportRows,
  toCsv,
  toMarkdown,
  type HighlightExportRow,
} from "./highlightExport";

const row = (patch: Partial<HighlightExportRow> = {}): HighlightExportRow => ({
  bookTitle: "书甲",
  bookAuthor: "作者甲",
  chapterTitle: "第一章",
  chapterIndex: 0,
  text: "划线原文",
  note: "笔记内容",
  style: "underline",
  color: "#ff0",
  createdAt: 1_700_000_000,
  ...patch,
});

describe("highlightExport", () => {
  it("字段顺序固定", () => {
    expect(EXPORT_FIELDS).toEqual([
      "书名",
      "作者",
      "章节标题",
      "划线原文",
      "笔记",
      "样式",
      "颜色",
      "创建时间",
    ]);
  });

  it("CSV 转义逗号/引号/换行，字段全部加引号", () => {
    const csv = toCsv([row({ text: 'a,b"c', note: "第一行\n第二行" })], false);
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe(
      '"书名","作者","章节标题","划线原文","笔记","样式","颜色","创建时间"',
    );
    expect(lines[1]).toContain('"a,b""c"');
    expect(lines[1]).toContain('"第一行\n第二行"');
  });

  it("CSV 带 BOM；空数据仍输出表头", () => {
    expect(toCsv([], true).startsWith("\uFEFF")).toBe(true);
    expect(toCsv([], false).split("\r\n")[0]).toContain('"书名"');
  });

  it("空章节标题保持空字段，时间格式化", () => {
    const csv = toCsv([row({ chapterTitle: "", createdAt: 1_700_000_000 })], false);
    expect(csv.split("\r\n")[1]).toContain('"","划线原文"');
    expect(formatExportTime(1_700_000_000)).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    expect(formatExportTime(0)).toBe("");
  });

  it("多书排序：书名→作者→章号→时间", () => {
    const rows = [
      row({ bookTitle: "B", bookAuthor: "A2", chapterIndex: 1 }),
      row({ bookTitle: "A", bookAuthor: "A1", chapterIndex: 2 }),
      row({ bookTitle: "A", bookAuthor: "A1", chapterIndex: 0, createdAt: 2 }),
      row({ bookTitle: "A", bookAuthor: "A1", chapterIndex: 0, createdAt: 1 }),
    ];
    const s = sortExportRows(rows);
    expect(s.map((r) => r.bookTitle + "-" + r.chapterIndex + "-" + r.createdAt)).toEqual([
      "A-0-1",
      "A-0-2",
      "A-2-1700000000",
      "B-1-1700000000",
    ]);
  });

  it("Markdown 基本结构与空数据", () => {
    expect(toMarkdown([])).toBe("");
    const md = toMarkdown([row({ text: "引用文字", note: "我的笔记" })]);
    expect(md).toContain("# 划线笔记导出");
    expect(md).toContain("## 书甲");
    expect(md).toContain("### 第一章");
    expect(md).toContain("> 引用文字");
    expect(md).toContain("笔记：我的笔记");
    expect(toMarkdown([row({ chapterTitle: "" })])).toContain("第 1 节");
  });

  it("预览截断", () => {
    expect(previewText("a\nb", 5)).toBe("a\nb");
    expect(previewText("1\n2\n3", 2)).toContain("…");
  });
});
