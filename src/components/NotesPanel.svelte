<!-- 笔记窗口：与书库窗口同款的书籍列表 —— 只列「有划线或添加过感想」的书。
     网格/列表/状态栏沿用书库的视觉与几何；列宽交给 CSS 自适应
     （minmax(min(168px,100%),1fr)），任何窗口尺寸下封面都不会越界被窗口边缘裁掉。 -->
<script lang="ts">
  import { api, coverUrl, bumpCoverBust } from "../lib/api";
  import { toast, ui, requestOpenBook, APP_VERSION } from "../lib/stores.svelte";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";

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
        const v = localStorage.getItem("flowread.notes.sort");
        return v === "title" || v === "count" ? v : "recent";
      } catch { return "recent"; }
    })(),
  );
  let view = $state<"grid" | "list">(
    (() => {
      try { return localStorage.getItem("flowread.notes.view") === "list" ? "list" : "grid"; }
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
    if (sortKey === "title") arr.sort((a, b) => a.title.localeCompare(b.title, "zh"));
    else if (sortKey === "count") arr.sort((a, b) => b.hlCount - a.hlCount || b.lastAt - a.lastAt);
    else arr.sort((a, b) => b.lastAt - a.lastAt);
    return arr;
  });
  const totalHl = $derived(shown.reduce((n, b) => n + b.hlCount, 0));
  const totalNotes = $derived(shown.reduce((n, b) => n + b.noteCount, 0));

  async function load() {
    loading = true;
    try {
      books = await api.listNoteBooks();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      loading = false;
    }
  }

  function setView(v: "grid" | "list") {
    view = v;
    try { localStorage.setItem("flowread.notes.view", v); } catch { /* ignore */ }
  }

  function setSort(v: string) {
    sortKey = v === "title" || v === "count" ? v : "recent";
    try { localStorage.setItem("flowread.notes.sort", sortKey); } catch { /* ignore */ }
  }

  async function openDetail(b: NoteBook) {
    detail = b;
    detailLoading = true;
    try {
      detailNotes = await api.listNotesOfBook(b.bookId);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      detailLoading = false;
    }
  }

  /** 打开原书：先关掉本窗口（它是整屏覆盖层，留着会盖住阅读界面）。 */
  function openBook(b: NoteBook) {
    if (b.deleted || b.bookId < 0) {
      toast("原书已删除，仅保留划线/感想", "error");
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
      toast(String(e), "error");
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
      toast(String(e), "error");
    }
  }

  function fmtDate(ms: number): string {
    if (!ms) return "";
    return new Date(ms).toLocaleDateString("zh-CN");
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

<div class="nl">
  <div class="nl-bar">
    <span class="nl-title">笔记</span>
    <span class="nl-count">{shown.length}</span>
    <input class="nl-search" placeholder="搜索书名 / 作者…" bind:value={query} />
    <select value={sortKey} onchange={(e) => setSort(e.currentTarget.value)}>
      <option value="recent">按最近划线</option>
      <option value="title">按书名</option>
      <option value="count">按划线条数</option>
    </select>
    <div class="spacer"></div>
    <button class="icon" title="刷新" aria-label="刷新" onclick={load}>⟳</button>
    <button class="icon" title="返回书库" aria-label="返回书库" onclick={() => (ui.notesOpen = false)}>×</button>
  </div>

  <div class="nl-body">
    {#if loading}
      <div class="hint">加载中…</div>
    {:else if !shown.length}
      <div class="hint">{query.trim()
        ? "无匹配结果"
        : "还没有划线或笔记\n阅读时划线或写感想，这里会按书汇总"}</div>
    {:else if view === "grid"}
      <div class="nl-grid">
        {#each shown as b (b.bookId)}
          <div
            class="card"
            class:active={detail?.bookId === b.bookId}
            role="button"
            tabindex="0"
            title={b.title || "已删除"}
            ondblclick={() => openBook(b)}
            onclick={() => openDetail(b)}
            onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") openDetail(b); }}
          >
            <div class="cover">
              {#if cover(b)}
                <img src={cover(b)} alt="《{b.title || "已删除"}》封面" loading="lazy" decoding="async" draggable="false" />
              {:else}
                <span class="cover-fallback">
                  <span class="cf-t">{b.title || "已删除"}</span>
                  {#if b.author}<span class="cf-a">{b.author}</span>{/if}
                </span>
              {/if}
              {#if b.deleted}<span class="tag tag-del">已删除</span>{/if}
              <span class="tag tag-hl">划线 {b.hlCount}</span>
              {#if b.noteCount > 0}<span class="tag tag-note">感想 {b.noteCount}</span>{/if}
            </div>
            <div class="meta">
              <div class="t">{b.title || "已删除"}</div>
              <div class="a">{b.author || (b.deleted ? "" : "未知作者")}</div>
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
              <img class="row-cover" src={cover(b)} alt="《{b.title || "已删除"}》封面" loading="lazy" decoding="async" draggable="false" />
            {:else}
              <span class="row-cover row-cover-fallback">{b.title || "已删除"}</span>
            {/if}
            <div class="row-meta">
              <div class="t">{b.title || "已删除"}</div>
              <div class="a">{b.author || (b.deleted ? "原书已删除" : "未知作者")}</div>
            </div>
            <div class="row-num">划线 {b.hlCount}{b.noteCount > 0 ? ` · 感想 ${b.noteCount}` : ""}</div>
            <div class="row-date">{fmtDate(b.lastAt)}</div>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="nl-status">
    <span>{shown.length} 本书 · 划线 {totalHl} 条 · 感想 {totalNotes} 条</span>
    <span class="build" title="FlowRead 版本">FlowRead {APP_VERSION}</span>
    <div class="spacer"></div>
    <button class="icon view-btn" class:active={view === "grid"} title="网格视图" aria-pressed={view === "grid"} onclick={() => setView("grid")}>▦</button>
    <button class="icon view-btn" class:active={view === "list"} title="列表视图" aria-pressed={view === "list"} onclick={() => setView("list")}>≡</button>
  </div>

  {#if detail}
    <div class="drawer-backdrop" onclick={() => (detail = null)}></div>
    <aside class="drawer" role="dialog" aria-label="划线明细">
      <div class="drawer-head">
        <div class="dh-main">
          <div class="dh-title">{detail.title || "已删除"}</div>
          <div class="dh-sub">
            {detail.deleted
              ? "原书已删除 · 划线/感想保留"
              : `划线 ${detail.hlCount} 条 · 感想 ${detail.noteCount} 条`}
          </div>
        </div>
        {#if !detail.deleted}
          <button onclick={() => openBook(detail!)}>打开原书</button>
        {/if}
        <button class="icon" title="关闭" onclick={() => (detail = null)}>×</button>
      </div>
      <div class="drawer-list">
        {#if detailLoading}
          <div class="hint">加载中…</div>
        {:else if !detailNotes.length}
          <div class="hint">这本书已经没有划线了</div>
        {:else}
          {#each detailNotes as n (n.id)}
            <div class="item">
              <div class="quote">{n.text}</div>
              {#if n.note}
                <div class="note">{n.note}</div>
              {:else}
                <div class="note none">（无感想）</div>
              {/if}
              <div class="foot">
                <span class="ch">{n.chapterTitle || `第 ${n.chapterIndex + 1} 节`} · {fmtDate(n.createdAt)}</span>
                {#if n.note}
                  <button class="icon" title="删除这条感想（保留划线）" onclick={() => removeNote(n)}>🗑</button>
                {:else}
                  <button class="icon" title="删除这条划线" onclick={() => removeHighlight(n)}>🗑</button>
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
    position: fixed;
    inset: 0;
    z-index: 1090;
    background: var(--bg);
    color: var(--fg);
    display: flex;
    flex-direction: column;
  }

  /* ---------- 顶栏（与书库 .toolbar 同一套间距/描边） ---------- */
  .nl-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-2, 8px);
    padding: var(--sp-3, 12px) var(--sp-4, 16px);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .nl-title {
    font-weight: 600;
    font-size: var(--fs-lg, 16px);
    color: var(--accent);
    letter-spacing: -0.015em;
    user-select: none;
  }

  .nl-count {
    font-size: var(--fs-xs, 12px);
    color: var(--accent);
    background: var(--accent-soft, var(--hover-bg));
    border-radius: var(--radius-pill, 999px);
    padding: 1px 8px;
  }

  .nl-search {
    width: 220px;
  }

  .spacer { flex: 1; }

  /* ---------- 内容区（与书库 .grid 同 padding） ---------- */
  .nl-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px 32px;
  }

  .hint {
    text-align: center;
    padding: 48px 24px;
    color: var(--fg2);
    white-space: pre-line;
    line-height: var(--lh-body, 1.6);
    font-size: var(--fs-md, 14px);
  }

  /* min(168px,100%) 保证容器比一列还窄时也不越界 —— 任何窗口尺寸都不裁封面。 */
  .nl-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(168px, 100%), 1fr));
    gap: 12px;
    align-content: start;
  }

  .card {
    padding: var(--sp-2, 8px);
    border-radius: var(--radius, 12px);
    cursor: pointer;
    transition: background var(--dur, 0.2s) var(--ease),
      transform var(--dur, 0.2s) var(--ease),
      box-shadow var(--dur, 0.2s) var(--ease);
    min-width: 0;
  }

  .card:hover,
  .card:focus-visible {
    background: var(--hover-bg);
    transform: translateY(-2px);
    box-shadow: var(--shadow-md);
  }

  .card.active { background: var(--accent-soft, var(--hover-bg)); }

  /* ---------- 封面（与书库 .cover 一致：2:3、8px 圆角、极淡描边） ---------- */
  .cover {
    position: relative;
    width: 100%;
    aspect-ratio: 2 / 3;
    border-radius: var(--radius-sm, 8px);
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
    transition: transform var(--dur, 0.2s) var(--ease);
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
    background: linear-gradient(160deg, var(--panel2), color-mix(in srgb, var(--panel2) 70%, #000 8%));
  }
  .cf-t { font-size: 14px; font-weight: 600; line-height: 1.35; color: var(--fg); overflow-wrap: anywhere; }
  .cf-a { font-size: 12px; color: var(--fg2); }

  .tag {
    position: absolute;
    font-size: var(--fs-2xs, 10px);
    line-height: 1;
    padding: 4px 8px;
    border-radius: var(--radius-sm, 8px);
    letter-spacing: var(--ls-wide, 0.015em);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    white-space: nowrap;
  }
  .tag-hl { right: 8px; top: 8px; background: rgba(0, 0, 0, 0.6); color: #fff; }
  .tag-note { right: 8px; top: 32px; background: color-mix(in srgb, var(--accent) 88%, transparent); color: var(--accent-fg, #fff); }
  .tag-del { left: 8px; top: 8px; background: color-mix(in srgb, var(--danger) 86%, transparent); color: #fff; }

  /* ---------- 卡片文字块（高度与书库 .meta 对齐：52px） ---------- */
  .meta {
    height: 52px;
    box-sizing: border-box;
    padding: 8px 4px 0;
    overflow: hidden;
  }
  .meta .t {
    font-size: var(--fs-md, 14px);
    line-height: 20px;
    font-weight: 500;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta .a {
    font-size: var(--fs-xs, 12px);
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
    gap: var(--sp-4, 16px);
    padding: var(--sp-2, 8px) var(--sp-3, 12px);
    border-radius: var(--radius-sm, 8px);
    cursor: pointer;
    transition: background var(--dur, 0.2s) var(--ease);
  }
  .row:hover, .row:focus-visible { background: var(--hover-row, rgba(0, 0, 0, 0.04)); }

  .row-cover {
    width: 34px;
    height: 48px;
    flex-shrink: 0;
    object-fit: cover;
    border-radius: var(--radius-xs, 6px);
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
  .row-meta .t { font-size: var(--fs-md, 14px); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-meta .a { font-size: var(--fs-xs, 12px); color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .row-num, .row-date {
    color: var(--fg2);
    font-size: var(--fs-xs, 12px);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .row-num { width: 130px; }
  .row-date { width: 90px; }

  /* ---------- 状态栏（与书库 .statusbar 一致） ---------- */
  .nl-status {
    display: flex;
    align-items: center;
    gap: var(--sp-2, 8px);
    padding: var(--sp-2, 8px) var(--sp-4, 16px);
    border-top: 1px solid var(--line);
    color: var(--fg2);
    font-size: var(--fs-xs, 12px);
    background: var(--panel);
    flex-shrink: 0;
  }
  .nl-status .build { font-size: var(--fs-xs, 12px); opacity: 0.55; }

  .view-btn.active {
    background: var(--accent-soft, var(--hover-bg));
    color: var(--accent);
    opacity: 1;
  }

  /* ---------- 划线明细抽屉 ---------- */
  .drawer-backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.35); z-index: 1090; }

  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(460px, 92vw);
    z-index: 1091;
    background: var(--bg-elev);
    border-left: 1px solid var(--line);
    box-shadow: -10px 0 32px rgba(0, 0, 0, 0.25);
    display: flex;
    flex-direction: column;
  }

  .drawer-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--line);
  }
  .dh-main { flex: 1; min-width: 0; }
  .dh-title { font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dh-sub { font-size: 12px; color: var(--fg2); margin-top: 2px; }

  .drawer-list { flex: 1; overflow-y: auto; padding: 12px 12px 24px; }

  .item {
    border: 1px solid var(--line);
    border-radius: var(--radius-md, 12px);
    padding: 12px;
    margin-bottom: 10px;
    background: var(--panel);
  }

  .quote {
    font-size: 13px;
    color: var(--fg2);
    border-left: 3px solid var(--line-strong);
    padding-left: 8px;
    margin-bottom: 8px;
    line-height: 1.6;
    overflow-wrap: anywhere;
  }

  .note { font-size: 14px; line-height: 1.65; white-space: pre-wrap; overflow-wrap: anywhere; }
  .note.none { color: var(--fg2); font-size: 13px; }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 8px;
  }
  .ch { font-size: 12px; color: var(--fg2); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
