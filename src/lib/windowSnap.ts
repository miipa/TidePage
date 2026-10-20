// ============================================================================
// 窗口吸附与整组移动（任务 2 / 任务 5 复用，集中一处）
//
// 单向移动模型：基准窗默认主窗口(label "main")，独立书窗也可以作为它拉出的面板窗的基准；
// 其余为卫星窗（AI 窗 / 面板独立窗），卫星窗自身也可能是下一级的基准。
//   · 拖基准窗 → 该窗 onMoved 广播 snap:base-moved（带 label）；卫星按【固定】offsetFromBase 盲跟贴住。
//   · 拖卫星窗 → 立即解除吸附（不带动任何窗）。跟随可能是两级链：主窗→书窗→面板窗，允许滞后。
//
// 吸附触发（本轮修正：不再"固定 offset 盲跟"，主/卫星两侧都会主动评估）：
//   · 基准窗移动：卫星收到 base-moved 后——已吸附则盲跟(mirrorToBase)；未吸附则节流评估一次
//     evaluateSnapOnce，边缘进阈值即吸附。→ 拖基准窗贴近卫星每次都能吸。
//   · 卫星移动：拖动中节流跑 evaluateSnapPreview（只算、不移、不写、不改 grouped）显示预吸附视觉；
//     停动 SETTLE_MS(≈松手) 后跑 evaluateSnapOnce 真正吸附。
//
// offsetFromBase 生命周期：只在 evaluateSnapOnce 吸附命中处写一次、解除时清；期间从不重算。
//
// 阈值：EDGE_SNAP=真正吸附的边缘差上限；PREVIEW_THRESHOLD=预吸附提示（略宽，给提前量）。
//   另一轴用「真实重叠 >0」参与判定；ALIGN_TOLER 仅用于命中后另一轴微对齐。
//
// 防回环：卫星自移前置 syncing + suppressUntil 时间窗 + expected 位置；onMoved 命中任一即视为回声直接 return。
// 坐标：一律原生物理坐标(PhysicalPosition)，不做换算比较。
// 生命周期：unlisten 句柄集中保存，stopSnap()/关闭时调用并 emit snap:leave/closed 摘组清偏移。
// 特效：预吸附/吸附成功通过 window CustomEvent(tp-snap-preview / tp-snap-flash) 通知本窗根组件(+page)渲染。
// ============================================================================

import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { prefersReducedMotion } from "./reducedMotion";

const BASE_LABEL = "main";
const EDGE_SNAP = 10;         // 物理 px：边缘差 ≤ 此值且另一轴真实重叠 → 真正吸附（6–12 取中）
const PREVIEW_THRESHOLD = 18; // 物理 px：预吸附提示阈值（略宽于 EDGE_SNAP，给提前量，12–20 区间）
const ALIGN_TOLER = 40;       // 物理 px：命中后另一轴微对齐容差（不参与「是否吸附」判定）
const SETTLE_MS = 180;        // ms：卫星窗停动此时长 ≈ 松手 → 评估一次吸附
const ECHO_EPS = 3;           // 物理 px：程序性自移回声位置容差
const SUPPRESS_MS = 140;      // ms：程序性 mirror 期间回声抑制窗口
const EVAL_THROTTLE_MS = 80;  // ms：主窗驱动的未吸附评估节流
const PREVIEW_THROTTLE_MS = 55; // ms：卫星拖动预吸附评估节流

type XY = { x: number; y: number };
type Geom = { x: number; y: number; w: number; h: number };

let started = false;
let isBase = false;
let myLabel = "";
let baseLabel = BASE_LABEL;              // 本窗的基准窗 label：默认主窗，面板窗可改为打开书的 reader-<id>
let baseProvider: (() => string) | null = null; // 面板窗提供的基准窗解析函数
let unlisteners: UnlistenFn[] = [];

