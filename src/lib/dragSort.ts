// 图标按钮组 / 状态项组的「真实拖拽排序」动作（鼠标 + 触屏，基于 Pointer Events）。
//
// 设计目标：
// - 指针按下后移动超过阈值（默认 6px）才进入拖拽；未超阈值视为普通点击，按钮功能照常触发。
// - 进入拖拽后，松手不再触发该按钮的 onclick（捕获阶段拦截一次 click）。
// - 原件淡出占位，另建一个跟随指针的「拿起」浮层（只动 transform，不触发布局）。
// - 落点用一根跟随目标位置的指示线（.tp-drag-caret）表达，比硬阴影线更轻、可动画。
// - 落点按指针位置与其它项中点比较，决定插入到目标之前/之后。
// - 只在本组内排序；拖出组（含 40px 余量）则取消，不改顺序。
//
// 用法：把动作挂在按钮组容器上，容器内每个可拖项加 `data-drag-id`。
// <div use:dragSort={{ onDrop }}><button data-drag-id="a">…</button>…</div>

export type DropPlace = "before" | "after";

/** G21：只有正常松手才提交排序；pointercancel（系统手势/窗口失焦）必须取消。 */
export function dragReleaseCommits(eventType: string): boolean {
  return eventType === "pointerup";
}

export interface DragSortOptions {
  /** 松手且进入过拖拽时回调；由父级更新顺序数组并持久化。 */
  onDrop: (from: string, target: string, place: DropPlace) => void;
  /** 判定轴：横向按钮组用 x（默认），纵向列表用 y。 */
  axis?: "x" | "y";
  /** 进入拖拽的位移阈值（px）。 */
  threshold?: number;
  /** 逐项判定是否禁止拖动；返回 true 的项不进入拖拽（仍可正常点击）。 */
  disabled?: (id: string, el: HTMLElement) => boolean;
}

const SRC_CLASS = "tp-drag-src";
const GHOST_CLASS = "tp-drag-ghost";
const CARET_CLASS = "tp-drag-caret";

