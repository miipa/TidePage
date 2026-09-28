<script lang="ts">
  import type { TocEntry } from "../lib/types";
  import { dragOutHandle } from "../lib/windowSnap";

  let { toc, current, onpick, onclose, onpopout }: {
    toc: TocEntry[];
    current: number;
    onpick: (index: number) => void;
    onclose: () => void;
    onpopout?: () => void;
  } = $props();

  let query = $state("");
  let listEl: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(600);

  // 固定行高 + 虚拟滚动：超大目录（如 7000+ 章）只渲染可视区，避免一次性建数万个 DOM 节点卡死。
  const ROW_H = 34;
  const OVERSCAN = 6;

  // 保留每一项在 toc 数组中的原始下标（pos）。章节导航（loadChapter / get_chapter）一律以
  // 「toc 数组下标」为准；TocEntry.index 是 spine 下标（EPUB 中常与数组下标不一致），
  // 不能拿它当章节号，否则会跳到错章、甚至越界渲染出空白章。
  const indexed = $derived(toc.map((t, pos) => ({ t, pos })));
  const filtered = $derived(
    query
      ? indexed.filter(({ t }) => t.title.toLowerCase().includes(query.toLowerCase()))
      : indexed
  );

  const total = $derived(filtered.length);
  const firstIdx = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const lastIdx = $derived(Math.min(total, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN));
  const visible = $derived(filtered.slice(firstIdx, lastIdx));

  let scrollRaf = 0;
  function onScroll(e: Event) {
    const el = e.currentTarget as HTMLDivElement;
    const top = el.scrollTop;
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      scrollTop = top;
    });
  }

  $effect(() => {
    if (!listEl) return;
    viewH = listEl.clientHeight || 600;
  });

  // 打开目录时滚动定位到当前章（用虚拟滚动计算目标位置并夹取）。
  $effect(() => {
    if (!listEl) return;
    const i = filtered.findIndex((x) => x.pos === current);
    if (i >= 0) {
      listEl.scrollTop = Math.max(0, i * ROW_H - viewH / 2 + ROW_H / 2);
      scrollTop = listEl.scrollTop;
    }
    // 仅在切换当前章/首次挂载时定位；输入搜索时也重新定位
  });
</script>

<div class="sidepanel">
  <div class="sp-head" use:dragOutHandle={onpopout} title="按住标题栏拖出面板外可弹为独立窗口">
    <span>目录</span>
    <div class="sp-head-acts">
      {#if onpopout}<button class="icon" title="弹出为独立窗口" onclick={onpopout}>⧉</button>{/if}
      <button class="icon" title="关闭" onclick={onclose}>×</button>
    </div>
  </div>
  <div class="sp-search">
    <input placeholder="搜索章节…" bind:value={query} />
  </div>
  <div class="sp-list" bind:this={listEl} onscroll={onScroll}>
    {#if total === 0}
      <div class="sp-empty">无匹配章节</div>
    {:else}
      <div class="sp-inner" style="height:{total * ROW_H}px;">
        {#each visible as item, vi (item.pos)}
          {@const i = firstIdx + vi}
          <button
            class="toc-item"
            class:current={item.pos === current}
            style="top:{i * ROW_H}px; height:{ROW_H}px;"
            onclick={() => onpick(item.pos)}
          >
            {item.t.title}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .sidepanel {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 320px;
    background: color-mix(in srgb, var(--rdr-bg) 94%, black 6%);
    border-right: 1px solid rgba(128, 128, 128, 0.2);
    z-index: 40;
    display: flex;
    flex-direction: column;
    box-shadow: 6px 0 18px rgba(0, 0, 0, 0.08), 18px 0 48px rgba(0, 0, 0, 0.14);
    animation: slide-in 0.34s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes slide-in {
    from { transform: translateX(-24px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .sp-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    font-weight: 600;
  }

  .sp-head-acts { display: flex; align-items: center; gap: 6px; }
  .sp-head-acts .icon { font-weight: 400; }

  .sp-search {
    padding: 0 12px 10px;
  }

  .sp-search input {
    width: 100%;
  }

  .sp-list {
    flex: 1;
    overflow-y: auto;
    padding: 0 6px 10px;
    position: relative;
  }

  .sp-inner {
    position: relative;
    width: 100%;
  }

  .toc-item {
    position: absolute;
    left: 0;
    right: 0;
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 8px;
    padding: 0 12px;
    cursor: pointer;
    font-size: 13px;
    line-height: 34px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-sizing: border-box;
  }

  .toc-item:hover {
    background: rgba(128, 128, 128, 0.12);
  }

  .toc-item.current {
    color: var(--accent);
    font-weight: 600;
    background: rgba(128, 128, 128, 0.08);
  }

  .sp-empty {
    padding: 24px;
    text-align: center;
    opacity: 0.5;
  }
</style>
