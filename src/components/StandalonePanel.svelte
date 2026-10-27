<!-- 独立面板窗口容器：由 +page.svelte 在 ?panel=xxx&book=<id> 时挂载。
     数据源约定（任务 5）：
       toc / bookmarks / stats → ① 按 URL 的 book id 首屏自拉现有 api（openBook/listBookmarks/BookStatsModal 自载）；
       highlights           → ①首屏自拉 + ② 订阅主窗广播 tidepage-hl-changed 实时刷新；
       notes                → NotesPanel 组件本体自载（① 自拉，未改其本体）。
     跳转类操作通过 Tauri event 广播回主窗（tidepage-goto-chapter / tidepage-goto-highlight）。
     吸附：本窗作为卫星窗 startSnap()，onDestroy 时 stopSnap() 清理监听与吸附组引用。 -->
<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { displayChapterTitle } from "../lib/chapterTitle";
  import { onMount, onDestroy } from "svelte";
  import { api } from "../lib/api";
  import { mergeOutlineTitles, retitlePdfPageToc, type PdfOutlineItem } from "../lib/pdfPage";
  import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { setSnapBaseLabelProvider, refreshSnapBase } from "../lib/windowSnap";
  import type { TocEntry, HighlightRow, BookmarkRow } from "../lib/types";
  import TocPanel from "./TocPanel.svelte";
  import HighlightsPanel from "./HighlightsPanel.svelte";
  import BookStatsModal from "./BookStatsModal.svelte";
  import NotesPanel from "./NotesPanel.svelte";

  const params = new URLSearchParams(window.location.search);
  const panel = params.get("panel") || "";

  // 本窗在读哪本书：URL 的 book 只是首屏初值。窗口 label 与书无关（panel-<kind>），
  // 同一扇窗会被复用到别的书上，所以必须能被主窗的 tidepage-book-changed 改写。
  let bookId = $state(Number(params.get("book") || "0"));
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
  let unBook: UnlistenFn | null = null;
  let unClosed: UnlistenFn | null = null;
  let unActive: UnlistenFn | null = null;
  const loadSeq = new RequestSeq();

  async function loadTitle() {
    // F6：只有最新一次加载可以写标题；换书后旧回包直接丢弃。
    const token = loadSeq.next();
    const bid = bookId;
    try {
      const rows = await api.listBooksByIds([bid]);
      if (!loadSeq.isCurrent(token) || bid !== bookId) return;
      title = rows?.[0]?.title ?? "";
    } catch { /* ignore */ }
  }

  /** 按当前 bookId 载入本面板的数据：首屏与「主窗换书」都走这一条。 */
  async function load() {
    if (!bookId) return;
    // F6：请求序号 + bookId 快照：主窗连续换书时只有最后一次能写 toc/curChapter/title。
    const token = loadSeq.next();
    const bid = bookId;
    const current = () => loadSeq.isCurrent(token) && bid === bookId;
    if (panel === "toc") {
      try {
        const r = await api.openBook(bid);
        const raw = r.toc ?? [];
        if (r.book?.format === "pdf") {
          // PDF：后端只给「第 N 页」；真实章节名是阅读器解析 outline 后存下的（本窗不重解析 PDF，
          // 省一次整文件读取）。没有存过就回落本界语言的页码标题。
          const saved = await api.getSetting("pdf.outline." + bid).catch(() => null);
          const outline = saved ? (JSON.parse(saved) as PdfOutlineItem[]) : [];
          const label = (n: number) => t("reader.pdf.page", { n });
          toc = outline.length
            ? mergeOutlineTitles(retitlePdfPageToc(raw, label), outline, label)
            : retitlePdfPageToc(raw, label);
        } else {
          toc = raw;
        }
        const p = await api.loadProgress(bid);
        if (!current()) return;
        title = r.book?.title ?? "";
        curChapter = p?.chapter_index ?? 0;
      } catch { /* ignore */ }
    } else if (panel === "bookmarks") {
      await loadTitle();
      try {
        const rows = await api.listBookmarks(bid);
        if (current()) bookmarks = rows;
      } catch { /* ignore */ }
    } else if (panel === "highlights") {
      await loadTitle();
      if (!current()) return;
      hlVersion++; // HighlightsPanel 只按 version 重载：换书也必须靠它把数据切过去
    } else if (panel === "stats") {
      await loadTitle();
      if (!current()) return;
      statsVersion++;
    }
  }

  // 本窗顶部标题栏固定 40px：写入全局变量让右上角 WindowControls 高度=40px、按钮垂直居中对齐。
  onMount(() => {
    document.documentElement.style.setProperty("--tp-titlebar-h", "40px");
  });

  // 吸附基准：面板窗吸到「打开这本书的独立书窗」；该窗不在时回退主窗。
  onMount(() => {
    setSnapBaseLabelProvider(() => (bookId ? `reader-${bookId}` : "main"));
    void refreshSnapBase();
  });

  onMount(async () => {
    // 订阅主窗广播。除「换书」外都只对当前书生效。
    unChap = await listen<{ bookId: number; index: number }>("tidepage-chapter-changed", (e) => {
      if (e.payload?.bookId === bookId && typeof e.payload.index === "number") {
        curChapter = e.payload.index;
      }
    });
    unBm = await listen<{ bookId: number }>("tidepage-bookmarks-changed", async (e) => {
      if (e.payload?.bookId === bookId) {
        try { bookmarks = await api.listBookmarks(bookId); } catch { /* ignore */ }
      }
    });
    unHl = await listen("tidepage-hl-changed", () => { hlVersion++; }); // ② 订阅主窗广播
    unStats = await listen<{ bookId: number }>("tidepage-stats-changed", (e) => {
      if (e.payload?.bookId === bookId) statsVersion++;
    });
    // 多本书窗并存：跟随最近激活的书窗（内容与吸附基准一起切过去）。
    unActive = await listen<{ bookId: number; title?: string }>("tidepage-book-active", async (e) => {
      const id = Number(e.payload?.bookId ?? 0);
      if (!id || id === bookId) return;
      bookId = id;
      title = e.payload?.title || "";
      void refreshSnapBase();
      await load();
    });
    unClosed = await listen<{ bookId: number }>("tidepage-book-closed", (e) => {
      if (!bookId || Number(e.payload?.bookId) !== bookId) return;
      // 当前书已关闭：清空面板内容回到空状态，等下一本打开再显示（笔记面板不绑书，不受影响）。
      bookId = 0;
      title = "";
      toc = [];
      bookmarks = [];
      curChapter = 0;
      hlVersion++;
      statsVersion++;
      void refreshSnapBase();
    });
    unBook = await listen<{ bookId: number; title?: string }>("tidepage-book-changed", async (e) => {
      const id = Number(e.payload?.bookId ?? 0);
      if (!id || id === bookId) return; // 0=主窗没在读书；同书不重复加载
      bookId = id;
      void refreshSnapBase();
      title = e.payload?.title || "";
      await load();
    });
    await load();
    // ready 握手：本窗监听已就绪，请主窗补发一次当前书。新建窗口时 URL 只是初值，
    // 而「换书」可能发生在监听挂上之前 —— 没有这一步新窗口就永远停在建窗时那本。
    void emit("tidepage-panel-ready", { panel });
  });

  onDestroy(() => {
    unHl?.();
    unChap?.();
    unBm?.();
    unStats?.();
    unBook?.();
    unClosed?.();
    unActive?.();
  });

  function gotoChapter(index: number) {
    void emit("tidepage-goto-chapter", { index });
  }
  function gotoHighlight(h: HighlightRow) {
    void emit("tidepage-goto-highlight", { id: h.id });
  }
  import { RequestSeq } from "../lib/requestSeq";
