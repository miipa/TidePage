<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { toast } from "../lib/stores.svelte";
  import type { BookSearchHit } from "../lib/types";

  let {
    bookId,
    initialQuery = "",
    x = 0,
    y = 0,
    below = false,
    onpick,
    onclose,
  }: {
    bookId: number;
    initialQuery?: string;
    x?: number;
    y?: number;
    below?: boolean;
    onpick: (chapterIndex: number, query: string) => void;
    onclose: () => void;
  } = $props();

  let query = $state(initialQuery);
  let hits: BookSearchHit[] = $state([]);
  let searching = $state(false);
  let searched = $state(false);
  let indexing = $state(false);
  let popEl: HTMLDivElement | undefined = $state();

  async function run() {
    const q = query.trim();
    if (q.length < 2) return;
    searching = true;
    try {
      hits = await api.searchInBook(bookId, q);
      searched = true;
    } catch (e) {
      toast(String(e), "error");
    } finally {
      searching = false;
    }
  }

  async function buildIndex() {
    indexing = true;
    try {
      await api.indexBookFts(bookId);
      await run();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      indexing = false;
    }
  }

  $effect(() => {
    const h = (e: PointerEvent) => {
      if (popEl && !popEl.contains(e.target as Node)) onclose();
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  onMount(() => {
    run();
  });

  const pos = $derived(
    `left:${x}px; top:${y}px; transform:translate(-50%, ${below ? "0" : "-100%"});`,
  );
</script>

<div class="sb-pop" bind:this={popEl} style={pos}>
  <div class="sb-head">
    <span>全书搜索</span>
    <button class="x" onclick={onclose} title="关闭">×</button>
  </div>
  <div class="sb-row">
    <input
      bind:value={query}
      placeholder="在本书中搜索…"
      onkeydown={(e) => e.key === "Enter" && run()}
    />
    <button class="primary" onclick={run} disabled={searching}>{searching ? "…" : "搜索"}</button>
  </div>
  <div class="sb-list">
    {#if searched && !hits.length}
      <div class="sb-empty">
        本书未找到匹配内容
        <div style="margin-top:8px">
          <button onclick={buildIndex} disabled={indexing}>{indexing ? "建立中…" : "建立/重建索引后重试"}</button>
        </div>
      </div>
    {/if}
    {#each hits as h (h.chapter_index + ":" + h.snippet)}
      <div class="sb-hit" onclick={() => onpick(h.chapter_index, query.trim())}>
        <div class="sb-ch">{h.chapter_title || `第 ${h.chapter_index + 1} 节`}</div>
        <div class="sb-sn">{@html h.snippet.replaceAll("<mark>", '<mark class="mk">')}</div>
      </div>
    {/each}
  </div>
</div>

<style>
  .sb-pop {
    position: fixed;
    z-index: 980;
    width: min(440px, 88vw);
    max-height: min(60vh, 520px);
    display: flex;
    flex-direction: column;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 94%, #202020 6%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 12px;
    box-shadow: 0 12px 34px rgba(0, 0, 0, 0.3);
    overflow: hidden;
  }

  .sb-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    font-size: 13px;
    font-weight: 600;
    border-bottom: 1px solid rgba(128, 128, 128, 0.16);
  }

  .sb-head .x {
    padding: 1px 8px;
    border-radius: 6px;
  }

  .sb-row {
    display: flex;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.12);
  }

  .sb-row input { flex: 1; min-width: 0; }

  .sb-list {
    overflow-y: auto;
    flex: 1;
    padding: 4px;
  }

  .sb-hit {
    padding: 7px 9px;
    border-radius: 8px;
    cursor: pointer;
  }

  .sb-hit:hover { background: rgba(128, 128, 128, 0.14); }

  .sb-ch { font-size: 12.5px; font-weight: 600; }

  .sb-sn {
    font-size: 12px;
    opacity: 0.8;
    line-height: 1.55;
    margin-top: 2px;
    word-break: break-word;
  }

  .sb-sn :global(mark.mk) {
    background: #ffe28a;
    color: inherit;
    padding: 0 2px;
    border-radius: 2px;
  }

  .sb-empty {
    padding: 22px;
    text-align: center;
    font-size: 12.5px;
    opacity: 0.55;
  }
</style>
