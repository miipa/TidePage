<!-- 独立面板窗口容器：由 +page.svelte 在 ?panel=xxx&book=<id> 时挂载。
     数据源约定（任务 5）：
       toc / bookmarks / stats → ① 按 URL 的 book id 首屏自拉现有 api（openBook/listBookmarks/BookStatsModal 自载）；
       highlights           → ①首屏自拉 + ② 订阅主窗广播 tidepage-hl-changed 实时刷新；
       notes                → NotesPanel 组件本体自载（① 自拉，未改其本体）。
     跳转类操作通过 Tauri event 广播回主窗（tidepage-goto-chapter / tidepage-goto-highlight）。
     吸附：本窗作为卫星窗 startSnap()，onDestroy 时 stopSnap() 清理监听与吸附组引用。 -->
<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { onMount, onDestroy } from "svelte";
  import { api } from "../lib/api";
  import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { ui } from "../lib/stores.svelte";
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
  // 读的是 .win-titlebar 的【实测高度】而不是把 40 再写一遍 —— 那个 40 原本在这个文件里
  // 手工同步了 4 处（这里的写死值 + .win-titlebar 的 height + 两处 inset/top 的 40px），
  // 改标题栏高度漏一处就出现「面板内容被标题栏压住」或「窗口按钮与标题栏中心线错位」。
  // 现在事实源头只有 .win-titlebar 的 height 一处，本变量与下面两处 inset/top 都从它派生。
  // 手法与 AiPanel 的 aiHeadPadYNow() 同一思路：读事实源头，而不是再写一遍数字。
  let titlebarH = $state(0);
  $effect(() => {
    if (titlebarH > 0) {
      document.documentElement.style.setProperty("--fr-titlebar-h", titlebarH + "px");
    }
  });

  /**
   * 发布本窗对应的书，供 stores 的 effReaderSurface() 取到「本书的阅读配色方案」。
   *
   * 缺口背景：--rdr-* 写在 <html> 上（applyReaderSurfaceVars），供拿不到 .reader 的地方用 ——
   * 独立阅读窗与独立面板窗都属这一类。effReaderSurface() 默认读 ui.currentBookId，
   * 而本窗不渲染 Reader，那两处写入（Reader 的 openBook / 退出清理）永远不会发生，
   * 于是本窗的 --rdr-* 只能退到全局阅读主题 —— 笔记/目录/划线/书签统计四个面板在独立窗里
   * 与该书在主窗口里的观感不一致。补上这一行，四个面板才跟随本书方案。
   *
   * ⚠️ 为什么在这里写 ui.currentBookId 是安全的：独立面板窗是**独立的 webview**，
   *    有自己独立的 stores 模块实例 —— 在这里赋值只影响这个窗口的 <html>，
   *    不会漏回主窗（主窗的 ui 是另一个实例）。
   *
   * bookId === 0（URL 没带 book 参数）时【不写】：那说明本窗没有明确对应的书，
   * 此时写 0 等于用"没有方案"去覆盖全局值，属于把状态清成意外值。
   */
  onMount(() => {
    if (bookId > 0) ui.currentBookId = bookId;
  });

  // 还原成"未打开任何书"。窗口被关闭时整个 JS 上下文随之销毁，这一步多半不会有人看见；
  // 它真正有用的是组件在窗口存活期间被卸载的情况。try/catch 是因为关闭过程中
  // stores 可能已经不可用，不能让清理抛错阻断上面那几条 un* 退订。
  onDestroy(() => {
    try {
      if (bookId > 0 && ui.currentBookId === bookId) ui.currentBookId = 0;
    } catch { /* 窗口正在关闭，忽略 */ }
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
  <div class="win-titlebar" bind:clientHeight={titlebarH} data-tauri-drag-region></div>
  {#if panel === "toc"}
    <TocPanel {toc} current={curChapter} onpick={(i) => gotoChapter(i)} />
  {:else if panel === "bookmarks"}
    <div class="sidepanel">
      <div class="sp-head"><span>{t("reader.bookmarks.title")}{title ? ` · ${title}` : ""}</span></div>
      <div class="sp-list">
        {#each bookmarks as bm (bm.id)}
          <div class="sp-item bm-item">
            <div class="bm-main" onclick={() => gotoChapter(bm.chapter_index)}>
              <div class="bm-title">{bm.title || t("common.untitled")}</div>
              {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
            </div>
          </div>
        {:else}
          <div class="sp-empty" data-testid="sp-empty">{t("common.bookmarksEmpty")}</div>
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
    <div class="pw-empty">{t("panel.unknown", { kind: panel })}</div>
  {/if}
</div>

<style>
  .panel-window-root {
    height: 100vh;
    /* 独立面板窗整窗都是阅读内容（目录 / 书签 / 划线 / 统计 / 笔记），
       底色跟阅读主题而非外壳皮肤。--rdr-bg/-fg 由 stores.svelte.ts 的
       applyReaderSurfaceVars() 写到 <html>（独立窗没有 .reader 可继承）。 */
    background: var(--rdr-bg);
    color: var(--rdr-fg);
    position: relative;
    overflow: hidden;
  }

/* 无边框窗口顶部标题栏拖拽区：z-index 高于面板内容(--z-drawer)、低于窗口控制按钮(--z-window)
     ⚠️ 这个 height 是本窗标题栏高度的【唯一事实源头】：--fr-titlebar-h 由它的实测值写入，
        下面两处 inset/top 的 fallback 也必须与它相等。改这里要一起改那两处。 */
  .win-titlebar {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: 40px;
    z-index: var(--z-sticky);
    user-select: none;
    -webkit-user-select: none;
  }

/* 独立窗内的 sidepanel 铺满整窗（覆盖组件内 position:absolute 的右侧贴边定位）；
     顶部下移一条标题栏的高度让开标题栏与窗口控制按钮，避免遮挡各面板自己的头部与关闭按钮。

     ⚠️ 下面两处的 var(--fr-titlebar-h, 40px) —— fallback 里那个 40px 与 .win-titlebar 的
        height 是【同一个值，不是巧合】：本窗 JS 把 .win-titlebar 的实测高度写进
        --fr-titlebar-h，而这条 inset/top 在 JS 跑之前（变量尚未写入）就必须已经正确，
        那时的正确值恰好就是标题栏的 CSS 高度。改标题栏高度时，这三处
        （.win-titlebar 的 height + 下面两个 fallback）要一起改。
        为什么不给 --fr-titlebar-h 一个全局初值：这个变量在四个窗口里各写各的
        （Library / Reader / AiPanel / StandalonePanel），值互不相同，
        在 :root 里写死一个初值只会在另外三个窗口里变成假值。 */
  .panel-window-root :global(.sidepanel) {
    position: absolute;
    inset: var(--fr-titlebar-h, 40px) 0 0 0;
    width: 100%;
    border: none;
    box-shadow: none;
    animation: none;
  }

  /* 笔记面板（NotesPanel 根 .nl 为 fixed inset:0）同样下移一条标题栏的高度（同上，fallback 同步）。 */
  .panel-window-root :global(.nl) {
    top: var(--fr-titlebar-h, 40px);
  }

  /* .sidepanel / .sp-head / .sp-list / .sp-item / .sp-empty 走 app.css 的全局基类。
     独立窗内没有阅读区（.reader 不存在，--rdr-* 未定义），由 :root 的兜底退回外壳面板色。 */
  .bm-item { display: flex; align-items: center; }
  .bm-main { flex: 1; min-width: 0; cursor: pointer; }
  .bm-main {
    transition: background var(--dur-fast) var(--ease);
  }
  .bm-main:hover { background: var(--hover-row); border-radius: var(--radius-sm); }
  .bm-title { font-size: var(--fs-sm); letter-spacing: var(--ls-sm); }
  .bm-excerpt {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.6;
    margin-top: 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pw-empty { padding: 40px; text-align: center; opacity: 0.6; }
</style>
