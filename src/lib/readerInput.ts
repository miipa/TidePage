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

/* ================= 键盘：控件焦点早退判定 ================= */

/** 会自己吃掉方向键的表单控件标签（select 原先漏掉了：焦点在下拉上按 → 会同时改选项并冒泡翻页）。 */
export const FORM_CONTROL_TAGS = ["INPUT", "TEXTAREA", "SELECT", "OPTION"] as const;

/** 判定所需的最小结构：真实 HTMLElement 天然满足，测试用假对象即可。 */
export interface FormControlNode {
  tagName: string;
  isContentEditable: boolean;
  /** false = 已从文档摘除 */
  isConnected: boolean;
  /** getClientRects().length：0 表示渲染为不可见（display:none / visibility:hidden / 零尺寸） */
  rectCount: number;
}

/**
 * 焦点元素是否属于「该早退」的表单控件。
 *
 * 返回 null = 不是控件（按键继续归阅读快捷键）；
 * 返回 "visible" / "hidden" = 是控件，按可见性区分早退理由（供 DEV 取证区分两类误吞）。
 *
 * 关键的一条是可见性：`isContentEditable` 在隐藏的 contenteditable 容器上同样为真，
 * 而 `<input type="hidden">` / display:none 的输入框也满足 tagName 判定 —— 原来只看标签名，
 * 这些「看不见的控件」会把方向键无声吃掉，表现就是「按了没反应」。
 */
export function isFormControlTarget(o: FormControlNode): "visible" | "hidden" | null {
  const tag = (o.tagName || "").toUpperCase();
  const isControl =
    (FORM_CONTROL_TAGS as readonly string[]).includes(tag) || !!o.isContentEditable;
  if (!isControl) return null;
  return o.isConnected && o.rectCount > 0 ? "visible" : "hidden";
}

/** DOM 适配层：把事件目标收敛成判定输入（Reader.onKeyDown 与 AiPanel.onReaderKeys 共用）。
 *  非元素目标（window/document）返回 null。
 *
 *  可见性优先用 Element.checkVisibility（同时覆盖 display:none、visibility:hidden、
 *  opacity:0、content-visibility:hidden）；**不能只看 getClientRects**——visibility:hidden
 *  的元素照旧生成盒子，getClientRects().length 仍大于 0，「看不见的控件吃掉按键」就漏掉了。
 *  没有该 API 的环境退回 getClientRects 盒子数。 */
export function formControlOf(el: unknown): "visible" | "hidden" | null {
  const e = el as
    | (Partial<HTMLElement> & { checkVisibility?: (options?: Record<string, boolean>) => boolean })
    | null;
  if (!e || typeof e.tagName !== "string") return null;
  const boxes = typeof e.getClientRects === "function" ? e.getClientRects().length : 1;
  let cssVisible: boolean | null = null;
  if (typeof e.checkVisibility === "function") {
    try {
      cssVisible = e.checkVisibility({
        // 兼容新老两套选项名（Chromium 105+ 用 check* / opacityProperty / visibilityProperty）。
        checkOpacity: true,
        checkVisibilityCSS: true,
        opacityProperty: true,
        visibilityProperty: true,
      });
    } catch {
      cssVisible = null;
    }
  }
  return isFormControlTarget({
    tagName: e.tagName,
    isContentEditable: !!e.isContentEditable,
    isConnected: e.isConnected !== false,
    rectCount: cssVisible === null ? boxes : cssVisible ? Math.max(1, boxes) : 0,
  });
}

/* ================= DEV：输入判定取证 ================= */

/** 一条输入判定结论（DEV 用，release 不产出）。 */
export interface InputVerdict {
  verdict: string;
  key: string;
  code: string;
  target: string;
  t: number;
}

const VERDICT_MAX = 80;
const devVerdicts: InputVerdict[] = [];

/** 判定回填器：Reader 在 DEV 下注册，把结论写回它最近一条原始事件记录，
 *  于是 __frDebug.inputLog() 的一条事件自带「最后落到哪」。 */
let verdictSink: ((verdict: string) => void) | undefined;

export function setVerdictSink(fn?: (verdict: string) => void): void {
  verdictSink = fn;
}

/** 事件里判定需要的字段（KeyboardEvent/WheelEvent/PointerEvent 都满足）。 */
export interface InputVerdictEvent {
  key?: string;
  code?: string;
  target?: EventTarget | null;
}

/**
 * 记录一条输入判定结论：`binding:next` / `swallowed:formControl(visible)` /
 * `swallowed:modal(settingsOpen)` / `swallowed:aiPanel(focus)` / `swallowed:loadingDedupe` /
 * `swallowed:edgeThrottle` / `slider:progressSeek` / `noMatch`。
 *
 * 为什么单独记结论而不是只记原始事件：原始事件只说明「事件进来了」，
 * 分不清「被守卫吞了」与「执行了但视觉没变化」（排版塌缩时翻页成功也看不出来）。
 * release 构建中 import.meta.env.DEV 为 false，整个函数体被摇掉。
 */
export function pushVerdict(verdict: string, e?: InputVerdictEvent): void {
  if (!import.meta.env.DEV) return;
  const el = (e?.target ?? null) as Element | null;
  devVerdicts.push({
    verdict,
    key: e?.key ?? "",
    code: e?.code ?? "",
    target: el && typeof el.className === "string" ? el.className : "",
    t: Date.now(),
  });
  if (devVerdicts.length > VERDICT_MAX) devVerdicts.splice(0, devVerdicts.length - VERDICT_MAX);
  verdictSink?.(verdict);
}

/** 最近 VERDICT_MAX 条判定结论（供 __frDebug.inputVerdicts() 取用）。 */
export function recentVerdicts(): InputVerdict[] {
  return devVerdicts.slice();
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
