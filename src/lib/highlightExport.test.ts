import { describe, it, expect } from "vitest";
import {
  EXPORT_FIELDS,
  csvField,
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

  it("CSV 公式注入防护：以 = + - @ TAB CR 开头的单元格前置单引号（F8）", () => {
    // 常见的 Excel 公式注入载荷：前置 ' 后不再被求值，引号/逗号转义保持原逻辑。
    expect(csvField("=cmd|' /C calc'!A0")).toBe('"\'=cmd|\' /C calc\'!A0"');
    expect(csvField("+1")).toBe('"\'+1"');
    expect(csvField("-2")).toBe('"\'-2"');
    expect(csvField("@x")).toBe('"\'@x"');
    expect(csvField("\t=1")).toBe('"\'\t=1"');
    expect(csvField("\r=x")).toBe('"\'\r=x"');
    // 普通中文、空串、普通英文不受影响。
    expect(csvField("中文")).toBe('"中文"');
    expect(csvField("")).toBe('""');
    expect(csvField("normal text")).toBe('"normal text"');
    // 引号加倍仍生效（前置 ' 与内部引号互不干扰）。
    expect(csvField('=a"b')).toBe('"\'=a""b"');
    // 表头与行尾/BOM 不受影响。
    const csv = toCsv([row({ text: "=1+1" })], true);
    expect(csv.startsWith("\uFEFF")).toBe(true);
    expect(csv.endsWith("\r\n")).toBe(true);
    expect(csv.split("\r\n")[0]).toContain('"书名"');
    expect(csv.split("\r\n")[1]).toContain('"\'=1+1"');
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

describe("toMarkdown 同名不同作者（G20）", () => {
  it("书名相同、作者不同各自成节，作者行按组输出", () => {
    const row = (author: string): HighlightExportRow => ({
      bookTitle: "同名书", bookAuthor: author, chapterIndex: 0, chapterTitle: "章",
      text: "内容", style: "", color: "", note: "", createdAt: 1,
    });
    const md = toMarkdown([row("作者甲"), row("作者乙")]);
    const sections = md.split("## ").filter((s) => s.startsWith("同名书"));
    expect(sections).toHaveLength(2);
    expect(sections[0]).toContain("作者：作者甲");
    expect(sections[1]).toContain("作者：作者乙");
  });
});
