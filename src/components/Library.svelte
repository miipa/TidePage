<script lang="ts">
  import { onMount } from "svelte";
  import { api, coverUrl, bumpCoverBust } from "../lib/api";
  import { appConfirm, appChoose, config, convertState, openBookInNewWindow, pendingOpen, saveConfig, toast, ui, APP_VERSION, LIBRARY_TOPBAR_IDS, type LibraryTopbarId } from "../lib/stores.svelte";
  import { dragSort } from "../lib/dragSort";
  import { listen } from "@tauri-apps/api/event";
  import type { Icon as TablerIcon } from "@tabler/icons-svelte";
  import {
    IconChartBar,
    IconCloud,
    IconDice,
    IconNotebook,
    IconSettings,
  } from "@tabler/icons-svelte";
  import BookDetailModal from "./BookDetailModal.svelte";
  import type { BookRow } from "../lib/types";

  // ===== 超大书库（实测 16 万本）：秩可寻址分页（id 有序索引 + 有界窗口取行）=====
  // 常驻内存只有两样（契约）：
  //   1) idIndex: BigInt64Array —— 当前 (query|sort) 下的有序 books.id（rank→id 为 O(1)）；
  //   2) rowMap: Map<id, BookRow|null> —— 仅当前可见窗口 ± 缓冲的行（有界前视环，会剪枝）。
  // 禁止保留 N 长的行对象数组；禁止在 N 长数组上 filter/slice。
  const GRID_COL_MIN = 168; // 网格最小列宽
  const ROW_H = 66;         // 列表行步长（行高 64 + 间距 2，与 .row / .list-inner CSS 一致）
  const WIN_PAD = 200;      // 窗口前后各多取的缓冲行数（减少连续滚动的取数次数）
  const ROWMAP_CAP = 3000;  // rowMap 常驻上限（超过即按当前窗口剪枝，保证内存有界）
  const IDS_CHUNK = 4096;   // 命令 2 单批 ids 上限（与后端 too_many_ids 对齐）

  let total = $state(0);
  // idIndex 为普通（非响应式）变量：整块替换，避免 16 万元素的深层代理开销。
  let idIndex: BigInt64Array | null = null;
  // rowMap 为普通 Map（非响应式）；内容变化时 bump winVersion 触发模板重算 bookAt。
  let rowMap = new Map<number, BookRow | null>();
  let winVersion = $state(0);
  // generation：每次「重建索引 / 换查询 / 换排序」自增；异步取数回来若不等即丢弃（STALE）。
  let generation = 0;
  // 是否处于「有序索引」档：query 为空走索引档；query 非空（搜索）回落 list_books_page（OFFSET）。
  // 另：N 超过后端上限（order_index_too_large）时也回落。
  let indexMode = $state(true);
  // ---- 搜索/回落档（query!=='' 或 N 超限）：沿用 OFFSET 窗口 ----
  const FETCH = 500;
  let booksWindow: BookRow[] = $state.raw([]);
  let rangeStart = $state(0);

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
  /** 刷新扫描进度（由后端 refresh-progress 事件驱动）。 */
  let refreshProgress = $state<{ phase: string; done: number; total: number } | null>(null);
  /** 封面/元数据后台生成进度（由后端 meta-progress 事件驱动；不阻塞其它操作）。 */
  let metaProgress = $state<{ phase: string; done: number; total: number } | null>(null);
  /** 让最后一帧进度（N/N）短暂可见后再隐藏，避免一闪而过看不到。 */
  let metaHideTimer: ReturnType<typeof setTimeout> | undefined;
  /** 长按刷新弹出的菜单（普通点击=只读书籍；长按=可刷新封面）。 */
  let refreshMenu = $state(false);

  // 书库顶栏右侧图标按钮注册表（可拖拽排序；顺序持久化到 config.library.topbarOrder）。
  interface LibBtn {
    id: LibraryTopbarId;
    label: string;
    title: string;
    icon: TablerIcon;
    run: () => void;
  }
  const LIB_BTNS: LibBtn[] = [
    { id: "webdav", label: "WebDAV 文件管理", title: "WebDAV 文件管理（向左滑也可打开）", icon: IconCloud, run: () => (ui.webdavOpen = true) },
    { id: "random", label: "随机打开", title: "随机打开一本", icon: IconDice, run: () => openRandom() },
    // 顶栏不再放「全文搜索」按钮（阅读界面仍有全文搜索）；原位置给「笔记」，进入与书库同款的笔记书籍视图。
    { id: "notes", label: "笔记", title: "笔记（有划线或感想的书籍，按书汇总）", icon: IconNotebook, run: () => (ui.notesOpen = true) },
    { id: "stats", label: "阅读统计", title: "阅读统计", icon: IconChartBar, run: () => (ui.statsOpen = true) },
    { id: "settings", label: "设置", title: "设置", icon: IconSettings, run: () => (ui.settingsOpen = true) },
  ];
  const libBtn = (id: string): LibBtn | undefined => LIB_BTNS.find((b) => b.id === id);
  const orderedLibBtns = $derived(
    config.library.topbarOrder.map((id) => libBtn(id)).filter((b): b is LibBtn => !!b),
  );
  /** 真实拖拽排序：把 from 插入到 target 之前/之后（顺序持久化到 config.library.topbarOrder）。 */
  function libDrop(from: string, target: string, place: "before" | "after") {
    if (!from || from === target) return;
    const arr = [...config.library.topbarOrder];
    const fi = arr.indexOf(from);
    if (fi < 0) return;
    arr.splice(fi, 1);
    const ti = arr.indexOf(target);
    if (ti < 0) return;
    arr.splice(place === "after" ? ti + 1 : ti, 0, from);
    config.library.topbarOrder = arr;
    saveConfig();
  }

  const refreshPhaseText = (p: { phase: string; done: number; total: number }): string => {
    if (p.phase === "check") return `对账缺失 ${p.done}/${p.total}`;
    if (p.phase === "scan") return `扫描新增 ${p.done}/${p.total}`;
    return "完成";
  };
  const refreshPct = $derived(
    refreshProgress && refreshProgress.total > 0
      ? Math.round((refreshProgress.done / refreshProgress.total) * 100)
      : 0,
  );
  const metaPct = $derived(
    metaProgress && metaProgress.total > 0
      ? Math.min(100, Math.round((metaProgress.done / metaProgress.total) * 100))
      : 0,
  );

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
  // 每次 gridW 变化（含窗口 resize）都重量一次，并取与设计值 GRID_PAD_MIN 的较大者：
  // padding 若在极端时序下读成 0，contentW 会虚高 → 末列溢出到 padding 外、被窗口右边缘裁掉；
  // 取 max 只会让内容区更窄，永不溢出。
  const GRID_PAD_MIN = 40; // 与 .grid { padding: 16px 20px 32px } 左右之和一致
  function gridPadXNow(): number {
    const el = gridEl;
    if (!el) return GRID_PAD_MIN;
    const cs = getComputedStyle(el);
    const l = parseFloat(cs.paddingLeft) || 0;
    const r = parseFloat(cs.paddingRight) || 0;
    return Math.max(GRID_PAD_MIN, l + r);
  }

  // 卡片几何：必须与 CSS 完全一致，否则行高预算错误会导致卡片被下一行遮挡。
  // CARD_PAD = .card padding；COVER_INSET = .card 内边距×2（封面可用宽度 = cardW - COVER_INSET）。
  const CARD_PAD = 8;                 // 与 .card { padding: 8px } 一致
  const COVER_INSET = CARD_PAD * 2;   // 16
  const CARD_GAP = 12;                // 列间水平留白（与 .grid-row column-gap 一致）
  const CARD_MAX = 216;               // 卡片宽度上限（超宽窗口下不无限拉大）
  const META_H = 52;                  // .meta 固定块高（标签行 + 作者行 + 上下内边距）
  const CARD_VGAP = 12;               // 行间垂直留白

  const contentW = $derived(Math.max(1, gridW - gridPadXNow()));
  // 列数 = 在 contentW 里能放下多少个「最小列宽 + 列间距」（与 CSS auto-fill 语义一致）；最少 1 列。
  const cols = $derived(
    Math.max(1, Math.floor((contentW + CARD_GAP) / (GRID_COL_MIN + CARD_GAP))),
  );
  // 每列实际宽度：由 CSS `repeat(cols, minmax(0,1fr))` + column-gap 精确均分 contentW，
  // 这里同式计算只为推导行高预算（封面高度）。列宽用 minmax(0,1fr) 永不溢出容器。
  const trackW = $derived((contentW - (cols - 1) * CARD_GAP) / cols);
  // 卡片显示宽度：不超过列宽，也不超过 216 上限（超宽时卡片在列内居中）。
  const cardW = $derived(Math.min(trackW, CARD_MAX));
  // 封面高度按「卡片内宽」算 3:2，保证行高预算与渲染一致。
  const coverH = $derived(Math.max(1, Math.round((cardW - COVER_INSET) * 1.5)));
  // 行高 = 卡片内边距×2 + 封面 + 标签块 + 行间距。
  const cellH = $derived(CARD_PAD * 2 + coverH + META_H + CARD_VGAP);

  // 网格虚拟窗口（行）
  const gridRows = $derived(Math.ceil(total / cols));
  // 窗口 resize 会改 cellH（卡片变宽→封面变高），scrollTop 可能短暂落在内容之外；
  // 首行钳到「最后一行之内」，避免虚窗口整体跑到末尾之外（整屏空白）。
  const gFirst = $derived(
    Math.max(0, Math.min(Math.max(0, gridRows - 1), Math.floor(scrollTop / cellH) - 3)),
  );
  const gLast = $derived(Math.min(gridRows, Math.ceil((scrollTop + gridH) / cellH) + 3));
  const gRows = $derived(Array.from({ length: Math.max(0, gLast - gFirst) }, (_, i) => gFirst + i));

  // 列表虚拟窗口（行）
  const lFirst = $derived(
    Math.max(0, Math.min(Math.max(0, total - 1), Math.floor(scrollTop / ROW_H) - 8)),
  );
  const lLast = $derived(Math.min(total, Math.ceil((scrollTop + gridH) / ROW_H) + 8));
  const lRows = $derived(Array.from({ length: Math.max(0, lLast - lFirst) }, (_, i) => lFirst + i));

  function bookAt(abs: number): BookRow | undefined {
    // 读 winVersion 建立响应式依赖：窗口数据到达后模板中的 bookAt 会重算。
    void winVersion;
    if (indexMode) {
      if (!idIndex || abs < 0 || abs >= idIndex.length) return undefined;
      const id = Number(idIndex[abs]);
      const r = rowMap.get(id);
      return r ?? undefined; // 未加载 / null 占位 → 渲染空槽，秩仍对齐
    }
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
    const seq = ++generation;
    try {
      const r = await api.listBooksPage(query, sortKey, offset, FETCH);
      if (seq !== generation) return;
      total = r.total;
      rangeStart = offset;
      booksWindow = r.rows;
    } catch (e) {
      if (seq === generation) toast(String(e), "error");
    } finally {
      if (seq === generation) loading = false;
    }
  }

  /** 回落档（搜索/超限）：窗口未覆盖 [start,end) 时按 FETCH 对齐拉取一段。 */
  function ensureRange(start: number, end: number) {
    if (total === 0 || end <= start) return;
    const rangeEnd = rangeStart + booksWindow.length;
    if (booksWindow.length && rangeStart <= start && rangeEnd >= end) return;
    loadRange(Math.max(0, Math.floor(start / FETCH) * FETCH));
  }

  // ===== 索引档：窗口取行 + 剪枝 + 重建 =====

  /** 剪枝 rowMap：超过上限时，只保留当前窗口 ± WIN_PAD 内的 id（有界前视环）。 */
  function pruneRowMap(s: number, e: number) {
    if (!idIndex || rowMap.size <= ROWMAP_CAP) return;
    const from = Math.max(0, s - WIN_PAD);
    const to = Math.min(idIndex.length, e + WIN_PAD);
    const keep = new Set<number>();
    for (let i = from; i < to; i++) keep.add(Number(idIndex[i]));
    for (const id of rowMap.keys()) if (!keep.has(id)) rowMap.delete(id);
  }

  /** 索引档：确保可见区间 [first,last) 的行已在 rowMap（缺哪取哪，分批 ≤ IDS_CHUNK）。 */
  async function ensureWindowIndex(first: number, last: number) {
    if (!idIndex) return;
    const len = idIndex.length;
    if (len === 0) return;
    const s = Math.max(0, Math.min(first, len));
    const e = Math.max(0, Math.min(last, len));
    if (e <= s) return;
    const need: number[] = [];
    for (let i = s; i < e; i++) {
      const id = Number(idIndex[i]);
      if (!rowMap.has(id)) need.push(id);
    }
    if (!need.length) return;
    const seq = generation;
    let nulls = 0;
    for (let i = 0; i < need.length; i += IDS_CHUNK) {
      const chunk = need.slice(i, i + IDS_CHUNK);
      let rows: (BookRow | null)[];
      try {
        rows = await api.listBooksByIds(chunk);
      } catch (err) {
        if (seq === generation) toast(String(err), "error");
        return;
      }
      if (seq !== generation) return; // STALE：期间发生了重建/换档，丢弃
      for (let j = 0; j < chunk.length; j++) {
        const v = rows[j] ?? null;
        rowMap.set(chunk[j], v);
        if (v === null) nulls++;
      }
    }
    pruneRowMap(s, e);
    winVersion++;
    // ❌ 死 id 收敛：idIndex 是快照，删书后其 rank 会长期留 null（空行 + 滚动高度偏大 +
    // 为死 id 白请求封面 IO）。窗口内 null 过多或连续多窗出现 → 防抖重建（按锚点重定位）。
    if (nulls > 0) {
      nullStreak++;
      if (nulls >= 8 || nullStreak >= 3) scheduleNullRebuild();
    } else {
      nullStreak = 0;
    }
  }

  // 死 id 收敛：防抖 1s 重建 idIndex（length 自然收敛，滚动高度/「共 N 本」随之校正）。
  let nullStreak = 0;
  let nullRebuildTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleNullRebuild() {
    if (nullRebuildTimer) return;
    nullRebuildTimer = setTimeout(() => {
      nullRebuildTimer = undefined;
      if (indexMode) void rebuildIndex(topAnchorId());
    }, 1000);
  }

  /** 索引档：强制刷新当前可见窗口的行（覆盖写，用于 cover/meta 更新，不重建 idIndex）。 */
  async function refreshVisibleIndex() {
    if (!indexMode || !idIndex) return;
    const len = idIndex.length;
    if (len === 0) return;
    const start = view === "grid" ? gFirst * cols : lFirst;
    const end = view === "grid" ? Math.min(len, gLast * cols) : Math.min(len, lLast);
    const s = Math.max(0, Math.min(start, len));
    const e = Math.max(0, Math.min(end, len));
    if (e <= s) return;
    const ids: number[] = [];
    for (let i = s; i < e; i++) ids.push(Number(idIndex[i]));
    const seq = generation;
    for (let i = 0; i < ids.length; i += IDS_CHUNK) {
      const chunk = ids.slice(i, i + IDS_CHUNK);
      let rows: (BookRow | null)[];
      try {
        rows = await api.listBooksByIds(chunk);
      } catch {
        return;
      }
      if (seq !== generation) return;
      for (let j = 0; j < chunk.length; j++) rowMap.set(chunk[j], rows[j] ?? null);
    }
    winVersion++;
  }

  /** 当前顶部可见行对应的 id（重建索引后据此重定位，保持视线不跳）。 */
  function topAnchorId(): number | undefined {
    if (!indexMode || !idIndex || idIndex.length === 0) return undefined;
    const step = view === "grid" ? cellH : ROW_H;
    const topRow = Math.floor(scrollTop / Math.max(1, step));
    const rank = view === "grid" ? topRow * cols : topRow;
    const r = Math.max(0, Math.min(rank, idIndex.length - 1));
    return Number(idIndex[r]);
  }

  /** 把某个 rank 滚到顶部（clamp 到 [0,len-1]）。 */
  function scrollToRank(rank: number) {
    if (!gridEl || !idIndex) return;
    const len = idIndex.length;
    const r = Math.max(0, Math.min(rank, Math.max(0, len - 1)));
    const step = view === "grid" ? cellH : ROW_H;
    const row = view === "grid" ? Math.floor(r / Math.max(1, cols)) : r;
    const top = row * step;
    gridEl.scrollTop = top;
    scrollTop = top;
  }

  /** 重建有序 id 索引（换排序 / 换查询 / 增删书 / 首次进入）。anchorId 存在则重定位，否则回顶。 */
  async function rebuildIndex(anchorId?: number) {
    const seq = ++generation;
    loading = true;
    try {
      const buf = await api.listOrderIndex(sortKey, query);
      if (seq !== generation) return;
      idIndex = new BigInt64Array(buf);
      total = idIndex.length;
      rowMap = new Map();
      indexMode = true;
      nullStreak = 0;
      clearTimeout(nullRebuildTimer);
      nullRebuildTimer = undefined;
      winVersion++;
      // 重定位：锚点 id 用一次线性扫定位（禁止建 id→rank 全量 Map）。找不到则回顶。
      if (anchorId != null && idIndex.length > 0) {
        let rank = -1;
        for (let i = 0; i < idIndex.length; i++) {
          if (Number(idIndex[i]) === anchorId) { rank = i; break; }
        }
        if (rank >= 0) scrollToRank(rank);
        else resetScroll();
      } else {
        resetScroll();
      }
    } catch (err) {
      const msg = String(err);
      if (seq !== generation) return;
      if (msg.includes("order_index_too_large")) {
        // N 超过后端上限 → 切 keyset/OFFSET 回落档（本轮以 list_books_page 承接）。
        indexMode = false;
        idIndex = null;
        rowMap = new Map();
        await loadRange(0);
      } else {
        toast(msg, "error");
      }
    } finally {
      if (seq === generation) loading = false;
    }
  }

  /** 全量重置：查询/排序变化或首次进入时调用。
   *  搜索档（query 非空）也走秩索引：list_order_index 带 query → 同样 O(W) 跳秩（不再回落 OFFSET）。
   *  仅当结果超后端上限（order_index_too_large）时，rebuildIndex 内部才切到 list_books_page 回落。 */
  function reload() {
    loading = true;
    total = 0;
    resetScroll();
    void rebuildIndex();
  }

  /** 增量刷新当前窗口（library-changed / meta / cover 用）：
   *  索引档 —— 增删书会改变 N 与顺序，需按锚点重建 idIndex；
   *  回落档 —— 重拉当前 OFFSET 窗口与 total。 */
  async function reloadWindow() {
    if (indexMode) {
      await rebuildIndex(topAnchorId());
    } else {
      const offset = Math.floor(Math.max(0, rangeStart) / FETCH) * FETCH;
      await loadRange(offset);
    }
  }

  // C1：路由重新进入本组件时 onMount 会重跑 reload()（本应用书库/阅读是 {#if} 切换，非 keep-alive）。
  // 另外响应全局 cover-updated 信号（ui.coverTick，来自 +page 根组件监听）重拉当前窗口，
  // 令扫描/打开插队扫出的新封面即时显示。queueMicrotask 逃离响应式追踪，避免误依赖内部变量成环。
  let coverTickSeen = ui.coverTick;
  $effect(() => {
    const t = ui.coverTick;
    if (t === coverTickSeen) return;
    coverTickSeen = t;
    // 封面更新不改变顺序：索引档只刷新可见窗口的行（不重建 idIndex）；回落档重拉窗口。
    queueMicrotask(() => {
      if (indexMode) void refreshVisibleIndex();
      else void reloadWindow();
    });
  });

  // 可见范围变化时按需拉取窗口（effect 内只发起异步请求，不阻塞主线程）。
  $effect(() => {
    if (total === 0) return;
    const first = view === "grid" ? gFirst * cols : lFirst;
    const last = view === "grid" ? Math.min(total, gLast * cols) : lLast;
    if (indexMode) ensureWindowIndex(first, last);
    else ensureRange(first, last);
  });

  // 方案 A：视口 index 区间变化时，把可见的未扫描书籍提升为高优先级扫描目标。
  // 200ms 防抖；依赖仅为视口 index 区间（不依赖 booksWindow/rowMap，避免回环）。
  // ids 数量 = 渲染窗口行数（O(W)），与滚动位置无关（约束 2）。
  let prioTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    if (total === 0) return;
    const start = view === "grid" ? gFirst * cols : lFirst;
    const end = view === "grid" ? gLast * cols : lLast;
    const s = Math.max(0, start);
    const e = Math.min(total, end);
    const mode = indexMode;
    clearTimeout(prioTimer);
    prioTimer = setTimeout(() => {
      const ids: number[] = [];
      if (mode && idIndex) {
        // 索引档：直接用 idIndex 的可见区间（后端按 meta_done!=1 过滤，无需前端持有行）。
        for (let i = s; i < e && i < idIndex.length; i++) ids.push(Number(idIndex[i]));
      } else {
        for (let i = s; i < e; i++) {
          const b = bookAt(i);
          if (b && b.meta_done !== 1) ids.push(b.id);
        }
      }
      if (ids.length) api.prioritizeCovers(ids).catch(() => {});
    }, 200);
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

  /** 就地修改窗口内某本书后触发重渲染。索引档：bump winVersion；回落档：替换 raw 数组引用。 */
  function touchWindow() {
    if (indexMode) winVersion++;
    else booksWindow = booksWindow.slice();
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
    const ok = await appConfirm(`从书库移除文件夹「${p}」？\n不会删除磁盘上的文件。`);
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
      toast(`书库已刷新：新增 ${r.added} 本、减少 ${r.removed} 本`);
    } catch (e) {
      toast(String(e), "error");
    }
    // 无论该书是否仍在索引，都停在这里（不进入空白阅读）
  }

  // 任务2：刷新按钮 → 按记录目录重扫（一次遍历，无实时监听）。
  // 16 万本下 IO 密集且耗时：立即给「进行中」提示并禁用按钮，完成后只增量刷新当前窗口。
  // 注意：普通刷新**只读书籍**（对账增删），不重算封面；重算封面走「长按 → 刷新封面」。
  async function refreshAll() {
    if (refreshing) return;
    refreshing = true;
    refreshProgress = { phase: "check", done: 0, total: 0 };
    try {
      const r = await api.refreshLibrary();
      toast(`书库已刷新：新增 ${r.added} 本、减少 ${r.removed} 本`);
      await reloadWindow();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      refreshing = false;
      refreshProgress = null;
    }
  }

  // 刷新按钮：单击=读书籍；长按（550ms）=弹出「刷新封面」选项。用 Pointer Events 兼容鼠标/触屏。
  let refreshPressTimer: ReturnType<typeof setTimeout> | undefined;
  let refreshLongPressed = false;
  function refreshDown(e: PointerEvent) {
    if (e.button !== 0 || refreshing) return;
    refreshLongPressed = false;
    refreshPressTimer = setTimeout(() => {
      refreshLongPressed = true;
      refreshMenu = true;
    }, 550);
  }
  function refreshUp() {
    clearTimeout(refreshPressTimer);
  }
  function refreshClick() {
    if (refreshLongPressed) {
      refreshLongPressed = false;
      return; // 长按已触发菜单，不再执行普通刷新
    }
    refreshAll();
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

  /** 删除分级（G1）：有笔记时给出**三个明确动作**的弹窗；无笔记直接删。 */
  async function deleteBook(b: BookRow, fromDisk: boolean) {
    let hasNotes = false;
    let noteCount = 0;
    try {
      const info = await api.bookNoteInfo(b.id);
      hasNotes = info.hasNotes;
      noteCount = info.noteCount;
    } catch { /* ignore */ }

    let keepNotes = true;
    if (hasNotes) {
      // 三个明确动作：保留笔记并删除 / 删除笔记并删除 / 取消。绝不把选择藏进「确定/取消」。
      const action = await appChoose(
        fromDisk
          ? `《${b.title}》有 ${noteCount} 条笔记。\n本地文件将移入回收站。\n\n请选择笔记的处理方式：`
          : `《${b.title}》有 ${noteCount} 条笔记。\n将从书库移除（本地文件保留）。\n\n请选择笔记的处理方式：`,
        [
          { id: "keep", label: "保留笔记并删除", kind: "plain" },
          { id: "del", label: "删除笔记并删除", kind: "danger" },
          { id: "cancel", label: "取消", kind: "plain" },
        ],
      );
      if (!action || action === "cancel") return;
      keepNotes = action === "keep";
    } else {
      const ok = await appConfirm(
        fromDisk
          ? `删除《${b.title}》？\n\n本地文件将移入回收站：\n${b.path}`
          : `从书库中移除《${b.title}》？\n本地文件保留。`,
      );
      if (!ok) return;
    }

    try {
      await api.deleteBooks([b.path], fromDisk, keepNotes);
      toast(
        fromDisk
          ? (hasNotes && keepNotes ? "已删除文件，笔记已保留" : hasNotes ? "已删除文件及笔记" : "已删除，文件已移入回收站")
          : (hasNotes && keepNotes ? "已移出书库，笔记已保留" : hasNotes ? "已移出书库及笔记" : "已从书库移除"),
      );
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
      b.cover_ver = (b.cover_ver ?? 0) + 1;
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
      b.cover_ver = (b.cover_ver ?? 0) + 1;
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
      toast(`已添加《${last.title}》到书库，原文件未移动`);
      await tryOpenBook(last);
    }
  }

  onMount(() => {
    // DEV 调试钩子：便于自动化验证删书弹窗（release 剥离）。
    if (import.meta.env.DEV) {
      (window as any).__frLibDebug = {
        deleteByPath: async (p: string, fromDisk = false) => {
          let b = booksWindow.find((x) => x.path === p);
          if (!b) {
            // 分页窗口里没有 → 直接查库
            const pg = await api.listBooksPage(p, "title", 0, 20);
            b = pg.rows.find((x) => x.path === p) ?? pg.rows[0];
          }
          if (!b) return false;
          void deleteBook(b, fromDisk);
          return true;
        },
      };
    }
    loadFolders();
    reload();
    // 启动竞态修复：首次 `meta-progress` 常在 webview 挂载前发出而丢失，仅靠事件会让底栏空着。
    // 挂载后主动拉一次当前进度快照初始化（若已有事件先到则保留更新值；后续帧仍由 meta-progress 事件更新）。
    api
      .scanProgress()
      .then((p) => {
        if (p && p.running && p.total > 0 && !metaProgress) {
          metaProgress = { phase: "meta", done: Math.min(p.done, p.total), total: p.total };
        }
      })
      .catch(() => {});
    // 监听只增量刷新当前窗口与 total，不重置滚动、不拉全量。
    // library-changed 也会在换/恢复默认底图后触发：先 bump 缓存破坏版本号，
    // 保证无内嵌封面的书立即换用新底图（有内嵌封面书走各自 cover_path，不受影响）。
    // 注意：这里**不触发** metaProgress / 扫描进度状态，避免进度条回退或跳变。
    const u1 = listen("library-changed", () => {
      bumpCoverBust();
      // 增删书改变 N 与顺序：索引档按锚点重建，回落档重拉窗口。
      reloadWindow();
    });
    const u2 = listen("library-meta", () => {
      // 仅封面/元数据（不改顺序）：索引档只刷新可见行，回落档重拉窗口。
      if (indexMode) void refreshVisibleIndex();
      else void reloadWindow();
    });
    const u4 = listen<{ phase: string; done: number; total: number }>("refresh-progress", (e) => {
      if (e.payload) refreshProgress = e.payload;
    });
    // 封面增量扫描进度（入队时发一帧 / 每 16 本发一帧 / done==total 必发）。
    const u5 = listen<{ phase: string; done: number; total: number }>("meta-progress", (e) => {
      if (e.payload) {
        clearTimeout(metaHideTimer);
        metaProgress = e.payload;
      }
    });
    // 扫完即停：让最后一帧（N/N）短暂可见后再隐藏，避免 3/3 一闪而过看不到。
    const u6 = listen("scan-finished", () => {
      clearTimeout(metaHideTimer);
      metaHideTimer = setTimeout(() => { metaProgress = null; }, 600);
    });
    // 某本书封面已更新（提取到新封面/手动换封面）：刷新可见窗口（不改顺序，不重建索引）。
    const u7 = listen("book-updated", () => {
      if (indexMode) void refreshVisibleIndex();
      else void reloadWindow();
    });
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
      clearTimeout(metaHideTimer);
      u1.then((f) => f());
      u2.then((f) => f());
      u4.then((f) => f());
      u5.then((f) => f());
      u6.then((f) => f());
      u7.then((f) => f());
      u3?.then((f) => f());
    };
  });
