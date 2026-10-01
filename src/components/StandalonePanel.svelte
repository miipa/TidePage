<!-- 独立面板窗口容器：由 +page.svelte 在 ?panel=xxx&book=<id> 时挂载。
     数据源约定（任务 5）：
       toc / bookmarks / stats → ① 按 URL 的 book id 首屏自拉现有 api（openBook/listBookmarks/BookStatsModal 自载）；
       highlights           → ①首屏自拉 + ② 订阅主窗广播 tidepage-hl-changed 实时刷新；
       notes                → NotesPanel 组件本体自载（① 自拉，未改其本体）。
     跳转类操作通过 Tauri event 广播回主窗（tidepage-goto-chapter / tidepage-goto-highlight）。
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
  let statsVersion = $state(0); // 统计实时刷新：主窗 flushSession → version++ 触发 BookStatsModal 重载

  let unHl: UnlistenFn | null = null;
  let unChap: UnlistenFn | null = null;
  let unBm: UnlistenFn | null = null;
  let unStats: UnlistenFn | null = null;

  async function loadTitle() {
    try {
      const rows = await api.listBooksByIds([bookId]);
      title = rows?.[0]?.title ?? "";
    } catch { /* ignore */ }
  }

  // 本窗顶部标题栏固定 40px：写入全局变量让右上角 WindowControls 高度=40px、按钮垂直居中对齐。
  onMount(() => {
    document.documentElement.style.setProperty("--fr-titlebar-h", "40px");
  });

  onMount(async () => {
    if (panel === "toc") {
      try {
        const r = await api.openBook(bookId);
        toc = r.toc ?? [];
        title = r.book?.title ?? "";
        const p = await api.loadProgress(bookId);
        curChapter = p?.chapter_index ?? 0;
      } catch { /* ignore */ }
      // 订阅主窗“当前章变化”广播：仅本窗对应 book 生效，TocPanel 会据 current 自动定位/高亮。
      unChap = await listen<{ bookId: number; index: number }>("tidepage-chapter-changed", (e) => {
        if (e.payload?.bookId === bookId && typeof e.payload.index === "number") {
          curChapter = e.payload.index;
        }
      });
    } else if (panel === "bookmarks") {
      await loadTitle();
      try { bookmarks = await api.listBookmarks(bookId); } catch { /* ignore */ }
      // 订阅主窗书签增删：仅本窗对应 book 生效，重新拉取列表。
      unBm = await listen<{ bookId: number }>("tidepage-bookmarks-changed", async (e) => {
        if (e.payload?.bookId === bookId) {
          try { bookmarks = await api.listBookmarks(bookId); } catch { /* ignore */ }
        }
      });
    } else if (panel === "highlights") {
      await loadTitle();
      unHl = await listen("tidepage-hl-changed", () => { hlVersion++; }); // ② 订阅主窗广播
    } else if (panel === "stats") {
      await loadTitle();
      // 订阅主窗阅读会话刷新：仅本窗对应 book 生效，version++ 触发 BookStatsModal 重载。
      unStats = await listen<{ bookId: number }>("tidepage-stats-changed", (e) => {
        if (e.payload?.bookId === bookId) statsVersion++;
      });
    }
  });

  onDestroy(() => {
    unHl?.();
    unChap?.();
    unBm?.();
    unStats?.();
  });

  function gotoChapter(index: number) {
    void emit("tidepage-goto-chapter", { index });
  }
  function gotoHighlight(h: HighlightRow) {
    void emit("tidepage-goto-highlight", { id: h.id });
  }
</script>

<div class="panel-window-root">
  <!-- 顶部标题栏：无边框窗口的拖拽区（空白处拖动、双击最大化/还原）；
       右上角自绘窗口控制按钮浮于其上。面板内容整体下移让开这条 40px 的标题栏。 -->
  <div class="win-titlebar" data-tauri-drag-region></div>
  {#if panel === "toc"}
    <TocPanel {toc} current={curChapter} onpick={(i) => gotoChapter(i)} />
  {:else if panel === "bookmarks"}
    <div class="sidepanel">
      <div class="sp-head"><span>书签{title ? ` · ${title}` : ""}</span></div>
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
    <HighlightsPanel {bookId} version={hlVersion} onjump={gotoHighlight} onchanged={() => {}} />
  {:else if panel === "stats"}
    <BookStatsModal {bookId} bookTitle={title} version={statsVersion} />
  {:else if panel === "notes"}
    <NotesPanel standalone />
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

  /* 无边框窗口顶部标题栏拖拽区：z-index 高于面板内容(40)、低于窗口控制按钮(100000) */
  .win-titlebar {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: 40px;
    z-index: 50;
    user-select: none;
    -webkit-user-select: none;
  }

  /* 独立窗内的 sidepanel 铺满整窗（覆盖组件内 position:absolute 的右侧贴边定位）；
     顶部下移 40px 让开标题栏与窗口控制按钮，避免遮挡各面板自己的头部与关闭按钮。 */
  .panel-window-root :global(.sidepanel) {
    position: absolute;
    inset: 40px 0 0 0;
    width: 100%;
    border: none;
    box-shadow: none;
    animation: none;
  }

  /* 笔记面板（NotesPanel 根 .nl 为 fixed inset:0）同样下移 40px。 */
  .panel-window-root :global(.nl) {
    top: 40px;
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
