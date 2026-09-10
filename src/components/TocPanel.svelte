<script lang="ts">
  import type { TocEntry } from "../lib/types";

  let { toc, current, onpick, onclose }: {
    toc: TocEntry[];
    current: number;
    onpick: (index: number) => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let listEl: HTMLDivElement | undefined = $state();

  const filtered = $derived(
    query ? toc.filter((t) => t.title.toLowerCase().includes(query.toLowerCase())) : toc
  );

  $effect(() => {
    if (listEl) {
      const el = listEl.querySelector(".toc-item.current");
      el?.scrollIntoView({ block: "center" });
    }
  });
</script>

<div class="sidepanel">
  <div class="sp-head">
    <span>目录</span>
    <button class="icon" onclick={onclose}>×</button>
  </div>
  <div class="sp-search">
    <input placeholder="搜索章节…" bind:value={query} />
  </div>
  <div class="sp-list" bind:this={listEl}>
    {#each filtered as t (t.index)}
      <button class="toc-item" class:current={t.index === current} onclick={() => onpick(t.index)}>
        {t.title}
      </button>
    {:else}
      <div class="sp-empty">无匹配章节</div>
    {/each}
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
    box-shadow: 8px 0 24px rgba(0, 0, 0, 0.1);
    animation: slide-in 0.18s ease;
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
  }

  .toc-item {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 8px;
    padding: 8px 12px;
    cursor: pointer;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
