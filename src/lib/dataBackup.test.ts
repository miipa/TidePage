import { describe, it, expect } from "vitest";
import {
  BACKUP_FORMAT,
  BACKUP_VERSION,
  formatBackupDate,
  isBackupEmpty,
  matchStatus,
  previewRows,
  validateBackupHeader,
} from "./dataBackup";

const preview = (patch = {}) => ({
  progress: 1,
  bookmarks: 0,
  highlights: 0,
  tags: 0,
  bookTags: 0,
  matchedBooks: 1,
  skipped: 0,
  conflicts: 0,
  hasShelves: false,
  empty: false,
  ...patch,
});

describe("dataBackup", () => {
  it("版本/格式校验", () => {
    expect(validateBackupHeader(BACKUP_FORMAT, BACKUP_VERSION)).toEqual({ ok: true });
    expect(validateBackupHeader("other", 1)).toEqual({ ok: false, error: "format" });
    expect(validateBackupHeader(BACKUP_FORMAT, 99)).toEqual({ ok: false, error: "version" });
    expect(validateBackupHeader(BACKUP_FORMAT, 0)).toEqual({ ok: false, error: "version" });
  });

  it("空备份判定", () => {
    expect(isBackupEmpty(preview({ progress: 0 }))).toBe(true);
    expect(isBackupEmpty(preview({ progress: 0, hasShelves: true }))).toBe(false);
    expect(isBackupEmpty(preview({ empty: true }))).toBe(true);
    expect(isBackupEmpty(preview())).toBe(false);
  });

  it("预览计数顺序", () => {
    const rows = previewRows(preview());
    expect(rows.map((r) => r.key)).toEqual([
      "progress", "bookmarks", "highlights", "tags", "bookTags",
      "matchedBooks", "skipped", "conflicts",
    ]);
    expect(rows[0].value).toBe(1);
  });

  it("日期与匹配状态", () => {
    expect(formatBackupDate(1_700_000_000_000)).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    expect(formatBackupDate(0)).toBe("");
    expect(matchStatus(preview({ skipped: 0 }))).toBe("all");
    expect(matchStatus(preview({ skipped: 2 }))).toBe("partial");
    expect(matchStatus(preview({ matchedBooks: 0, skipped: 2 }))).toBe("none");
  });
});
