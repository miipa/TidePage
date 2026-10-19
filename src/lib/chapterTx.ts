/**
 * 章节切换事务的纯判定与过渡几何（DOM 与动画留在 Reader.svelte）。
 *
 * 为什么抽出来：解锁判据（A 静止间隔 + B 落地信号）与两类过渡的位移/时长都是「输入 → 结论」，
 * 它们的边界（同向续期、反向放行、短章 maxScroll=0、动画未收尾、硬兜底）只能靠单测覆盖；
 * 留在组件里就只能靠反复手测，且很容易在后续改动里悄悄漂移。
 */

/** 滚动模式的章节过渡档位：层叠覆盖 / 阻尼弹簧 / 关闭。
 *  与 SmoothScroller 的边缘阻尼（bounce/strength/threshold）无关 —— 后者恒为常量，不随档位变。 */
export type ChapterTransition = "cover" | "spring" | "none";

/** 翻页模式（单栏/双栏）的章节过渡档位：阻尼弹簧 / 左右滑动 / 关闭。
 *  刻意只有三档：不做视差侧滑 —— 横屏翻页里它与「左右滑动」的观感区分度不足，
 *  却多出一整套退场缩放/压暗参数与接缝阴影，维护成本高于收益。 */
export type PageTransition = "h-spring" | "h-slide" | "none";

/** 过渡轴向：滚动模式纵向、翻页模式横向。 */
export type TxAxis = "x" | "y";

/** 解锁判据 A 的静止间隔（ms）：距最后一次同向输入超过它才算「停手」。
 *  实测惯性尾巴的相邻间隔 —— Chromium 合成滚轮动量 ≈5.5ms，trackpad 衰减流 ≈8–33ms；
 *  120ms 足以认定停手，又远小于人的下一次操作间隔。 */
export const TX_IDLE_MS = 120;
/** 硬兜底（ms）：只防事件流异常（动画不结束、落地信号丢失），不参与正常路径。 */
export const TX_HARD_MS = 2500;
/** 解锁轮询间隔（ms）。 */
export const TX_POLL_MS = 24;

/** 层叠覆盖：时长 ms / 旧章视差上移比例（0.35×可视高）/ 缓动。 */
export const TX_COVER_MS = 720;
export const TX_COVER_PARALLAX = 0.35;
export const TX_COVER_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";
/** 阻尼弹簧：时长 ms / 整层移出比例（104%，缓动带过冲）/ 缓动。 */
export const TX_SPRING_MS = 780;
export const TX_SPRING_SHIFT = 1.04;
export const TX_SPRING_EASE = "cubic-bezier(0.34, 1.48, 0.64, 1)";

/* ================= 翻页模式（横向）常量：来源集中在下面注释 =================
 * 参考件：D:\Downloads\gemini-code-1792322097526.html
 *   h-spring 第 387/391（780ms）与第 402-417（入场缓动 = 第 147 行 --spring-overshoot）
 * 现有翻页位移：src/app.css:110 `--dur-page: 0.22s`、:116 `--ease-page: cubic-bezier(0.22, 1, 0.36, 1)`
 *   —— h-slide 沿用这一组，观感与普通翻页一致。
 * 刻意不实现参考件的 h-flip（书脊翻转）与 h-cube，也不引入 --flip-origin。 */
export const TX_H_SPRING_MS = 780;
/** 整页入场缓动带过冲（参考件 147 行 --spring-overshoot）。
 *  翻页模式的阻尼弹簧档与跨章共用这一条：**动画的是整页双层**（真实 .clip 整页滑入、
 *  舞台里的克隆整页移出），过冲露出的只是 `.tx-stage` 的 var(--rdr-bg) 垫底，不会看到相邻页。
 *  内容条（.col-page）只负责页码定位，不参与动画 —— 上一版把弹簧做成内容条曲线，
 *  平移必然把相邻页带进视口，与「不露邻页」不可兼得，已废弃。 */
export const TX_H_SPRING_IN_EASE = "cubic-bezier(0.28, 1.35, 0.58, 1)";
/** 退场缓动：只作用于位移（不做透明度/缩放/亮度淡化 —— 旧实现会给旧页留半透明虚影）。 */
export const TX_H_SPRING_OUT_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";
export const TX_H_SLIDE_MS = 220;
export const TX_H_SLIDE_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

/** 章内翻页要挂的类：`""` = 用 `.col-page` 基础规则（左右滑动档，内容条 220ms 平移）；
 *  `tx-fx-none` = 关闭档（零位移、文字直接替换）。
 *  阻尼弹簧档**不挂类**：它走整页双层过渡（与切章同一套骨架），内容条只做无过渡定位。 */
export function pageTurnFxClass(mode: PageTransition): "" | "tx-fx-none" {
  return mode === "none" ? "tx-fx-none" : "";
}

/** 快照克隆的节点数上限：实测 202 节点 0ms、1002 节点 0.3ms、3002 节点 2ms、12002 节点 2.3ms；
 *  取 20000（线性外推 ≈4ms，仍在一帧内）以上退化为关闭档路径。 */
export const TX_SNAPSHOT_MAX_NODES = 20_000;

/** 档位归一化：缺失/非法一律按 cover（旧配置里根本没有这个键，不能让它变成 undefined）。 */
export function transitionModeOf(v: unknown): ChapterTransition {
  return v === "spring" || v === "none" ? v : "cover";
}

/** 翻页档位归一化：显式枚举三项，其余（含已删除的视差侧滑档、缺失值）一律回落 h-slide。
 *  不靠默认分支吞旧值 —— 旧配置里存着那个历史档位时必须可预期地落到 h-slide。 */