</script>

<div class="library" onpointerdown={onPtrDown} onpointerup={onPtrUp}>
  <div class="toolbar">
    <span class="logo">FlowRead</span>
    <button class="primary" onclick={addFolder}>＋ 添加文件夹</button>
    <div class="refresh-wrap">
      <button
        onpointerdown={refreshDown}
        onpointerup={refreshUp}
        onpointerleave={refreshUp}
        onpointercancel={refreshUp}
        onclick={refreshClick}
        disabled={refreshing}
        title="点击：只读取书籍；长按：刷新封面"
      >{refreshing ? "刷新中…" : "⟳ 刷新"}</button>
      {#if refreshMenu}
        <div class="refresh-menu" role="menu">
          <button class="rm-item" onclick={() => { refreshMenu = false; refreshAll(); }}>重新扫描书库</button>
        </div>
        <div class="rm-backdrop" onclick={() => (refreshMenu = false)}></div>
      {/if}
    </div>
    <input class="search" placeholder="搜索书名 / 作者…" bind:value={queryInput} />
    <select bind:value={sortKey} onchange={onSortChange}>
      <option value="lastread">按最近阅读</option>
      <option value="added">按添加时间</option>
      <option value="title">按书名</option>
      <option value="recent">按文件时间</option>
    </select>
    <div class="spacer"></div>
    <div class="lib-tb-group" use:dragSort={{ onDrop: libDrop }}>
      {#each orderedLibBtns as b (b.id)}
        {@const Icon = b.icon}
        <button
          class="icon lib-tb-btn tip-left"
          data-drag-id={b.id}
          aria-label={b.title}
          data-tip={b.title}
          onclick={b.run}
        >
          <Icon size={20} stroke={1.75} />
        </button>
      {/each}
    </div>
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
          <div
            class="grid-row"
            style="top:{r * cellH}px; height:{cellH}px; grid-template-columns:repeat({cols}, minmax(0, 1fr)); column-gap:{CARD_GAP}px;"
          >
            {#each Array(cols) as _, ci (ci)}
              {@const abs = r * cols + ci}
              {#if abs < total}
                {@const b = bookAt(abs)}
                {#if b}
                  <div
                    class="card"
                    style="max-width:{CARD_MAX}px;"
                    ondblclick={() => tryOpenBook(b)}
                    oncontextmenu={(e) => openCtx(e, b)}
                  >
                    <div class="cover" style="height:{coverH}px;">
                      <img src={coverUrl(b)} alt="《{b.title}》封面" loading="lazy" decoding="async" draggable="false" />
                      <span class="fmt">{b.format.toUpperCase()}</span>
                    </div>
                    <div class="meta">
                      <div class="t" title={b.title}>{b.title}</div>
                      <div class="a">{b.author}</div>
                    </div>
                  </div>
                {:else}
                  <div class="card card-slot" style="max-width:{CARD_MAX}px;">
                    <div class="cover" style="height:{coverH}px;"></div>
                    <div class="meta"></div>
                  </div>
                {/if}
              {/if}
            {/each}
          </div>
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
              <img class="row-cover" src={coverUrl(b)} alt="《{b.title}》封面" loading="lazy" decoding="async" />
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
    <span class="build" title="FlowRead 版本">FlowRead {APP_VERSION}</span>
    {#if refreshProgress && refreshProgress.phase !== "done"}
      <div class="scan-progress" title="书库刷新扫描进度">
        <div class="scan-bar"><div class="scan-fill" style="width:{refreshPct}%"></div></div>
        <span class="scan-text">{refreshPhaseText(refreshProgress)} · {refreshPct}%</span>
      </div>
    {/if}
    {#if metaProgress && metaProgress.phase !== "done" && metaProgress.total > 0}
      <div class="scan-progress meta-progress" title="封面/元数据后台生成进度（不影响使用）">
        <div class="scan-bar"><div class="scan-fill meta-fill" style="width:{metaPct}%"></div></div>
        <span class="scan-text">生成封面 {metaProgress.done}/{metaProgress.total} · {metaPct}%</span>
      </div>
    {/if}
    <div class="spacer"></div>
    <button class="icon view-btn" class:active={view === "grid"} title="网格视图" aria-pressed={view === "grid"} onclick={() => setView("grid")}>▦</button>
    <button class="icon view-btn" class:active={view === "list"} title="列表视图" aria-pressed={view === "list"} onclick={() => setView("list")}>≡</button>
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
        <button class="danger" onclick={() => { deleteBook(ctxMenu!.book, true); ctxMenu = null; }}>🗑 删除本地文件</button>
      </div>
    </div>
  {/if}

  {#if detailBook}
    <BookDetailModal book={detailBook} onclose={() => { detailBook = null; touchWindow(); }} />
  {/if}
</div>

<style>
  /* ============================================================
     书库页观感重做（依据《界面设计规范》）
     仅改视觉：布局结构（顶栏 + 网格/列表 + 状态栏）保持不变。
     - 间距全部走 4px 栅格
     - 字号只用 12/13/14/16 阶梯
     - 圆角 6/8/12/16/999
     - 阴影分层更柔：1px 极淡描边 + 轻阴影
     - 悬停 ≥8% 亮度差；禁用 40% 不透明
     ============================================================ */
  .library {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  /* ---------- 顶栏 ---------- */
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-2, 8px);
    padding: var(--sp-3, 12px) var(--sp-4, 16px);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .logo {
    font-weight: 600;
    font-size: var(--fs-lg, 16px);
    color: var(--accent);
    margin-right: var(--sp-3, 12px);
    letter-spacing: -0.015em;
    user-select: none;
  }

  .search {
    width: 220px;
  }

  .spacer { flex: 1; }

  /* ---------- 文件夹胶囊 ---------- */
  .folders {
    display: flex;
    gap: var(--sp-2, 8px);
    padding: var(--sp-3, 12px) var(--sp-4, 16px) 0;
    flex-wrap: wrap;
  }

  .folder-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2, 8px);
    background: var(--panel2);
    border: 1px solid var(--line);
    border-radius: var(--radius-pill, 999px);
    padding: 4px 8px 4px 12px;
    font-size: var(--fs-xs, 12px);
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
    border-radius: var(--radius-sm, 8px);
    transition: background var(--dur, 0.2s) var(--ease);
  }

  .folder-chip .x:hover {
    background: var(--hover-bg, rgba(0, 0, 0, 0.06));
    color: var(--fg);
  }

  /* ---------- 网格 ---------- */
  .grid {
    flex: 1;
    overflow-y: auto;
    /* 左右 20px 与 gridPadX 推导一致；底部多留一屏呼吸空间 */
    padding: 16px 20px 32px;
    position: relative;
  }

  .grid-inner {
    position: relative;
    width: 100%;
    will-change: transform;
  }

  /* 每一「行」是一个绝对定位、占满内容宽的 CSS 网格：列数与列宽交给
     grid-template-columns: repeat(cols, minmax(0,1fr)) 精确均分，minmax 下限 0
     保证列宽可收缩、永不撑破容器 —— 任意窗口尺寸下末列都不会越过右边缘被裁切。 */
  .grid-row {
    position: absolute;
    left: 0;
    right: 0;
    display: grid;
    justify-items: center; /* 卡片受 max-width 限制时在列内居中，超宽窗口不拉伸变形 */
    align-content: start;
  }

  .card {
    width: 100%;
    box-sizing: border-box;
    cursor: pointer;
    border-radius: var(--radius, 12px);
    padding: var(--sp-2, 8px);
    /* 悬停抬升：≥8% 亮度差 + 柔和阴影；位移 -2px（规范第 7 节） */
    transition: background var(--dur, 0.2s) var(--ease),
      transform var(--dur, 0.2s) var(--ease),
      box-shadow var(--dur, 0.2s) var(--ease);
  }

  .card:hover {
    background: var(--hover-bg);
    transform: translateY(-2px);
    box-shadow: var(--shadow-md);
  }

  .card:active {
    transform: translateY(0);
  }

  /* 未加载完成的占位槽：只占位保持列对齐，不响应交互、无悬停效果 */
  .card-slot {
    cursor: default;
    pointer-events: none;
  }
  .card-slot:hover {
    background: transparent;
    transform: none;
    box-shadow: none;
  }

  /* 封面：外层 8px 圆角容器，内层图片裁切；用极淡描边替代重阴影 */
  .cover {
    position: relative;
    width: 100%;
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

  .card:hover .cover img {
    transform: scale(1.03);
  }

  /* 格式角标：半透明深底 + 白色小字，10px 字距 +0.015em */
  .fmt {
    position: absolute;
    right: 8px;
    top: 8px;
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    font-size: var(--fs-2xs, 10px);
    line-height: 1;
    padding: 4px 8px;
    border-radius: var(--radius-sm, 8px);
    letter-spacing: var(--ls-wide, 0.015em);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }

  /* 固定块高（与脚本 META_H=52 一致）：标题 20 + 间距 1 + 作者 18 + 上内边距 8 + 余量 5，
     保证每张卡片等高、行预算准确，卡片不会被下一行遮挡。 */
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

  /* ---------- 列表 ---------- */
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
    gap: var(--sp-4, 16px);
    padding: var(--sp-2, 8px) var(--sp-3, 12px);
    border-radius: var(--radius-sm, 8px);
    cursor: pointer;
    transition: background var(--dur, 0.2s) var(--ease);
  }

  .row:hover { background: var(--hover-row, rgba(0, 0, 0, 0.04)); }

  .row-cover {
    width: 34px;
    height: 48px;
    object-fit: cover;
    border-radius: var(--radius-xs, 6px);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm);
  }

  .row-meta { flex: 1; min-width: 0; }
  .row-meta .t { font-size: var(--fs-md, 14px); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-meta .a { font-size: var(--fs-xs, 12px); color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-size, .row-date { color: var(--fg2); font-size: var(--fs-xs, 12px); width: 90px; text-align: right; font-variant-numeric: tabular-nums; }

  /* ---------- 空状态 ---------- */
  .hint {
    text-align: center;
    padding: 48px 24px;
    color: var(--fg2);
    white-space: pre-line;
    line-height: var(--lh-body, 1.6);
    font-size: var(--fs-md, 14px);
  }

  /* ---------- 状态栏 ---------- */
  .statusbar {
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

  .statusbar .build {
    font-size: var(--fs-xs, 12px);
    opacity: 0.55;
  }

  .lib-tb-group {
    display: flex;
    align-items: center;
    gap: var(--sp-1, 4px);
    position: relative;
  }

  .lib-tb-btn {
    touch-action: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    min-height: 30px;
    height: 30px;
    padding: 0;
    border-radius: var(--radius-sm);
  }

  /* 当前视图高亮：让用户知道自己在哪个视图（skill: active nav indication） */
  .view-btn.active {
    background: var(--accent-soft, var(--hover-bg));
    color: var(--accent);
    opacity: 1;
  }

  /* ---------- 刷新进度 ---------- */
  /* 刷新按钮 + 长按菜单 */
  .refresh-wrap { position: relative; display: inline-flex; }

  .refresh-menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 120;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: var(--radius, 12px);
    box-shadow: var(--shadow-lg);
    animation: ctx-in 0.12s cubic-bezier(0, 0, 0.2, 1);
  }

  .refresh-menu .rm-item {
    border: none;
    background: transparent;
    box-shadow: none;
    text-align: left;
    padding: 8px 12px;
    border-radius: var(--radius-sm, 8px);
    font-size: var(--fs-sm, 13px);
    color: var(--fg);
  }

  .refresh-menu .rm-item:hover:not(:disabled) { background: var(--hover-bg); border-color: transparent; }

  .rm-backdrop { position: fixed; inset: 0; z-index: 119; }

  .scan-progress {
    display: flex;
    align-items: center;
    gap: var(--sp-2, 8px);
    min-width: 180px;
  }

  .meta-progress .meta-fill { background: var(--accent); opacity: 0.7; }

  .scan-bar {
    flex: 1;
    height: 4px;
    border-radius: var(--radius-pill, 999px);
    background: color-mix(in srgb, var(--fg2) 22%, transparent);
    overflow: hidden;
  }

  .scan-fill {
    height: 100%;
    background: var(--accent);
    border-radius: var(--radius-pill, 999px);
    transition: width var(--dur, 0.2s) linear;
  }

  .scan-text {
    white-space: nowrap;
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
  }

  /* ---------- 右键菜单 ---------- */
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
    border-radius: var(--radius, 12px);
    box-shadow: var(--shadow-lg);
    padding: 4px;
    min-width: 190px;
    z-index: 101;
    animation: ctx-in var(--dur-fast, 0.12s) var(--ease-in, cubic-bezier(0, 0, 0.2, 1));
  }

  @keyframes ctx-in {
    from { opacity: 0; transform: translateY(-4px) scale(0.98); }
    to { opacity: 1; transform: none; }
  }

  .ctx button {
    border: none;
    background: transparent;
    text-align: left;
    padding: 8px 12px;
    border-radius: var(--radius-sm, 8px);
    font-size: var(--fs-sm, 13px);
    color: var(--fg);
    transition: background var(--dur, 0.2s) var(--ease);
  }

  .ctx button:hover { background: var(--hover-bg, rgba(0, 0, 0, 0.06)); }
  .ctx button.danger { color: var(--danger); }
  .ctx button.danger:hover { background: color-mix(in srgb, var(--danger) 12%, transparent); }
</style>
