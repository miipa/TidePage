/**
 * 「减少动效」判定的唯一来源。
 *
 * 为什么要共用：CSS 的 `@media (prefers-reduced-motion)` 只管得到 CSS 动画/过渡，
 * **管不到 JS 驱动的动效**——惯性滚动、橡皮筋回弹、拖拽视觉反馈都是每帧写
 * style，属��行为。原先 `Reader.svelte` 里有一份私有判定，`smoothScroll.ts`
 * 则完全没有，于是「开了减少动效 → 平滑滚动照旧惯性飞出去」是个无障碍缺口。
 *
 * 现在两份都引用这里，保证同一个开关、同一个语义。
 *
 * 取值刻意每次实时查询而不缓存：用户可以在系统设置里随时切换，
 * 缓存会让切换要重启应用才生效。
 */
export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/** 只订阅、不查询：用于需要在开关变化时立刻改变策略的常驻逻辑（拖拽视觉反馈）。 */
export function onReducedMotionChange(cb: (reduced: boolean) => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
  const handler = (e: MediaQueryListEvent) => cb(e.matches);
  mq.addEventListener("change", handler);
  return () => mq.removeEventListener("change", handler);
}