// 卫星窗状态
let grouped = false;
let offsetFromBase: XY | null = null; // 只在吸附命中时写、解除时清
let syncing = false;
let expected: XY | null = null;
let suppressUntil = 0;
let settleTimer: ReturnType<typeof setTimeout> | undefined;
let lastSatPos: XY = { x: 0, y: 0 };
let evalPending = false;
let previewPending = false;
let previewOn = false;
// 广播节流：16ms 内的连续移动只发最后一次位置（尾部必发）
let bcTimer: ReturnType<typeof setTimeout> | undefined;
let bcPending: XY | null = null;
// 抬升节流：吸附成组期间跟随移动很频繁，50ms 内只发一次置窗序命令
let raiseTimer: ReturnType<typeof setTimeout> | undefined;

function near(a: XY, b: XY, eps = ECHO_EPS): boolean {
  return Math.abs(a.x - b.x) <= eps && Math.abs(a.y - b.y) <= eps;
}

async function geom(win: { outerPosition: () => Promise<{ x: number; y: number }>; outerSize: () => Promise<{ width: number; height: number }> }): Promise<Geom> {
  const p = await win.outerPosition();
  const s = await win.outerSize();
  return { x: p.x, y: p.y, w: s.width, h: s.height };
}

/** 计算卫星相对基准窗的最优吸附（边缘差 ≤ maxDist 且另一轴真实重叠）。 */
function bestSnap(sp: { x: number; y: number }, ss: { width: number; height: number }, base: Geom, maxDist: number): { dist: number; target: XY } | null {
  const sat = { left: sp.x, top: sp.y, right: sp.x + ss.width, bottom: sp.y + ss.height };
  const b = { left: base.x, top: base.y, right: base.x + base.w, bottom: base.y + base.h };
  const vOverlap = Math.min(sat.bottom, b.bottom) - Math.max(sat.top, b.top) > 0;
  const hOverlap = Math.min(sat.right, b.right) - Math.max(sat.left, b.left) > 0;
  let best = maxDist + 1;
  let target: XY | null = null;
  const consider = (d: number, tx: number, ty: number) => { if (d < best) { best = d; target = { x: Math.round(tx), y: Math.round(ty) }; } };
  if (vOverlap) {
    const d1 = Math.abs(sat.left - b.right); if (d1 <= maxDist) consider(d1, b.right, alignAxis(sat.top, b.top));
    const d2 = Math.abs(sat.right - b.left); if (d2 <= maxDist) consider(d2, b.left - ss.width, alignAxis(sat.top, b.top));
  }
  if (hOverlap) {
    const d3 = Math.abs(sat.top - b.bottom); if (d3 <= maxDist) consider(d3, alignAxis(sat.left, b.left), b.bottom);
    const d4 = Math.abs(sat.bottom - b.top); if (d4 <= maxDist) consider(d4, alignAxis(sat.left, b.left), b.top - ss.height);
  }
  return target ? { dist: best, target } : null;
}

function alignAxis(satEdge: number, baseEdge: number): number {
  return Math.abs(satEdge - baseEdge) <= ALIGN_TOLER ? baseEdge : satEdge;
}

// —— 特效通知：本窗根组件(+page) 监听渲染 ——
function setPreview(on: boolean): void {
  if (on === previewOn) return;
  previewOn = on;
  try { window.dispatchEvent(new CustomEvent("tp-snap-preview", { detail: on })); } catch { /* ignore */ }
}
function flashSnap(): void {
  try { window.dispatchEvent(new CustomEvent("tp-snap-flash")); } catch { /* ignore */ }
}

/** 合并广播：16ms 内的连续移动只发最后一次位置（尾部必发，停止时不会漏最后一位）。 */
function broadcastMoved(pos: XY): void {
  bcPending = pos;
  if (bcTimer !== undefined) return;
  bcTimer = setTimeout(() => {
    bcTimer = undefined;
    const p = bcPending;
    bcPending = null;
    if (p) void emit("snap:base-moved", { label: myLabel, x: p.x, y: p.y });
  }, 16);
}

/** 把本窗抬到基准窗之上（不激活、不置顶其它程序）；50ms 内只发一次。 */
function raiseAboveBase(): void {
  if (!grouped || baseLabel === myLabel) return;
  if (raiseTimer !== undefined) return;
  raiseTimer = setTimeout(() => {
    raiseTimer = undefined;
    if (!grouped) return;
    void invoke("raise_above", { baseLabel }).catch(() => {});
  }, 50);
}

