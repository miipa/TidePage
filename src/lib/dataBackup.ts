/** D4：完整备份预览的纯函数（版本/空备份/计数/日期/摘要）；文件 IO 在后端。 */
import type { DataBackupPreview } from "./api";

export const BACKUP_FORMAT = "tidepage-backup";
export const BACKUP_VERSION = 1;

export type BackupHeaderError = "format" | "version";

/** 校验备份头：format 必须匹配，version 必须等于当前支持版本。 */
export function validateBackupHeader(
  format: string,
  version: number,
): { ok: true } | { ok: false; error: BackupHeaderError } {
  if (format !== BACKUP_FORMAT) return { ok: false, error: "format" };
  if (version !== BACKUP_VERSION) return { ok: false, error: "version" };
  return { ok: true };
}

/** 空备份：无任何行、无标签、无书架（配置可独立存在）。 */
export function isBackupEmpty(p: Pick<DataBackupPreview, "progress" | "bookmarks" | "highlights" | "tags" | "bookTags" | "hasShelves" | "empty">): boolean {
  return (
    !!p.empty ||
    (p.progress + p.bookmarks + p.highlights + p.tags + p.bookTags === 0 && !p.hasShelves)
  );
}

/** 预览计数行（固定顺序，便于 UI/测试）。 */
export function previewRows(
  p: Pick<DataBackupPreview, "progress" | "bookmarks" | "highlights" | "tags" | "bookTags" | "matchedBooks" | "skipped" | "conflicts">,
): { key: "progress" | "bookmarks" | "highlights" | "tags" | "bookTags" | "matchedBooks" | "skipped" | "conflicts"; value: number }[] {
  return [
    { key: "progress", value: p.progress },
    { key: "bookmarks", value: p.bookmarks },
    { key: "highlights", value: p.highlights },
    { key: "tags", value: p.tags },
    { key: "bookTags", value: p.bookTags },
    { key: "matchedBooks", value: p.matchedBooks },
    { key: "skipped", value: p.skipped },
    { key: "conflicts", value: p.conflicts },
  ];
}

/** 备份日期：合法毫秒时间戳 → YYYY-MM-DD HH:mm；非法返回空串。 */
export function formatBackupDate(ts: number): string {
  if (!ts || !Number.isFinite(ts)) return "";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 方向提示：全部匹配 / 部分匹配 / 无匹配（恢复前给用户判断）。 */
export function matchStatus(p: Pick<DataBackupPreview, "matchedBooks" | "skipped">): "all" | "partial" | "none" {
  if (p.skipped === 0) return "all";
  if (p.matchedBooks > 0) return "partial";
  return "none";
}
