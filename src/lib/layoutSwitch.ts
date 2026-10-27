/**
 * B5：阅读布局切换时的位置换算（纯函数）。
 * 切换前按旧布局口径取章内比例，切换后按新布局落位；同一布局返回 null 表示不处理。
 * 章节本身不参与换算（分量相对当前章），因此只需传入两种布局各自的章内比例。
 */
export type ReaderLayout = "page" | "scroll";

export function layoutSwitchRatio(
  from: ReaderLayout,
  to: ReaderLayout,
  pageRatio: number,
  scrollRatio: number,
): number | null {
  if (from === to) return null;
  const v = from === "page" ? pageRatio : scrollRatio;
  if (!Number.isFinite(v)) return 0;
  return Math.max(0, Math.min(1, v));
}
