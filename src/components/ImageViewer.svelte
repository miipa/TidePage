<script lang="ts">
  import type { BookImage } from "../lib/types";
  import { t } from "../lib/i18n/index.svelte";
  import { outDissolve } from "../lib/motion";
import { IconChevronLeft, IconChevronRight } from "@tabler/icons-svelte";

  let {
    images = [],
    index = 0,
    onindex,
    onclose,
  }: {
    images: BookImage[];
    index: number;
    onindex: (i: number) => void;
    onclose: () => void;
  } = $props();

  let wheelLock = 0;
  /** 缩放倍率：1 = 「适配」（=100%），上限 8 倍、下限 10%（与提示词的 10%–800% 一致）。 */
  let scale = $state(1);
  /** 单击图片时在「适配 ↔ 上次放大倍率」之间切换，沿用改造前 zoomed 的手感。 */
  let lastZoom = $state(2.5);
  let stageEl: HTMLDivElement | undefined = $state();
  let imgEl: HTMLImageElement | undefined = $state();
  let natW = $state(0);
  let natH = $state(0);
  let stageW = $state(0);
  let stageH = $state(0);
  /** 适配时四周留白（原 .iv-stage 的 padding 搬到这里，好让内容原点 = 滚动原点，锚点数学才精确）。 */
  const PAD_X = 72;
  const PAD_Y = 56;
  const ZOOM_STEP = 1.25;

  const total = $derived(images.length);
  const current = $derived(images[Math.max(0, Math.min(index, total - 1))] ?? null);

  /** 适配尺寸：按舞台可用区等比缩到装得下，且**不放大原图**（避免小图被硬撑成糊图）。 */
  const fit = $derived.by(() => {
    if (!natW || !natH || !stageW || !stageH) return { w: 0, h: 0 };
    const aw = Math.max(80, stageW - PAD_X * 2);
    const ah = Math.max(80, stageH - PAD_Y * 2);
    const r = Math.min(aw / natW, ah / natH, 1);
    return { w: Math.max(1, Math.round(natW * r)), h: Math.max(1, Math.round(natH * r)) };
  });
  const dispW = $derived(Math.max(24, Math.round(fit.w * scale)));
  const dispH = $derived(Math.max(24, Math.round(fit.h * scale)));
  const pct = $derived(Math.round(scale * 100));
  /** 图比舞台小就居中留白；比舞台大就 0 边距，让内容原点与滚动原点重合（锚点缩放的前提）。 */
  const mT = $derived(Math.max(0, Math.round((stageH - dispH) / 2)));
  const mR = $derived(Math.max(0, Math.round((stageW - dispW) / 2)));

  function clamp(i: number): number {
    if (total <= 0) return 0;
    return ((i % total) + total) % total;
  }
  function prev() {
    if (total > 1) onindex(clamp(index - 1));
  }
  function next() {
    if (total > 1) onindex(clamp(index + 1));
  }

  function measure() {
    const el = stageEl;
    if (!el) return;
    stageW = el.clientWidth;
    stageH = el.clientHeight;
  }

  function onImgLoad(e: Event) {
    const i = e.currentTarget as HTMLImageElement;
    natW = i.naturalWidth;
    natH = i.naturalHeight;
    measure();
  }

  function clampScale(k: number): number {
    return Math.max(0.1, Math.min(8, k));
  }

  /** 缩放并保持锚点（默认舞台中心）下的像素不动。
   *  做法：把锚点换算成「内容坐标」，缩放 f 倍后同一内容点落在 anchor*f，据此重设滚动量；
   *  浏览器会自动把滚动量夹到合法范围（所以「拖出视野找不回来」不会发生）。 */
  function zoomTo(k: number, anchorX?: number, anchorY?: number) {
    const el = stageEl;
    const k1 = clampScale(k);
    if (!el) {
      scale = k1;
      return;
    }
    const rect = el.getBoundingClientRect();
    const px = anchorX == null ? el.clientWidth / 2 : anchorX - rect.left;
    const py = anchorY == null ? el.clientHeight / 2 : anchorY - rect.top;
    // 内容坐标必须把「居中留白」减掉：图比舞台小时有 mR/mT 留白，一旦放大到超出舞台，
    // 留白会从正数变成 0；不减掉这一项，锚点就会跟着留白的变化跳（表现为第一次放大整幅图位移）。
    const cx = px + el.scrollLeft - mR;
    const cy = py + el.scrollTop - mT;
    const f = k1 / scale;
    scale = k1;
    // 等一帧：宽度/边距要等 DOM 落定后再写滚动量，否则用的是旧尺寸，锚点会跑偏
    requestAnimationFrame(() => {
      const s = stageEl;
      if (!s) return;
      // 缩放后的留白按新尺寸重算：scroll = 新留白 + 锚点内容偏移×倍率 − 光标位置
      const mR1 = Math.max(0, (s.clientWidth - fit.w * k1) / 2);
      const mT1 = Math.max(0, (s.clientHeight - fit.h * k1) / 2);
      s.scrollLeft = mR1 + cx * f - px;
      s.scrollTop = mT1 + cy * f - py;
    });
  }

  /* 拖拽平移：只有内容确实超出视口才拦截（否则保留单击语义）。 */
  let dragPid = -1;
  let dragX = 0;
  let dragY = 0;
  let dragL = 0;
  let dragT = 0;
  let dragging = false;
  let suppressClick = false;

  function onStagePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const el = stageEl;
    if (!el) return;
    if (el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight) return;
    // 每次按下都复位：否则「拖拽后点空白（click 落在 stage，不消费标记）」会把下一次点图片吞掉
    suppressClick = false;
    dragPid = e.pointerId;
    dragX = e.clientX;
    dragY = e.clientY;
    dragL = el.scrollLeft;
    dragT = el.scrollTop;
    dragging = false;
    el.setPointerCapture?.(e.pointerId);
  }

  function onStagePointerMove(e: PointerEvent) {
    if (e.pointerId !== dragPid) return;
    const dx = e.clientX - dragX;
    const dy = e.clientY - dragY;
    if (!dragging) {
      if (Math.hypot(dx, dy) < 4) return; // 4px 阈值区分单击与拖拽
      dragging = true;
    }
    const el = stageEl;
    if (!el) return;
    el.scrollLeft = dragL - dx;
    el.scrollTop = dragT - dy;
  }

  function onStagePointerUp(e: PointerEvent) {
    if (e.pointerId !== dragPid) return;
    dragPid = -1;
    if (dragging) {
      dragging = false;
      suppressClick = true; // 拖拽过：随后那次 click 不再切成缩放
    }
  }

  /** 单击图片：适配 ↔ 上次放大倍率（拖动过则忽略本次点击）。 */
  function onImgClick(e: MouseEvent) {
    e.stopPropagation();
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if (scale > 1) {
      lastZoom = scale;
      zoomTo(1);
    } else {
      zoomTo(lastZoom > 1 ? lastZoom : 2.5);
    }
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    e.stopPropagation();
    // Alt+滚轮 = 以光标为锚点缩放；不带 Alt 仍是上一张/下一张（保留 220ms 节流）
    if (e.altKey) {
      zoomTo(scale * (e.deltaY < 0 ? 1.1 : 1 / 1.1), e.clientX, e.clientY);
      return;
    }
    const now = Date.now();
    if (now < wheelLock) return;
    if (Math.abs(e.deltaY) < 8) return;
    wheelLock = now + 220;
    if (e.deltaY > 0) next();
    else prev();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      // stopImmediatePropagation：阻止同一 window 上 Reader 的快捷键监听（否则 Esc 会同时触发“返回书库”）
      e.stopImmediatePropagation();
      onclose();
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      e.stopImmediatePropagation();
      prev();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      e.stopImmediatePropagation();
      next();
    } else if (e.key === "+" || e.key === "=") {
      e.preventDefault();
      e.stopImmediatePropagation();
      zoomTo(scale * ZOOM_STEP);
    } else if (e.key === "-" || e.key === "_") {
      e.preventDefault();
      e.stopImmediatePropagation();
      zoomTo(scale / ZOOM_STEP);
    } else if (e.key === "0") {
      e.preventDefault();
      e.stopImmediatePropagation();
      zoomTo(1);
    }
  }

  // 切图/换源：回到适配并清掉平移量（不能让上一张的缩放带着跑，否则新图一上来就在视野外）
  $effect(() => {
    void index;
    void current?.url;
    scale = 1;
    requestAnimationFrame(() => {
      measure();
      const el = stageEl;
      if (el) {
        el.scrollLeft = 0;
        el.scrollTop = 0;
      }
    });
  });

  // 窗口尺寸变化：重算适配尺寸；同时在图片已缓存（onload 不会再触发）时补一次自然尺寸
  $effect(() => {
    measure();
    const im = imgEl;
    if (im && im.complete && !natW) {
      natW = im.naturalWidth;
      natH = im.naturalHeight;
    }
    const on = () => measure();
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  });

  // 交互定义：点击图片之外的空白区域或「Esc」关闭图库；点击图片切换缩放。
  // 不放置右上角 ×：该位置被全局窗口控件（WindowControls，z-index 100000）覆盖，会误触关闭软件。
