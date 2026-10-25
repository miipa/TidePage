/** D1：划线/笔记导出（纯函数）。
 * 固定字段顺序：书名、作者、章节标题、划线原文、笔记、样式、颜色、创建时间。
 * 不依赖 i18n（标题由调用方传或固定中文），保证导出内容稳定可测。 */

export interface HighlightExportRow {
  bookTitle: string;
  bookAuthor: string;
  chapterTitle: string;
  chapterIndex: number;
  text: string;
  note: string;
  style: string;
  color: string;
  createdAt: number;
}

export const EXPORT_FIELDS = [
  "书名",
  "作者",
  "章节标题",
  "划线原文",
  "笔记",
  "样式",
  "颜色",
  "创建时间",
] as const;

/** 多书合并排序：书名 → 作者 → 章号 → 创建时间（稳定、可测）。 */
export function sortExportRows(rows: HighlightExportRow[]): HighlightExportRow[] {
  return [...rows].sort((a, b) => {
    const k1 = a.bookTitle.localeCompare(b.bookTitle, "zh");
    if (k1 !== 0) return k1;
    const k2 = a.bookAuthor.localeCompare(b.bookAuthor, "zh");
    if (k2 !== 0) return k2;
    if (a.chapterIndex !== b.chapterIndex) return a.chapterIndex - b.chapterIndex;
    return a.createdAt - b.createdAt;
  });
}

/** 毫秒或秒时间戳 → 可读时间（秒级自动识别）。 */
export function formatExportTime(ts: number): string {
  if (!ts || !Number.isFinite(ts)) return "";
  const ms = ts < 1e12 ? ts * 1000 : ts;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** CSV 字段：始终加引号，引号加倍（逗号/换行/引号均安全）。 */
function csvField(s: string): string {
  return '"' + (s ?? "").replace(/"/g, '""') + '"';
}

/** 导出 CSV；UTF-8 BOM 便于 Excel 识别中文。空数据仍输出表头。 */
export function toCsv(rows: HighlightExportRow[], withBom = true): string {
  const lines = [EXPORT_FIELDS.map((f) => csvField(f)).join(",")];
  for (const r of sortExportRows(rows)) {
    lines.push(
      [
        r.bookTitle,
        r.bookAuthor,
        r.chapterTitle,
        r.text,
        r.note,
        r.style,
        r.color,
        formatExportTime(r.createdAt),
      ]
        .map(csvField)
        .join(","),
    );
  }
  return (withBom ? "\uFEFF" : "") + lines.join("\r\n") + "\r\n";
}

/** 导出 Markdown：按书分节，划线引用、笔记单列；空值不产生噪声。空数据返回空串。 */
export function toMarkdown(rows: HighlightExportRow[]): string {
  if (!rows.length) return "";
  const sorted = sortExportRows(rows);
  const out: string[] = ["# 划线笔记导出", ""];
  let lastBook = "";
  for (const r of sorted) {
    if (r.bookTitle !== lastBook) {
      lastBook = r.bookTitle;
      out.push(`## ${r.bookTitle || "未命名书籍"}`);
      if (r.bookAuthor.trim()) out.push(`作者：${r.bookAuthor}`);
      out.push("");
    }
    const chapter = r.chapterTitle.trim() || `第 ${r.chapterIndex + 1} 节`;
    out.push(`### ${chapter}`);
    for (const line of (r.text || "").split(/\r?\n/)) out.push(`> ${line}`);
    if (r.note.trim()) out.push("", `笔记：${r.note}`);
    const meta = [r.style, r.color, formatExportTime(r.createdAt)].filter((x) => x && x.trim());
    if (meta.length) out.push("", `（${meta.join(" · ")}）`);
    out.push("");
  }
  return out.join("\n").trimEnd() + "\n";
}

/** 预览：最多 maxLines 行，超出以省略行标记。 */
export function previewText(content: string, maxLines = 14): string {
  const lines = content.split(/\r?\n/);
  if (lines.length <= maxLines) return content;
  return lines.slice(0, maxLines).join("\n") + "\n…";
}
