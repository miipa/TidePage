<!-- 笔记窗口：与书库窗口同款的书籍列表 —— 只列「有划线或添加过感想」的书。
     网格/列表/状态栏沿用书库的视觉与几何；列宽交给 CSS 自适应
     （minmax(min(168px,100%),1fr)），任何窗口尺寸下封面都不会越界被窗口边缘裁掉。 -->
<script lang="ts">
  import { dateLocale, errText, t } from "../lib/i18n/index.svelte";
  import { displayChapterTitle } from "../lib/chapterTitle";
  import { api, coverUrl, bumpCoverBust } from "../lib/api";
  import { toast, ui, requestOpenBook, APP_VERSION, openPanelWindow } from "../lib/stores.svelte";
  import { onMount } from "svelte";
  import ExportPreview from "./ExportPreview.svelte";
  import { sortExportRows, type HighlightExportRow } from "../lib/highlightExport";
  import { outDissolve, outSlideR } from "../lib/motion";
  import { listen } from "@tauri-apps/api/event";
  import {
    IconArrowLeft,
    IconArrowsSplit2,
    IconLayoutGrid,
    IconLayoutList,
    IconRefresh,
    IconTrash,
    IconX,
  } from "@tabler/icons-svelte";

  // standalone=true：作为独立面板窗口挂载，隐藏“返回书库”的 ×（改用右上角窗口控件关闭）。
  let { standalone = false }: { standalone?: boolean } = $props();

  type NoteBook = {
    bookId: number; title: string; author: string; deleted: boolean;
    hlCount: number; noteCount: number; lastAt: number;
    coverPath: string; coverVer: number;
  };
  type NoteItem = { id: number; chapterIndex: number; chapterTitle: string; text: string; note: string; createdAt: number };

  let books: NoteBook[] = $state([]);
  let loading = $state(true);
  let query = $state("");
  let sortKey = $state<"recent" | "title" | "count">(
    (() => {
      try {
        const v = localStorage.getItem("tidepage.notes.sort");
        return v === "title" || v === "count" ? v : "recent";
      } catch { return "recent"; }
    })(),
  );
  let view = $state<"grid" | "list">(
    (() => {
      try { return localStorage.getItem("tidepage.notes.view") === "list" ? "list" : "grid"; }
      catch { return "grid"; }
    })(),
  );
  let detail: NoteBook | null = $state(null);
  let detailNotes: NoteItem[] = $state([]);
  let detailLoading = $state(false);

  // 搜索/排序都只作用在这个小列表上（有划线的书通常远少于书库总量）。
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const arr = books.filter(
      (b) => !q || b.title.toLowerCase().includes(q) || b.author.toLowerCase().includes(q),
    );
    if (sortKey === "title") arr.sort((a, b) => a.title.localeCompare(b.title, dateLocale()));
    else if (sortKey === "count") arr.sort((a, b) => b.hlCount - a.hlCount || b.lastAt - a.lastAt);
    else arr.sort((a, b) => b.lastAt - a.lastAt);
    return arr;
  });
  const totalHl = $derived(shown.reduce((n, b) => n + b.hlCount, 0));
  const totalNotes = $derived(shown.reduce((n, b) => n + b.noteCount, 0));

  /** D1：全库导出——按书取全量 listHighlights（不用 searchHighlights 的 500 上限），分批让出事件循环。 */
  async function loadAllExportRows(): Promise<HighlightExportRow[]> {
    const list = await api.listNoteBooks();
    const out: HighlightExportRow[] = [];
    for (let i = 0; i < list.length; i++) {
      const b = list[i];
      try {
        const hs = await api.listHighlights(b.bookId);
        for (const h of hs) {
          out.push({
            bookTitle: b.title,
            bookAuthor: b.author ?? "",
            chapterTitle: h.chapter_title,
            chapterIndex: h.chapter_index,
            text: h.text,
            note: h.note ?? "",
            style: h.style ?? "",
            color: h.color ?? "",
            createdAt: h.created_at,
          });
        }
      } catch {
        /* 单本失败不阻断全库导出 */
      }
      if (i % 5 === 4) await new Promise((r) => setTimeout(r, 0));
    }
    return sortExportRows(out);
  }

  async function load() {
    loading = true;
    try {
      books = await api.listNoteBooks();
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      loading = false;
    }
  }

  function setView(v: "grid" | "list") {
    view = v;
    try { localStorage.setItem("tidepage.notes.view", v); } catch { /* ignore */ }
  }

  function setSort(v: string) {
    sortKey = v === "title" || v === "count" ? v : "recent";
    try { localStorage.setItem("tidepage.notes.sort", sortKey); } catch { /* ignore */ }
  }

  /** 弹出为独立窗口：成功后关掉内联笔记面板（逻辑照搬原 +page.svelte 的浮动按钮）。 */
  async function popout() {
    const ok = await openPanelWindow("notes", ui.currentBookId || 0, t("library.topbar.notes.label"));
    if (ok) ui.notesOpen = false;
  }

  async function openDetail(b: NoteBook) {
    detail = b;
    detailLoading = true;
    try {
      detailNotes = await api.listNotesOfBook(b.bookId);
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      detailLoading = false;
    }
  }

  /** 打开原书：先关掉本窗口（它是整屏覆盖层，留着会盖住阅读界面）。 */
  function openBook(b: NoteBook) {
    if (b.deleted || b.bookId < 0) {
      toast(t("notes.bookDeleted"), "error");
      return;
    }
    detail = null;
    ui.notesOpen = false;
    requestOpenBook(b.bookId);
  }

  /** 删这条感想，保留划线。 */
  async function removeNote(n: NoteItem) {
    try {
      await api.setHighlightNote(n.id, "");
      detailNotes = detailNotes.map((x) => (x.id === n.id ? { ...x, note: "" } : x));
      await load();
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  /** 删划线本身（连带它的感想）。 */
  async function removeHighlight(n: NoteItem) {
    try {
      await api.deleteHighlight(n.id);
      detailNotes = detailNotes.filter((x) => x.id !== n.id);
      await load();
      if (detail && detailNotes.length === 0) detail = null;
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  function fmtDate(ms: number): string {
    if (!ms) return "";
    return new Date(ms).toLocaleDateString(dateLocale());
  }

  /** 封面 URL 与书库同源：有内嵌封面走 per-book 端点，否则共享默认封面端点。
   *  原书已删除（无 books 行）→ 空串，卡片改用书名文字兜底。 */
  function cover(b: NoteBook): string {
    if (b.deleted || b.bookId < 0) return "";
    return coverUrl({ id: b.bookId, cover_path: b.coverPath || null, cover_ver: b.coverVer });
  }

  onMount(() => {
    load();
    // notes-changed：划线/感想的增删改（后端在 add/update/delete_highlight 与删书时发出）。
    const un = listen("notes-changed", () => load());
    // 书库侧改名/换封面/删书 → 同一份数据要跟着变（bumpCoverBust 让封面 URL 失效）。
    const un2 = listen("library-changed", () => { bumpCoverBust(); void load(); });
    const un3 = listen("book-updated", () => { bumpCoverBust(); void load(); });
    return () => {
      un.then((f) => f());
      un2.then((f) => f());
      un3.then((f) => f());
    };
  });
</script>

<div class="nl" out:outDissolve>
  <div class="nl-bar">
    <!-- 返回（原为条尾 ✕）：条首摆放后不再与右上角自绘窗口按钮并排出现两个关闭图标 -->
    {#if !standalone}<button class="icon" data-testid="nl-close" title={t("common.action.back")} aria-label={t("common.action.back")} onclick={() => (ui.notesOpen = false)}><IconArrowLeft /></button>{/if}
    <span class="nl-title" data-testid="nl-title">{t("notes.title")}</span>
    <span class="nl-count">{shown.length}</span>
    <input class="nl-search" data-testid="nl-search" placeholder={t("library.toolbar.searchPlaceholder")} bind:value={query} />
    <select data-testid="nl-sort" value={sortKey} onchange={(e) => setSort(e.currentTarget.value)}>
      <option value="recent">{t("notes.sort.recent")}</option>
      <option value="title">{t("notes.sort.title")}</option>
      <option value="count">{t("notes.sort.count")}</option>
    </select>
    <div class="spacer"></div>
    {#if !standalone}<button class="icon" data-testid="nl-popout" title={t("notes.popoutTip")} aria-label={t("notes.popoutTip")} onclick={popout}><IconArrowsSplit2 /></button>{/if}
    <button class="icon" data-testid="nl-refresh" title={t("common.action.refresh")} aria-label={t("common.action.refresh")} onclick={load}><IconRefresh /></button>
    <ExportPreview loadRows={loadAllExportRows} defaultName="tidepage-notes" />
  </div>

  <div class="nl-body">
    {#if loading}
      <div class="empty-state">{t("common.loading")}</div>
    {:else if !shown.length}
      <div class="empty-state" data-testid="nl-empty">{query.trim()
        ? t("library.empty.noMatch")
        : t("notes.empty")}</div>
    {:else if view === "grid"}
      <div class="nl-grid">
        {#each shown as b (b.bookId)}
          <div
            class="card"
            class:active={detail?.bookId === b.bookId}
            role="button"
            tabindex="0"
            title={b.title || t("common.toast.deleted")}
            ondblclick={() => openBook(b)}
            onclick={() => openDetail(b)}
            onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") openDetail(b); }}
          >
            <div class="cover">
              {#if cover(b)}
                <img src={cover(b)} alt={t("library.book.coverAlt", { title: b.title || t("common.toast.deleted") })} loading="lazy" decoding="async" draggable="false" />
              {:else}
                <span class="cover-fallback">
                  <span class="cf-t">{b.title || t("common.toast.deleted")}</span>
                  {#if b.author}<span class="cf-a">{b.author}</span>{/if}
                </span>
              {/if}
              {#if b.deleted}<span class="tag tag-del">{t("common.toast.deleted")}</span>{/if}
              <span class="tag tag-hl">{t("common.hlWord")} {b.hlCount}</span>
              {#if b.noteCount > 0}<span class="tag tag-note">{t("common.noteWord")} {b.noteCount}</span>{/if}
            </div>
            <div class="meta">
              <div class="t">{b.title || t("common.toast.deleted")}</div>
              <div class="a">{b.author || (b.deleted ? "" : t("notes.unknownAuthor"))}</div>
            </div>
          </div>
        {/each}
      </div>
    {:else}
      <div class="nl-list">
        {#each shown as b (b.bookId)}
          <div
            class="row"
            role="button"
            tabindex="0"
            ondblclick={() => openBook(b)}
            onclick={() => openDetail(b)}
            onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") openDetail(b); }}
          >
            {#if cover(b)}
              <img class="row-cover" src={cover(b)} alt={t("library.book.coverAlt", { title: b.title || t("common.toast.deleted") })} loading="lazy" decoding="async" draggable="false" />
            {:else}
              <span class="row-cover row-cover-fallback">{b.title || t("common.toast.deleted")}</span>
            {/if}
            <div class="row-meta">
              <div class="t">{b.title || t("common.toast.deleted")}</div>
              <div class="a">{b.author || (b.deleted ? t("notes.bookDeletedShort") : t("notes.unknownAuthor"))}</div>
            </div>
            <div class="row-num">{t("common.hlWord")} {b.hlCount}{b.noteCount > 0 ? t("notes.noteCountLine", { n: b.noteCount }) : ""}</div>
            <div class="row-date">{fmtDate(b.lastAt)}</div>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="nl-status">
    <span>{shown.length}{t("notes.statLine", { hl: totalHl, note: totalNotes })}</span>
    <span class="build" title={t("common.app.versionTip")}>TidePage {APP_VERSION}</span>
    <div class="spacer"></div>
    <button class="icon view-btn" class:active={view === "grid"} data-testid="nl-view-grid" title={t("common.view.grid")} aria-pressed={view === "grid"} onclick={() => setView("grid")}><IconLayoutGrid /></button>
    <button class="icon view-btn" class:active={view === "list"} data-testid="nl-view-list" title={t("common.view.list")} aria-pressed={view === "list"} onclick={() => setView("list")}><IconLayoutList /></button>
  </div>

  {#if detail}
    <div class="drawer-backdrop" out:outDissolve onclick={() => (detail = null)}></div>
    <aside class="drawer" out:outSlideR role="dialog" aria-label={t("notes.detailTitle")}>
      <div class="drawer-head">
        <div class="dh-main">
          <div class="dh-title">{detail.title || t("common.toast.deleted")}</div>
          <div class="dh-sub">
            {detail.deleted
              ? t("notes.detailDeleted")
              : t("notes.detailCounts", { hl: detail.hlCount, note: detail.noteCount })}
          </div>
        </div>
        {#if !detail.deleted}
          <button data-testid="nl-open-book" onclick={() => openBook(detail!)}>{t("notes.openBook")}</button>
        {/if}
        <button class="icon" data-testid="nl-detail-close" title={t("common.action.close")} onclick={() => (detail = null)}><IconX /></button>
      </div>
      <div class="drawer-list">
        {#if detailLoading}
          <div class="empty-state">{t("common.loading")}</div>
        {:else if !detailNotes.length}
          <div class="empty-state">{t("notes.detailEmpty")}</div>
        {:else}
          {#each detailNotes as n (n.id)}
            <div class="item">
              <div class="quote">{n.text}</div>
              {#if n.note}
                <div class="note">{n.note}</div>
              {:else}
                <div class="note none">{t("notes.noNote")}</div>
              {/if}
              <div class="foot">
                <span class="ch">{displayChapterTitle(n.chapterTitle, n.chapterIndex, (nn) => t("common.sectionN", { n: nn }))} · {fmtDate(n.createdAt)}</span>
                {#if n.note}
                  <button class="icon" data-testid="nl-del-note" title={t("notes.delNoteTip")} onclick={() => removeNote(n)}><IconTrash /></button>
                {:else}
                  <button class="icon" data-testid="nl-del-hl" title={t("highlight.deleteTip")} onclick={() => removeHighlight(n)}><IconTrash /></button>
                {/if}
              </div>
            </div>
          {/each}
        {/if}
      </div>
    </aside>
  {/if}
</div>

<style>
  .nl {
    animation: tp-fade-in var(--dur-fast) var(--ease-premium) both;
    position: fixed;
    /* 顶栏之下：右上角只归自绘窗口按钮，笔记条自成一行；
       独立笔记窗由此露出 40px 的 .win-titlebar 拖动区 */
    top: var(--tp-titlebar-h, 44px);
    left: 0;
    right: 0;
    bottom: 0;
    z-index: var(--z-drawer);
    background: var(--bg);
    color: var(--fg);
    display: flex;
    flex-direction: column;
  }

  /* ---------- 顶栏（与书库 .toolbar 同一套间距/描边） ---------- */
  .nl-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .nl-title {
    font-weight: 600;
    font-size: var(--fs-lg);
    color: var(--accent);
    letter-spacing: -0.015em;
    user-select: none;
  }

  .nl-count {
    font-size: var(--fs-xs);
    color: var(--accent);
    background: var(--accent-soft);
    border-radius: var(--radius-pill);
    padding: 1px 8px;
  }

  .nl-search {
    /* 条首新增「返回」、条尾新增「弹出」后，窄窗下让搜索框先收缩，避免整行换行挤压 */
    flex: 0 1 220px;
    min-width: 0;
  }

  .spacer { flex: 1; }

  /* ---------- 内容区（与书库 .grid 同 padding） ---------- */
  .nl-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px 32px;
  }

  /* min(168px,100%) 保证容器比一列还窄时也不越界 —— 任何窗口尺寸都不裁封面。 */
  .nl-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(168px, 100%), 1fr));
    gap: 12px;
    align-content: start;
  }

  .card {
    padding: var(--sp-2);
    border-radius: var(--radius-lg);
    cursor: pointer;
    transition: background var(--dur) var(--ease),
      transform var(--dur) var(--ease),
      box-shadow var(--dur) var(--ease);
    min-width: 0;
  }

  .card:hover,
  .card:focus-visible {
    background: var(--hover-bg);
    transform: translateY(-2px);
    box-shadow: var(--shadow-md);
  }

  .card.active { background: var(--accent-soft); }

  /* ---------- 封面（与书库 .cover 一致：2:3、8px 圆角、极淡描边） ---------- */
  .cover {
    position: relative;
    width: 100%;
    aspect-ratio: 2 / 3;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: var(--panel2);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm);
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform var(--dur) var(--ease);
  }

  .card:hover .cover img { transform: scale(1.03); }

  .cover-fallback {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 12px;
    text-align: center;
    background: var(--panel2);
  }
  .cf-t { font-size: var(--fs-md); font-weight: 600; line-height: 1.35; color: var(--fg); overflow-wrap: anywhere; }
  .cf-a { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); color: var(--fg2); }

  .tag {
    position: absolute;
    font-size: var(--fs-2xs);
    line-height: 1;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    letter-spacing: var(--ls-xs);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18);
    white-space: nowrap;
  }
  .tag-hl { right: 8px; top: 8px; background: rgba(0, 0, 0, 0.62); color: #fff; }
  .tag-note { right: 8px; top: 32px; background: color-mix(in srgb, var(--accent) 88%, transparent); color: var(--accent-fg); }
  .tag-del { left: 8px; top: 8px; background: color-mix(in srgb, var(--danger) 86%, transparent); color: #fff; }

  /* ---------- 卡片文字块（高度与书库 .meta 对齐：52px） ---------- */
  .meta {
    height: 52px;
    box-sizing: border-box;
    padding: 8px 4px 0;
    overflow: hidden;
  }
  .meta .t {
    font-size: var(--fs-md);
    line-height: 20px;
    font-weight: 500;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta .a {
    font-size: var(--fs-xs);
    line-height: 18px;
    color: var(--fg2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-top: 1px;
  }

  /* ---------- 列表视图（与书库 .row 一致：64px 行） ---------- */
  .nl-list { display: flex; flex-direction: column; }

  .row {
    height: 64px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--dur) var(--ease);
  }
  .row:hover, .row:focus-visible { background: var(--hover-row); }

  /* 列表行封面：32×48 = 1.5，与 Library 列表行、网格封面、书籍详情同一比例 */
  .row-cover {
    width: 32px;
    height: 48px;
    flex-shrink: 0;
    object-fit: cover;
    border-radius: var(--radius-xs);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm);
  }
  .row-cover-fallback {
    display: grid;
    place-items: center;
    text-align: center;
    font-size: 9px;
    line-height: 1.15;
    padding: 2px;
    overflow: hidden;
    background: var(--panel2);
    color: var(--fg2);
    word-break: break-all;
  }

  .row-meta { flex: 1; min-width: 0; }
  .row-meta .t { font-size: var(--fs-md); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-meta .a { font-size: var(--fs-xs); color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .row-num, .row-date {
    color: var(--fg2);
    font-size: var(--fs-xs);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .row-num { width: 130px; }
  .row-date { width: 90px; }

  /* ---------- 状态栏（与书库 .statusbar 一致） ---------- */
  .nl-status {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-4);
    border-top: 1px solid var(--line);
    color: var(--fg2);
    font-size: var(--fs-xs);
    background: var(--panel);
    flex-shrink: 0;
  }
  .nl-status .build { font-size: var(--fs-xs); opacity: 0.55; }

  .view-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    opacity: 1;
  }

  /* ---------- 划线明细抽屉 ---------- */
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    backdrop-filter: blur(3px) saturate(1.08);
    -webkit-backdrop-filter: blur(3px) saturate(1.08);
    z-index: var(--z-drawer);
    animation: tp-fade-in var(--dur-scrim) var(--ease-in) both;
  }

  .drawer {
    position: fixed;
    top: var(--tp-titlebar-h, 44px);
    right: 0;
    bottom: 0;
    width: min(460px, 92vw);
    z-index: var(--z-popover);
    background: var(--bg-elev);
    border-left: 1px solid var(--line);
    box-shadow: -6px 0 18px rgba(0, 0, 0, 0.14), -20px 0 52px rgba(0, 0, 0, 0.22);
    display: flex;
    flex-direction: column;
    animation: tp-drawer-in-right var(--dur-lift) var(--ease-premium) both;
  }

  .drawer-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--line);
  }
  .dh-main { flex: 1; min-width: 0; }
  .dh-title { font-size: var(--fs-md); font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dh-sub { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); color: var(--fg2); margin-top: 2px; }

  .drawer-list { flex: 1; overflow-y: auto; padding: 12px 12px 24px; }

  .item {
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    padding: 12px;
    margin-bottom: 10px;
    background: var(--panel);
  }

  .quote {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    color: var(--fg2);
    border-left: 3px solid var(--line-strong);
    padding-left: 8px;
    margin-bottom: 8px;
    line-height: 1.6;
    overflow-wrap: anywhere;
  }

  .note { font-size: var(--fs-md); line-height: 1.65; white-space: pre-wrap; overflow-wrap: anywhere; }
  .note.none { color: var(--fg2); font-size: var(--fs-sm); letter-spacing: var(--ls-sm); }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 8px;
  }
  .ch { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); color: var(--fg2); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
