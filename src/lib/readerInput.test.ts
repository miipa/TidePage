import { describe, it, expect } from "vitest";
import {
  DRAG_MIN_PX,
  WHEEL_ACC_IDLE_MS,
  WHEEL_LOCK_MS,
  WHEEL_PANEL_SEL,
  WHEEL_FIELD_SEL,
  WHEEL_TURN_IDLE,
  WHEEL_TURN_THRESHOLD,
  classifyPointerUp,
  isWheelLocal,
  mangaWheelDelta,
  normalizeWheelDelta,
  reduceWheelTurn,
  scrollKeyRoute,
  shouldDedupeLoad,
  type WheelLocalNode,
} from "./readerInput";

/* ---------- 假元素：只实现 isWheelLocal 用到的最小结构 ---------- */
type FakeNode = Omit<WheelLocalNode, "parentElement" | "closest"> & {
  parentElement: FakeNode | null;
  closest(selectors: string): FakeNode | null;
  sel: Set<string>;
};

function makeNode(o: {
  sh?: number;
  ch?: number;
  sw?: number;
  cw?: number;
  oy?: string;
  ox?: string;
  panel?: boolean;
  field?: boolean;
  parent?: FakeNode | null;
}): FakeNode {
  const el: FakeNode = {
    scrollHeight: o.sh ?? 0,
    clientHeight: o.ch ?? 0,
    scrollWidth: o.sw ?? 0,
    clientWidth: o.cw ?? 0,
    parentElement: o.parent ?? null,
    // node 环境没有 getComputedStyle，isWheelLocal 会退回读这里的内联样式
    style: { overflowY: o.oy ?? "visible", overflowX: o.ox ?? "visible" },
    sel: new Set<string>(),
    closest(selectors: string) {
      let n: FakeNode | null = el;
      while (n) {
        if (n.sel.has(selectors)) return n;
        n = n.parentElement;
      }
      return null;
    },
  };
  if (o.panel) el.sel.add(WHEEL_PANEL_SEL);
  if (o.field) el.sel.add(WHEEL_FIELD_SEL);
  return el;
}

describe("normalizeWheelDelta — 三种 deltaMode 归一到「格」", () => {
  it("deltaMode=0（像素）原样", () => {
    expect(normalizeWheelDelta(0, 100)).toBe(100);
    expect(normalizeWheelDelta(0, -37.5)).toBe(-37.5);
  });
  it("deltaMode=1（行）×16（沿用既有约定）", () => {
    expect(normalizeWheelDelta(1, 3)).toBe(48);
    expect(normalizeWheelDelta(1, -2)).toBe(-32);
  });
  it("deltaMode=2（页）×100：旧实现不换算，deltaY=1 永远攒不到阈值", () => {
    expect(normalizeWheelDelta(2, 1)).toBe(100);
    expect(normalizeWheelDelta(2, -1)).toBe(-100);
  });
});

