import { describe, it, expect } from "vitest";
import { effectiveState, READ_FULL_PCT } from "./readState";
import type { BookRow } from "./types";

const base: BookRow = {
  id: 1, path: "/b", title: "t", author: "a", format: "epub",
  size: 0, mtime: 0, added_at: 0, cover_path: null, meta_done: 0,
  last_read: 0, total_chars: 0, total_text_chars: 0,
  cover_ver: 0, read_pct: 0, read_state: 0, last_opened: 0,
};
const row = (o: Partial<BookRow>): BookRow => ({ ...base, ...o });

describe("READ_FULL_PCT", () => {
  it("阈值固定为 99.5（与后端同源）", () => {
    expect(READ_FULL_PCT).toBe(99.5);
  });
});

describe("effectiveState — 固定 5 分支与优先级", () => {
  it("手动已读 read_state=1 → read（压过 read_pct/last_opened）", () => {
    expect(effectiveState(row({ read_state: 1, read_pct: 0, last_opened: 0 }))).toBe("read");
  });
  it("手动未读 read_state=2 → unread（压过满进度）", () => {
    expect(effectiveState(row({ read_state: 2, read_pct: 100, last_opened: 5 }))).toBe("unread");
  });
  it("自动态且从未打开 last_opened=0 → unread（即便 read_pct=100）", () => {
    expect(effectiveState(row({ read_state: 0, last_opened: 0, read_pct: 100 }))).toBe("unread");
  });
  it("已打开且 read_pct=99.5（边界含）→ read", () => {
    expect(effectiveState(row({ read_state: 0, last_opened: 5, read_pct: 99.5 }))).toBe("read");
  });
  it("已打开且 read_pct=99.4 → reading", () => {
    expect(effectiveState(row({ read_state: 0, last_opened: 5, read_pct: 99.4 }))).toBe("reading");
  });
  it("已打开且 read_pct=0 → reading", () => {
    expect(effectiveState(row({ read_state: 0, last_opened: 5, read_pct: 0 }))).toBe("reading");
  });
});
