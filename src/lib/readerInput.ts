/**
 * 阅读界面输入判定的纯函数：滚轮归一化与翻页阈值、滚轮本地归属、按键布局路由、换章去重。
 *
 * 为什么单独抽出来：这几条判定决定「外接映射设备（把按键模拟成滚轮/键盘/鼠标）发来的事件
 * 到底落到正文还是面板」，是本轮排查的核心；只有无副作用的纯函数才能用 vitest 覆盖真实设备
 * 会发的各种 deltaMode / 面板滚动组合。真正读 DOM 的部分（closest / scrollHeight /
 * getComputedStyle）以最小结构接口表达，测试用假元素即可。
 */

/* ================= 滚轮：deltaMode 归一化 + 翻页阈值 ================= */

/** 三种 deltaMode 统一折算到「格」（≈像素）：LINE×16 沿用既有约定；PAGE×100 保证
 *  deltaMode=2 且 deltaY=1 的映射设备单次就达阈值（旧实现不换算 → 永远攒不到 100）。 */
export const WHEEL_LINE_PX = 16;
export const WHEEL_PAGE_PX = 100;
/** 翻页阈值：单次滚动即有感知的量级（旧值 100 对小步进设备过大）。 */
export const WHEEL_TURN_THRESHOLD = 40;
/** 连续翻页锁窗口：只抑制「连着翻页」，不丢弃锁内已累积的量。 */
export const WHEEL_LOCK_MS = 240;
/** 累积量静默衰减：超过该间隔没有新滚轮事件就归零，避免残留跨交互累积导致「碰一下自己翻页」。 */
export const WHEEL_ACC_IDLE_MS = 300;

export function normalizeWheelDelta(deltaMode: number, deltaY: number): number {
  if (deltaMode === 1) return deltaY * WHEEL_LINE_PX;
  if (deltaMode === 2) return deltaY * WHEEL_PAGE_PX;
  return deltaY;
}

export interface WheelTurnState {
  /** 已累积、尚未消费的滚动量（格）。 */
  acc: number;
  /** 上一次有效（deltaY≠0）滚轮事件的时间戳。 */
  lastAt: number;
  /** 连续翻页锁到期时间戳。 */
  lockUntil: number;
}

/** 初始状态：lastAt=0 会让首次事件走「已静默」分支，acc 从 0 起算。 */
export const WHEEL_TURN_IDLE: WheelTurnState = { acc: 0, lastAt: 0, lockUntil: 0 };

export interface WheelTurnResult {
  state: WheelTurnState;
  /** 本次是否应翻页：1=下一页，-1=上一页，0=不翻。 */
  turn: -1 | 0 | 1;
}

/**
 * 滚轮翻页判定（纯函数，Reader.onWheel 翻页分支逐事件调用）。
 *
 * 相对旧实现的三处行为差异，都是为了不再吞掉小步进设备的事件：
 *  ① 三种 deltaMode 先归一化（旧代码只处理 LINE，PAGE 直接按 1 算）；
 *  ② 锁窗口内**不清零**累积量，只把它夹到「已够一页」——事件被保留，锁一过期立刻生效；
 *  ③ 静默超过 WHEEL_ACC_IDLE_MS 才归零，短间隔内的累积全部有效。
 */
export function reduceWheelTurn(
  s: WheelTurnState,
  deltaMode: number,
  deltaY: number,
  now: number,
): WheelTurnResult {
  // 纯横向滚动（deltaY=0）不参与翻页：既不推进 lastAt 也不动 acc，
  // 否则横向事件会把纵向累积量的衰减计时一直续期。
  if (deltaY === 0) return { state: s, turn: 0 };
  const acc = (now - s.lastAt > WHEEL_ACC_IDLE_MS ? 0 : s.acc) + normalizeWheelDelta(deltaMode, deltaY);
  let lockUntil = s.lockUntil;
  let turn: -1 | 0 | 1 = 0;
  if (acc >= WHEEL_TURN_THRESHOLD) turn = 1;
  else if (acc <= -WHEEL_TURN_THRESHOLD) turn = -1;
  if (turn !== 0 && now < lockUntil) {
    // 锁内不翻页，但把累积量夹到正好一页：既不无界增长，也保留「锁过期即翻」的意图。
    return { state: { acc: turn * WHEEL_TURN_THRESHOLD, lastAt: now, lockUntil }, turn: 0 };
  }
  if (turn !== 0) lockUntil = now + WHEEL_LOCK_MS;
  // 消费一份，余量（连滚多格）留给锁过期后的第一个事件立即生效；turn=0 时原样保留 acc。
  return { state: { acc: acc - turn * WHEEL_TURN_THRESHOLD, lastAt: now, lockUntil }, turn };
}

