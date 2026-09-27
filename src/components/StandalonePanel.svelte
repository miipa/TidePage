<!-- 独立面板窗口容器：由 +page.svelte 在 ?panel=xxx&book=<id> 时挂载。
     数据源约定（任务 5）：
       toc / bookmarks / stats → ① 按 URL 的 book id 首屏自拉现有 api（openBook/listBookmarks/BookStatsModal 自载）；
       highlights           → ①首屏自拉 + ② 订阅主窗广播 flowread-hl-changed 实时刷新；
       notes                → NotesPanel 组件本体自载（① 自拉，未改其本体）。
     跳转类操作通过 Tauri event 广播回主窗（flowread-goto-chapter / flowread-goto-highlight）。
     吸附：本窗作为卫星窗 startSnap()，onDestroy 时 stopSnap() 清理监听与吸附组引用。 -->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api } from "../lib/api";
  import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { TocEntry, HighlightRow, BookmarkRow } from "../lib/types";
  import TocPanel from "./TocPanel.svelte";
  import HighlightsPanel from "./HighlightsPanel.svelte";
  import BookStatsModal from "./BookStatsModal.svelte";
  import NotesPanel from "./NotesPanel.svelte";

  const params = new URLSearchParams(window.location.search);
  const panel = params.get("panel") || "";
  const bookId = Number(params.get("book") || "0");

  let title = $state("");
  let toc: TocEntry[] = $state([]);
  let curChapter = $state(0);
  let bookmarks: BookmarkRow[] = $state([]);
  let hlVersion = $state(0); // ② 实时刷新：主窗改动 → 本地 version++ 触发 HighlightsPanel 重载

  let unHl: UnlistenFn | null = null;

  async function closeWin() {
    try {
      const m = await import("@tauri-apps/api/window");
      await m.getCurrentWindow().close();
    } catch {
      try { window.close(); } catch { /* ignore */ }
    }
  }

  async function loadTitle() {
    try {
      const rows = await api.listBooksByIds([bookId]);
      title = rows?.[0]?.title ?? "";
    } catch { /* ignore */ }
  }

  onMount(async () => {
    if (panel === "toc") {
      try {
        const r = await api.openBook(bookId);
        toc = r.toc ?? [];
        title = r.book?.title ?? "";
        const p = await api.loadProgress(bookId);
        curChapter = p?.chapter_index ?? 0;
      } catch { /* ignore */ }
    } else if (panel === "bookmarks") {
      await loadTitle();
      try { bookmarks = await api.listBookmarks(bookId); } catch { /* ignore */ }
    } else if (panel === "highlights") {
      await loadTitle();
      unHl = await listen("flowread-hl-changed", () => { hlVersion++; }); // ② 订阅主窗广播
    } else if (panel === "stats") {
      await loadTitle();
    }
  });

  onDestroy(() => {
    unHl?.();
  });

  function gotoChapter(index: number) {
    void emit("flowread-goto-chapter", { index });
  }
  function gotoHighlight(h: HighlightRow) {
    void emit("flowread-goto-highlight", { id: h.id });
  }
</script>

<div class="panel-window-root">
  {#if panel === "toc"}
    <TocPanel {toc} current={curChapter} onpick={(i) => gotoChapter(i)} onclose={closeWin} />
  {:else if panel === "bookmarks"}
    <div class="sidepanel">
      <div class="sp-head"><span>书签{title ? ` · ${title}` : ""}</span><button class="icon" title="关闭" onclick={closeWin}>×</button></div>
      <div class="sp-list">
        {#each bookmarks as bm (bm.id)}
          <div class="sp-item bm-item">
            <div class="bm-main" onclick={() => gotoChapter(bm.chapter_index)}>
              <div class="bm-title">{bm.title || "未命名"}</div>
              {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
            </div>
          </div>
        {:else}
          <div class="sp-empty">暂无书签</div>
        {/each}
      </div>
    </div>
  {:else if panel === "highlights"}
    <HighlightsPanel {bookId} version={hlVersion} onjump={gotoHighlight} onclose={closeWin} onchanged={() => {}} />
  {:else if panel === "stats"}
    <BookStatsModal {bookId} bookTitle={title} onclose={closeWin} />
  {:else if panel === "notes"}
    <NotesPanel />
  {:else}
    <div class="pw-empty">未知面板：{panel}</div>
  {/if}
</div>

<style>
  .panel-window-root {
    height: 100vh;
    background: var(--rdr-bg, #f5f1e8);
    color: var(--rdr-fg, #3b3630);
    position: relative;
    overflow: hidden;
  }

  /* 独立窗内的 sidepanel 铺满整窗（覆盖组件内 position:absolute 的右侧贴边定位） */
  .panel-window-root :global(.sidepanel) {
    position: absolute;
    inset: 0;
    width: 100%;
    border: none;
    box-shadow: none;
    animation: none;
  }

  .sidepanel {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--rdr-bg, #f5f1e8);
  }

  .sp-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    font-weight: 600;
  }

  .sp-list { flex: 1; overflow-y: auto; padding: 0 6px 10px; }

  .sp-item { border-radius: 8px; }
  .bm-item { display: flex; align-items: center; }
  .bm-main { flex: 1; padding: 8px 12px; cursor: pointer; }
  .bm-main:hover { background: rgba(128, 128, 128, 0.12); border-radius: 8px; }
  .bm-title { font-size: 13px; }
  .bm-excerpt { font-size: 12px; opacity: 0.6; margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-empty { padding: 24px; text-align: center; opacity: 0.5; }
  .pw-empty { padding: 40px; text-align: center; opacity: 0.6; }
</style>
