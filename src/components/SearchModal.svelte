<script lang="ts">
  import { errText, t } from "../lib/i18n/index.svelte";
  import { api } from "../lib/api";
  import { pendingOpen, ui, toast } from "../lib/stores.svelte";
  import type { SearchHit } from "../lib/types";
  import { displayChapterTitle } from "../lib/chapterTitle";

  let query = $state("");
  let hits: SearchHit[] = $state([]);
  let searching = $state(false);
  let searched = $state(false);
  let indexBtn = $state("");
  /** 本次弹窗是否已为当前书触发过索引（组件随弹窗挂载，重开即复位）。 */
  let indexRequested = false;

  /** 当前书在阅读中/关书时不再自动建索引（P9）：用户在搜索弹窗里搜一次，就按需建一次
   *  （auto=false，不受体积/章数上限约束，大书也能建）。失败静默：搜索本身不依赖这一步。 */
  function ensureCurrentBookIndex() {
    const id = ui.currentBookId;
    if (!id || indexRequested) return;
    indexRequested = true;
    api
      .indexBookFts(id, false)
      .then((n) => {
        if (n > 0) toast(t("search.indexBuilt", { n }));
      })
      .catch(() => {});
  }

  async function doSearch() {
    ensureCurrentBookIndex();
    if (query.trim().length < 2) return;
    searching = true;
    try {
      hits = await api.searchAll(query.trim());
      searched = true;
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      searching = false;
    }
  }

  async function rebuild() {
    indexBtn = t("search.rebuilding");
    try {
      // 后端 rebuild_fts 已返回全库重建的总章数；再逐本调 index_book_fts 会被单实例闸门
      // 全部挡成 0（总数显示成 0），所以直接用返回值。
      const n = await api.rebuildFts();
      toast(t("search.rebuildDone", { n }));
    } catch (e) {
      toast(errText(e), "error");
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
      window.dispatchEvent(new CustomEvent("tidepage-jump", { detail: h.chapter_index }));
    }, 1200);
  }
</script>

<div class="modal-body">
  <h3>{t("search.title")}</h3>
  <div class="row-inline" style="margin-bottom:12px">
    <input
      data-testid="search-input"
      placeholder={t("search.placeholder")}
      bind:value={query}
      onkeydown={(e) => e.key === "Enter" && doSearch()}
      style="flex:1"
    />
    <button class="primary" data-testid="search-go" onclick={doSearch} disabled={searching}>{t("common.action.search")}</button>
    <button data-testid="search-rebuild" onclick={rebuild} disabled={!!indexBtn}>{indexBtn || t("search.rebuild")}</button>
  </div>

  {#if searched && !hits.length}
    <div class="empty-state">{t("search.empty")}</div>
  {:else}
    <div class="hits">
      {#each hits as h (h.book_id + ":" + h.chapter_index + h.snippet)}
        <div class="hit" onclick={() => openHit(h)}>
          <div class="hb">{h.book_title} <span class="hc">· {displayChapterTitle(h.chapter_title, h.chapter_index, (n) => t("common.sectionN", { n }))}</span></div>
          <div class="hs">{@html h.snippet.replaceAll('<mark>', '<mark class="mk">')}</div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="close-row">
    <button data-testid="search-close" onclick={() => (ui.searchOpen = false)}>{t("common.action.close")}</button>
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
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease);
  }

  .hit:hover {
    background: var(--panel2);
  }

  .hb {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    font-weight: 600;
  }

  .hc {
    font-weight: 400;
    color: var(--fg2);
  }

  .hs {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    color: var(--fg2);
    margin-top: 2px;
    line-height: 1.5;
  }

  /* 命中片段底色：以 --warning 为色相做浅底，跟随皮肤而非固定黄（与书内搜索一致） */
  .hs :global(mark.mk) {
    background: color-mix(in srgb, var(--warning) 32%, var(--bg-elev));
    color: inherit;
    padding: 0 2px;
    border-radius: var(--radius-xs);
  }

  .close-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 10px;
  }
</style>
