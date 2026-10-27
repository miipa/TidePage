import { onReducedMotionChange, prefersReducedMotion } from "./reducedMotion";

export interface EdgeEvent {
  edge: -1 | 0 | 1;
  carry: number;
}

/** F7：越界累计的纯状态机（便于单测，也避免 wheelBy 里两套口径各算一次）。
 * 约定：待跨章量 pending 记录原始越界量；视觉回弹 over 只用于装饰且封顶；
 * 跨章触发时 carry 带上本次全部原始越界量并把 over 清零，保证同一位移只计一次。 */
export interface OverscrollState {
  over: number;
  pending: number;
  threshold: number;
  strength: number;
  bounce: boolean;
}

export interface OverscrollResult {
  over: number;
  pending: number;
  edge: -1 | 0 | 1;
  carry: number;
}

export function accumulateOverscroll(s: OverscrollState, delta: number): OverscrollResult {
  if (delta === 0) {
    return { over: s.over, pending: 0, edge: 0, carry: 0 };
  }
  const sign = delta > 0 ? 1 : -1;
  // 反向越界重置累计（先回弹到 0 再往另一头累计，语义与旧实现一致）。
  const pending = s.pending === 0 || Math.sign(s.pending) === sign ? s.pending + delta : delta;
  if (!s.bounce) {
    // 非回弹：越界直接全部转成跨章量，视觉回弹清零。
    return { over: 0, pending: 0, edge: sign as -1 | 1, carry: delta };
  }
  const damp = 1.0 / (1 + s.strength * 1.1);
  const cap = s.threshold * 0.55;
  const over = Math.max(-cap, Math.min(cap, s.over + delta * damp));
  if (Math.abs(pending) >= s.threshold) {
    // 跨章：carry 取全部原始越界量（含本轮与之前累计），over 清零避免同一位移双计。
    return { over: 0, pending: 0, edge: sign as -1 | 1, carry: pending };
  }
  return { over, pending, edge: 0, carry: 0 };
}

export class SmoothScroller {
  private target = 0;
  private current = 0;
  private max = 0;
  private over = 0;
  private pending = 0;
  private raf = 0;
  /** 上一次 tick 的 performance.now()；0 表示循环未开始（用于首帧 dt 初始化）。 */
  private last = 0;
  private apply: (renderPos: number) => void;
  private onStop?: (ratio: number) => void;
  private stopTimer: ReturnType<typeof setTimeout> | undefined;
  /** 订阅「减少动效」变化的取消函数；destroy() 必须调用它，否则监听会泄漏。 */
  private stopReducedMotionWatch?: () => void;

  /** 60Hz 基准：每 16.667ms 逼近目标的比例 / 回弹衰减比例。 */
  private static readonly BASE_MS = 16.667;
  private static readonly APPROACH = 0.22;
  private static readonly DECAY = 0.76;

  bounce = true;
  strength = 0.5;
  threshold = 140;

  constructor(apply: (renderPos: number) => void) {
    this.apply = apply;
    // 用户在应用运行中打开「减少动效」时，立刻中止进行中的动量逼近与橡皮筋回弹
    // （直接落到目标并渲染一帧）。此前只在「下一次」交互时才重新查询 matchMedia，
    // 已经飞出去的滚动不会被纠正 —— 订阅把这个缺口补上。
    // 关闭时不做任何特殊处理：下一次 wheelBy/jumpTo 自然恢复惯性与回弹。
    this.stopReducedMotionWatch = onReducedMotionChange((reduced) => {
      if (!reduced) return;
      if (this.raf === 0 && this.over === 0) return; // 本来就静止，无需干预
      cancelAnimationFrame(this.raf);
      this.raf = 0;
      this.current = this.target;
      this.over = 0;
      this.pending = 0;
      this.render();
      clearTimeout(this.stopTimer);
      this.stopTimer = setTimeout(() => this.onStop?.(this.ratio()), 200);
    });
  }

  setMax(m: number) {
    this.max = m;
    this.target = Math.max(0, Math.min(this.target, m));
    this.current = Math.max(0, Math.min(this.current, m));
  }

  get pos() {
    return this.current;
  }