describe("reduceWheelTurn — 翻页阈值 / 锁窗口 / 时间衰减", () => {
  const T = 10_000; // 基准时间戳

  it("像素模式一格滚轮（100）单次即翻页", () => {
    const r = reduceWheelTurn(WHEEL_TURN_IDLE, 0, 100, T);
    expect(r.turn).toBe(1);
    expect(r.state.lockUntil).toBe(T + WHEEL_LOCK_MS);
    expect(r.state.acc).toBe(100 - WHEEL_TURN_THRESHOLD);
  });

  it("页模式 deltaY=1 单次即翻页（根因 3：旧实现 d=1 永远攒不到 100）", () => {
    expect(reduceWheelTurn(WHEEL_TURN_IDLE, 2, 1, T).turn).toBe(1);
  });

  it("行模式 3 行（48）达阈值；不足阈值的小步进会累积", () => {
    expect(reduceWheelTurn(WHEEL_TURN_IDLE, 1, 3, T).turn).toBe(1);
    const a = reduceWheelTurn(WHEEL_TURN_IDLE, 0, 20, T);
    expect(a.turn).toBe(0);
    expect(a.state.acc).toBe(20);
    const b = reduceWheelTurn(a.state, 0, 20, T + 50);
    expect(b.turn).toBe(1);
  });

  it("锁窗口内不再连翻，但累积量被夹住而非清零（锁一过期立刻生效）", () => {
    const first = reduceWheelTurn(WHEEL_TURN_IDLE, 0, 100, T);
    expect(first.turn).toBe(1);
    const locked = reduceWheelTurn(first.state, 0, 100, T + 50);
    expect(locked.turn).toBe(0);
    expect(locked.state.acc).toBe(WHEEL_TURN_THRESHOLD); // 夹到「已够一页」，不是 0
    const after = reduceWheelTurn(locked.state, 0, 1, T + WHEEL_LOCK_MS + 10);
    expect(after.turn).toBe(1);
  });

  it("锁内反复滚动不会让累积量无界增长", () => {
    let s = reduceWheelTurn(WHEEL_TURN_IDLE, 0, 100, T).state;
    for (let i = 1; i <= 20; i++) {
      s = reduceWheelTurn(s, 0, 100, T + i * 10).state;
      expect(Math.abs(s.acc)).toBeLessThanOrEqual(WHEEL_TURN_THRESHOLD);
    }
  });

  it("静默超过 WHEEL_ACC_IDLE_MS 后累积量归零（残留不跨交互）", () => {
    const half = reduceWheelTurn(WHEEL_TURN_IDLE, 0, 20, T);
    expect(half.state.acc).toBe(20);
    const later = reduceWheelTurn(half.state, 0, 20, T + WHEEL_ACC_IDLE_MS + 1);
    expect(later.turn).toBe(0);
    expect(later.state.acc).toBe(20); // 从 0 重新起算，而不是 40
    const within = reduceWheelTurn(half.state, 0, 20, T + WHEEL_ACC_IDLE_MS - 1);
    expect(within.turn).toBe(1);
  });

  it("反方向累积 → 上一页", () => {
    expect(reduceWheelTurn(WHEEL_TURN_IDLE, 0, -100, T).turn).toBe(-1);
  });

  it("deltaY=0（纯横向）不动状态也不翻页", () => {
    const s = { acc: 12, lastAt: T, lockUntil: 0 };
    const r = reduceWheelTurn(s, 0, 0, T + 10);
    expect(r.turn).toBe(0);
    expect(r.state).toBe(s);
  });
});

describe("isWheelLocal — 只有「真的可滚」的容器才归本地", () => {
  it("正文里的目标没有面板祖先 → 归正文", () => {
    const p = makeNode({ sh: 3000, ch: 500, oy: "auto" });
    const t = makeNode({ parent: p });
    expect(isWheelLocal(t, "y")).toBe(false);
  });

  it("表单控件直接归本地（无论祖先）", () => {
    const host = makeNode({ panel: true });
    const f = makeNode({ field: true, parent: host });
    expect(isWheelLocal(f, "y")).toBe(true);
    const outside = makeNode({ field: true });
    expect(isWheelLocal(outside, "y")).toBe(true);
  });

  it("面板内可滚列表 → 本地（面板自己的列表必须能自己滚）", () => {
    const panel = makeNode({ panel: true, ch: 600, sh: 600, oy: "hidden" });
    const list = makeNode({ oy: "auto", sh: 2000, ch: 600, parent: panel });
    const item = makeNode({ parent: list });
    expect(isWheelLocal(item, "y")).toBe(true);
  });

  it("抽屉展开但内容不可滚 → 回落正文（旧实现一刀切导致滚了没动静）", () => {
    const panel = makeNode({ panel: true, oy: "hidden", sh: 600, ch: 600 });
    const clip = makeNode({ oy: "hidden", sh: 4000, ch: 600, parent: panel }); // 内容溢出但 overflow:hidden
    const txt = makeNode({ parent: clip });
    expect(isWheelLocal(txt, "y")).toBe(false);
  });

  it("面板宿主本身可滚 → 本地", () => {
    const panel = makeNode({ panel: true, oy: "auto", sh: 1200, ch: 600 });
    expect(isWheelLocal(panel, "y")).toBe(true);
  });

  it("按轴判定：纵向可滚的面板不吞横向滚轮", () => {
    const panel = makeNode({ panel: true, oy: "auto", sh: 2000, ch: 600, ox: "hidden", sw: 400, cw: 400 });
    expect(isWheelLocal(panel, "y")).toBe(true);
    expect(isWheelLocal(panel, "x")).toBe(false);
  });

  it("内容刚好不溢出（scrollHeight==clientHeight）不算可滚", () => {
    const panel = makeNode({ panel: true, oy: "auto", sh: 600, ch: 600 });
    expect(isWheelLocal(panel, "y")).toBe(false);
  });

  it("非法目标（null / 普通对象）→ false", () => {
    expect(isWheelLocal(null)).toBe(false);
    expect(isWheelLocal({})).toBe(false);
    expect(isWheelLocal(undefined)).toBe(false);
  });
});