</script>

<div class="panel-window-root">
  <!-- 顶部标题栏：无边框窗口的拖拽区（空白处拖动、双击最大化/还原）；
       右上角自绘窗口控制按钮浮于其上。面板内容整体下移让开这条 40px 的标题栏。 -->
  <div class="win-titlebar" data-tauri-drag-region></div>
  {#if !bookId && panel !== "notes"}
    <div class="pw-empty">{t("panel.noBook")}</div>
  {:else if panel === "toc"}
    <TocPanel {toc} current={curChapter} onpick={(i) => gotoChapter(i)} />
  {:else if panel === "bookmarks"}
    <div class="sidepanel">
      <div class="sp-head"><span>{t("reader.bookmarks.title")}{title ? ` · ${title}` : ""}</span></div>
      <div class="sp-list">
        {#each bookmarks as bm (bm.id)}
          <div class="sp-item bm-item">
            <div class="bm-main" onclick={() => gotoChapter(bm.chapter_index)}>
              <div class="bm-title">{displayChapterTitle(bm.title, bm.chapter_index, (n) => t("common.sectionN", { n }))}</div>
              {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
            </div>
          </div>
        {:else}
          <div class="sp-empty" data-testid="sp-empty">{t("common.bookmarksEmpty")}</div>
        {/each}
      </div>
    </div>
  {:else if panel === "highlights"}
    <HighlightsPanel {bookId} version={hlVersion} bookTitle={title} onjump={gotoHighlight} onchanged={() => {}} />
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

  /* 无边框窗口顶部标题栏拖拽区：z-index 高于面板内容(--z-drawer)、低于窗口控制按钮(--z-window) */
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