/* ================= 滚轮：归属（本地 vs 正文） ================= */

export type WheelAxis = "x" | "y";

/** 面板/浮层作用域：命中其中一层才需要进一步判断「它自己能不能滚」。 */
export const WHEEL_PANEL_SEL = ".sidepanel, .ai-panel, .tb-panel, .panel-slot, .set-pane";
/** 表单控件：滚轮永远归它自己（数值字段步进 / 下拉展开），不参与正文翻页。 */
export const WHEEL_FIELD_SEL = "input, textarea, select";

/** 判定所需的最小元素结构：真实 DOM Element 天然满足，测试用假对象即可。 */
export interface WheelLocalNode {
  scrollHeight: number;
  clientHeight: number;
  scrollWidth: number;
  clientWidth: number;
  parentElement: WheelLocalNode | null;
  closest(selectors: string): WheelLocalNode | null;
  style?: { overflowX?: string; overflowY?: string };
}

function isWheelLocalNode(x: unknown): x is WheelLocalNode {
  if (!x || typeof x !== "object") return false;
  const el = x as Partial<WheelLocalNode>;
  return (
    typeof el.closest === "function" &&
    typeof el.scrollHeight === "number" &&
    typeof el.clientHeight === "number"
  );
}

/** 读轴向 overflow：浏览器里必须用计算值（滚动多半由 class 而非内联样式提供）；
 *  无 DOM 环境（vitest/node）没有 getComputedStyle，退回内联 style，便于假元素单测。 */
function overflowOf(el: WheelLocalNode, axis: WheelAxis): string {
  const g = globalThis as unknown as {
    getComputedStyle?: (e: Element) => { overflowX: string; overflowY: string };
  };
  const src =
    typeof g.getComputedStyle === "function"
      ? g.getComputedStyle(el as unknown as Element)
      : el.style;
  const v = axis === "x" ? src?.overflowX : src?.overflowY;
  return (v || "visible").toLowerCase();
}

/** 「真的可滚」：该轴 overflow 允许用户滚动，且内容确实超出可视盒。 */
function axisScrollable(el: WheelLocalNode, axis: WheelAxis): boolean {
  const ov = overflowOf(el, axis);
  if (ov !== "auto" && ov !== "scroll" && ov !== "overlay") return false;
  return axis === "x" ? el.scrollWidth > el.clientWidth + 1 : el.scrollHeight > el.clientHeight + 1;
}

/**
 * 滚轮是否归「目标自己」处理（true=Reader 放行，交给目标容器原生滚动）。
 *
 * 语义：表单控件直接归本地；否则只有当目标落在面板作用域内、且从目标到面板宿主的
 * 某一层**真的可滚**时才归本地。抽屉展开了但内容不可滚时返回 false → 滚轮回落到正文
 * （旧实现按固定选择器一刀切，光标停在不可滚的抽屉上时两边都不动）；面板内可滚列表
 * 仍返回 true，自己滚自己的。
 */
export function isWheelLocal(target: unknown, axis: WheelAxis = "y"): boolean {
  if (!isWheelLocalNode(target)) return false;
  if (target.closest(WHEEL_FIELD_SEL)) return true;
  const host = target.closest(WHEEL_PANEL_SEL);
  if (!host) return false; // 不在面板作用域：滚轮归正文
  let el: WheelLocalNode | null = target;
  while (el) {
    if (axisScrollable(el, axis)) return true;
    if (el === host) break;
    el = el.parentElement;
  }
  return false;
}

