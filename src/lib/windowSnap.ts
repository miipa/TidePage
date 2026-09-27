// ============================================================================
// 窗口吸附与整组移动（任务 2 / 任务 5 复用，集中一处）
//
// 单向移动模型：基准窗 = 主窗口(label "main")；其余为卫星窗（AI 窗 / 面板独立窗）。
//   · 拖主窗  → 主窗 onMoved 广播 snap:base-moved；卫星按【固定】offsetFromBase 盲跟贴住。
//   · 拖卫星窗 → 立即解除吸附（不带动任何窗）。整组移动只有「主窗动→卫星跟」一条链。
//
// 吸附触发（本轮修正：不再"固定 offset 盲跟"，主/卫星两侧都会主动评估）：
//   · 主窗移动：卫星收到 base-moved 后——已吸附则盲跟(mirrorToBase)；未吸附则节流评估一次
//     evaluateSnapOnce，边缘进阈值即吸附。→ 拖主窗贴近卫星每次都能吸。
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
// 特效：预吸附/吸附成功通过 window CustomEvent(fr-snap-preview / fr-snap-flash) 通知本窗根组件(+page)渲染。
// ============================================================================

import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";

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
  try { window.dispatchEvent(new CustomEvent("fr-snap-preview", { detail: on })); } catch { /* ignore */ }
}
function flashSnap(): void {
  try { window.dispatchEvent(new CustomEvent("fr-snap-flash")); } catch { /* ignore */ }
}

/** 启动本窗吸附逻辑（主窗=基准；其余=卫星）。幂等。 */
export async function startSnap(): Promise<void> {
  if (started) return;
  started = true;
  const win = getCurrentWindow();
  myLabel = win.label;
  isBase = myLabel === BASE_LABEL;

  if (isBase) {
    // 基准窗：被用户拖动即广播位置（始终广播，未吸附卫星也能据此评估并吸上）
    const un = await win.onMoved(({ payload }) => {
      void emit("snap:base-moved", { x: payload.x, y: payload.y });
    });
    unlisteners.push(un);
    return;
  }

  // 卫星窗
  const un = await win.onMoved(({ payload }) => { onSatelliteMoved({ x: payload.x, y: payload.y }); });
  unlisteners.push(un);
  unlisteners.push(await listen<XY>("snap:base-moved", (e) => { void onBaseMoved(e.payload); }));
  unlisteners.push(await listen<{ label: string }>("snap:closed", (e) => {
    if (e.payload.label === BASE_LABEL) { grouped = false; offsetFromBase = null; setPreview(false); } // 主窗关闭→解除，防悬空
  }));
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
  const baseWin = await WebviewWindow.getByLabel(BASE_LABEL);
  if (!baseWin) { setPreview(false); return; }
  let base: Geom, ss: { width: number; height: number }, sp: { x: number; y: number };
  try { base = await geom(baseWin); ss = await self.outerSize(); sp = await self.outerPosition(); } catch { return; }
  setPreview(!!bestSnap(sp, ss, base, PREVIEW_THRESHOLD)); // 进预吸附阈值 → 亮，离开 → 灭
}

// 松手/主窗贴近后评估一次：边缘差 ≤ EDGE_SNAP 且另一轴真实重叠才吸附
async function evaluateSnapOnce(): Promise<void> {
  if (grouped) return;
  const self = getCurrentWindow();
  const baseWin = await WebviewWindow.getByLabel(BASE_LABEL);
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
  try { await emit("snap:join", { label: myLabel }); } catch { /* ignore */ }
}

// ============================================================================
// Svelte action：按住面板标题栏拖出面板外 → 拉成独立窗（任务 5）
// ============================================================================
const DRAG_OUT = 30; // px：进入「拖出」态的位移阈值

export function dragOutHandle(node: HTMLElement, onout?: () => void) {
  let sx = 0, sy = 0, active = false, dragging = false;
  const panelEl = () => node.closest(".sidepanel") as HTMLElement | null;

  function feedback(on: boolean) {
    const p = panelEl();
    if (!p) return;
    if (on) { p.style.outline = "2px dashed var(--accent, #4a6b8a)"; p.style.outlineOffset = "-2px"; p.style.opacity = "0.82"; node.style.cursor = "grabbing"; }
    else { p.style.outline = ""; p.style.outlineOffset = ""; p.style.opacity = ""; node.style.cursor = ""; }
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