</script>

<svelte:window onkeydown={onKey} />

<!-- 阻止事件冒泡到阅读区：不触发翻页/区域动作/选区 -->
<div
  class="iv-backdrop"
  out:outDissolve
  role="dialog"
  aria-label={t("common.imgViewer.title")}
  tabindex="-1"
  onpointerdown={(e) => e.stopPropagation()}
  onwheel={onWheel}
  oncontextmenu={(e) => e.preventDefault()}
>
  <div class="iv-top">
    <span class="iv-meta" title={current?.chapter_title || ""}>
      {index + 1} / {total}{current?.chapter_title ? " · " + current.chapter_title : ""}
    </span>
  </div>

  <button class="iv-nav left" data-testid="iv-prev" title={t("common.imgViewer.prev")} onclick={prev} disabled={total <= 1}><IconChevronLeft /></button>

  <div
    class="iv-stage"
    class:dragging
    bind:this={stageEl}
    onpointerdown={onStagePointerDown}
    onpointermove={onStagePointerMove}
    onpointerup={onStagePointerUp}
    onpointercancel={onStagePointerUp}
    onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
  >
    {#if current}
      <img
        bind:this={imgEl}
        src={current.url}
        alt=""
        draggable="false"
        onload={onImgLoad}
        style="width:{dispW}px;height:{dispH}px;margin:{mT}px {mR}px;"
        onclick={onImgClick}
      />
    {:else}
      <div class="iv-empty" data-testid="iv-empty">{t("common.imgViewer.empty")}</div>
    {/if}
  </div>

  <button class="iv-nav right" data-testid="iv-next" title={t("common.imgViewer.next")} onclick={next} disabled={total <= 1}><IconChevronRight /></button>

  <div class="iv-zoom" role="group" aria-label={t("common.imgViewer.zoomPct")}>
    <button class="iv-zbtn" data-testid="iv-zoom-out" title={t("common.imgViewer.zoomOut")} onclick={() => zoomTo(scale / ZOOM_STEP)}>−</button>
    <span class="iv-zpct" data-testid="iv-zoom-pct">{pct}%</span>
    <button class="iv-zbtn" data-testid="iv-zoom-in" title={t("common.imgViewer.zoomIn")} onclick={() => zoomTo(scale * ZOOM_STEP)}>+</button>
    <button class="iv-zbtn iv-fit" data-testid="iv-fit" title={t("common.imgViewer.fit")} onclick={() => { if (scale > 1) lastZoom = scale; zoomTo(1); }}>{t("common.imgViewer.fit")}</button>
  </div>

  <div class="iv-hint">{t("common.imgViewer.hint")}</div>
</div>

<style>
  /* 灯箱：刻意不用 --scrim，保留近黑的深遮罩（看图时背景必须彻底退场） */
  .iv-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: rgba(0, 0, 0, 0.86);
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
    animation: fr-fade-in var(--dur-scrim) var(--ease-in) both;
  }

  /* 内边距改由图片 margin 表达（见 mT/mR）：内容原点 = 舞台原点，锚点缩放才不跑偏。
     display:block 而非 flex —— flex 居中在内容超出时会把左侧裁掉且滚动不到。 */
  .iv-stage {
    position: absolute;
    inset: 0;
    display: block;
    overflow: auto;
    box-sizing: border-box;
    overscroll-behavior: contain;
  }

  .iv-stage.dragging { cursor: grabbing; }

  .iv-stage img {
    display: block;
    max-width: none;
    max-height: none;
    object-fit: contain;
    cursor: zoom-in;
    user-select: none;
    border-radius: var(--radius-xs);
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4), 0 24px 70px rgba(0, 0, 0, 0.5);
  }

  .iv-top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 48px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px;
    color: #eee;
    background: linear-gradient(to bottom, rgba(0, 0, 0, 0.55), transparent);
    z-index: var(--z-raised);
  }

  .iv-meta {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    opacity: 0.9;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 70%;
  }

  .iv-nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    z-index: var(--z-raised);
    width: 46px;
    height: 76px;
    border: none;
    background: rgba(255, 255, 255, 0.1);
    color: #fff;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .iv-nav:hover:not(:disabled) { background: rgba(255, 255, 255, 0.24); }
  .iv-nav:disabled { opacity: 0.25; cursor: default; }
  /* 抵消 app.css 全局 button:active{transform:translateY(1px)}：否则按下时 -50% 居中位移
     被覆盖，图标会向下跳动半个高度。这里用更高特异度把居中位移固定住。 */
  .iv-nav:active:not(:disabled) { transform: translateY(-50%); }
  .iv-nav.left { left: 12px; }
  .iv-nav.right { right: 12px; }

  .iv-zoom {
    position: absolute;
    bottom: 34px;
    left: 50%;
    transform: translateX(-50%);
    z-index: var(--z-raised);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    border-radius: var(--radius-md);
    background: rgba(0, 0, 0, 0.5);
    color: #fff;
  }
  .iv-zbtn {
    min-width: 30px;
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-xs);
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .iv-zbtn:hover { background: rgba(255, 255, 255, 0.26); }
  .iv-fit { font-size: var(--fs-xs); }
  .iv-zpct {
    min-width: 52px;
    text-align: center;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.9;
  }

  .iv-hint {
    position: absolute;
    bottom: 12px;
    left: 0;
    right: 0;
    text-align: center;
    color: rgba(255, 255, 255, 0.55);
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    z-index: var(--z-raised);
    pointer-events: none;
  }

  .iv-empty { color: rgba(255, 255, 255, 0.6); font-size: var(--fs-md); }
</style>