/* ================= 键盘：按布局路由 ================= */

/** 「下滚一段/上滚一段」在当前布局下的实际执行分支。 */
export type ScrollKeyRoute = "pan" | "step" | "page";

/**
 * scrollDown / scrollUp 的布局路由。
 * 关键的一条是 "page"：翻页布局下 ↑/↓ 必须等价于 next/prev（只翻页、不跨章）——
 * 旧实现没有这个分支，键事件落到 scrollStep → maxScroll 恒为 0 → handleScrollEdge，
 * 既被 320ms 节流吃掉、又会一按跳整章。
 */
export function scrollKeyRoute(o: {
  mangaHorizontal: boolean;
  scrollLayout: boolean;
}): ScrollKeyRoute {
  if (o.mangaHorizontal) return "pan";
  return o.scrollLayout ? "step" : "page";
}

/* ================= 抬起事件：选区 / 拖拽 / 点击 ================= */

/** 点击判定的最小有效拖拽位移（px）：低于它视为设备抖动/微移，不算拖拽。
 *  数值与判定顺序是两件事——这里只定阈值，顺序见 classifyPointerUp。 */
export const DRAG_MIN_PX = 8;

export type PointerUpIntent = "select" | "drag" | "click";

/**
 * 正文区主键抬起时的意图分类。关键在**顺序**：选区最优先、与位移无关。
 *
 * 旧实现把「位移超过 DRAG_MIN_PX」排在选区判定之前：拖选 1–2 个字常常只有 4–8px，
 * 于是先落到点击分支 —— `removeAllRanges()` 清掉刚拖出的选区再按热区翻页（划词回归）。
 * 现在的顺序：
 *  ① 有有效选区 → 划词菜单（不论拖了多远，双击选词这种 0px 位移同样算）；
 *  ② 没选中但拖过阈值 → 拖拽手势，忽略（不翻页）；
 *  ③ 否则 → 点击（划线 / 图片 / 热区）。
 * 时长判定不参与：映射设备常模拟带延时的长按，按时间丢弃会让点击完全没反应。
 */
export function classifyPointerUp(o: { hasSelection: boolean; moved: number }): PointerUpIntent {
  if (o.hasSelection) return "select";
  return o.moved > DRAG_MIN_PX ? "drag" : "click";
}

/* ================= 漫画横向平移：位移换算 ================= */

/**
 * 漫画横向平移的单次位移量。
 *
 * 有横向量（trackpad / Shift+滚轮）时原样使用 deltaX —— 既有横向平移语义不变；
 * 没有横向量时回落到纵向 deltaY，并用与正文/滚动模式**同一套**换算（normalizeWheelDelta）。
 * 旧实现的回落只换算 line（deltaMode=1），deltaMode=2 的映射设备 deltaY=1 每下只挪 1px，
 * 表现为「漫画上滚轮没反应」。注意这里是**连续平移**，不引入翻页阈值。
 */
export function mangaWheelDelta(deltaX: number, deltaMode: number, deltaY: number): number {
  return deltaX !== 0 ? deltaX : normalizeWheelDelta(deltaMode, deltaY);
}

/* ================= 换章：同章在途去重 ================= */

/**
 * 本次 loadChapter 是否应直接忽略（复用在途请求）。
 *
 * 只对「纯重复」去重：同章、在途、且**不带任何定位参数**（restore / entry 都为空）。
 * 带 entry（落章首/章末）或 restore（恢复进度）的请求是显式定位，不是「连按翻页的重复」，
 * 必须照常发起并 ++loadGeneration —— 否则加载途中点目录里的同一章 / 书签跳该章章末会被吞掉。
 * gen 语义保留给「不同章节之间」的跳转，不能删。
 */
export function shouldDedupeLoad(
  loading: boolean,
  loadingIdx: number,
  target: number,
  o?: { restore?: unknown; entry?: unknown },
): boolean {
  if (o?.restore != null || o?.entry != null) return false;
  return loading && loadingIdx >= 0 && loadingIdx === target;
}
