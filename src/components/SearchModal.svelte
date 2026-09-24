<script lang="ts">
  import { api } from "../lib/api";
  import { pendingOpen, ui, toast } from "../lib/stores.svelte";
  import type { SearchHit } from "../lib/types";

  let query = $state("");
  let hits: SearchHit[] = $state([]);
  let searching = $state(false);
  let searched = $state(false);
  let indexBtn = $state("");

  async function doSearch() {
    if (query.trim().length < 2) return;
    searching = true;
    try {
      hits = await api.searchAll(query.trim());
      searched = true;
    } catch (e) {
      toast(String(e), "error");
    } finally {
      searching = false;
    }
  }

  async function rebuild() {
    indexBtn = "重建中…";
    try {
      await api.rebuildFts();
      toast("索引已清空，现在开始重新索引全部书籍，后台进行");
      const books = await api.listBooks();
      let n = 0;
      for (const b of books) {
        try {
          const c = await api.indexBookFts(b.id);
          n += c;
        } catch { /* skip */ }
      }
      toast(`全文索引完成，共索引 ${n} 章`);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      indexBtn = "";
    }
  }

  function openHit(h: SearchHit) {
    pendingOpen.bookId = h.book_id;
    pendingOpen.nonce++;
    ui.view = "reader";
    ui.searchOpen = false;
    setTimeout(() => {
      window.dispatchEvent(new CustomEvent("flowread-jump", { detail: h.chapter_index }));
    }, 1200);
  }
</script>

<div class="modal-body">
  <h3>全文搜索</h3>
  <div class="row-inline" style="margin-bottom:12px">
    <input
      placeholder="输入关键词，至少 2 字…"
      bind:value={query}
      onkeydown={(e) => e.key === "Enter" && doSearch()}
      style="flex:1"
    />
    <button class="primary" onclick={doSearch} disabled={searching}>搜索</button>
    <button onclick={rebuild} disabled={!!indexBtn}>{indexBtn || "重建索引"}</button>
  </div>

  {#if searched && !hits.length}
    <div class="empty">无结果。若书籍很多，先点「重建索引」。</div>
  {:else}
    <div class="hits">
      {#each hits as h (h.book_id + ":" + h.chapter_index + h.snippet)}
        <div class="hit" onclick={() => openHit(h)}>
          <div class="hb">{h.book_title} <span class="hc">· {h.chapter_title || h.chapter_index + 1 + " 章"}</span></div>
          <div class="hs">{@html h.snippet.replaceAll('<mark>', '<mark class="mk">')}</div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="close-row">
    <button onclick={() => (ui.searchOpen = false)}>关闭</button>
  </div>
</div>

<style>
  .hits {
    max-height: 55vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .hit {
    padding: 8px 12px;
    border-radius: 8px;
    cursor: pointer;
  }

  .hit:hover {
    background: var(--panel2);
  }

  .hb {
    font-size: 13px;
    font-weight: 600;
  }

  .hc {
    font-weight: 400;
    color: var(--fg2);
  }

  .hs {
    font-size: 13px;
    color: var(--fg2);
    margin-top: 2px;
    line-height: 1.5;
  }

  .hs :global(mark.mk) {
    background: #ffe28a;
    color: inherit;
    padding: 0 2px;
    border-radius: 2px;
  }

  .empty {
    padding: 30px;
    text-align: center;
    color: var(--fg2);
  }

  .close-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 10px;
  }
</style>
