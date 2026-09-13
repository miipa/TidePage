<script lang="ts">
  import { onMount } from "svelte";
  import { api, coverUrl } from "../lib/api";
  import { appConfirm, config, convertState, openBookInNewWindow, pendingOpen, toast, ui, APP_BUILD } from "../lib/stores.svelte";
  import { listen } from "@tauri-apps/api/event";
  import type { BookRow } from "../lib/types";

  let books: BookRow[] = $state([]);
  let folders: string[] = $state([]);
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

  function setView(v: "grid" | "list") {
    view = v;
    try { localStorage.setItem("flowread.lib.view", v); } catch { /* ignore */ }
  }

  let gridEl: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let gridH = $state(800);
  let gridW = $state(1200);
  let ctxMenu = $state<{ x: number; y: number; book: BookRow } | null>(null);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let arr = q
      ? books.filter((b) => b.title.toLowerCase().includes(q) || b.author.toLowerCase().includes(q))
      : books;
    if (sortKey === "lastread") arr = [...arr].sort((a, b) => (b.last_read || 0) - (a.last_read || 0));
    else if (sortKey === "title") arr = [...arr].sort((a, b) => a.title.localeCompare(b.title, "zh"));
    else if (sortKey === "recent") arr = [...arr].sort((a, b) => b.mtime - a.mtime);
    return arr;
  });

  const PAD = 16;
  const cols = $derived(Math.max(2, Math.round((gridW - PAD * 2) / 168)));
  const cellW = $derived((gridW - PAD * 2) / cols);
  const cardW = $derived(Math.min(216, Math.max(120, cellW - 10)));
  const coverH = $derived((cardW - 12) * 1.5);
  const cellH = $derived(coverH + 62);
  const rows = $derived(Math.ceil(filtered.length / cols));
  const firstRow = $derived(Math.max(0, Math.floor(scrollTop / cellH) - 3));
  const lastRow = $derived(Math.min(rows, Math.ceil((scrollTop + gridH) / cellH) + 3));
  const visible = $derived(
    Array.from({ length: Math.max(0, lastRow - firstRow) }, (_, i) => firstRow + i)
  );

  async function refresh() {
    try {
      const [b, f] = await Promise.all([api.listBooks(), api.listFolders()]);
      books = b;
      folders = f;
      loading = false;
    } catch (e) {
      toast(String(e), "error");
      loading = false;
    }
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

  // 任务2：刷新按钮 → 按记录目录重扫（一次遍历，无实时监听）
  async function refreshAll() {
    try {
      const r = await api.refreshLibrary();
      toast(`书库已刷新：移除 ${r.removed} 本、新增 ${r.added} 本`);
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function openRandom() {
    try {
      const b = await api.randomBook();
      if (b) openBook(b);
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
      refresh();
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
      toast("封面已更新");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function reextract(b: BookRow) {
    try {
      const name = await api.reextractCover(b.id);
      b.cover_path = name;
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
    await refresh();
    if (last) {
      toast(`已添加《${last.title}》到书库（原文件未移动）`);
      openBook(last);
    }
  }

  onMount(() => {
    refresh();
    const u1 = listen("library-changed", () => refresh());
    const u2 = listen("library-meta", () => refresh());
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
    <button onclick={refreshAll}>⟳ 刷新</button>
    <input class="search" placeholder="搜索书名 / 作者…" bind:value={query} />
    <select bind:value={sortKey} onchange={saveSort}>
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
    onscroll={(e) => (scrollTop = (e.currentTarget as HTMLDivElement).scrollTop)}
  >
    {#if loading}
      <div class="hint">加载中…</div>
    {:else if !filtered.length}
      <div class="hint">
        {books.length ? "无匹配结果" : "书库为空\n点击「添加文件夹」导入本地书籍\n支持 TXT / EPUB / MOBI / AZW3"}
      </div>
    {:else if view === "grid"}
      <div class="grid-inner" style="height:{rows * cellH}px; position:relative;">
        {#each visible as r (r)}
          {#each filtered.slice(r * cols, r * cols + cols) as b, ci (b.id)}
            <div
              class="card"
              style="left:{PAD + ci * cellW + (cellW - cardW) / 2}px; top:{r * cellH}px; width:{cardW}px;"
              ondblclick={() => openBook(b)}
              oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, book: b }; }}
            >
              <div class="cover" style="height:{coverH}px;">
                <img src={coverUrl(b)} alt="" loading="lazy" draggable="false" />
                <span class="fmt">{b.format.toUpperCase()}</span>
              </div>
              <div class="meta">
                <div class="t" title={b.title}>{b.title}</div>
                <div class="a">{b.author}</div>
              </div>
            </div>
          {/each}
        {/each}
      </div>
    {:else}
      <div class="list-inner">
        {#each filtered as b (b.id)}
          <div class="row" ondblclick={() => openBook(b)} oncontextmenu={(e) => { e.preventDefault(); ctxMenu = { x: e.clientX, y: e.clientY, book: b }; }}>
            <img class="row-cover" src={coverUrl(b)} alt="" loading="lazy" />
            <div class="row-meta">
              <div class="t">{b.title}</div>
              <div class="a">{b.author}</div>
            </div>
            <div class="row-size">{fmtSize(b.size)}</div>
            <div class="row-date">{b.last_read ? "读于 " + fmtTime(b.last_read) : fmtTime(b.added_at)}</div>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="statusbar">
    <span>{books.length} 本书</span>
    <span class="build">FlowRead {APP_BUILD}</span>
    <div class="spacer"></div>
    <button class="icon" title="网格视图" onclick={() => setView("grid")}>▦</button>
    <button class="icon" title="列表视图" onclick={() => setView("list")}>≡</button>
  </div>

  {#if ctxMenu}
    <div class="ctx-backdrop" onclick={() => (ctxMenu = null)} oncontextmenu={(e) => { e.preventDefault(); ctxMenu = null; }}>
      <div class="ctx" style="left:{Math.min(ctxMenu.x, gridW - 200)}px; top:{ctxMenu.y}px;">
        <button onclick={() => { openBook(ctxMenu!.book); ctxMenu = null; }}>📖 打开</button>
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
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .row {
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
