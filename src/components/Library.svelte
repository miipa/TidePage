<script lang="ts">
  import { onMount } from "svelte";
  import { api, coverUrl } from "../lib/api";
  import { appConfirm, config, convertState, openBookInNewWindow, pendingOpen, toast, ui, APP_BUILD } from "../lib/stores.svelte";
  import { listen } from "@tauri-apps/api/event";
  import BookDetailModal from "./BookDetailModal.svelte";
  import type { BookRow } from "../lib/types";

  // ===== 超大书库（实测 16 万本）：前端永不全量持有书籍 =====
  // 只保留「按需窗口 + total」：过滤/排序/分页全部下推 Rust，单次 IPC 至多 FETCH 行。
  const FETCH = 500;        // 单次分页窗口（Rust 端 limit 上限 500）
  const GRID_COL_MIN = 168; // 网格最小列宽
  const ROW_H = 66;         // 列表行步长（行高 64 + 间距 2，与 .row / .list-inner CSS 一致）

  let total = $state(0);
  // $state.raw：窗口数组不递归代理（避免 16 万对象的深层响应式开销），整体替换更新。
  let booksWindow: BookRow[] = $state.raw([]);
  let rangeStart = $state(0);
  let loadSeq = 0;

  let folders: string[] = $state([]);
  let queryInput = $state("");
  let query = $state("");
  let sortKey = $state<"added" | "lastread" | "title" | "recent">(
    (() => {
      try {
        const v = localStorage.getItem("flowread.lib.sort");
        return v === "lastread" || v === "title" || v === "recent" ? v : "added";
      } catch { return "added"; }
    })()
  );
  let view = $state<"grid" | "list">(
    (() => {
      try { return localStorage.getItem("flowread.lib.view") === "list" ? "list" : "grid"; }
      catch { return "grid"; }
    })()
  );
  let loading = $state(true);
  let refreshing = $state(false);

  function saveSort() {
    try { localStorage.setItem("flowread.lib.sort", sortKey); } catch { /* ignore */ }
  }

  let swipeStartX = 0;
  let swipeStartT = 0;
  let lastOpenT = 0;
  function onPtrDown(e: PointerEvent) {
    swipeStartX = e.clientX;
    swipeStartT = Date.now();
  }
  function onPtrUp(e: PointerEvent) {
    const dx = e.clientX - swipeStartX;
    const fast = Date.now() - swipeStartT < 250;
    if (fast && dx < -60 && ui.view === "library" && Date.now() - lastOpenT > 400) {
      ui.webdavOpen = true;
      lastOpenT = Date.now();
    }
  }

  /** 切换视图：重置滚动，避免网格的大 scrollTop 直接套到列表。 */
  function setView(v: "grid" | "list") {
    if (v === view) return;
    view = v;
    try { localStorage.setItem("flowread.lib.view", v); } catch { /* ignore */ }
    resetScroll();
  }

  function resetScroll() {
    scrollTop = 0;
    if (gridEl) gridEl.scrollTop = 0;
  }

  let gridEl: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let gridH = $state(800);
  let gridW = $state(1200);
  let gridPadX = $state(40);
  let ctxMenu = $state<{ x: number; y: number; book: BookRow } | null>(null);
  let ctxEl: HTMLDivElement | undefined = $state();
  let ctxPos = $state({ left: 0, top: 0 });
  let detailBook: BookRow | null = $state(null);

  /** 右键菜单：先按鼠标位置显示，渲染后按菜单实际尺寸夹取到视口内（避免贴近底部/右侧被遮住）。 */
  function openCtx(e: MouseEvent, b: BookRow) {
    e.preventDefault();
    ctxMenu = { x: e.clientX, y: e.clientY, book: b };
    ctxPos = { left: e.clientX, top: e.clientY };
  }

  $effect(() => {
    if (!ctxMenu || !ctxEl) return;
    const r = ctxEl.getBoundingClientRect();
    const maxX = window.innerWidth - r.width - 8;
    const maxY = window.innerHeight - r.height - 8;
    ctxPos = {
      left: Math.max(8, Math.min(ctxMenu.x, maxX)),
      top: Math.max(8, Math.min(ctxMenu.y, maxY)),
    };
  });

  // 真实内容宽 = clientWidth - 左右 padding（用 computed style 取，不写死），
  // 保证首列左边距与末列右边距对称，且末列不出右侧边界。
  $effect(() => {
    const el = gridEl;
    if (!el) return;
    const cs = getComputedStyle(el);
    const l = parseFloat(cs.paddingLeft) || 0;
    const r = parseFloat(cs.paddingRight) || 0;
    gridPadX = l + r;
  });

  const contentW = $derived(Math.max(1, gridW - gridPadX));
  const cols = $derived(Math.max(2, Math.floor(contentW / GRID_COL_MIN)));
  const cellW = $derived(contentW / cols);
  const cardW = $derived(Math.min(216, Math.max(120, cellW - 12)));
  const coverH = $derived((cardW - 12) * 1.5);
  const cellH = $derived(coverH + 62);

  // 网格虚拟窗口（行）
  const gridRows = $derived(Math.ceil(total / cols));
  const gFirst = $derived(Math.max(0, Math.floor(scrollTop / cellH) - 3));
  const gLast = $derived(Math.min(gridRows, Math.ceil((scrollTop + gridH) / cellH) + 3));
  const gRows = $derived(Array.from({ length: Math.max(0, gLast - gFirst) }, (_, i) => gFirst + i));

  // 列表虚拟窗口（行）
  const lFirst = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - 8));
  const lLast = $derived(Math.min(total, Math.ceil((scrollTop + gridH) / ROW_H) + 8));
  const lRows = $derived(Array.from({ length: Math.max(0, lLast - lFirst) }, (_, i) => lFirst + i));

  function bookAt(abs: number): BookRow | undefined {
    return booksWindow[abs - rangeStart];
  }

  // rAF 节流滚动：回调里只更新一个标量，不做任何 O(n) 全量遍历。
  let scrollRaf = 0;
  let pendingTop = 0;
  function onScroll(e: Event) {
    pendingTop = (e.currentTarget as HTMLDivElement).scrollTop;
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      if (pendingTop !== scrollTop) scrollTop = pendingTop;
    });
  }

  async function loadRange(offset: number) {
    const seq = ++loadSeq;
    try {
      const r = await api.listBooksPage(query, sortKey, offset, FETCH);
      if (seq !== loadSeq) return;
      total = r.total;
      rangeStart = offset;
      booksWindow = r.rows;
    } catch (e) {
      if (seq === loadSeq) toast(String(e), "error");
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  /** 窗口未覆盖 [start,end) 时，按 FETCH 对齐拉取一段；已覆盖则 no-op（不会循环）。 */
  function ensureRange(start: number, end: number) {
    if (total === 0 || end <= start) return;
    const rangeEnd = rangeStart + booksWindow.length;
    if (booksWindow.length && rangeStart <= start && rangeEnd >= end) return;
    loadRange(Math.max(0, Math.floor(start / FETCH) * FETCH));
  }

  /** 全量重置：查询/排序变化或首次进入时调用（回到顶部并重拉窗口）。 */
  function reload() {
    loadSeq++;
    loading = true;
    total = 0;
    rangeStart = 0;
    booksWindow = [];
    resetScroll();
    loadRange(0);
  }

  /** 增量刷新：只重拉当前窗口与 total，不重置滚动、不拉全量（library-changed/meta 用）。 */
  async function reloadWindow() {
    const offset = Math.floor(Math.max(0, rangeStart) / FETCH) * FETCH;
    await loadRange(offset);
  }

  // 可见范围变化时按需拉取窗口（effect 内只发起异步请求，不阻塞主线程）。
  $effect(() => {
    if (total === 0) return;
    if (view === "grid") ensureRange(gFirst * cols, Math.min(total, gLast * cols));
    else ensureRange(lFirst, lLast);
  });

  // 搜索 250ms 防抖后再请求（输入不触发全量过滤）。
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const q = queryInput;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (q !== query) {
        query = q;
        reload();
      }
    }, 250);
    return () => clearTimeout(searchTimer);
  });

  function onSortChange() {
    saveSort();
    reload();
  }

  /** 就地修改窗口内某本书后，替换数组引用以触发重渲染（raw 数组不深层代理）。 */
  function touchWindow() {
    booksWindow = booksWindow.slice();
  }

  async function loadFolders() {
    try { folders = await api.listFolders(); } catch { /* ignore */ }
  }

  async function addFolder() {
    const p = await api.pickFolder("选择书籍文件夹");
    if (!p) return;
    try {
      await api.addFolder(p);
      folders = await api.listFolders();
      toast("已添加，正在扫描…");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function removeFolder(p: string) {
    const ok = await appConfirm(`从书库移除文件夹 "${p}"？\n（不会删除磁盘上的文件）`);
    if (!ok) return;
    await api.removeFolder(p);
    folders = await api.listFolders();
    await api.rescan();
  }

  function openBook(b: BookRow) {
    if (config.reader.multiWindow) {
      openBookInNewWindow({ id: b.id, title: b.title });
    } else {
      pendingOpen.bookId = b.id;
      pendingOpen.nonce++;
      ui.view = "reader";
    }
  }

  // 任务3：打开入口先查文件是否存在；不存在则弹确认刷新，绝不进入空白阅读。
  // 确认 → 刷新目录后停下（不继续打开、不触碰旧 reader 状态）；取消 → 直接返回。
  async function tryOpenBook(b: BookRow) {
    let exists = true;
    try {
      exists = await api.fileExists(b.path);
    } catch {
      exists = true; // 检查失败时不阻断（交由打开流程自行报错）
    }
    if (exists) {
      openBook(b);
      return;
    }
    const ok = await appConfirm(`文件已不存在：\n${b.path}\n\n是否刷新目录？`);
    if (!ok) return;
    try {
      const r = await api.refreshLibrary();
      toast(`书库已刷新：移除 ${r.removed} 本、新增 ${r.added} 本`);
    } catch (e) {
      toast(String(e), "error");
    }
    // 无论该书是否仍在索引，都停在这里（不进入空白阅读）
  }

  // 任务2：刷新按钮 → 按记录目录重扫（一次遍历，无实时监听）。
  // 16 万本下 IO 密集且耗时：立即给「进行中」提示并禁用按钮，完成后只增量刷新当前窗口。
  async function refreshAll() {
    if (refreshing) return;
    refreshing = true;
    toast("正在刷新书库…（大书库可能耗时较久）");
    try {
      const r = await api.refreshLibrary();
      toast(`书库已刷新：移除 ${r.removed} 本、新增 ${r.added} 本`);
      await reloadWindow();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      refreshing = false;
    }
  }

  async function openRandom() {
    try {
      const b = await api.randomBook();
      if (b) tryOpenBook(b);
      else toast("书库为空");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function deleteBook(b: BookRow, fromDisk: boolean) {
    const msg = fromDisk
      ? `删除《${b.title}》？\n\n将同时删除本地文件（移入回收站）：\n${b.path}`
      : `从书库中移除《${b.title}》？\n（本地文件保留）`;
    const ok = await appConfirm(msg);
    if (!ok) return;
    try {
      await api.deleteBooks([b.path], fromDisk);
      toast(fromDisk ? "已删除（移入回收站）" : "已从书库移除");
      await reloadWindow();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function setCover(b: BookRow) {
    const p = await api.pickFile("选择封面图片", [
      { name: "图片", exts: ["png", "jpg", "jpeg", "webp", "bmp"] },
    ]);
    if (!p) return;
    try {
      await api.setCustomCover(b.id, p);
      b.cover_path = `custom-${b.id}.png`;
      touchWindow();
      toast("封面已更新");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function reextract(b: BookRow) {
    try {
      const name = await api.reextractCover(b.id);
      b.cover_path = name;
      touchWindow();
      toast("封面已重新提取");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function openConvert(b: BookRow) {
    convertState.book = { id: b.id, title: b.title, path: b.path, format: b.format };
    convertState.open = true;
  }

  function fmtSize(n: number): string {
    if (n > 1048576) return (n / 1048576).toFixed(1) + " MB";
    if (n > 1024) return (n / 1024).toFixed(0) + " KB";
    return n + " B";
  }

  function fmtTime(ms: number): string {
    if (!ms) return "—";
    return new Date(ms).toLocaleDateString("zh-CN");
  }

  const BOOK_EXT = ["txt", "epub", "mobi", "azw3", "prc"];

  async function handleDrop(paths: string[]) {
    const valid = paths.filter((p) =>
      BOOK_EXT.includes((p.split(".").pop() || "").toLowerCase())
    );
    if (!valid.length) return;
    let last: BookRow | null = null;
    for (const p of valid) {
      try {
        last = await api.addDroppedBook(p);
      } catch (e) {
        toast(`添加失败 ${p.split(/[\\/]/).pop()}：${e}`, "error");
      }
    }
    await reloadWindow();
    if (last) {
      toast(`已添加《${last.title}》到书库（原文件未移动）`);
      await tryOpenBook(last);
    }
  }

  onMount(() => {
    loadFolders();
    reload();
    // 监听只增量刷新当前窗口与 total，不重置滚动、不拉全量。
    const u1 = listen("library-changed", () => reloadWindow());
    const u2 = listen("library-meta", () => reloadWindow());
    let u3: Promise<() => void> | null = null;
    import("@tauri-apps/api/webview")
      .then((m) => {
        u3 = m.getCurrentWebview().onDragDropEvent((ev) => {
          if (ev.payload.type === "drop" || ev.payload.type === "enter") {
            if (ev.payload.type === "drop") handleDrop(ev.payload.paths);
          }
        });
      })
      .catch(() => {});
    return () => {
      u1.then((f) => f());
      u2.then((f) => f());
      u3?.then((f) => f());
    };
  });
</script>

<div class="library" onpointerdown={onPtrDown} onpointerup={onPtrUp}>
  <div class="toolbar">
    <span class="logo">FlowRead</span>
    <button onclick={addFolder}>＋ 添加文件夹</button>
    <button onclick={refreshAll} disabled={refreshing}>{refreshing ? "刷新中…" : "⟳ 刷新"}</button>
    <input class="search" placeholder="搜索书名 / 作者…" bind:value={queryInput} />
    <select bind:value={sortKey} onchange={onSortChange}>
      <option value="lastread">按最近阅读</option>
      <option value="added">按添加时间</option>
      <option value="title">按书名</option>
      <option value="recent">按文件时间</option>
    </select>
    <div class="spacer"></div>
    <button class="icon" title="WebDAV 文件管理（向左滑也可打开）" onclick={() => (ui.webdavOpen = true)}>☁</button>
    <button class="icon" title="随机打开一本" onclick={openRandom}>🎲</button>
    <button class="icon" title="全文搜索" onclick={() => (ui.searchOpen = true)}>🔍</button>
    <button class="icon" title="阅读统计" onclick={() => (ui.statsOpen = true)}>📈</button>
    <button class="icon" title="设置" onclick={() => (ui.settingsOpen = true)}>⚙</button>
  </div>

  {#if folders.length}
    <div class="folders">
      {#each folders as f (f)}
        <span class="folder-chip" title="点击 × 移除">
          📁 {f}
          <button class="x" onclick={() => removeFolder(f)}>×</button>
        </span>
      {/each}
    </div>
  {/if}

  <div
    class="grid"
    class:list-view={view === "list"}
    bind:this={gridEl}
    bind:clientHeight={gridH}
    bind:clientWidth={gridW}
    onscroll={onScroll}
  >
    {#if loading && total === 0}
      <div class="hint">加载中…</div>
    {:else if total === 0}
      <div class="hint">
        {query.trim() ? "无匹配结果" : "书库为空\n点击「添加文件夹」导入本地书籍\n支持 TXT / EPUB / MOBI / AZW3"}
      </div>
    {:else if view === "grid"}
      <div class="grid-inner" style="height:{gridRows * cellH}px;">
        {#each gRows as r (r)}
          {#each Array(cols) as _, ci (ci)}
            {@const abs = r * cols + ci}
            {#if abs < total}
              {@const b = bookAt(abs)}
              {#if b}
                <div
                  class="card"
                  style="left:{ci * cellW + (cellW - cardW) / 2}px; top:{r * cellH}px; width:{cardW}px;"
                  ondblclick={() => tryOpenBook(b)}
                  oncontextmenu={(e) => openCtx(e, b)}
                >
                  <div class="cover" style="height:{coverH}px;">
                    <img src={coverUrl(b)} alt="" loading="lazy" decoding="async" draggable="false" />
                    <span class="fmt">{b.format.toUpperCase()}</span>
                  </div>
                  <div class="meta">
                    <div class="t" title={b.title}>{b.title}</div>
                    <div class="a">{b.author}</div>
                  </div>
                </div>
              {/if}
            {/if}
          {/each}
        {/each}
      </div>
    {:else}
      <div class="list-inner" style="height:{total * ROW_H}px;">
        {#each lRows as i (i)}
          {@const b = bookAt(i)}
          {#if b}
            <div
              class="row"
              style="top:{i * ROW_H}px"
              ondblclick={() => tryOpenBook(b)}
              oncontextmenu={(e) => openCtx(e, b)}
            >
              <img class="row-cover" src={coverUrl(b)} alt="" loading="lazy" decoding="async" />
              <div class="row-meta">
                <div class="t">{b.title}</div>
                <div class="a">{b.author}</div>
              </div>
              <div class="row-size">{fmtSize(b.size)}</div>
              <div class="row-date">{b.last_read ? "读于 " + fmtTime(b.last_read) : fmtTime(b.added_at)}</div>
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  </div>

  <div class="statusbar">
    <span>{total} 本书</span>
    <span class="build">FlowRead {APP_BUILD}</span>
    <div class="spacer"></div>
    <button class="icon" title="网格视图" onclick={() => setView("grid")}>▦</button>
    <button class="icon" title="列表视图" onclick={() => setView("list")}>≡</button>
  </div>

  {#if ctxMenu}
    <div class="ctx-backdrop" onclick={() => (ctxMenu = null)} oncontextmenu={(e) => { e.preventDefault(); ctxMenu = null; }}>
      <div class="ctx" bind:this={ctxEl} style="left:{ctxPos.left}px; top:{ctxPos.top}px;">
        <button onclick={() => { tryOpenBook(ctxMenu!.book); ctxMenu = null; }}>📖 打开</button>
        <button onclick={() => { detailBook = ctxMenu!.book; ctxMenu = null; }}>ℹ️ 详细信息</button>
        {#if ctxMenu.book.format === "txt"}
          <button onclick={() => { openConvert(ctxMenu!.book); ctxMenu = null; }}>⇄ 转换为 EPUB</button>
        {/if}
        <button onclick={() => { setCover(ctxMenu!.book); ctxMenu = null; }}>🖼 设置封面</button>
        <button onclick={() => { reextract(ctxMenu!.book); ctxMenu = null; }}>🔄 重新提取封面</button>
        <button onclick={() => { api.revealInExplorer(ctxMenu!.book.path); ctxMenu = null; }}>📂 所在文件夹</button>
        <button class="danger" onclick={() => { deleteBook(ctxMenu!.book, false); ctxMenu = null; }}>↩ 仅移出书库</button>
        <button class="danger" onclick={() => { deleteBook(ctxMenu!.book, true); ctxMenu = null; }}>🗑 删除（含本地文件）</button>
      </div>
    </div>
  {/if}

  {#if detailBook}
    <BookDetailModal book={detailBook} onclose={() => { detailBook = null; touchWindow(); }} />
  {/if}
</div>

<style>
  .library {
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }

  .logo {
    font-weight: 700;
    font-size: 16px;
    color: var(--accent);
    margin-right: 8px;
    letter-spacing: 0.5px;
  }

  .search {
    width: 220px;
  }

  .spacer { flex: 1; }

  .folders {
    display: flex;
    gap: 8px;
    padding: 8px 16px 0;
    flex-wrap: wrap;
  }

  .folder-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--panel2);
    border: 1px solid var(--line);
    border-radius: 20px;
    padding: 3px 6px 3px 12px;
    font-size: 12px;
    color: var(--fg2);
    max-width: 420px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .folder-chip .x {
    border: none;
    background: transparent;
    padding: 0 4px;
    cursor: pointer;
    color: var(--fg2);
  }

  .grid {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    position: relative;
  }

  .grid-inner {
    position: relative;
    width: 100%;
    will-change: transform;
  }

  .card {
    position: absolute;
    cursor: pointer;
    border-radius: var(--radius);
    padding: 6px;
    transition: background 0.15s, transform 0.15s ease, box-shadow 0.18s ease;
  }

  .card:hover {
    background: var(--panel2);
    transform: translateY(-3px);
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.14);
  }

  .card:active {
    transform: translateY(0);
  }

  .cover {
    position: relative;
    width: 100%;
    border-radius: 8px;
    overflow: hidden;
    box-shadow: 0 3px 10px rgba(0, 0, 0, 0.18);
    background: var(--panel2);
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .fmt {
    position: absolute;
    right: 5px;
    top: 5px;
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 4px;
    letter-spacing: 0.5px;
  }

  .meta { padding: 7px 2px 2px; }

  .meta .t {
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta .a {
    font-size: 11px;
    color: var(--fg2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-top: 2px;
  }

  .list-inner {
    position: relative;
    width: 100%;
    will-change: transform;
  }

  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 64px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 12px;
    border-radius: 8px;
    cursor: pointer;
  }

  .row:hover { background: var(--panel2); }

  .row-cover {
    width: 34px;
    height: 48px;
    object-fit: cover;
    border-radius: 4px;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2);
  }

  .row-meta { flex: 1; min-width: 0; }
  .row-meta .t { font-size: 14px; }
  .row-meta .a { font-size: 12px; color: var(--fg2); }
  .row-size, .row-date { color: var(--fg2); font-size: 12px; width: 90px; text-align: right; }

  .hint {
    text-align: center;
    padding: 80px 20px;
    color: var(--fg2);
    white-space: pre-line;
    line-height: 2;
  }

  .statusbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 16px;
    border-top: 1px solid var(--line);
    color: var(--fg2);
    font-size: 12px;
    background: var(--panel);
  }

  .statusbar .build {
    font-size: 11px;
    opacity: 0.55;
  }

  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
  }

  .ctx {
    position: fixed;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
    padding: 5px;
    min-width: 190px;
    z-index: 101;
  }

  .ctx button {
    border: none;
    background: transparent;
    text-align: left;
    padding: 8px 14px;
    border-radius: 6px;
    font-size: 13px;
  }

  .ctx button:hover { background: var(--panel2); }
  .ctx button.danger { color: var(--danger); }
</style>
