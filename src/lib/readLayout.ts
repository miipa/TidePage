/**
 * 阅读区几何的纯函数：翻页模式的可见盒（.clip）与滚动模式的内缩（.col-scroll）
 * 必须与 measurePages() 的排版列宽同源。
 *
 * 为什么单独抽出来：旧实现里 JS 用 `Math.max(160, W - mh*2)` 排版，CSS 的 .clip
 * 却直接写 `left/right: var(--rdr-mh)`（无下限）——窗口稍窄 + 默认边距 320px 时
 * 排版按 ≥160px 铺开、可见盒只剩几十像素，正文被裁成每行 1~2 字（看着像竖排、右侧永久看不见）。
 * 这里给出几何值的唯一来源：先算出「留得下的最大左右内缩」，JS 与 CSS 都用它。
 */

/** 正文区绝对下限（px）：字号极小时也不让可用宽度塌到没法排版。 */
export const MIN_AVAIL = 160;
/** 单列最小列宽（px）：低于它排版无意义。 */
export const MIN_COL_W = 40;
/** 单列最少容纳的字符数（按字号折算）：中文正文每行低于这个数不可读。 */
export const MIN_COL_CHARS = 8;

const num = (v: number, fallback: number): number => (Number.isFinite(v) ? v : fallback);

/** 一屏（单栏/双栏）排版所需的最小可用宽度：列宽下限 × 列数 + 列间距。 */
export function minColWidth(o: { duo: boolean; colGap: number; fontSize: number }): number {
  const cols = o.duo ? 2 : 1;
  const col = Math.max(MIN_COL_W, num(o.fontSize, 0) * MIN_COL_CHARS);
  const gap = Math.max(0, num(o.colGap, 0));
  return Math.max(MIN_AVAIL, cols * col + (cols - 1) * gap);
}

/**
 * 有效左右内缩（px，非负）：把用户设置的 marginH 夹到「内容区仍留得下 minColWidth」的上限。
 * 只会收窄、不会放大用户设置；内容区为 0 宽时返回 0。
 *
 * 不变式（measurePages 依赖它）：contentW - 2*clampMarginH ≥ minColWidth，
 * 于是 clipW = contentW - 2*mhEff ≥ effAvail = 排版实际占用宽度 —— 可见盒永不窄于正文。
 */
export function clampMarginH(o: {
  contentW: number;
  marginH: number;
  duo: boolean;
  colGap: number;
  fontSize: number;
}): number {
  const W = num(o.contentW, 0);
  const mh = Math.max(0, num(o.marginH, 0));
  if (W <= 0) return 0;
  const room = Math.max(0, (W - minColWidth(o)) / 2);
  return Math.min(mh, room);
}
