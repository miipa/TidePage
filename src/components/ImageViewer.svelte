<script lang="ts">
  import type { BookImage } from "../lib/types";

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

  // 交互定义：仅「右上角 ×」与「Esc」关闭图库；点击图片切换缩放；
  // 点击图片四周留白（含图片上方）不关闭，避免误退出。
</script>

<svelte:window onkeydown={onKey} />

<!-- 阻止事件冒泡到阅读区：不触发翻页/区域动作/选区 -->
<div
  class="iv-backdrop"
  role="dialog"
  aria-label="图片查看器"
  tabindex="-1"
  onpointerdown={(e) => e.stopPropagation()}
  onwheel={onWheel}
  oncontextmenu={(e) => e.preventDefault()}
>
  <div class="iv-top">
    <span class="iv-meta" title={current?.chapter_title || ""}>
      {index + 1} / {total}{current?.chapter_title ? " · " + current.chapter_title : ""}
    </span>
    <div class="iv-actions">
      <button class="iv-btn" title="适应/原始大小" onclick={() => (zoomed = !zoomed)}>{zoomed ? "适应" : "放大"}</button>
      <button class="iv-btn" title="关闭 (Esc)" onclick={onclose}>×</button>
    </div>
  </div>

  <button class="iv-nav left" title="上一张 (←)" onclick={prev} disabled={total <= 1}>‹</button>

  <div class="iv-stage">
    {#if current}
      <img
        class:zoomed
        src={current.url}
        alt=""
        draggable="false"
        onclick={(e) => { e.stopPropagation(); zoomed = !zoomed; }}
      />
    {:else}
      <div class="iv-empty">本书未找到图片</div>
    {/if}
  </div>

  <button class="iv-nav right" title="下一张 (→)" onclick={next} disabled={total <= 1}>›</button>

  <div class="iv-hint">滚轮 / ← → 切换，点击图片缩放，× 或 Esc 关闭</div>
</div>

<style>
  .iv-backdrop {
    position: fixed;
    inset: 0;
    z-index: 2000;
    background: rgba(0, 0, 0, 0.86);
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
    animation: iv-in 0.14s ease;
  }

  @keyframes iv-in {
    from { opacity: 0; }
    to { opacity: 1; }
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
    border-radius: 4px;
    box-shadow: 0 8px 40px rgba(0, 0, 0, 0.5);
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
    z-index: 2;
  }

  .iv-meta {
    font-size: 13px;
    opacity: 0.9;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 70%;
  }

  .iv-actions { display: flex; gap: 8px; }

  .iv-btn {
    background: rgba(255, 255, 255, 0.12);
    border: none;
    color: #fff;
    font-size: 13px;
    padding: 5px 12px;
    border-radius: 8px;
    cursor: pointer;
  }

  .iv-btn:hover { background: rgba(255, 255, 255, 0.22); }

  .iv-nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    z-index: 2;
    width: 46px;
    height: 76px;
    border: none;
    background: rgba(255, 255, 255, 0.1);
    color: #fff;
    font-size: 30px;
    line-height: 1;
    border-radius: 10px;
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
    font-size: 12px;
    z-index: 2;
    pointer-events: none;
  }

  .iv-empty { color: rgba(255, 255, 255, 0.6); font-size: 14px; }
</style>
