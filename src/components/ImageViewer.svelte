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

  let zoomed = $state(false);
  let wheelLock = 0;

  const total = $derived(images.length);
  const current = $derived(images[Math.max(0, Math.min(index, total - 1))] ?? null);

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

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    e.stopPropagation();
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
    }
  }

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

  <div class="iv-stage" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
    {#if current}
      <img
        class:zoomed
        src={current.url}
        alt=""
        draggable="false"
        onclick={(e) => { e.stopPropagation(); zoomed = !zoomed; }}
      />
    {:else}
      <div class="iv-empty" data-testid="iv-empty">{t("common.imgViewer.empty")}</div>
    {/if}
  </div>

  <button class="iv-nav right" data-testid="iv-next" title={t("common.imgViewer.next")} onclick={next} disabled={total <= 1}><IconChevronRight /></button>

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

 .iv-stage {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: auto;
    padding: 56px 72px;
    box-sizing: border-box;
  }

  .iv-stage img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    cursor: zoom-in;
    user-select: none;
    border-radius: var(--radius-xs);
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4), 0 24px 70px rgba(0, 0, 0, 0.5);
  }

  .iv-stage img.zoomed {
    max-width: none;
    max-height: none;
    cursor: zoom-out;
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