export function dragSort(node: HTMLElement, options: DragSortOptions) {
  let opts = options;

  let pointerId = -1;
  let startX = 0;
  let startY = 0;
  let grabDX = 0;
  let grabDY = 0;
  let src: HTMLElement | null = null;
  let ghost: HTMLElement | null = null;
  let caret: HTMLElement | null = null;
  let dragging = false;
  let target: HTMLElement | null = null;
  let place: DropPlace = "before";

  // 容器需要作为定位参照；若为 static 则就地设为 relative（不改动已有定位）。
  if (getComputedStyle(node).position === "static") node.style.position = "relative";

  function items(): HTMLElement[] {
    return Array.from(node.querySelectorAll<HTMLElement>("[data-drag-id]"));
  }

  function makeGhost(from: HTMLElement, rect: DOMRect) {
    ghost = from.cloneNode(true) as HTMLElement;
    ghost.classList.remove(SRC_CLASS);
    ghost.removeAttribute("data-drag-id");
    ghost.classList.add(GHOST_CLASS);
    ghost.style.width = `${rect.width}px`;
    ghost.style.height = `${rect.height}px`;
    ghost.style.transform = `translate(${rect.left}px, ${rect.top}px)`;
    document.body.appendChild(ghost);
  }

  function makeCaret() {
    caret = document.createElement("div");
    caret.className = CARET_CLASS;
    caret.dataset.axis = opts.axis ?? "x";
    caret.style.display = "none";
    node.appendChild(caret);
  }

  function clearMarks() {
    for (const el of items()) el.classList.remove(SRC_CLASS);
    ghost?.remove();
    caret?.remove();
    ghost = null;
    caret = null;
  }

  function blockClick() {
    const swallow = (ev: Event) => {
      ev.stopPropagation();
      ev.preventDefault();
    };
    window.addEventListener("click", swallow, true);
    setTimeout(() => window.removeEventListener("click", swallow, true), 400);
  }

  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const el = (e.target as HTMLElement | null)?.closest?.("[data-drag-id]") as HTMLElement | null;
    // 仅允许按钮组容器的直接子项参与拖拽（避免嵌套组互相干扰）。
    if (!el || el.parentElement !== node) return;
    // 逐项禁用：展开中的项不进入拖拽，点击/内部交互照常。
    if (opts.disabled?.(el.dataset.dragId as string, el)) return;
    src = el;
    pointerId = e.pointerId;
    startX = e.clientX;
    startY = e.clientY;
    const rect = el.getBoundingClientRect();
    grabDX = e.clientX - rect.left;
    grabDY = e.clientY - rect.top;
    dragging = false;
    target = null;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  }

  function onMove(e: PointerEvent) {
    if (e.pointerId !== pointerId || !src) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (!dragging) {
      if (Math.hypot(dx, dy) < (opts.threshold ?? 6)) return;
      dragging = true;
      src.classList.add(SRC_CLASS);
      makeGhost(src, src.getBoundingClientRect());
      makeCaret();
    }
    if (ghost) {
      ghost.style.transform = `translate(${e.clientX - grabDX}px, ${e.clientY - grabDY}px)`;
    }
    const axis = opts.axis ?? "x";
    const p = axis === "x" ? e.clientX : e.clientY;
    const box = node.getBoundingClientRect();
    const inside =
      axis === "x"
        ? e.clientX >= box.left - 40 && e.clientX <= box.right + 40
        : e.clientY >= box.top - 40 && e.clientY <= box.bottom + 40;

    if (!inside) {
      target = null;
      if (caret) caret.style.display = "none";
      return;
    }
    let best: HTMLElement | null = null;
    let bestDist = Infinity;
    let bestPlace: DropPlace = "before";
    for (const el of items()) {
      if (el === src) continue;
      const r = el.getBoundingClientRect();
      const center = axis === "x" ? r.left + r.width / 2 : r.top + r.height / 2;
      const dist = Math.abs(p - center);
      if (dist < bestDist) {
        bestDist = dist;
        best = el;
        bestPlace = p < center ? "before" : "after";
      }
    }
    target = best;
    place = bestPlace;
    if (caret) {
      if (!target) {
        caret.style.display = "none";
      } else {
        const r = target.getBoundingClientRect();
        caret.style.display = "";
        caret.style.transform =
          axis === "x"
            ? `translateX(${(place === "before" ? r.left : r.right) - box.left}px)`
            : `translateY(${(place === "before" ? r.top : r.bottom) - box.top}px)`;
      }
    }
  }

  function unbindWindow() {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", onCancel);
  }

  /** G21：取消路径——清标记/解绑，但绝不回调 onDrop（旧实现 pointercancel 直接走 finish 提交）。 */
  function cancelDrag() {
    unbindWindow();
    clearMarks();
    dragging = false;
    pointerId = -1;
    src = null;
    target = null;
  }

  function onCancel(e: PointerEvent) {
    if (e.pointerId === pointerId) cancelDrag();
  }

  function finish() {
    unbindWindow();
    const wasDragging = dragging;
    const from = src;
    const to = target;
    const pl = place;
    clearMarks();
    dragging = false;
    pointerId = -1;
    src = null;
    target = null;
    if (wasDragging && from && to && to !== from) {
      // 一旦进入拖拽，松手不触发按钮 onclick。
      blockClick();
      opts.onDrop(from.dataset.dragId as string, to.dataset.dragId as string, pl);
    }
  }

  function onUp(e: PointerEvent) {
    if (e.pointerId !== pointerId) return;
    if (!dragReleaseCommits(e.type)) {
      cancelDrag();
      return;
    }
    finish();
  }

  node.addEventListener("pointerdown", onDown);
  return {
    update(next: DragSortOptions) {
      opts = next;
    },
    destroy() {
      // 拖拽途中宿主被卸载（如设置面板在拖提示词时被关掉）时，finish() 不会执行，
      // 三个 window 监听会永久残留，onMove 之后每次 pointermove 仍在跑 items()
      // + 逐项 getBoundingClientRect()。故这里兜底解绑 —— 不能调 finish()：
      // 那会顺手触发 onDrop，而宿主已经没了，落点无意义。
      unbindWindow();
      node.removeEventListener("pointerdown", onDown);
      clearMarks();
    },
  };
}