/** 设置本窗的基准窗来源（面板窗按当前书解析到 reader-<id>）；传 null 回到主窗。 */
export function setSnapBaseLabelProvider(fn: (() => string) | null): void {
  baseProvider = fn;
}

/** 重新解析本窗的基准窗；变化时解除现有吸附（offsetFromBase 是相对旧基准算的，留着会跳位）。
 *  注意：本函数可能早于 startSnap() 被调用（子组件 onMount 先于父组件），那时 myLabel 还是空串，
 *  解除吸附会发出一条无人匹配的 snap:leave —— 无副作用，属预期。 */
export async function refreshSnapBase(): Promise<void> {
  const want = baseProvider ? baseProvider() : BASE_LABEL;
  let next = BASE_LABEL;
  if (want && want !== BASE_LABEL) {
    try { if (await WebviewWindow.getByLabel(want)) next = want; } catch { /* ignore */ }
  }
  if (next === baseLabel) return;
  if (grouped) {
    grouped = false;
    offsetFromBase = null;
    setPreview(false);
    try { await emit("snap:leave", { label: myLabel }); } catch { /* ignore */ }
  }
  baseLabel = next;
}

/** 启动本窗吸附逻辑（主窗=基准；其余=卫星，基准可能是主窗或独立书窗）。幂等。 */
export async function startSnap(): Promise<void> {
  if (started) return;
  started = true;
  const win = getCurrentWindow();
  myLabel = win.label;
  isBase = myLabel === BASE_LABEL;

  // 只注册一条 onMoved：每个窗都可能是某个卫星（面板窗）的基准，移动必须无条件广播；
  // 广播要早于 onSatelliteMoved 的静音判断——否则书窗跟随主窗时不再转发，主窗→书窗→面板窗的链会断。
  const un = await win.onMoved(({ payload }) => {
    broadcastMoved({ x: payload.x, y: payload.y });
    if (isBase) return;
    onSatelliteMoved({ x: payload.x, y: payload.y });
  });
  unlisteners.push(un);
  // 每个窗被聚焦时广播：吸附组里的卫星据此重新把本窗抬到基准窗之上。
  unlisteners.push(await win.onFocusChanged(({ payload }) => {
    if (payload) void emit("snap:base-focused", { label: myLabel });
  }));
  if (isBase) return;
  unlisteners.push(await listen<{ label: string }>("snap:base-focused", (e) => {
    if (e.payload.label === baseLabel && grouped) raiseAboveBase();
  }));
  unlisteners.push(await listen<{ label: string; x: number; y: number }>("snap:base-moved", (e) => {
    if (e.payload.label !== baseLabel) return;
    void onBaseMoved({ x: e.payload.x, y: e.payload.y });
  }));
  unlisteners.push(await listen<{ label: string }>("snap:closed", (e) => {
    if (e.payload.label !== baseLabel) return;
    grouped = false;
    offsetFromBase = null;
    setPreview(false);
    void refreshSnapBase();
  }));
  await refreshSnapBase();
}

/** 停止并清理。 */
export async function stopSnap(): Promise<void> {
  if (!started) return;
  started = false;
  try {
    if (!isBase && grouped) await emit("snap:leave", { label: myLabel });
    await emit("snap:closed", { label: myLabel });
  } catch { /* ignore */ }
  for (const un of unlisteners) { try { un(); } catch { /* ignore */ } }
  unlisteners = [];
  if (settleTimer !== undefined) { clearTimeout(settleTimer); settleTimer = undefined; }
  if (bcTimer !== undefined) { clearTimeout(bcTimer); bcTimer = undefined; }
  bcPending = null;
  if (raiseTimer !== undefined) { clearTimeout(raiseTimer); raiseTimer = undefined; }
  setPreview(false);
  grouped = false; offsetFromBase = null; syncing = false; expected = null; suppressUntil = 0;
  evalPending = false; previewPending = false;
}

