/** R9：翻页模式超高元素判定（纯函数，Reader 只负责 DOM 收集/加类）。
 *
 * 行为：
 * - keep：元素高度不超过一页 → 保持现有 CSS 的 avoid 规则（不跨页）；
 * - fragment：非媒体元素比一页还高 → 打 .tp-page-break，允许在内部断页（内容完整、不截断）；
 * - fit：媒体元素比一页还高（适宽/缩放导致）→ 打 .tp-page-fit，缩到一页内。
 * pageHeight<=0 / 非法高度一律 keep（防御，避免首帧误判）。
 */

export type TallBlockAction = "keep" | "fragment" | "fit";

/** 与一页高度的容差：避免亚像素/边框导致整页元素被误判为超高。 */
export const PAGE_HEIGHT_EPSILON_PX = 2;

export function tallBlockAction(
  height: number,
  pageHeight: number,
  isMedia: boolean,
): TallBlockAction {
  if (!Number.isFinite(height) || !Number.isFinite(pageHeight) || pageHeight <= 0) return "keep";
  if (height <= pageHeight - PAGE_HEIGHT_EPSILON_PX) return "keep";
  return isMedia ? "fit" : "fragment";
}

/* ---------- R13-A2：书内 CSS 压过分页规则时的兜底（仅翻页模式） ---------- */

/** 从计算样式/内联样式探测到的关键值（兼容旧 page-break-* 名称）。 */
export interface PagedStyleProbe {
  breakInside: string;
  breakAfter: string;
  display: string;
  float: string;
}

/** 需要强制写入的内联样式项。 */
export interface PagedStyleFix {
  breakInside: boolean;
  breakAfter: boolean;
  displayBlock: boolean;
  floatNone: boolean;
}

/**
 * 检测「本应整块保留」的元素是否被书内样式顶掉了分页规则。
 * - break-inside 不是 avoid（含 auto/inherit/空）→ 需要强制 avoid；
 * - 标题 break-after 不是 avoid → 需要强制 avoid；
 * - display 为 inline/inline-* → 不参与块级 fragmentation，需要 block；
 * - float 非 none（且非空）→ 浮动元素不参与分页，需要 none。
 * 无冲突返回 null。
 */
export function detectPagedStyleConflict(
  probe: PagedStyleProbe,
  isHeading: boolean,
): PagedStyleFix | null {
  const norm = (s: string) => (s ?? "").trim().toLowerCase();
  const breakInside = norm(probe.breakInside);
  const breakAfter = norm(probe.breakAfter);
  const display = norm(probe.display);
  const float = norm(probe.float);
  const fix: PagedStyleFix = {
    breakInside: breakInside !== "avoid",
    breakAfter: isHeading && breakAfter !== "avoid",
    displayBlock: display.startsWith("inline"),
    floatNone: float !== "" && float !== "none",
  };
  if (!fix.breakInside && !fix.breakAfter && !fix.displayBlock && !fix.floatNone) return null;
  return fix;
}
