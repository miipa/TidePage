export interface PopupAnchor {
  /** 锚点（选区/划线）顶部 y */
  top: number;
  /** 锚点底部 y */
  bottom: number;
  /** 锚点水平中心 x */
  cx: number;
}

export interface PopupPlacement {
  left: number;
  top: number;
}

/**
 * 把一个固定定位的浮层放到锚点附近并夹取到视口内：
 * 优先放在下方，下方空间不足且上方更宽裕时翻到上方，最后按浮层实际尺寸夹取边界。
 */
export function placePopup(
  anchor: PopupAnchor,
  width: number,
  height: number,
  margin = 8,
  gap = 8,
): PopupPlacement {
  const vw = typeof window !== "undefined" ? window.innerWidth : 1280;
  const vh = typeof window !== "undefined" ? window.innerHeight : 800;
  const spaceBelow = vh - anchor.bottom - margin;
  const spaceAbove = anchor.top - margin;
  const below = spaceBelow >= height || spaceBelow >= spaceAbove;
  let top = below ? anchor.bottom + gap : anchor.top - height - gap;
  let left = anchor.cx - width / 2;
  left = Math.max(margin, Math.min(left, vw - width - margin));
  top = Math.max(margin, Math.min(top, vh - height - margin));
  return { left, top };
}
