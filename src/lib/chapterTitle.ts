/** R10-1/R10-2：章节标题显示规范化（纯函数，前端唯一口径）。
 *
 * 规则与后端 epub.rs::normalize_split_title 完全一致：
 * 1) 若标题以「文件名.扩展名（html/htm/xhtml，大小写不敏感）」结尾，去掉该文件名与它前面的
 *    分隔符（空白 / - / — / – 的任意组合），再 trim 尾部；
 * 2) 结果为空，或全部是 ASCII/全角数字（如 1、１２、003）→ 视为无意义标题，显示「第 N 节」；
 * 3) 其它标题原样（trim 后）显示。
 *
 * 只作用于显示层：不改数据库、不改旧书签/搜索索引里的旧 title 值。
 */

const EXTENSIONS = new Set(["html", "htm", "xhtml"]);
const SEP_RE = /[\s\-—–]/;

/** 去掉「文件名.扩展名（html/htm/xhtml）」后缀；不匹配时原样返回。 */
export function stripChapterFilenameSuffix(raw: string): string {
  const t = raw.replace(/\s+$/u, "");
  const dot = t.lastIndexOf(".");
  if (dot < 0) return raw;
  const ext = t.slice(dot + 1).toLowerCase();
  if (!EXTENSIONS.has(ext)) return raw;
  const prefix = t.slice(0, dot);
  let cut = 0;
  for (let i = prefix.length - 1; i >= 0; i--) {
    if (SEP_RE.test(prefix[i])) {
      cut = i;
      break;
    }
  }
  const token = prefix.slice(cut);
  if (token.replace(/^[\s\-—–]+|[\s\-—–]+$/gu, "") === "") return raw;
  const head = prefix.slice(0, cut).replace(/[\s\-—–]+$/u, "").trimEnd();
  return head;
}

/** 是否为无意义标题（空 / 纯 ASCII 或全角数字）。 */
export function isNumericTitle(raw: string): boolean {
  const t = raw.trim();
  if (!t) return true;
  return [...t].every((c) => /[0-9]/.test(c) || /[０-９]/.test(c));
}

/** 规范化后的章节标题（可能为空串，表示应显示「第 N 节」）。 */
export function normalizeChapterTitle(raw: string): string {
  const t = stripChapterFilenameSuffix(raw).trim();
  if (isNumericTitle(t)) return "";
  return t;
}

/** 显示用标题：有意义的标题原样返回，否则用 sectionNText(chapterIndex+1)（如「第 N 节」）。 */
export function displayChapterTitle(
  raw: string,
  chapterIndex: number,
  sectionNText: (n: number) => string,
): string {
  const t = normalizeChapterTitle(raw);
  return t || sectionNText(chapterIndex + 1);
}