export function pageTransitionOf(v: unknown): PageTransition {
  if (v === "h-spring" || v === "h-slide" || v === "none") return v;
  return "h-slide";
}

export interface TxGeometry {
  /** 动画时长（ms）。 */
  durMs: number;
  /** 缓动曲线。 */
  easing: string;
  /** 旧章（快照）位移终点：向下续章为负（上移），向上切章为正（下移）。 */
  outTo: number;
  /** 新章（真实 .scroll-area）入场起点：向下续章从下方（+可视高）进来，向上切章从上方（-可视高）进来。 */
  inFrom: number;
  /** 位移轴向：滚动模式 y、翻页模式 x（调用方据此拼 translate3d 的哪一个分量）。 */
  axis: TxAxis;
}

/** 过渡几何：档位 + 方向 + 可视高 → 位移与时长。dir > 0 表示向下续章。
 *  axis 只影响调用方拼哪一个位移分量（数值仍按同一可视跨度计算）。 */
export function transitionGeometry(
  mode: Exclude<ChapterTransition, "none">,
  dir: number,
  viewportH: number,
  axis: TxAxis = "y",
): TxGeometry {
  const h = Number.isFinite(viewportH) ? Math.max(1, Math.round(viewportH)) : 1;
  const down = dir >= 0;
  const cover = mode === "cover";
  const shift = Math.round(h * (cover ? TX_COVER_PARALLAX : TX_SPRING_SHIFT));
  return {
    durMs: cover ? TX_COVER_MS : TX_SPRING_MS,
    easing: cover ? TX_COVER_EASE : TX_SPRING_EASE,
    outTo: down ? -shift : shift,
    inFrom: down ? h : -h,
    axis,
  };
}

export interface PageTxGeometry {
  /** 动画时长（ms）。 */
  durMs: number;
  /** 新页入场缓动。 */
  easingIn: string;
  /** 旧页退场缓动：只作用于位移（退场层保持不透明、无滤镜，不得出现旧页虚影）。 */
  easingOut: string;
  /** 新页（真实 .clip）入场起点：向后翻页从右侧 +span 进，向前翻页从左侧 -span 进。 */
  inFrom: number;
  /** 旧页（舞台里的克隆 .clip）退场终点。 */
  outTo: number;
}

/**
 * 翻页模式过渡几何：档位 + 方向 + 横向跨度 → 位移与时长。
 *
 * 跨度必须传**有效可视宽 effAvail**（不是页码步进 stepPx）：双栏时 stepPx − effAvail = gap，
 * 用 stepPx 做入场起点会在画面边缘留一条 gap 宽的缝（单栏 gap 不进步进，两者相等）。
 */
export function pageTransitionGeometry(
  mode: Exclude<PageTransition, "none">,
  dir: number,
  span: number,
): PageTxGeometry {
  const s = Number.isFinite(span) ? Math.max(1, Math.round(span)) : 1;
  const down = dir >= 0;
  if (mode === "h-spring") {
    return {
      durMs: TX_H_SPRING_MS,
      easingIn: TX_H_SPRING_IN_EASE,
      easingOut: TX_H_SPRING_OUT_EASE,
      inFrom: down ? s : -s,
      outTo: down ? -s : s,
    };
  }
  return {
    durMs: TX_H_SLIDE_MS,
    easingIn: TX_H_SLIDE_EASE,
    easingOut: TX_H_SLIDE_EASE,
    inFrom: down ? s : -s,
    outTo: down ? -s : s,
  };
}

export type TxUnlockStep = "wait" | "frame" | "unlock";

/**
 * 解锁判据 = A 且 B，另加动画收尾与「已过一帧」。
 *
 * A：距最后一次同向输入 ≥ TX_IDLE_MS；
 * B：chapterNum === 目标章号 **且** loadingChapter === false；
 * 已过一帧：await tick() 后再等一个 rAF（B 的落地帧先渲染出来，避免解锁瞬间被写入端抢先）。
 *
 * 短章 maxScroll === 0 属合法，这里不参与判定；也刻意不用 #chapter-content 是否非空判别
 * （它是固定空壳，await 期间仍挂着旧章）。
 */
export function txUnlockStep(o: {
  now: number;
  lastInputAt: number;
  animDone: boolean;
  framePassed: boolean;
  chapterNum: number;
  target: number;
  loading: boolean;
}): TxUnlockStep {
  if (o.now - o.lastInputAt < TX_IDLE_MS) return "wait";
  if (o.chapterNum !== o.target || o.loading) return "wait";
  if (!o.animDone) return "wait";
  return o.framePassed ? "unlock" : "frame";
}

/** 锁定期内的被动输入处置：true = 吞掉（同向，并续期判据 A），false = 立即彻底放行（反向）。 */
export function txSwallows(inputDir: number, txDir: number): boolean {
  return Math.sign(inputDir) === Math.sign(txDir);
}

/** 锁定期内按被动滚动源处理的按键（其余按键交给 onKeyDown 走显式导航）。 */
export const TX_SCROLL_KEYS: ReadonlySet<string> = new Set([
  "ArrowDown",
  "ArrowUp",
  "PageDown",
  "PageUp",
  " ",
]);

/** 按键 → 滚动方向（下=1 / 上=-1）；不是滚动键返回 0。 */
export function txKeyDir(key: string): -1 | 0 | 1 {
  if (!TX_SCROLL_KEYS.has(key)) return 0;
  return key === "ArrowDown" || key === "PageDown" || key === " " ? 1 : -1;
}
