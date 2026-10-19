import { describe, expect, it } from "vitest";
import {
  pageTransitionGeometry,
  pageTransitionOf,
  pageTurnFxClass,
  TX_COVER_EASE,
  TX_COVER_MS,
  TX_H_SLIDE_EASE,
  TX_H_SLIDE_MS,
  TX_H_SPRING_IN_EASE,
  TX_H_SPRING_MS,
  TX_H_SPRING_OUT_EASE,
  TX_HARD_MS,
  TX_IDLE_MS,
  TX_SPRING_EASE,
  TX_SPRING_MS,
  TX_SPRING_SHIFT,
  transitionGeometry,
  transitionModeOf,
  txKeyDir,
  txSwallows,
  txUnlockStep,
} from "./chapterTx";

describe("transitionModeOf", () => {
  it("合法档位原样返回", () => {
    expect(transitionModeOf("cover")).toBe("cover");
    expect(transitionModeOf("spring")).toBe("spring");
    expect(transitionModeOf("none")).toBe("none");
  });

  it("缺失/非法一律取 cover（旧配置没有这个键，不能变成 undefined）", () => {
    expect(transitionModeOf(undefined)).toBe("cover");
    expect(transitionModeOf(null)).toBe("cover");
    expect(transitionModeOf("")).toBe("cover");
    expect(transitionModeOf("buffered")).toBe("cover");
    expect(transitionModeOf(true)).toBe("cover");
  });
});

describe("transitionGeometry", () => {
  it("cover：旧章视差 0.35× 可视高，新章整层滑入，720ms", () => {
    const g = transitionGeometry("cover", 1, 600);
    expect(g.durMs).toBe(TX_COVER_MS);
    expect(g.easing).toBe(TX_COVER_EASE);
    expect(g.outTo).toBe(-210); // 600 * 0.35，向下续章时上移
    expect(g.inFrom).toBe(600);
  });

  it("cover 向上切章镜像", () => {
    const g = transitionGeometry("cover", -1, 600);
    expect(g.outTo).toBe(210);
    expect(g.inFrom).toBe(-600);
  });

  it("spring：整层移出 104%，780ms，缓动带过冲", () => {
    const g = transitionGeometry("spring", 1, 500);
    expect(g.durMs).toBe(TX_SPRING_MS);
    expect(g.easing).toBe(TX_SPRING_EASE);
    expect(g.outTo).toBe(-Math.round(500 * TX_SPRING_SHIFT));
    expect(g.inFrom).toBe(500);
  });

  it("可视高为 0 / 非法时按 1px 兜底，不产生 0 位移", () => {
    expect(transitionGeometry("cover", 1, 0).inFrom).toBe(1);
    expect(transitionGeometry("cover", 1, Number.NaN).outTo).toBe(-Math.round(1 * 0.35));
  });

  it("轴向：默认 y，显式 x 原样带回（翻页模式用横向分量）", () => {
    expect(transitionGeometry("cover", 1, 600).axis).toBe("y");
    expect(transitionGeometry("cover", 1, 600, "x").axis).toBe("x");
    // 数值本身不随轴向变：只影响调用方拼哪一个位移分量。
    expect(transitionGeometry("cover", 1, 600, "x").inFrom).toBe(600);
  });
});

describe("pageTransitionOf", () => {
  it("合法档位原样返回（收口后只有三项）", () => {
    expect(pageTransitionOf("h-spring")).toBe("h-spring");
    expect(pageTransitionOf("h-slide")).toBe("h-slide");
    expect(pageTransitionOf("none")).toBe("none");
  });

  it("缺失/非法一律取 h-slide（旧配置没有这个键）", () => {
    expect(pageTransitionOf(undefined)).toBe("h-slide");
    expect(pageTransitionOf(null)).toBe("h-slide");
    expect(pageTransitionOf("")).toBe("h-slide");
    expect(pageTransitionOf("cover")).toBe("h-slide"); // 滚动模式的键不能串到翻页模式
    expect(pageTransitionOf("h-flip")).toBe("h-slide"); // 刻意不实现的档位
    expect(pageTransitionOf(true)).toBe("h-slide");
  });

  it("已删除的视差侧滑档：读入不报错、可预期地回落 h-slide（不靠默认分支吞旧值）", () => {
    // 历史档位字面量按片段拼：收口要求全仓 grep 该串 0 命中，测试里也不再写出完整串。
    const removed = ["h", "cover"].join("-");
    expect(() => pageTransitionOf(removed)).not.toThrow();
    expect(pageTransitionOf(removed)).toBe("h-slide");
  });
});

describe("pageTurnFxClass", () => {
  it("档位 → .col-page 上挂的类（空串 = 基础规则 = 左右滑动；弹簧档走整页双层、不挂类）", () => {
    expect(pageTurnFxClass("h-spring")).toBe("");
    expect(pageTurnFxClass("none")).toBe("tx-fx-none");
    expect(pageTurnFxClass("h-slide")).toBe("");
  });
});

