<script lang="ts">
  import { api } from "../lib/api";
  import { HIGHLIGHT_STYLES, toast } from "../lib/stores.svelte";
  import type { HighlightRow } from "../lib/types";

  let {
    bookId,
    version = 0,
    onjump,
    onclose,
    onchanged,
  }: {
    bookId: number;
    version?: number;
    onjump: (h: HighlightRow) => void;
    onclose: () => void;
    onchanged: () => void;
  } = $props();

  let query = $state("");
  let rows: HighlightRow[] = $state([]);
  let loading = $state(false);

  async function load() {
    loading = true;
    try {
      rows = query.trim()
        ? await api.searchHighlights(query.trim())
        : await api.listHighlights(bookId);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void version;
    void query;
    const t = setTimeout(load, 200);
    return () => clearTimeout(t);
  });

  async function del(h: HighlightRow) {
    try {
      await api.deleteHighlight(h.id);
      rows = rows.filter((r) => r.id !== h.id);
      onchanged();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function changeStyle(h: HighlightRow, style: string) {
    try {
      await api.updateHighlight(h.id, style, h.note);
      h.style = style;
      onchanged();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function when(ts: number): string {
    try {
      return new Date(ts).toLocaleString();
    } catch {
      return "";
    }
  }
</script>

<div class="sidepanel">
  <div class="sp-head">
    <span>划线 / 批注</span>
    <button class="icon" onclick={onclose}>×</button>
  </div>
  <div class="sp-search">
    <input
      placeholder="搜索划线（全库：正文/书名/章节）…"
      bind:value={query}
    />
  </div>
  <div class="sp-list">
    {#if loading && !rows.length}
      <div class="sp-empty">加载中…</div>
    {/if}
    {#each rows as h (h.id)}
      <div class="sp-item hl-item">
        <div class="hl-top" onclick={() => onjump(h)} title="点击跳转到该划线位置">
          <div class="bm-title">
            {h.chapter_title || `第 ${h.chapter_index + 1} 节`}
            {#if query.trim()}<span class="hl-book">· {h.book_title}</span>{/if}
          </div>
          <div class="bm-excerpt hl-text">{h.text}</div>
          {#if h.note}<div class="hl-note">{h.note}</div>{/if}
          <div class="hl-time">{when(h.created_at)}</div>
        </div>
        <div class="hl-acts">
          <select
            title="修改划线样式"
            value={h.style}
            onchange={(e) => changeStyle(h, (e.currentTarget as HTMLSelectElement).value)}
          >
            {#each HIGHLIGHT_STYLES as s (s.id)}
              <option value={s.id}>{s.name}</option>
            {/each}
          </select>
          <button class="icon" title="删除划线" onclick={() => del(h)}>×</button>
        </div>
      </div>
    {:else}
      {#if !loading}
        <div class="sp-empty">暂无划线，选中正文文字即可划线。</div>
      {/if}
    {/each}
  </div>
</div>

<style>
  .sidepanel {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 340px;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 95%, black 5%);
    border-left: 1px solid rgba(128, 128, 128, 0.2);
    z-index: 40;
    display: flex;
    flex-direction: column;
    box-shadow: -8px 0 24px rgba(0, 0, 0, 0.1);
    animation: slide-in 0.18s ease;
  }

  @keyframes slide-in {
    from { transform: translateX(24px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .sp-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    font-weight: 600;
    border-bottom: 1px solid rgba(128, 128, 128, 0.15);
  }

  .sp-search {
    padding: 8px 12px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.1);
  }

  .sp-search input { width: 100%; box-sizing: border-box; }

  .sp-list { flex: 1; overflow-y: auto; }

  .sp-item { padding: 10px 14px; border-bottom: 1px solid rgba(128, 128, 128, 0.08); }

  .hl-top { cursor: pointer; }

  .hl-top:hover .bm-title { text-decoration: underline; }

  .bm-title { font-size: 13px; font-weight: 500; }
  .hl-book { font-weight: 400; opacity: 0.65; }
  .bm-excerpt {
    font-size: 12.5px;
    opacity: 0.78;
    margin-top: 3px;
    line-height: 1.55;
  }
  .hl-text {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .hl-note { font-size: 12px; opacity: 0.6; margin-top: 3px; font-style: italic; }
  .hl-time { font-size: 11px; opacity: 0.45; margin-top: 3px; }

  .hl-acts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }

  .hl-acts select { font-size: 12px; padding: 2px 6px; }
  .sp-empty { padding: 24px; text-align: center; opacity: 0.5; font-size: 13px; }
</style>
