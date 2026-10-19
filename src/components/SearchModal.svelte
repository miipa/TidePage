<script lang="ts">
  import { errText, t } from "../lib/i18n/index.svelte";
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
      toast(errText(e), "error");
    } finally {
      searching = false;
    }
  }

  async function rebuild() {
    indexBtn = t("search.rebuilding");
    try {
      await api.rebuildFts();
      toast(t("search.rebuildStarted"));
      let n = 0;
      // 分批按 id 遍历全库（每批 500）：listBooks 会把 17 万条整行物化并一次性序列化，
      // 一次 IPC 就是几十 MB 的行对象 + JSON 文本；这里每批只有 500 个 i64，内存与书数无关。
      const ID_BATCH = 500;
      for (let offset = 0; ; offset += ID_BATCH) {
        const ids = await api.listBookIdsPage(offset, ID_BATCH);
        if (!ids.length) break;
        for (const id of ids) {
          try {
            n += await api.indexBookFts(id);
          } catch { /* skip */ }
        }
      }
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
          <div class="hb">{h.book_title} <span class="hc">· {h.chapter_title || t("common.chapterN", { n: h.chapter_index + 1 })}</span></div>
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