// —— 主窗移动到达卫星：已吸附→盲跟贴住；未吸附→节流评估是否进入吸附阈值 ——
async function onBaseMoved(basePos: XY): Promise<void> {
  if (grouped) { await mirrorToBase(basePos); return; } // 已吸附：按固定 offset 盲跟，保证极速拖不丢
  scheduleEvaluate();                                    // 未吸附：主窗贴近本窗 → 评估→可能吸上
}

async function mirrorToBase(basePos: XY): Promise<void> {
  if (!grouped || !offsetFromBase) return;
  syncing = true;
  suppressUntil = performance.now() + SUPPRESS_MS;
  const target = { x: Math.round(basePos.x + offsetFromBase.x), y: Math.round(basePos.y + offsetFromBase.y) };
  expected = target;
  try { await getCurrentWindow().setPosition(new PhysicalPosition(target.x, target.y)); } catch { /* ignore */ }
  raiseAboveBase();
  queueMicrotask(() => { syncing = false; });
}

function scheduleEvaluate(): void {
  if (evalPending || grouped) return;
  evalPending = true;
  setTimeout(() => { evalPending = false; void evaluateSnapOnce(); }, EVAL_THROTTLE_MS);
}

// 卫星窗自身移动
function onSatelliteMoved(pos: XY): void {
  if (syncing) return;                                   // 程序性自移
  if (performance.now() < suppressUntil) return;         // mirror 回声抑制窗口
  if (expected && near(pos, expected)) { expected = null; return; } // 精确回声吸收
  // —— 用户直接拖动卫星窗：立即解除吸附，不带动任何窗 ——
  if (grouped) {
    grouped = false;
    offsetFromBase = null;
    void emit("snap:leave", { label: myLabel });
  }
  lastSatPos = pos;
  schedulePreview();                                     // 拖动中：预吸附视觉（只算不移）
  if (settleTimer !== undefined) clearTimeout(settleTimer);
  settleTimer = setTimeout(() => { settleTimer = undefined; void evaluateSnapOnce(); }, SETTLE_MS); // 松手→真正吸附
}

// 预吸附评估：只计算最小边缘差，进阈值→显示预吸附视觉；不 setPosition/不写 offset/不改 grouped
function schedulePreview(): void {
  if (previewPending) return;
  previewPending = true;
  setTimeout(() => { previewPending = false; void evaluateSnapPreview(); }, PREVIEW_THROTTLE_MS);
}
async function evaluateSnapPreview(): Promise<void> {
  if (grouped) { setPreview(false); return; }
  const self = getCurrentWindow();
  let baseWin = await WebviewWindow.getByLabel(baseLabel);
  // 基准窗可能已被销毁而没送达 snap:closed：先重解析回退，否则永远只认一扇不存在的窗。
  if (!baseWin && baseLabel !== BASE_LABEL) {
    await refreshSnapBase();
    baseWin = await WebviewWindow.getByLabel(baseLabel);
  }
  if (!baseWin) { setPreview(false); return; }
  let base: Geom, ss: { width: number; height: number }, sp: { x: number; y: number };
  try { base = await geom(baseWin); ss = await self.outerSize(); sp = await self.outerPosition(); } catch { return; }
  setPreview(!!bestSnap(sp, ss, base, PREVIEW_THRESHOLD)); // 进预吸附阈值 → 亮，离开 → 灭
}

