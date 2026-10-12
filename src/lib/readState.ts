import type { BookRow } from "./types";

/** 有效阅读状态三值。 */
export type ReadState = "unread" | "reading" | "read";

/**
 * 「已读满格」阈值（progress.pct 量纲 0–100）。前端唯一来源。
 * 与后端 db::READ_FULL_PCT (src-tauri/src/db.rs ~L460) 逐字同源——改此值必须同步后端常量；
 * 跨语言镜像，测试无法自动校验两侧一致。
 */
export const READ_FULL_PCT = 99.5;

/**
 * 由书行推导有效阅读状态。与后端 effective_state_case_sql() (src-tauri/src/db.rs ~L466) 的 CASE 逐字同源：
 *   read_state=1 → read；read_state=2 → unread；last_opened=0 → unread；read_pct>=READ_FULL_PCT → read；else reading。
 * 任一侧改动必须同步另一侧（§0.3 红线 3）。
 */
export function effectiveState(b: BookRow): ReadState {
  if (b.read_state === 1) return "read";
  if (b.read_state === 2) return "unread";
  if (!b.last_opened) return "unread";
  if ((b.read_pct || 0) >= READ_FULL_PCT) return "read";
  return "reading";
}
