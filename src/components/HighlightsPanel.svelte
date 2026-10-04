<script lang="ts">
  import { dateLocale, t } from "../lib/i18n/index.svelte";
import { IconArrowsSplit2, IconTrash, IconX } from "@tabler/icons-svelte";
  import { api } from "../lib/api";
  import { HIGHLIGHT_STYLES, appConfirm, config, saveConfig, toast } from "../lib/stores.svelte";
  import { labelOfId } from "../lib/i18n/index.svelte";
  import { dragOutHandle } from "../lib/windowSnap";
  import type { HighlightRow } from "../lib/types";
  import ColorPalette from "./ColorPalette.svelte";

  let {
    bookId,
    version = 0,
    onjump,
    onclose,
    onchanged,
    onpopout,
  }: {
    bookId: number;
    version?: number;
    onjump: (h: HighlightRow) => void;
    onclose?: () => void;
    onchanged: () => void;
    onpopout?: () => void;
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
    const ok = await appConfirm(t("highlight.deleteConfirm", { text: h.text.slice(0, 60) }));
    if (!ok) return;
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
      await api.updateHighlight(h.id, style, h.color, h.note);
      h.style = style;
      onchanged();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function changeColor(h: HighlightRow, color: string) {
    try {
      await api.updateHighlight(h.id, h.style, color, h.note);
      h.color = color;
      onchanged();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function when(ts: number): string {
    try {
      return new Date(ts).toLocaleString(dateLocale());
    } catch {
      return "";
    }
  }
</script>

<div class="sidepanel">
  <div class="sp-head" data-testid="hlp-head" use:dragOutHandle={onpopout} title={t("common.panel.popoutTip")}>
    <span>{t("highlight.panelTitle")}</span>
    <div style="display:flex; gap:6px; align-items:center;">
      {#if onpopout}<button class="icon" data-testid="hlp-popout" title={t("common.panel.popout")} onclick={onpopout}><IconArrowsSplit2 /></button>{/if}
      {#if onclose}<button class="icon" data-testid="hlp-close" title={t("common.action.close")} onclick={onclose}><IconX /></button>{/if}
    </div>
  </div>
  <!-- 搜索框 + 清除键：包裹层只提供定位参照与右侧留白（padding-right 契约与书库
     .search-wrap 同一个，见 app.css 的 .fr-clear 小节），× 的规格全走公共类 .fr-clear。
     .sp-search 的 border-bottom 留在外层，没挪到包裹层上。 -->
  <div class="sp-search">
    <span class="hlp-search-wrap">
      <input
        data-testid="hlp-search"
        placeholder={t("highlight.searchPlaceholder")}
        bind:value={query}
      />
      {#if query}
        <button
          class="fr-clear"
          data-testid="hlp-search-clear"
          title={t("common.action.clear")}
          aria-label={t("library.toolbar.clearSearchAria")}
          onclick={() => (query = "")}><IconX size={14} stroke={1.5} /></button>
      {/if}
    </span>
  </div>
  <div class="sp-list">
    {#if loading && !rows.length}
      <div class="sp-empty">{t("common.loading")}</div>
    {/if}
    {#each rows as h (h.id)}
      <div class="sp-item hl-item">
        <div class="hl-top" onclick={() => onjump(h)} title={t("highlight.jumpTip")}>
          <div class="bm-title">
            {h.chapter_title || t("common.sectionN", { n: h.chapter_index + 1 })}
            {#if query.trim()}<span class="hl-book">· {h.book_title}</span>{/if}
          </div>
          <div class="bm-excerpt hl-text">{h.text}</div>
          {#if h.note}<div class="hl-note">{h.note}</div>{/if}
          <div class="hl-time">{when(h.created_at)}</div>
        </div>
        <div class="hl-acts">
          <select
            data-testid="hlp-style"
            title={t("highlight.styleTitle")}
            value={h.style}
            onchange={(e) => changeStyle(h, (e.currentTarget as HTMLSelectElement).value)}
          >
            {#each HIGHLIGHT_STYLES as s (s.id)}
              <option value={s.id}>{labelOfId("hlStyle", s.id)}</option>
            {/each}
          </select>
          <ColorPalette
            bind:colors={config.reader.highlightColors}
            value={h.color}
            applyLabel={t("common.action.apply")}
            onapply={(c) => changeColor(h, c)}
            onpalette={saveConfig}
          />
          <span class="flex1"></span>
          <button class="icon" data-testid="hlp-del" title={t("highlight.deleteTip")} onclick={() => del(h)}><IconTrash /></button>
        </div>
      </div>
    {:else}
      {#if !loading}
        <div class="sp-empty">{t("highlight.empty")}</div>
      {/if}
    {/each}
  </div>
</div>

<style>
  /* .sidepanel / .sp-head / .sp-list / .sp-item / .sp-empty 走 app.css 的全局基类。 */
  .sp-search {
    padding: var(--sp-2) var(--sp-3);
    border-bottom: 1px solid var(--rdr-line);
  }

  /* 清除键的包裹层：position 给绝对定位的 × 提供参照（.fr-clear 容器契约第一条），
     宽度接住原来 input 直接挂在 .sp-search 下时的 100%。 */
  .hlp-search-wrap { display: block; width: 100%; position: relative; }

  /* padding-right = 键宽 20 + 键的 inset 4 + 余量 2，与书库 .search-wrap .search 同一契约 */
  .sp-search input { width: 100%; box-sizing: border-box; padding-right: 26px; }

  .hl-top { cursor: pointer; }

  .hl-top .bm-title { transition: text-decoration-color var(--dur-fast) var(--ease); }
  .hl-top:hover .bm-title { text-decoration: underline; }

  .bm-title { font-size: var(--fs-sm); letter-spacing: var(--ls-sm); font-weight: 500; }
  .hl-book { font-weight: 400; opacity: 0.65; }
  .bm-excerpt {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
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
  .hl-note { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); opacity: 0.6; margin-top: 3px; font-style: italic; }
  .hl-time {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.45;
    margin-top: 3px;
    font-variant-numeric: tabular-nums;
  }

  .hl-acts {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    margin-top: var(--sp-1);
    flex-wrap: wrap;
  }

  .hl-acts select { font-size: var(--fs-xs); padding: 2px var(--sp-1); }
  .flex1 { flex: 1; }
</style>