// 松手/主窗贴近后评估一次：边缘差 ≤ EDGE_SNAP 且另一轴真实重叠才吸附
async function evaluateSnapOnce(): Promise<void> {
  if (grouped) return;
  const self = getCurrentWindow();
  let baseWin = await WebviewWindow.getByLabel(baseLabel);
  // 同 evaluateSnapPreview：基准窗消失时先回退到主窗，再评估吸附。
  if (!baseWin && baseLabel !== BASE_LABEL) {
    await refreshSnapBase();
    baseWin = await WebviewWindow.getByLabel(baseLabel);
  }
  if (!baseWin) return;
  let base: Geom, ss: { width: number; height: number }, sp: { x: number; y: number };
  try { base = await geom(baseWin); ss = await self.outerSize(); sp = await self.outerPosition(); } catch { return; }

  const hit = bestSnap(sp, ss, base, EDGE_SNAP);
  if (!hit) { setPreview(false); return; } // 边缘不相邻 → 不吸（隔空不吸）

  const t = hit.target;
  syncing = true;
  suppressUntil = performance.now() + SUPPRESS_MS;
  expected = t;
  try { await self.setPosition(new PhysicalPosition(t.x, t.y)); } catch { /* ignore */ }
  queueMicrotask(() => { syncing = false; });
  offsetFromBase = { x: t.x - base.x, y: t.y - base.y }; // 唯一写入点
  grouped = true;
  setPreview(false);
  flashSnap();                                            // 吸附成功特效
  raiseAboveBase();                                       // 吸上后立即贴到基准窗之上
  try { await emit("snap:join", { label: myLabel }); } catch { /* ignore */ }
}

// ============================================================================
// Svelte action：按住面板标题栏拖出面板外 → 拉成独立窗（任务 5）
// ============================================================================
const DRAG_OUT = 30; // px：进入「拖出」态的位移阈值

export function dragOutHandle(node: HTMLElement, onout?: () => void) {
  let sx = 0, sy = 0, active = false, dragging = false;
  // AI 抽屉的根是 .ai-panel（不是 .sidepanel），一并认；否则它的拖出反馈与
  // 「松开在面板内=取消」都判不出来。
  const panelEl = () => node.closest(".sidepanel, .ai-panel") as HTMLElement | null;

  function feedback(on: boolean) {
    const p = panelEl();
    if (!p) return;
    // 「减少动效」下简化视觉反馈：去掉虚线描边与半透明（两者都是「运动/形变」类暗示），
    // 只保留 cursor 与一个静态的 inset 描边，保证「正在拖出」这件事仍然可感知。
    // 正确做法不是给这里加动画——拖拽跟随本来就该即时——而是把反馈做减法。
    if (on) {
      p.style.outline = prefersReducedMotion() ? "" : "2px dashed var(--accent)";
      p.style.outlineOffset = prefersReducedMotion() ? "" : "-2px";
      p.style.opacity = prefersReducedMotion() ? "" : "0.82";
      p.style.boxShadow = prefersReducedMotion() ? "inset 0 0 0 2px var(--accent)" : "";
      node.style.cursor = "grabbing";
    } else {
      p.style.outline = "";
      p.style.outlineOffset = "";
      p.style.opacity = "";
      p.style.boxShadow = "";
      node.style.cursor = "";
    }
  }

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement)?.closest("button,input,select,textarea,a")) return; // 避开交互元素
    sx = e.clientX; sy = e.clientY; active = true; dragging = false;
    try { node.setPointerCapture(e.pointerId); } catch { /* ignore */ }
  }
  function move(e: PointerEvent) {
    if (!active) return;
    if (!dragging && Math.hypot(e.clientX - sx, e.clientY - sy) > DRAG_OUT) { dragging = true; feedback(true); }
  }
  function up(e: PointerEvent) {
    if (!active) return;
    active = false;
    try { node.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
    if (!dragging) return;
    dragging = false; feedback(false);
    const rect = panelEl()?.getBoundingClientRect();
    const inside = !!rect && e.clientX >= rect.left && e.clientX <= rect.right && e.clientY >= rect.top && e.clientY <= rect.bottom;
    if (!inside) onout?.(); // 释放到面板外 → 拉出独立窗；面板内 → 取消
  }
  function cancel() { if (!active) return; active = false; dragging = false; feedback(false); }

  node.style.touchAction = "none";
  node.addEventListener("pointerdown", down);
  node.addEventListener("pointermove", move);
  node.addEventListener("pointerup", up);
  node.addEventListener("pointercancel", cancel);
  return {
    update(next?: () => void) { onout = next; },
    destroy() {
      node.removeEventListener("pointerdown", down);
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerup", up);
      node.removeEventListener("pointercancel", cancel);
    },
  };
}