describe("scrollKeyRoute — 非滚动布局的按键路由", () => {
  it("翻页布局 → page（↑/↓ 等价 next/prev，不跨章）", () => {
    expect(scrollKeyRoute({ mangaHorizontal: false, scrollLayout: false })).toBe("page");
  });
  it("滚动布局 → step（保持滚一段）", () => {
    expect(scrollKeyRoute({ mangaHorizontal: false, scrollLayout: true })).toBe("step");
  });
  it("漫画横向优先 → pan", () => {
    expect(scrollKeyRoute({ mangaHorizontal: true, scrollLayout: true })).toBe("pan");
    expect(scrollKeyRoute({ mangaHorizontal: true, scrollLayout: false })).toBe("pan");
  });
});

describe("shouldDedupeLoad — 只对「纯重复」去重", () => {
  it("① 同章在途、不带定位 → 去重（复用在途，不再 ++loadGeneration）", () => {
    expect(shouldDedupeLoad(true, 7, 7)).toBe(true);
    expect(shouldDedupeLoad(true, 7, 7, {})).toBe(true);
  });
  it("② 同章、带 entry（落章首/章末）→ 不去重（显式定位必须执行）", () => {
    expect(shouldDedupeLoad(true, 7, 7, { entry: { at: "tail" } })).toBe(false);
    expect(shouldDedupeLoad(true, 7, 7, { entry: { at: "head", extra: 20 } })).toBe(false);
  });
  it("②b 同章、带 restore（恢复进度）→ 不去重", () => {
    expect(shouldDedupeLoad(true, 7, 7, { restore: { chapter_ratio: 0.5 } })).toBe(false);
  });
  it("③ 不同章 / loadingIdx 非法 → 不去重", () => {
    expect(shouldDedupeLoad(true, 7, 8)).toBe(false);
    expect(shouldDedupeLoad(true, -1, -1)).toBe(false);
  });
  it("④ 未在加载 → 不去重", () => {
    expect(shouldDedupeLoad(false, 7, 7)).toBe(false);
  });
});

describe("classifyPointerUp — 选区最优先的意图顺序", () => {
  it("① 有选区就走划词菜单，哪怕位移很小（拖选 1–2 个字的划词回归）", () => {
    expect(classifyPointerUp({ hasSelection: true, moved: 0 })).toBe("select");
    expect(classifyPointerUp({ hasSelection: true, moved: 4 })).toBe("select");
    expect(classifyPointerUp({ hasSelection: true, moved: 6 })).toBe("select"); // ≤ DRAG_MIN_PX
    expect(classifyPointerUp({ hasSelection: true, moved: 50 })).toBe("select");
  });
  it("② 没选中但拖过阈值 → 拖拽手势，不翻页", () => {
    expect(classifyPointerUp({ hasSelection: false, moved: 9 })).toBe("drag");
    expect(classifyPointerUp({ hasSelection: false, moved: 500 })).toBe("drag");
  });
  it("③ 位移在阈值内且无选区 → 点击（含阈值边界）", () => {
    expect(classifyPointerUp({ hasSelection: false, moved: 0 })).toBe("click");
    expect(classifyPointerUp({ hasSelection: false, moved: 3 })).toBe("click");
    expect(classifyPointerUp({ hasSelection: false, moved: DRAG_MIN_PX })).toBe("click");
  });
});

describe("mangaWheelDelta — 漫画横向平移换算", () => {
  it("横向量优先且原样使用（既有横向平移语义不变）", () => {
    expect(mangaWheelDelta(12, 0, 88)).toBe(12);
    expect(mangaWheelDelta(-30, 2, 0)).toBe(-30); // deltaMode=2 也不换算 deltaX
  });
  it("无横向量 → 纵向回落用同一套换算（deltaMode=2 每下 100px，不再只挪 1px）", () => {
    expect(mangaWheelDelta(0, 2, 1)).toBe(100);
    expect(mangaWheelDelta(0, 1, 3)).toBe(48);
    expect(mangaWheelDelta(0, 0, 42)).toBe(42);
  });
});