  get targetPos() {
    return this.target;
  }

  ratio() {
    return this.max > 0 ? Math.max(0, Math.min(1, this.current / this.max)) : 0;
  }

  reset() {
    this.over = 0;
    this.pending = 0;
  }

  /**
   * 丢弃当前越界量（窗口补齐后的续章调用方用它避免回弹一帧）。
   * 目标落点不变，只把累加器与回弹位移清零——既消除视觉回弹，又保留真实滚动位置。
   */
  consumeEdge() {
    this.over = 0;
    this.pending = 0;
  }

  jumpTo(pos: number, immediate = true) {
    this.target = Math.max(0, Math.min(pos, this.max));
    this.over = 0;
    this.pending = 0;
    // 「减少动效」下无论调用方传什么都是直接到位（见 wheelBy 里的同款说明）。
    if (immediate || prefersReducedMotion()) {
      this.current = this.target;
      this.apply(this.current);
    } else {
      this.ensureLoop();
    }
  }

  wheelBy(delta: number): EdgeEvent {
    let to = this.target + delta;
    let over = 0;
    if (to < 0) {
      over = to;
      to = 0;
    } else if (to > this.max) {
      over = to - this.max;
      to = this.max;
    }
    this.target = to;

    const acc = accumulateOverscroll(
      {
        over: this.over,
        pending: this.pending,
        threshold: this.threshold,
        strength: this.strength,
        bounce: this.bounce,
      },
      over,
    );
    this.over = acc.over;
    this.pending = acc.pending;
    const edge = acc.edge;
    const carry = acc.carry;

    // 「减少动效」：目标位置与越界量照常计算（章节补齐等调用方依赖 edge/carry），
    // 但跳过惯性逼近与橡皮筋回弹——直接落到目标并渲染一帧。
    // CSS 的 prefers-reduced-motion 管不到这个 rAF 循环，只能在 JS 里退化。
    if (prefersReducedMotion()) {
      this.current = this.target;
      this.over = 0;
      this.render();
      return { edge, carry };
    }

    this.ensureLoop();
    return { edge, carry };
  }

  private ensureLoop() {
    if (!this.raf) {
      // 新一轮循环的首帧按基准 dt 初始化，避免使用上一次循环遗留的时间戳导致跳帧。
      this.last = 0;
      this.raf = requestAnimationFrame(this.tick);
    }
  }

  private render() {
    this.apply(this.current + this.over);
  }

  private tick = () => {
    this.raf = 0;
    // 帧率无关：按两次 rAF 的真实间隔把 60Hz 基准比例换算成本帧比例。
    // 180Hz(dt≈5.56ms) 与 60Hz(dt≈16.67ms) 在相同墙钟时间下推进量一致；
    // dt 上限 100ms 防止后台标签页恢复/卡顿后的跳变。
    const now = performance.now();
    let dt = this.last ? now - this.last : SmoothScroller.BASE_MS;
    if (dt <= 0) dt = SmoothScroller.BASE_MS;
    if (dt > 100) dt = 100;
    this.last = now;
    const frames = dt / SmoothScroller.BASE_MS;
    const kApproach = 1 - Math.pow(1 - SmoothScroller.APPROACH, frames);
    const kDecay = Math.pow(SmoothScroller.DECAY, frames);

    let cont = false;
    const d = this.target - this.current;
    if (Math.abs(d) > 0.6) {
      this.current += d * kApproach;
      cont = true;
    } else {
      this.current = this.target;
    }
    if (Math.abs(this.over) > 0.6) {
      this.over *= kDecay;
      cont = true;
    } else {
      this.over = 0;
    }
    this.render();
    if (cont) {
      this.raf = requestAnimationFrame(this.tick);
    } else {
      clearTimeout(this.stopTimer);
      this.stopTimer = setTimeout(() => this.onStop?.(this.ratio()), 200);
    }
  };

  onSettled(cb: (ratio: number) => void) {
    this.onStop = cb;
  }

  destroy() {
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    clearTimeout(this.stopTimer);
    this.stopReducedMotionWatch?.();
    this.stopReducedMotionWatch = undefined;
  }
}