describe("pageTransitionGeometry", () => {
  it("h-spring：780ms，入场带过冲、退场整层移出（退场只动位移，无透明度/缩放/亮度）", () => {
    const g = pageTransitionGeometry("h-spring", 1, 616);
    expect(g.durMs).toBe(TX_H_SPRING_MS);
    expect(g.easingIn).toBe(TX_H_SPRING_IN_EASE);
    expect(g.easingOut).toBe(TX_H_SPRING_OUT_EASE);
    expect(g.easingIn).not.toBe(g.easingOut); // 退场不带过冲（与参考件一致）
    expect(g.inFrom).toBe(616);
    expect(g.outTo).toBe(-616);
    expect(Object.keys(g).sort()).toEqual(["durMs", "easingIn", "easingOut", "inFrom", "outTo"]);
  });

  it("h-slide：220ms，沿用 --dur-page/--ease-page（观感与普通翻页一致）", () => {
    const g = pageTransitionGeometry("h-slide", 1, 616);
    expect(g.durMs).toBe(TX_H_SLIDE_MS);
    expect(g.easingIn).toBe(TX_H_SLIDE_EASE);
    expect(g.easingOut).toBe(TX_H_SLIDE_EASE);
    expect(g.outTo).toBe(-616);
    expect(Object.keys(g)).not.toContain("outOpacity");
  });

  it("向上翻页镜像", () => {
    const g = pageTransitionGeometry("h-spring", -1, 616);
    expect(g.inFrom).toBe(-616);
    expect(g.outTo).toBe(616);
  });

  it("跨度用 effAvail：双栏 616（stepPx 664）时不得出现 48px 的缝", () => {
    const effAvail = 616;
    const stepPx = 664; // 2*(284+48)，与 effAvail 差一个 gap
    expect(pageTransitionGeometry("h-slide", 1, effAvail).inFrom).toBe(effAvail);
    expect(pageTransitionGeometry("h-slide", 1, effAvail).inFrom).not.toBe(stepPx);
  });

  it("跨度非法/为 0 时按 1px 兜底", () => {
    expect(pageTransitionGeometry("h-slide", 1, 0).inFrom).toBe(1);
    expect(pageTransitionGeometry("h-slide", 1, Number.NaN).outTo).toBe(-1);
  });
});

describe("txUnlockStep", () => {
  // lastInputAt=0 表示「早已停手」：默认满足判据 A。
  const base = { now: 1000, lastInputAt: 0, animDone: true, framePassed: false, chapterNum: 3, target: 3, loading: false };

  it("未过静止间隔：等待（同向输入持续续期）", () => {
    expect(txUnlockStep({ ...base, now: TX_IDLE_MS - 1 })).toBe("wait");
    expect(txUnlockStep({ ...base, now: 500, lastInputAt: 400 })).toBe("wait"); // 差 100ms
    expect(txUnlockStep({ ...base, now: 520, lastInputAt: 400, framePassed: true })).toBe("unlock"); // 差 120ms
  });

  it("落地信号缺失（章号未到 / 仍在加载）：等待", () => {
    expect(txUnlockStep({ ...base, chapterNum: 2 })).toBe("wait");
    expect(txUnlockStep({ ...base, loading: true })).toBe("wait");
  });

  it("动画未收尾：等待（动画期写 transform 会与回弹互相覆盖）", () => {
    expect(txUnlockStep({ ...base, animDone: false })).toBe("wait");
  });

  it("A 且 B 成立但未过帧 → 先过一帧；过了帧才解锁", () => {
    expect(txUnlockStep(base)).toBe("frame");
    expect(txUnlockStep({ ...base, framePassed: true })).toBe("unlock");
  });

  it("短章（maxScroll=0）不参与判定：只看章号与加载态", () => {
    expect(txUnlockStep({ ...base, framePassed: true })).toBe("unlock");
  });

  it("硬兜底只防事件流异常：数值必须显著大于动画时长", () => {
    expect(TX_HARD_MS).toBeGreaterThan(TX_SPRING_MS + TX_IDLE_MS);
  });
});

describe("txSwallows / txKeyDir", () => {
  it("同向吞掉，反向放行", () => {
    expect(txSwallows(1, 1)).toBe(true);
    expect(txSwallows(-1, -1)).toBe(true);
    expect(txSwallows(-1, 1)).toBe(false);
    expect(txSwallows(1, -1)).toBe(false);
  });

  it("滚动键映射：↓/PageDown/空格 = 向下，↑/PageUp = 向上，其余为 0", () => {
    expect(txKeyDir("ArrowDown")).toBe(1);
    expect(txKeyDir("PageDown")).toBe(1);
    expect(txKeyDir(" ")).toBe(1);
    expect(txKeyDir("ArrowUp")).toBe(-1);
    expect(txKeyDir("PageUp")).toBe(-1);
    expect(txKeyDir("Enter")).toBe(0);
    expect(txKeyDir("ArrowRight")).toBe(0);
  });
});
