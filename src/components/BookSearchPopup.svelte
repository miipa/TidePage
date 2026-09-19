<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { toast } from "../lib/stores.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import type { BookSearchHit } from "../lib/types";

  let {
    bookId,
    initialQuery = "",
    anchor,
    canBack = false,
    onpick,
    onback,
    onclose,
  }: {
    bookId: number;
    initialQuery?: string;
    anchor: PopupAnchor;
    canBack?: boolean;
    onpick: (chapterIndex: number, query: string) => void;
    onback: () => void;
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
      await api.ensureBookText(bookId);
      await run();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      indexing = false;
    }
  }

  let activeIdx = $state(-1);
  let posStyle = $state("visibility:hidden;");

  $effect(() => {
    const h = (e: PointerEvent) => {
      if (popEl && !popEl.contains(e.target as Node)) onclose();
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  // 内容高度随结果变化，需重新测量摆放（顶部/底部自动翻转并夹取到视口内）
  $effect(() => {
    void anchor.top;
    void anchor.bottom;
    void anchor.cx;
    void hits;
    void searching;
    void searched;
    if (!popEl) return;
    requestAnimationFrame(() => {
      if (!popEl) return;
      const p = placePopup(anchor, popEl.offsetWidth, popEl.offsetHeight);
      posStyle = `left:${p.left}px; top:${p.top}px;`;
    });
  });

  onMount(() => {
    run();
  });

  function pick(i: number, h: BookSearchHit) {
    activeIdx = i;
    onpick(h.chapter_index, query.trim());
  }
</script>

<div class="sb-pop" bind:this={popEl} style={posStyle}>
  <div class="sb-head">
    <span>全书搜索</span>
    <div class="sb-head-acts">
      {#if canBack}
        <button class="mini" onclick={onback} title="返回跳转前的位置">↩ 返回</button>
      {/if}
      <button class="x" onclick={onclose} title="关闭">×</button>
    </div>
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
          <button onclick={buildIndex} disabled={indexing}>{indexing ? "载入中…" : "重新载入全文后重试"}</button>
        </div>
      </div>
    {/if}
    {#each hits as h, i (i)}
      <div class="sb-hit" class:active={activeIdx === i} onclick={() => pick(i, h)}>
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

  .sb-head-acts {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .sb-head .x {
    padding: 1px 8px;
    border-radius: 6px;
  }

  .sb-head .mini {
    font-size: 12px;
    padding: 2px 8px;
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
  .sb-hit.active { background: color-mix(in srgb, var(--accent, #4a6b8a) 22%, transparent); }

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
