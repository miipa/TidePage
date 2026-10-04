<script lang="ts">
  import { onMount } from "svelte";
  import { api, coverUrl, bumpCoverBust } from "../lib/api";
  import { appConfirm, appChoose, config, convertState, openBookInNewWindow, pendingOpen, saveConfig, toast, ui, APP_VERSION, LIBRARY_TOPBAR_IDS, type LibraryTopbarId } from "../lib/stores.svelte";
  import { dateLocale, errText, t, type MessageKey } from "../lib/i18n/index.svelte";
  import { dragSort } from "../lib/dragSort";
  import { listen } from "@tauri-apps/api/event";
  import type { Icon as TablerIcon } from "@tabler/icons-svelte";
  import {
    IconChartBar,
    IconCloud,
    IconDice,
    IconNotebook,
    IconSettings,
    IconBook2,
    IconInfoCircle,
    IconArrowsExchange,
    IconPhoto,
    IconRefresh,
    IconCircleCheck,
    IconCircle,
    IconFolder,
    IconArrowBackUp,
    IconTrash,
    IconArrowsSort,
    IconCheck,
    IconLayoutGrid,
    IconLayoutList,
    IconPlus,
    IconX,
    IconBooks,
    IconSearchOff,
  } from "@tabler/icons-svelte";
  import BookDetailModal from "./BookDetailModal.svelte";
  import ProgressRing from "./ProgressRing.svelte";
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
  /** 文件夹列表读取失败（list_folders 抛错）：胶囊排仍要留「全部」+ 报错，否则用户无法切回全部。 */
  let foldersFailed = $state(false);
  // 文件夹单选作用域（null=「全部」）。持久化到 tidepage.lib.folder（仿 tidepage.lib.filter 的存取）。
  // 选中后：排序 / 分类筛选 / 随机打开 均只在该文件夹目录内生效（后端按路径前缀下推 SQLite）。
  let selectedFolder = $state<string | null>(
    (() => {
      try { return localStorage.getItem("tidepage.lib.folder") || null; }
      catch { return null; }
    })()
  );
  let queryInput = $state("");
  let query = $state("");
  let sortKey = $state<"added" | "lastread" | "title" | "recent" | "size">(
    (() => {
      try {
        const v = localStorage.getItem("tidepage.lib.sort");
        return v === "lastread" || v === "title" || v === "recent" || v === "size" ? v : "added";
      } catch { return "added"; }
    })()
  );
  /** 是否反向排序（与 tidepage.lib.sort 同款持久化）。 */
  let reversed = $state(
    (() => {
      try { return localStorage.getItem("tidepage.lib.sortDir") === "1"; }
      catch { return false; }
    })()
  );
  let view = $state<"grid" | "list">(
    (() => {
      try { return localStorage.getItem("tidepage.lib.view") === "list" ? "list" : "grid"; }
      catch { return "grid"; }
    })()
  );
  let loading = $state(true);
  let refreshing = $state(false);
  /** 刷新扫描进度（由后端 refresh-progress 事件驱动）。 */
  let refreshProgress = $state<{ phase: string; done: number; total: number } | null>(null);
  /** 添加目录扫描进度（由后端 folder-scan-progress 事件驱动；只统计本次目录的条目）。 */
  let folderScan = $state<{ running: boolean; scanned: number; added: number } | null>(null);
  /** 封面/元数据后台生成进度（由后端 meta-progress 事件驱动；不阻塞其它操作）。 */
  let metaProgress = $state<{ phase: string; done: number; total: number } | null>(null);
  /** 让最后一帧进度（N/N）短暂可见后再隐藏，避免一闪而过看不到。 */
  let metaHideTimer: ReturnType<typeof setTimeout> | undefined;
  /** library-changed 防抖：移除大目录等场景可能短时多次触发，合并为一次全量刷新，
   *  避免叠加多次整库索引重建把主线程压死。 */
  let libChangedTimer: ReturnType<typeof setTimeout> | undefined;
  /** 长按刷新弹出的菜单（普通点击=只读书籍；长按=可刷新封面）。 */
  let refreshMenu = $state(false);

  // 书库顶栏右侧图标按钮注册表（可拖拽排序；顺序持久化到 config.library.topbarOrder）。
  // 注意：这里只存 i18n 键（labelKey/titleKey），不存译文 —— 译文在 orderedLibBtns 里现取，
  // 否则模块初始化时定下的中文切语言后不会更新。
  interface LibBtn {
    id: LibraryTopbarId;
    labelKey: MessageKey;
    titleKey: MessageKey;
    icon: TablerIcon;
    run: () => void;
  }
  interface ResolvedLibBtn extends Omit<LibBtn, "labelKey" | "titleKey"> {
    label: string;
    title: string;
  }
  const LIB_BTNS: LibBtn[] = [
    { id: "webdav", labelKey: "library.topbar.webdav.label", titleKey: "library.topbar.webdav.title", icon: IconCloud, run: () => (ui.webdavOpen = true) },
    { id: "random", labelKey: "library.topbar.random.label", titleKey: "library.topbar.random.title", icon: IconDice, run: () => openRandom() },
    // 顶栏不再放「全文搜索」按钮（阅读界面仍有全文搜索）；原位置给「笔记」，进入与书库同款的笔记书籍视图。
    { id: "notes", labelKey: "library.topbar.notes.label", titleKey: "library.topbar.notes.title", icon: IconNotebook, run: () => (ui.notesOpen = true) },
    { id: "stats", labelKey: "library.topbar.stats.label", titleKey: "library.topbar.stats.title", icon: IconChartBar, run: () => (ui.statsOpen = true) },
    { id: "settings", labelKey: "library.topbar.settings.label", titleKey: "library.topbar.settings.title", icon: IconSettings, run: () => (ui.settingsOpen = true) },
  ];
  const libBtn = (id: string): LibBtn | undefined => LIB_BTNS.find((b) => b.id === id);
  const orderedLibBtns: ResolvedLibBtn[] = $derived(
    config.library.topbarOrder
      .map((id) => libBtn(id))
      .filter((b): b is LibBtn => !!b)
      // t() 读 locale 这个 $state，故此 $derived 会随语言切换重算。
      .map((b) => ({ id: b.id, icon: b.icon, run: b.run, label: t(b.labelKey), title: t(b.titleKey) })),
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
    if (p.phase === "check") return t("library.refresh.phase.check", { done: p.done, total: p.total });
    if (p.phase === "scan") return t("library.refresh.phase.scan", { done: p.done, total: p.total });
    return t("library.refresh.phase.done");
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
    try { localStorage.setItem("tidepage.lib.sort", sortKey); } catch { /* ignore */ }
  }

  function saveSortDir() {
    try { localStorage.setItem("tidepage.lib.sortDir", reversed ? "1" : "0"); } catch { /* ignore */ }
  }

  /** 各排序的默认方向（true=降序，如 size 默认大→小）；反向按钮整体翻转。 */
  const SORT_DESC: Record<"added" | "lastread" | "title" | "recent" | "size", boolean> = {
    added: true,
    lastread: true,
    recent: true,
    size: true,
    title: false,
  };
  const desc = $derived(reversed ? !SORT_DESC[sortKey] : SORT_DESC[sortKey]);

  // ===== 分类筛选（格式 + 阅读状态）=====
  // 进度「已满」阈值：与后端 db::READ_FULL_PCT 逐字同源（progress.pct 量纲 0–100）。
  const READ_FULL_PCT = 99.5;
  type ReadState = "unread" | "reading" | "read";
  const ALL_STATES: ReadState[] = ["unread", "reading", "read"];
  // 显示名按 id 查本地化表（状态 id 本身是要写进查询参数的持久值，绝不能翻译）。
  const STATE_LABEL_KEYS: Record<ReadState, MessageKey> = {
    unread: "library.state.unread",
    reading: "library.state.reading",
    read: "library.state.read",
  };
  const stateLabel = (s: ReadState): string => t(STATE_LABEL_KEYS[s]);

  // 从 localStorage 恢复筛选（key 与 tidepage.lib.sort 同款）；脏值过滤。
  const _initFilter = (() => {
    try {
      const raw = localStorage.getItem("tidepage.lib.filter");
      if (raw) {
        const o = JSON.parse(raw);
        return {
          formats: Array.isArray(o.formats) ? o.formats.filter((x: unknown) => typeof x === "string") : [],
          states: Array.isArray(o.states)
            ? o.states.filter((x: unknown) => x === "unread" || x === "reading" || x === "read")
            : [],
        };
      }
    } catch { /* ignore */ }
    return { formats: [] as string[], states: [] as string[] };
  })();
  let availableFormats = $state<string[]>([]);
  let filterFormats = $state<string[]>(_initFilter.formats);
  let filterStates = $state<string[]>(_initFilter.states as ReadState[]);
  let filterPanelOpen = $state(false);
  // 任一维度有选中即视为「已筛选」。空数组=不约束（回完整书库）。
  const hasFilter = $derived(filterFormats.length > 0 || filterStates.length > 0);

  function saveFilter() {
    try {
      localStorage.setItem("tidepage.lib.filter", JSON.stringify({ formats: filterFormats, states: filterStates }));
    } catch { /* ignore */ }
  }

  // ===== 文件夹单选作用域 =====
  function saveFolder() {
    try {
      if (selectedFolder) localStorage.setItem("tidepage.lib.folder", selectedFolder);
      else localStorage.removeItem("tidepage.lib.folder");
    } catch { /* ignore */ }
  }

  /** 选中某文件夹（null=全部）：持久化并全量重载（排序/分类/随机随之限定作用域）。 */
  function selectFolder(f: string | null) {
    if (selectedFolder === f) return;
    selectedFolder = f;
    saveFolder();
    reload();
  }

  /** 有效阅读状态：与后端 build_book_filter 的 CASE 表达式逐字同源（§0.3 红线 3）。 */
  function effectiveState(b: BookRow): ReadState {
    if (b.read_state === 1) return "read";
    if (b.read_state === 2) return "unread";
    if (!b.last_opened) return "unread";
    if ((b.read_pct || 0) >= READ_FULL_PCT) return "read";
    return "reading";
  }

  /** 进度环显示信息：显示条件统一为 effective_read_state != 'unread'（Task E）。 */
  function ringInfo(b: BookRow): { show: boolean; full: boolean; pct: number } {
    const st = effectiveState(b);
    if (st === "unread") return { show: false, full: false, pct: 0 };
    const full = b.read_state === 1; // 手动已读 → 满环 + ✓，不显示真实 read_pct
    return { show: true, full, pct: full ? 100 : b.read_pct || 0 };
  }

  function toggleFormat(f: string) {
    filterFormats = filterFormats.includes(f) ? filterFormats.filter((x) => x !== f) : [...filterFormats, f];
    saveFilter();
    reload();
  }
  function toggleState(s: ReadState) {
    filterStates = filterStates.includes(s) ? filterStates.filter((x) => x !== s) : [...filterStates, s];
    saveFilter();
    reload();
  }
  function clearFilter() {
    if (!hasFilter) return;
    filterFormats = [];
    filterStates = [];
    saveFilter();
    reload();
  }

  /** 拉取库内实际格式并动态生成标签；顺带清理「已选中但已不存在」的格式（失效清理）。 */
  async function loadFormats() {
    try {
      availableFormats = await api.listFormats();
      const set = new Set(availableFormats);
      const kept = filterFormats.filter((f) => set.has(f));
      if (kept.length !== filterFormats.length) {
        filterFormats = kept;
        saveFilter();
        reload(); // 死选中被清后回到正确结果集
      }
    } catch { /* ignore */ }
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
    try { localStorage.setItem("tidepage.lib.view", v); } catch { /* ignore */ }
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

  // 网格视口尺寸用 ResizeObserver 主动量取（替代 bind:clientWidth/Height）：
  // 无论横向、纵向、还是斜向（对角）拉伸，只要 .grid 的 border-box 变化就同步刷新
  // gridW（决定列数/封面宽度）与 gridH（决定可见行数），避免某一方向 resize 不触发重排、
  // 出现底部空白或末行被窗口边缘裁掉半截。滚动条出现/消失改变内容宽也会一并纠正。
  $effect(() => {
    const el = gridEl;
    if (!el) return;
    const measure = () => {
      gridW = el.clientWidth;
      gridH = el.clientHeight;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });

  // 顶栏实测高度写入全局 CSS 变量：供固定在右上角的 WindowControls 让自身高度=顶栏高度，
  // 按钮垂直居中即与顶栏图标中心线重合，自动对齐（顶栏换行等高度变化时跟随更新）。
  let tbH = $state(0);
  $effect(() => {
    if (tbH > 0) {
      document.documentElement.style.setProperty("--fr-titlebar-h", tbH + "px");
    }
  });

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

  // ===== 多选（长按进入 / 右键拉框）=====
  // 选中集合用普通 Set（避免 16 万元素被 $state 深层代理带来额外开销），
  // 由 selVersion 提供响应式：isSelected/selCount 读它建立模板依赖。
  let selectMode = $state(false);
  let selection = new Set<number>();
  let selVersion = $state(0);
  /** 回落档（indexMode===false）全选时标记「仅选中已加载部分」。 */
  let selectPartial = $state(false);

  function isSelected(id: number): boolean {
    void selVersion;
    return selection.has(id);
  }
  function selCount(): number {
    void selVersion;
    return selection.size;
  }

  // 长按（550ms，鼠标/触屏通用）：超过 6px 或提前松手即取消。
  let lpTimer: ReturnType<typeof setTimeout> | undefined;
  let lpId = -1;
  let lpX = 0;
  let lpY = 0;
  /** 长按已触发：松手时抑制紧随的 click/dblclick，避免刚进入选择又被单击切换。 */
  let lpFired = false;
  /** 抑制阈值时间戳（长按松手后短暂生效，防止紧随的 click/dblclick 把刚进入的选择又切换掉）。 */
  let suppressUntil = 0;
  function suppressClicks() {
    suppressUntil = Date.now() + 200;
  }
  function clearLongPress() {
    if (lpTimer !== undefined) clearTimeout(lpTimer);
    lpTimer = undefined;
    lpId = -1;
  }

  function enterSelect(id: number) {
    selectMode = true;
    selection = new Set([id]);
    selVersion++;
    selectPartial = false;
    lpFired = true;
  }

  function toggleSelect(id: number) {
    if (selection.has(id)) selection.delete(id);
    else selection.add(id);
    selVersion++;
  }

  function exitSelect() {
    if (!selectMode) return;
    selectMode = false;
    selection = new Set();
    selVersion++;
    selectPartial = false;
    clearLongPress();
  }

  function cardPointerDown(e: PointerEvent, b: BookRow) {
    if (e.button !== 0 || selectMode) return;
    clearLongPress();
    lpId = b.id;
    lpX = e.clientX;
    lpY = e.clientY;
    lpTimer = setTimeout(() => {
      lpTimer = undefined;
      if (lpId !== b.id) return;
      enterSelect(b.id);
    }, 550);
  }

  function libPointerMove(e: PointerEvent) {
    if (lpTimer === undefined) return;
    if (Math.abs(e.clientX - lpX) > 6 || Math.abs(e.clientY - lpY) > 6) clearLongPress();
  }

  function libPointerUp(e: PointerEvent) {
    onPtrUp(e);
    if (lpFired) {
      lpFired = false;
      suppressClicks();
    }
    clearLongPress();
  }

  function onCardClick(e: MouseEvent, b: BookRow) {
    if (Date.now() < suppressUntil) { e.stopPropagation(); return; }
    if (!selectMode) return;
    e.stopPropagation();
    toggleSelect(b.id);
  }

  function onCardDblClick(e: MouseEvent, b: BookRow) {
    if (selectMode || Date.now() < suppressUntil) { e.preventDefault(); e.stopPropagation(); return; }
    tryOpenBook(b);
  }

  function onGridClick(e: MouseEvent) {
    if (Date.now() < suppressUntil) return;
    if (!selectMode) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest?.(".card, .row")) return;
    exitSelect();
  }

  // 左键拉框：在 .grid 空白处按下并拖动 > 6px 进入；起点在卡片/行上时不启动（交给长按/单击/双击）。
  let boxPid = -1;
  let boxStartX = 0;
  let boxStartY = 0;
  let boxLastX = 0;
  let boxLastY = 0;
  let boxMoved = false;
  let boxActive = $state(false);
  let boxEl: HTMLDivElement | undefined = $state();
  // 非响应式：拖框期间「当前帧框内命中」（普通 Set）+ 单 rAF 循环（绘制 / 自动滚动 / 命中重算）。
  let boxHits = new Set<number>();
  let boxGridRect: DOMRect | null = null;
  let boxScrollDy = 0;
  let boxRaf = 0;

  /** 夹取到 .grid 视口内的框（指针可越界到底栏外，但绘制/命中不超出网格）。 */
  function clampedBox() {
    const r = boxGridRect;
    let sx = boxStartX, sy = boxStartY, lx = boxLastX, ly = boxLastY;
    if (r) {
      sx = Math.max(r.left, Math.min(r.right, sx));
      lx = Math.max(r.left, Math.min(r.right, lx));
      sy = Math.max(r.top, Math.min(r.bottom, sy));
      ly = Math.max(r.top, Math.min(r.bottom, ly));
    }
    return { x0: Math.min(sx, lx), y0: Math.min(sy, ly), x1: Math.max(sx, lx), y1: Math.max(sy, ly) };
  }

  function paintBox() {
    if (!boxEl) return;
    const { x0, y0, x1, y1 } = clampedBox();
    boxEl.style.transform = `translate(${x0}px, ${y0}px)`;
    boxEl.style.width = `${x1 - x0}px`;
    boxEl.style.height = `${y1 - y0}px`;
  }

  /** 按指针相对 grid 上下边缘的越界距离算每帧滚动量（速度随距离增大，上限 ~40px）。 */
  function computeScrollDy(): number {
    const r = boxGridRect;
    if (!r) return 0;
    const thr = 24, maxDy = 40;
    if (boxLastY >= r.bottom - thr) {
      return Math.min(maxDy, Math.max(2, (boxLastY - (r.bottom - thr)) * 0.6));
    }
    if (boxLastY <= r.top + thr) {
      return -Math.min(maxDy, Math.max(2, (r.top + thr - boxLastY) * 0.6));
    }
    return 0;
  }

  /** 每帧重建 boxHits：先清空，再填入「当前夹取后的框 ∩ 当前可见 .card/.row」的 id。
   *  即 boxHits 只表示**当前帧框内集合**（非累计并集）——拉远再收回、框变小后松开，
   *  选中即为松开瞬间框内的书。框退化为 <1px 时也先 clear，避免残留上一帧命中。
   *  自动滚动时同样每帧清空重算：滚进当前框的书会被选中、滚出当前框的书会被移出。 */
  function updateBoxHits() {
    const el = gridEl;
    if (!el) return;
    boxHits.clear();
    const { x0, y0, x1, y1 } = clampedBox();
    if (x1 - x0 < 1 || y1 - y0 < 1) return;
    const nodes = el.querySelectorAll<HTMLElement>(".card[data-book-id], .row[data-book-id]");
    for (const n of nodes) {
      const r = n.getBoundingClientRect();
      if (r.left < x1 && r.right > x0 && r.top < y1 && r.bottom > y0) {
        const id = Number(n.dataset.bookId);
        if (id) boxHits.add(id);
      }
    }
  }

  /** 单 rAF 循环：自动滚动 + 绘制 + 当前帧命中重算（指针不动时也持续滚动）。 */
  function boxLoop() {
    boxRaf = 0;
    const el = gridEl;
    if (!el) return;
    boxScrollDy = computeScrollDy();
    if (boxScrollDy !== 0) el.scrollTop += boxScrollDy;
    paintBox();
    updateBoxHits();
    boxRaf = requestAnimationFrame(boxLoop);
  }

  function stopBoxLoop() {
    if (boxRaf) {
      cancelAnimationFrame(boxRaf);
      boxRaf = 0;
    }
    boxScrollDy = 0;
  }

  function onGridPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest?.(".card, .row")) return; // 起点在卡片/行上：不启动拉框
    boxPid = e.pointerId;
    boxStartX = e.clientX;
    boxStartY = e.clientY;
    boxLastX = e.clientX;
    boxLastY = e.clientY;
    boxMoved = false;
    boxGridRect = gridEl?.getBoundingClientRect() ?? null;
  }

  function onGridPointerMove(e: PointerEvent) {
    if (e.pointerId !== boxPid) return;
    if (!boxMoved) {
      if (Math.hypot(e.clientX - boxStartX, e.clientY - boxStartY) <= 6) return;
      boxMoved = true;
      boxActive = true;
      e.preventDefault(); // 起框首帧阻止文本选中（.grid.boxing 已禁选中）
      gridEl?.setPointerCapture?.(e.pointerId);
      if (!boxRaf) boxRaf = requestAnimationFrame(boxLoop);
    }
    boxLastX = e.clientX;
    boxLastY = e.clientY;
  }

  function commitBoxSelection() {
    // 松手：把**当前帧** boxHits（= 松开瞬间框内集合）并入 selection：
    // 未进入选择模式先清空 selection；已在模式中则与既有选中 union。
    // 跨多次独立拉框仍累加，但单次拉框内以松开瞬间的框为准。
    if (!selectMode) {
      selectMode = true;
      selection = new Set();
      selectPartial = false;
    }
    for (const id of boxHits) selection.add(id);
    selVersion++;
    boxHits.clear();
  }

  function onGridPointerUp(e: PointerEvent) {
    if (e.pointerId !== boxPid) return;
    boxPid = -1;
    stopBoxLoop();
    if (gridEl?.hasPointerCapture?.(e.pointerId)) gridEl.releasePointerCapture(e.pointerId);
    if (boxMoved) {
      boxLastX = e.clientX;
      boxLastY = e.clientY;
      updateBoxHits(); // 最后一帧补算
      commitBoxSelection();
      boxActive = false;
      suppressClicks(); // 抑制松手后紧随的 click，避免 onGridClick 立刻退出选择
    }
    boxMoved = false;
    boxGridRect = null;
  }

  function onGridPointerCancel(e: PointerEvent) {
    if (e.pointerId !== boxPid) return;
    boxPid = -1;
    stopBoxLoop();
    boxMoved = false;
    boxActive = false;
    boxHits.clear();
    boxGridRect = null;
  }

  function onGridPointerLeave(e: PointerEvent) {
    if (e.pointerId !== boxPid) return;
    // 拖框期间已 setPointerCapture，指针越界仍继续（自动滚动），不在此停止。
    if (boxMoved) return;
    boxPid = -1;
    boxGridRect = null;
  }

  /** 全选：索引档取 idIndex 全量 id；回落档仅取已加载窗口行并说明。 */
  function selectAll() {
    if (indexMode && idIndex) {
      const s = new Set<number>();
      for (let i = 0; i < idIndex.length; i++) s.add(Number(idIndex[i]));
      selection = s;
      selectPartial = false;
    } else {
      const s = new Set<number>();
      for (const b of booksWindow) if (b) s.add(b.id);
      selection = s;
      selectPartial = true;
    }
    selVersion++;
  }

  /** 反选：全量 id 集合减去当前选中（回落档仅在已加载行内反选）。 */
  function invertSelection() {
    if (indexMode && idIndex) {
      for (let i = 0; i < idIndex.length; i++) {
        const id = Number(idIndex[i]);
        if (selection.has(id)) selection.delete(id);
        else selection.add(id);
      }
      selectPartial = false;
    } else {
      for (const b of booksWindow) {
        if (!b) continue;
        if (selection.has(b.id)) selection.delete(b.id);
        else selection.add(b.id);
      }
    }
    selVersion++;
  }

  /** 批量移出/删除：按 ids 分批取 path（跳过死 id），一次性 deleteBooks 避免多次重建索引。 */
  async function batchDelete(fromDisk: boolean) {
    if (!selection.size) return;
    const count = selection.size;
    const action = await appChoose(
      fromDisk
        ? t("library.batch.msg.fromDisk", { count })
        : t("library.batch.msg.fromLib", { count }),
      [
        { id: "keep", label: t("library.batch.action.keep"), kind: "plain" },
        { id: "del", label: t("library.batch.action.del"), kind: "danger" },
        { id: "cancel", label: t("common.confirm.cancel"), kind: "plain" },
      ],
    );
    if (!action || action === "cancel") return;
    const keepNotes = action === "keep";
    const ids = [...selection];
    const paths: string[] = [];
    for (let i = 0; i < ids.length; i += IDS_CHUNK) {
      const chunk = ids.slice(i, i + IDS_CHUNK);
      let rows: (BookRow | null)[];
      try {
        rows = await api.listBooksByIds(chunk);
      } catch (e) {
        toast(errText(e), "error");
        return;
      }
      // 注意：这里**不能**因 generation 变化（期间有重建/换档）就 return ——
      // 用户已在确认弹窗里明确同意删除整批，静默丢弃会让「已选中 N 本」凭空消失且无任何提示。
      // ids→path 是按 id 直查的，索引重建不影响其正确性。
      for (const r of rows) if (r) paths.push(r.path);
    }
    if (!paths.length) {
      toast(t("toast.book.notFound"), "error");
      return;
    }
    let ok = false;
    try {
      await api.deleteBooks(paths, fromDisk, keepNotes);
      ok = true;
      toast(
        fromDisk
          ? (keepNotes
              ? t("toast.book.deletedKeepNotes", { count: paths.length })
              : t("toast.book.deletedWithNotes", { count: paths.length }))
          : (keepNotes
              ? t("toast.book.removedKeepNotes", { count: paths.length })
              : t("toast.book.removedWithNotes", { count: paths.length })),
      );
    } catch (e) {
      toast(errText(e), "error");
    }
    // 成功路径不显式 reloadWindow：后端 delete_books 成功必 emit library-changed，由监听统一刷一次。
    // 失败路径自己刷：DB 可能已提交删除（只是 trash 阶段失败）→ 刷新后选中集里的 id 已成死 id，
    // 故两条路径都退出多选，避免工具栏「已选 N 本」与实际不符。
    if (!ok) await reloadWindow();
    exitSelect();
  }

  function onLibKey(e: KeyboardEvent) {
    if (e.key === "Escape" && selectMode) {
      e.preventDefault();
      exitSelect();
    }
  }

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
      const r = await api.listBooksPage(query, sortKey, offset, FETCH, filterFormats, filterStates, desc, selectedFolder ?? undefined);
      if (seq !== generation) return;
      total = r.total;
      rangeStart = offset;
      booksWindow = r.rows;
    } catch (e) {
      if (seq === generation) toast(errText(e), "error");
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

  /** 在飞取数按 (generation, 起, 止) 合并：重建后的显式取数与取窗口 $effect 的重跑会命中
   *  同一把 key → 只发一次 list_books_by_ids（否则同一屏重复往返）。
   *  key 带 generation：重建后旧 promise 天然失效，不会挡住新索引的取数。 */
  const inflightWindows = new Map<string, Promise<void>>();

  /** 索引档：确保可见区间 [first,last) 的行已在 rowMap（缺哪取哪，分批 ≤ IDS_CHUNK）。 */
  function ensureWindowIndex(first: number, last: number): Promise<void> {
    if (!idIndex) return Promise.resolve();
    const len = idIndex.length;
    if (len === 0) return Promise.resolve();
    const s = Math.max(0, Math.min(first, len));
    const e = Math.max(0, Math.min(last, len));
    if (e <= s) return Promise.resolve();
    const need: number[] = [];
    for (let i = s; i < e; i++) {
      const id = Number(idIndex[i]);
      if (!rowMap.has(id)) need.push(id);
    }
    if (!need.length) return Promise.resolve();
    const seq = generation;
    const key = `${seq}:${s}:${e}`;
    const same = inflightWindows.get(key);
    if (same) return same;
    const p = fetchWindowRows(s, e, need, seq);
    inflightWindows.set(key, p);
    const done = () => { if (inflightWindows.get(key) === p) inflightWindows.delete(key); };
    p.then(done, done);
    return p;
  }

  /** 取回 need 里的行写进 rowMap（批次 ≤ IDS_CHUNK）。
   *  STALE（期间发生重建/换档）：静默丢弃 —— 新一轮 rebuildIndex 末尾会显式 refetchVisibleWindow 接手。
   *  取数失败：winVersion++ 让模板重算 + 有限重试，绝不留下「取数失败 = 永久空卡槽」。 */
  async function fetchWindowRows(s: number, e: number, need: number[], seq: number): Promise<void> {
    let nulls = 0;
    for (let i = 0; i < need.length; i += IDS_CHUNK) {
      const chunk = need.slice(i, i + IDS_CHUNK);
      let rows: (BookRow | null)[];
      try {
        rows = await api.listBooksByIds(chunk);
      } catch (err) {
        if (seq === generation) {
          toast(errText(err), "error");
          winVersion++;
          scheduleWindowRetry();
        }
        return;
      }
      if (seq !== generation) return; // STALE：期间发生了重建/换档，丢弃（由新重建接手）
      for (let j = 0; j < chunk.length; j++) {
        const v = rows[j] ?? null;
        rowMap.set(chunk[j], v);
        if (v === null) nulls++;
      }
    }
    windowRetry = 0;
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

  /** 取数失败后的有限重试（400/900/1800ms，最多 3 次，成功即清零）。
   *  没有它：一次 list_books_by_ids 抖动就留下永久空卡槽 —— total/scrollTop 都没变，
   *  取窗口的 $effect 不会重跑，也没有任何事件会自愈。 */
  const WINDOW_RETRY_DELAYS = [400, 900, 1800];
  let windowRetry = 0;
  let windowRetryTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleWindowRetry() {
    if (windowRetry >= WINDOW_RETRY_DELAYS.length) return;
    const delay = WINDOW_RETRY_DELAYS[windowRetry++];
    clearTimeout(windowRetryTimer);
    windowRetryTimer = setTimeout(() => {
      windowRetryTimer = undefined;
      if (!indexMode || !idIndex) return;
      void refetchVisibleWindow();
    }, delay);
  }

  /** 索引档可见窗口取数：按当前 view + 视口区间发起取行。
   *  由取窗口的 $effect 与 rebuildIndex 共用 —— 后者在整块清空 rowMap 后**显式**调一次，
   *  不再依赖 effect 因 total/scrollTop「恰好变了」而重跑（文件夹内删书时两者常常都不变）。 */
  function refetchVisibleWindow(): Promise<void> {
    if (!indexMode || !idIndex || total === 0) return Promise.resolve();
    const first = view === "grid" ? gFirst * cols : lFirst;
    const last = view === "grid" ? Math.min(total, gLast * cols) : lLast;
    return ensureWindowIndex(first, last);
  }

  // 死 id 收敛：防抖 1s 重建 idIndex（length 自然收敛，滚动高度/「共 N 本」随之校正）。
  let nullStreak = 0;
  let nullRebuildTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleNullRebuild() {
    if (nullRebuildTimer) return;
    nullRebuildTimer = setTimeout(() => {
      nullRebuildTimer = undefined;
      if (indexMode) void rebuildIndex(topAnchor());
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
        // 覆盖写取数失败：同样不能「取回来却无人重取」→ 重算模板 + 走有限重试。
        winVersion++;
        scheduleWindowRetry();
        return;
      }
      if (seq !== generation) return; // STALE：期间发生重建，新重建末尾会显式重取可见窗口
      for (let j = 0; j < chunk.length; j++) rowMap.set(chunk[j], rows[j] ?? null);
    }
    winVersion++;
  }

  /** 当前顶部可见行的锚点：同时给出 id 与**旧秩**。
   *  删书重建时锚点 id 多半就是刚被删掉的那本（不在新索引里）→ 用秩兜底保持视线。 */
  function topAnchor(): { id: number; rank: number } | undefined {
    if (!indexMode || !idIndex || idIndex.length === 0) return undefined;
    const step = view === "grid" ? cellH : ROW_H;
    const topRow = Math.floor(scrollTop / Math.max(1, step));
    const rank = view === "grid" ? topRow * cols : topRow;
    const r = Math.max(0, Math.min(rank, idIndex.length - 1));
    return { id: Number(idIndex[r]), rank: r };
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

  /** 重建有序 id 索引（换排序 / 换查询 / 增删书 / 首次进入）。
   *  锚点处理：按 id 重定位；id 在新索引里找不到（多半就是刚被删掉的那本）时按秩兜底
   *  （min(旧秩, 新长度-1)），绝不直接回顶。 */
  async function rebuildIndex(anchor?: { id: number; rank: number }) {
    const seq = ++generation;
    loading = true;
    windowRetry = 0;
    // 计时打点②：整库有序 id 索引重建（后端取序列 + 前端建 BigInt64Array + 锚点定位）耗时。
    console.time("[tidepage] rebuildIndex");
    try {
      const buf = await api.listOrderIndex(sortKey, query, filterFormats, filterStates, desc, selectedFolder ?? undefined);
      if (seq !== generation) return;
      idIndex = new BigInt64Array(buf);
      total = idIndex.length;
      rowMap = new Map();
      indexMode = true;
      nullStreak = 0;
      clearTimeout(nullRebuildTimer);
      nullRebuildTimer = undefined;
      winVersion++;
      // 重定位：锚点 id 用一次线性扫定位（禁止建 id→rank 全量 Map）。找不到则按秩兜底。
      if (anchor && idIndex.length > 0) {
        let rank = -1;
        for (let i = 0; i < idIndex.length; i++) {
          if (Number(idIndex[i]) === anchor.id) { rank = i; break; }
        }
        scrollToRank(rank >= 0 ? rank : Math.min(anchor.rank, idIndex.length - 1));
      } else {
        resetScroll();
      }
      // rowMap 已整块清空 → 可见窗口作废，必须**显式**取一次。
      // 旧实现把这件事交给取窗口的 $effect，而它只依赖 total/scrollTop/视图几何：
      //  文件夹内删一本时 total 与 scrollTop 常常都不变（N-1 后锚点又落回原值），
      //  effect 不重跑 → rowMap 永远空 → 整页空白且此后无事件自愈。
      await refetchVisibleWindow();
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
        // 其它错误（IPC 抖动/DB 忙）：rowMap 可能已被上一次重建清空，继续留在索引档就是永久空白。
        // 落到 OFFSET 回落档并拉首屏，保证至少看得见书；下次 reload()/换档会自动回到索引档。
        if (rowMap.size === 0) {
          indexMode = false;
          idIndex = null;
          total = 0;
          await loadRange(0);
          loading = false; // loadRange 自增了 generation，finally 里的 loading=false 不再命中
        }
      }
    } finally {
      if (seq === generation) loading = false;
      console.timeEnd("[tidepage] rebuildIndex");
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
      await rebuildIndex(topAnchor());
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
    if (indexMode) {
      void refetchVisibleWindow();
      return;
    }
    ensureRange(view === "grid" ? gFirst * cols : lFirst, view === "grid" ? Math.min(total, gLast * cols) : lLast);
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

  /** 反向排序：翻转方向并持久化，重新加载当前列表。 */
  function toggleReverse() {
    reversed = !reversed;
    saveSortDir();
    reload();
  }

  /** 就地修改窗口内某本书后触发重渲染。索引档：bump winVersion；回落档：替换 raw 数组引用。 */
  function touchWindow() {
    if (indexMode) winVersion++;
    else booksWindow = booksWindow.slice();
  }

  /** 用「新对象身份」替换窗口内某本书（id 不变，属性来自 patch）。
   *  必须换新对象：bookAt() 从非响应式 rowMap / $state.raw booksWindow 取值，
   *  就地改属性对象身份不变 → 模板中 {@const b = bookAt()} 引用不变 →
   *  下游 ringInfo(b) / coverUrl(b) 不重算，角标与封面不刷新。换新对象后才会重求值、即时刷新。 */
  function replaceBook(id: number, patch: Partial<BookRow>) {
    if (indexMode) {
      const cur = rowMap.get(id);
      if (cur) {
        rowMap.set(id, { ...cur, ...patch });
        winVersion++;
      }
    } else {
      booksWindow = booksWindow.map((x) => (x && x.id === id ? { ...x, ...patch } : x));
    }
  }

  async function loadFolders() {
    try {
      folders = await api.listFolders();
      foldersFailed = false;
      // 选中的文件夹已不存在（如上次会话移除 / 外部改动）→ 回到「全部」并重载。
      if (selectedFolder && !folders.includes(selectedFolder)) {
        selectedFolder = null;
        saveFolder();
        reload();
      }
    } catch (e) {
      // 绝不能 `catch {}` 吞掉：folders 为空会让整排胶囊（含「全部」）消失，
      // 用户连回退作用域的入口都没有，而且没有任何提示。改为保留「全部」+ 报错。
      foldersFailed = true;
      toast(t("toast.library.foldersFailed", { detail: errText(e) }), "error");
    }
  }

  async function addFolder() {
    const p = await api.pickFolder(t("library.dialog.pickFolder"));
    if (!p) return;
    try {
      await api.addFolder(p);
      await loadFolders();
      toast(t("toast.library.addedScanning"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  async function removeFolder(p: string) {
    const ok = await appConfirm(t("library.folder.removeConfirm", { path: p }));
    if (!ok) return;
    // remove_folder 已在后端分块删除完成；书籍列表刷新交给 library-changed 监听
    // （reloadWindow+loadFormats），这里只更新目录胶囊，避免显式 reloadWindow 与监听叠加造成
    // 两次全量重建索引。选中的文件夹被移除时，loadFolders 内部会回到「全部」并 reload()。
    await api.removeFolder(p);
    await loadFolders();
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
    const ok = await appConfirm(t("library.book.missingConfirm", { path: b.path }));
    if (!ok) return;
    try {
      const r = await api.refreshLibrary();
      toast(t("toast.library.refreshed", { added: r.added, removed: r.removed }));
    } catch (e) {
      toast(errText(e), "error");
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
      toast(t("toast.library.refreshed", { added: r.added, removed: r.removed }));
      await reloadWindow();
    } catch (e) {
      toast(errText(e), "error");
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
      const b = await api.randomBookFiltered(query, sortKey, filterFormats, filterStates, selectedFolder ?? undefined);
      if (b) tryOpenBook(b);
      else toast(t("toast.library.filterEmpty"));
    } catch (e) {
      toast(errText(e), "error");
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
          ? t("library.delete.noteMsg.fromDisk", { title: b.title, count: noteCount })
          : t("library.delete.noteMsg.fromLib", { title: b.title, count: noteCount }),
        [
          { id: "keep", label: t("library.delete.action.keep"), kind: "plain" },
          { id: "del", label: t("library.delete.action.del"), kind: "danger" },
          { id: "cancel", label: t("common.confirm.cancel"), kind: "plain" },
        ],
      );
      if (!action || action === "cancel") return;
      keepNotes = action === "keep";
    } else {
      const ok = await appConfirm(
        fromDisk
          ? t("library.delete.msg.fromDisk", { title: b.title, path: b.path })
          : t("library.delete.msg.fromLib", { title: b.title }),
      );
      if (!ok) return;
    }

    try {
      await api.deleteBooks([b.path], fromDisk, keepNotes);
      toast(
        fromDisk
          ? (hasNotes && keepNotes
              ? t("toast.book.deletedFileKeepNotes")
              : hasNotes
                ? t("toast.book.deletedFileWithNotes")
                : t("toast.book.deletedFile"))
          : (hasNotes && keepNotes
              ? t("toast.book.removedKeepNotesOne")
              : hasNotes
                ? t("toast.book.removedWithNotesOne")
                : t("toast.book.removedOne")),
      );
      // 成功路径**不**再显式 reloadWindow：后端 delete_books 返回 Ok 前必 emit library-changed，
      // 由监听（120ms 防抖）统一刷新一次。这里再刷一次就是「两次重建 + 两次 rowMap 清空」。
    } catch (e) {
      toast(errText(e), "error");
      // 失败路径必须自己刷一次：后端可能已提交 DB 删除、只在 trash::delete 阶段失败
      // （此时不会 emit library-changed）→ 否则界面留着已删的书（idIndex 里是死 id）。
      await reloadWindow();
    }
  }

  async function setCover(b: BookRow) {
    const p = await api.pickFile(t("library.dialog.pickCover"), [
      { name: t("library.dialog.imageFilter"), exts: ["png", "jpg", "jpeg", "webp", "bmp"] },
    ]);
    if (!p) return;
    try {
      await api.setCustomCover(b.id, p);
      // 换新对象身份（不就地改属性），令 coverUrl 重算并触发封面 <img> 重挂即时刷新。
      replaceBook(b.id, { cover_path: `custom-${b.id}.png`, cover_ver: (b.cover_ver ?? 0) + 1 });
      toast(t("toast.library.coverUpdated"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  async function reextract(b: BookRow) {
    try {
      const name = await api.reextractCover(b.id);
      // 换新对象身份（不就地改属性），令封面 <img> 重挂即时刷新。
      replaceBook(b.id, { cover_path: name, cover_ver: (b.cover_ver ?? 0) + 1 });
      toast(t("toast.library.coverReextracted"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  function openConvert(b: BookRow) {
    convertState.book = { id: b.id, title: b.title, path: b.path, format: b.format };
    convertState.open = true;
  }

  /** Task F：右键标记阅读状态。state=read/unread。
   *  一致性：idIndex 是后端秩快照。若当前有状态筛选生效，标记后该书 effective 状态会变，
   *  应从结果里出现/消失 → 必须 reload()（rebuildIndex 重算 idIndex/total）；否则仅就地刷新。
   *  联动进度：
   *   - 标记已读：read_state=1（角标✓、effectiveState=read），并把真实进度写到末尾（打开定位到末尾、显示 100%）。
   *   - 标记未读：read_state=2（无角标、effectiveState=unread），并把进度重置为开头（打开回到开头）。
   *  之后用户真实阅读时，Reader 保存进度会自动释放手动覆盖（read_state→0），状态随进度推导。 */
  async function markReadState(b: BookRow, state: "read" | "unread") {
    try {
      await api.setReadState(b.id, state);
      const newState = state === "read" ? 1 : 2;
      let newPct = b.read_pct;
      if (state === "read") {
        // 需要章节数才能把进度定位到「最后一章末尾」；Library 手里没有 toc，故用 openBook 取。
        let lastChapter = 0;
        try {
          const r = await api.openBook(b.id);
          lastChapter = Math.max(0, r.toc.length - 1);
        } catch { /* 取不到章节数则退化为把当前章置末尾（chapter 0） */ }
        await api.saveProgress(b.id, {
          chapter_index: lastChapter,
          chapter_ratio: 1,
          pct: 100,
          updated_at: Date.now(),
          device: "pc",
        });
        newPct = 100;
      } else {
        // 未读：进度重置到开头（chapter 0 / ratio 0 / pct 0），保证 Reader 再打开定位到开头。
        await api.saveProgress(b.id, {
          chapter_index: 0,
          chapter_ratio: 0,
          pct: 0,
          updated_at: Date.now(),
          device: "pc",
        });
        newPct = 0;
      }
      if (filterStates.length > 0) {
        await reloadWindow(); // 状态筛选生效：重建秩索引，令该书按新状态出现/消失
      } else {
        // 换新对象身份刷新该行：就地改属性不会被追踪，角标环不会重算（问题根因）。
        replaceBook(b.id, { read_state: newState, read_pct: newPct });
      }
      toast(state === "read" ? t("toast.library.markedRead") : t("toast.library.markedUnread"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  function fmtSize(n: number): string {
    if (n > 1048576) return (n / 1048576).toFixed(1) + " MB";
    if (n > 1024) return (n / 1024).toFixed(0) + " KB";
    return n + " B";
  }

  function fmtTime(ms: number): string {
    if (!ms) return "—";
    // 日期格式跟随界面语言（不再写死 "zh-CN"）；读 dateLocale() 保证切语言后列表重渲染。
    return new Date(ms).toLocaleDateString(dateLocale());
  }

  const BOOK_EXT = ["txt", "epub", "mobi", "azw3", "prc"];

  async function handleDrop(paths: string[]) {
    let any = false;
    let addedFolders = 0;
    let addedBooks = 0;
    let last: BookRow | null = null;
    for (const p of paths) {
      // 目录：直接登记为书库根（含子目录，不弹任何设置），走与「添加文件夹」同款流程。
      let isDir = false;
      try {
        isDir = await api.pathIsDir(p);
      } catch { /* 判定失败则按文件处理 */ }
      if (isDir) {
        any = true;
        try {
          await api.addFolder(p);
          addedFolders++;
        } catch (e) {
          toast(t("toast.library.addFolderFailed", { name: String(p.split(/[\\/]/).pop() ?? ""), detail: errText(e) }), "error");
        }
        continue;
      }
      if (BOOK_EXT.includes((p.split(".").pop() || "").toLowerCase())) {
        any = true;
        try {
          last = await api.addDroppedBook(p);
          addedBooks++;
        } catch (e) {
          toast(t("toast.library.addBookFailed", { name: String(p.split(/[\\/]/).pop() ?? ""), detail: errText(e) }), "error");
        }
      }
    }
    if (!any) return;
    // 有书籍文件落库：按当前排序全量刷新（resetScroll + 重建 id 索引），让新书出现在其排序位置。
    // 仅有目录时不在此刷新：交给「添加目录」扫描完成事件（folder-scan-progress running:false）统一 reload()，
    // 避免与扫描过程中的 library-changed 叠加过多刷新（既不漏刷，也不重复刷）。
    if (addedBooks > 0) {
      await reload();
    } else if (!addedFolders) {
      await reloadWindow(); // 无新增（如全部失败）：保守刷新当前窗口
    }
    if (addedFolders) {
      await loadFolders();
      toast(t("toast.library.foldersAdded", { count: addedFolders }));
    }
    if (last) {
      toast(t("toast.library.bookAdded", { title: last.title }));
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
    loadFormats();
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
      // 防抖合并：移除大目录/批量删除时该事件可能与其它事件叠加，120ms 内只做一次全量刷新，
      // 避免多次整库索引重建叠加冻结 UI。计时打点①：从收到事件到窗口刷新完成的总耗时。
      clearTimeout(libChangedTimer);
      libChangedTimer = setTimeout(() => {
        console.time("[tidepage] library-changed→refresh");
        // 增删书改变 N 与顺序：索引档按锚点重建，回落档重拉窗口。
        void reloadWindow().finally(() => {
          console.timeEnd("[tidepage] library-changed→refresh");
        });
        // 格式可能新增/消失：刷新格式列表并清理失效选中。
        loadFormats();
      }, 120);
    });
    const u2 = listen("library-meta", () => {
      // 仅封面/元数据（不改顺序）：索引档只刷新可见行，回落档重拉窗口。
      if (indexMode) void refreshVisibleIndex();
      else void reloadWindow();
    });
    const u4 = listen<{ phase: string; done: number; total: number }>("refresh-progress", (e) => {
      if (e.payload) refreshProgress = e.payload;
    });
    // 添加目录扫描进度（专用事件）：只统计本次目录的条目；结束时提示新增书籍数。
    const u8 = listen<{ running: boolean; scanned: number; added: number }>("folder-scan-progress", (e) => {
      const p = e.payload;
      if (!p) return;
      if (p.running) {
        folderScan = p;
      } else {
        folderScan = null;
        toast(p.added > 0 ? t("toast.library.folderScanAdded", { count: p.added }) : t("toast.library.folderScanNone"));
        // 扫描完成后按当前排序全量刷新：resetScroll() + 按 sortKey/query/desc 重建 id 索引，
        // 确保新书出现在其排序位置并回到顶部（reloadWindow 会锚定原顶部 id，新书可能不在可视区）。
        if (p.added > 0) reload();
      }
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
      clearTimeout(libChangedTimer);
      clearTimeout(windowRetryTimer);
      clearTimeout(nullRebuildTimer);
      clearTimeout(prioTimer);
      u1.then((f) => f());
      u2.then((f) => f());
      u4.then((f) => f());
      u5.then((f) => f());
      u6.then((f) => f());
      u7.then((f) => f());
      u8.then((f) => f());
      u3?.then((f) => f());
    };
  });
</script>

<svelte:window onkeydown={onLibKey} />

<div class="library" onpointerdown={onPtrDown} onpointerup={libPointerUp} onpointermove={libPointerMove} onpointercancel={clearLongPress}>
  <div class="toolbar" data-tauri-drag-region bind:clientHeight={tbH}>
    {#if selectMode}
      <span class="sel-count" data-testid="lib-sel-count">{t("library.toolbar.selCount", { count: selCount() })}</span>
      <button data-testid="lib-sel-all" onclick={selectAll}>{t("library.toolbar.selectAll")}</button>
      <button data-testid="lib-sel-invert" onclick={invertSelection}>{t("library.toolbar.invert")}</button>
      <button data-testid="lib-sel-remove" onclick={() => batchDelete(false)}>{t("library.ctx.removeOnly")}</button>
      <button class="danger" data-testid="lib-sel-delete" onclick={() => batchDelete(true)}>{t("common.action.delete")}</button>
      <button data-testid="lib-sel-cancel" onclick={exitSelect}>{t("common.confirm.cancel")}</button>
      {#if selectPartial}<span class="sel-note" data-testid="lib-sel-partial">{t("library.toolbar.selectPartial")}</span>{/if}
      <div class="spacer" data-tauri-drag-region></div>
    {:else}
    <span class="logo">TidePage</span>
    <button class="primary" data-testid="lib-btn-add-folder" onclick={addFolder}><IconPlus />{t("library.toolbar.addFolder")}</button>
    <div class="refresh-wrap">
      <button
        onpointerdown={refreshDown}
        onpointerup={refreshUp}
        onpointerleave={refreshUp}
        onpointercancel={refreshUp}
        onclick={refreshClick}
        disabled={refreshing}
        data-testid="lib-btn-refresh"
        title={t("library.toolbar.refreshTip")}
      ><IconRefresh />{refreshing ? t("library.toolbar.refreshing") : t("library.toolbar.refresh")}</button>
      {#if refreshMenu}
        <div class="refresh-menu" role="menu">
          <button class="rm-item" data-testid="lib-btn-rescan" onclick={() => { refreshMenu = false; refreshAll(); }}>{t("library.toolbar.rescan")}</button>
        </div>
        <div class="rm-backdrop" onclick={() => (refreshMenu = false)}></div>
      {/if}
    </div>
    <!-- 搜索框 + 清除键：外壳只提供定位参照与固定总宽（见 .search-wrap 的注释）。
         × 的规格走 app.css 的公共类 .fr-clear，与设置弹窗搜索框那颗完全同一份配方。
         只在有输入时渲染：空搜索框上常驻一颗灰按钮既没有可清的东西又是纯噪声。
         onclick 只把绑定值置空 —— queryInput 的 250ms 防抖与 reload() 照常经它生效，
         本轮不碰任何查询逻辑。data-testid 必须留在 input 自身（测试读它的 placeholder）。 -->
    <span class="search-wrap">
      <input class="search" data-testid="lib-search" placeholder={t("library.toolbar.searchPlaceholder")} bind:value={queryInput} />
      {#if queryInput}
        <button
          class="fr-clear"
          data-testid="lib-search-clear"
          title={t("common.action.clear")}
          aria-label={t("library.toolbar.clearSearchAria")}
          onclick={() => (queryInput = "")}
        ><IconX size={14} stroke={1.5} /></button>
      {/if}
    </span>
    <select data-testid="lib-sort" bind:value={sortKey} onchange={onSortChange}>
      <option value="lastread">{t("library.sort.lastread")}</option>
      <option value="added">{t("library.sort.added")}</option>
      <option value="title">{t("library.sort.title")}</option>
      <option value="recent">{t("library.sort.recent")}</option>
      <option value="size">{t("library.sort.size")}</option>
    </select>
    <button class="icon" data-testid="lib-btn-reverse" data-tip={t("library.toolbar.reverseTip")} aria-label={t("library.toolbar.reverseTip")} onclick={toggleReverse}>
      <IconArrowsSort size={20} stroke={1.5} />
    </button>
    <div class="filter-wrap">
      <button
        class="cat-btn"
        class:active={filterPanelOpen || hasFilter}
        aria-expanded={filterPanelOpen}
        data-testid="lib-btn-filter"
        onclick={() => (filterPanelOpen = !filterPanelOpen)}
      >{t("library.toolbar.category")}{#if hasFilter} · {filterFormats.length + filterStates.length}{/if}</button>
      {#if hasFilter}
        <button class="filter-chip" data-testid="lib-filter-chip" title={t("library.toolbar.clearFilterTip")} onclick={clearFilter}>
          {t("library.toolbar.filtered")} <span class="fx"><IconX size={14} stroke={1.5} /></span>
        </button>
      {/if}
    </div>
    <div class="spacer" data-tauri-drag-region></div>
    <div class="lib-tb-group" use:dragSort={{ onDrop: libDrop }}>
      {#each orderedLibBtns as b (b.id)}
        {@const Icon = b.icon}
        <button
          class="icon lib-tb-btn tip-left"
          data-drag-id={b.id}
          data-testid={"lib-btn-" + b.id}
          aria-label={b.title}
          data-tip={b.title}
          onclick={b.run}
        >
          <Icon size={20} stroke={1.5} />
        </button>
      {/each}
    </div>
    {/if}
  </div>

  {#if filterPanelOpen}
    <div class="filter-panel">
      <div class="fp-group">
        <span class="fp-label">{t("library.filter.format")}</span>
        <div class="fp-tags">
          {#each availableFormats as f (f)}
            <button class="tag" class:active={filterFormats.includes(f)} onclick={() => toggleFormat(f)}>
              {f.toUpperCase()}
            </button>
          {/each}
          {#if !availableFormats.length}<span class="fp-empty">{t("library.filter.empty")}</span>{/if}
        </div>
      </div>
      <div class="fp-group">
        <span class="fp-label">{t("library.filter.readState")}</span>
        <div class="fp-tags">
          {#each ALL_STATES as s (s)}
            <button class="tag" class:active={filterStates.includes(s)} onclick={() => toggleState(s)}>
              {stateLabel(s)}
            </button>
          {/each}
        </div>
      </div>
      {#if hasFilter}
        <button class="fp-clear" data-testid="lib-filter-clear" onclick={clearFilter}>{t("library.filter.clear")}</button>
      {/if}
    </div>
  {/if}

  {#if folders.length || foldersFailed}
    <div class="folders">
      <button
        class="folder-chip"
        class:active={selectedFolder === null}
        data-testid="lib-folder-all"
        title={t("library.folder.allTip")}
        onclick={() => selectFolder(null)}
      >{t("library.folder.all")}</button>
      {#each folders as f (f)}
        <span
          class="folder-chip"
          class:active={selectedFolder === f}
          role="button"
          tabindex="0"
          data-testid="lib-folder-chip"
          title={t("library.folder.chipTip", { path: f })}
          onclick={() => selectFolder(f)}
          onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectFolder(f); } }}
        >
          <IconFolder size={16} stroke={1.5} /> {f}
          <button
            class="x"
            title={t("library.folder.removeTip")}
            aria-label={t("library.folder.removeTip")}
            onclick={(e) => { e.stopPropagation(); removeFolder(f); }}
            onkeydown={(e) => { e.stopPropagation(); }}
          ><IconX size={14} stroke={1.5} /></button>
        </span>
      {/each}
      {#if foldersFailed}<span class="fp-empty" data-testid="lib-folders-failed">{t("library.folders.failed")}</span>{/if}
    </div>
  {/if}

  <div
    class="grid"
    class:boxing={boxActive}
    bind:this={gridEl}
    onscroll={onScroll}
    onclick={onGridClick}
    onpointerdown={onGridPointerDown}
    onpointermove={onGridPointerMove}
    onpointerup={onGridPointerUp}
    onpointercancel={onGridPointerCancel}
    onpointerleave={onGridPointerLeave}
  >
    {#if loading && total === 0}
      <div class="empty-state" data-testid="lib-hint-loading">{t("library.empty.loading")}</div>
    {:else if total === 0}
      <div class="lib-empty" data-testid="lib-hint-empty">
        <span class="lib-empty-ico">
          {#if query.trim()}<IconSearchOff stroke={1.5} />{:else}<IconBooks stroke={1.5} />{/if}
        </span>
        <p class="lib-empty-t">{query.trim() ? t("library.empty.noMatch") : t("library.empty.blankTitle")}</p>
        {#if !query.trim()}<p class="lib-empty-d">{t("library.empty.blankHint")}</p>{/if}
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
                  {@const ri = ringInfo(b)}
                  {@const cu = coverUrl(b)}
                  {@const sel = isSelected(b.id)}
                  <div
                    class="card"
                    class:selected={sel}
                    data-book-id={b.id}
                    style="max-width:{CARD_MAX}px;"
                    onpointerdown={(e) => cardPointerDown(e, b)}
                    onclick={(e) => onCardClick(e, b)}
                    ondblclick={(e) => onCardDblClick(e, b)}
                    oncontextmenu={(e) => openCtx(e, b)}
                  >
                    <div class="cover" style="height:{coverH}px;">
                      <!-- 用 {#key cu} 让封面 URL 变化时 <img> 整体重挂（webview 对已挂载自定义协议 img 换 src 不可靠重绘）：
                           换默认底图（仅无内嵌封面书的 URL 含 &db= 变化）或换/重提取封面（cover_ver 变化）即时生效；
                           有内嵌封面书 URL 不变、不重挂、不闪白。 -->
                      {#key cu}
                        <img src={cu} alt={t("library.book.coverAlt", { title: b.title })} loading="lazy" decoding="async" draggable="false" />
                      {/key}
                      <span class="fmt">{b.format.toUpperCase()}</span>
                      {#if selectMode}
                        <span class="sel-badge" class:on={sel} aria-hidden="true">{#if sel}<IconCheck />{/if}</span>
                      {/if}
                      {#if ri.show && config.library.showCoverProgress}
                        <ProgressRing pct={ri.pct} full={ri.full} />
                      {/if}
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
            {@const ri = ringInfo(b)}
            {@const cu = coverUrl(b)}
            {@const sel = isSelected(b.id)}
            <div
              class="row"
              class:selected={sel}
              data-book-id={b.id}
              style="top:{i * ROW_H}px"
              onpointerdown={(e) => cardPointerDown(e, b)}
              onclick={(e) => onCardClick(e, b)}
              ondblclick={(e) => onCardDblClick(e, b)}
              oncontextmenu={(e) => openCtx(e, b)}
            >
              {#if selectMode}
                <span class="sel-badge row-badge" class:on={sel} aria-hidden="true">{#if sel}<IconCheck />{/if}</span>
              {/if}
              <div class="row-cover-wrap">
                {#key cu}
                  <img class="row-cover" src={cu} alt={t("library.book.coverAlt", { title: b.title })} loading="lazy" decoding="async" />
                {/key}
                {#if ri.show && config.library.showCoverProgress}
                  <ProgressRing pct={ri.pct} full={ri.full} size={22} />
                {/if}
              </div>
              <div class="row-meta">
                <div class="t">{b.title}</div>
                <div class="a">{b.author}</div>
              </div>
              <div class="row-size">{fmtSize(b.size)}</div>
              <div class="row-date">{b.last_read ? t("library.book.readAt", { date: fmtTime(b.last_read) }) : fmtTime(b.added_at)}</div>
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  </div>

  <div class="statusbar" data-testid="lib-statusbar">
    <span data-testid="lib-statusbar-count">{t("library.statusbar.count", { count: total })}</span>
    <span class="build" title={t("common.app.versionTip")}>TidePage {APP_VERSION}</span>
    {#if refreshProgress && refreshProgress.phase !== "done"}
      <div class="scan-progress" title={t("library.statusbar.refreshScanTip")}>
        <div class="scan-bar">
          {#if refreshProgress.total > 0}
            <div class="scan-fill" style="width:{refreshPct}%"></div>
          {:else}
            <div class="scan-fill indeterminate"></div>
          {/if}
        </div>
        <span class="scan-text">
          {refreshPhaseText(refreshProgress)}{#if refreshProgress.total > 0} · {refreshPct}%{/if}
        </span>
      </div>
    {/if}
    {#if folderScan && folderScan.running}
      <div class="scan-progress" title={t("library.statusbar.folderScanTip")}>
        <div class="scan-bar"><div class="scan-fill indeterminate"></div></div>
        <span class="scan-text" data-testid="lib-statusbar-folderscan">{t("library.statusbar.folderScanText", { count: folderScan.scanned })}</span>
      </div>
    {/if}
    {#if metaProgress && metaProgress.phase !== "done" && metaProgress.total > 0}
      <div class="scan-progress meta-progress" title={t("library.statusbar.metaTip")}>
        <div class="scan-bar"><div class="scan-fill meta-fill" style="width:{metaPct}%"></div></div>
        <span class="scan-text" data-testid="lib-statusbar-meta">{t("library.statusbar.metaText", { done: metaProgress.done, total: metaProgress.total, pct: metaPct })}</span>
      </div>
    {/if}
    <div class="spacer"></div>
    <button class="icon view-btn" class:active={view === "grid"} data-testid="lib-view-grid" title={t("common.view.grid")} aria-pressed={view === "grid"} onclick={() => setView("grid")}><IconLayoutGrid /></button>
    <button class="icon view-btn" class:active={view === "list"} data-testid="lib-view-list" title={t("common.view.list")} aria-pressed={view === "list"} onclick={() => setView("list")}><IconLayoutList /></button>
  </div>

  <div class="sel-box" class:on={boxActive} bind:this={boxEl}></div>

  {#if ctxMenu}
    <div class="ctx-backdrop" onclick={() => (ctxMenu = null)} oncontextmenu={(e) => { e.preventDefault(); ctxMenu = null; }}>
      <div class="ctx ctx-menu" bind:this={ctxEl} style="left:{ctxPos.left}px; top:{ctxPos.top}px;">
        <button class="ctx-item" data-testid="lib-ctx-open" onclick={() => { tryOpenBook(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconBook2 size={16} stroke={1.5} /></span>{t("library.ctx.open")}</button>
        <button class="ctx-item" data-testid="lib-ctx-detail" onclick={() => { detailBook = ctxMenu!.book; ctxMenu = null; }}><span class="ci"><IconInfoCircle size={16} stroke={1.5} /></span>{t("library.ctx.detail")}</button>
        {#if ctxMenu.book.format === "txt"}
          <button class="ctx-item" data-testid="lib-ctx-convert" onclick={() => { openConvert(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconArrowsExchange size={16} stroke={1.5} /></span>{t("library.ctx.convert")}</button>
        {/if}
        <div class="ctx-sep"></div>
        <button class="ctx-item" data-testid="lib-ctx-cover" onclick={() => { setCover(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconPhoto size={16} stroke={1.5} /></span>{t("library.ctx.setCover")}</button>
        <button class="ctx-item" data-testid="lib-ctx-reextract" onclick={() => { reextract(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconRefresh size={16} stroke={1.5} /></span>{t("library.ctx.reextract")}</button>
        {#if effectiveState(ctxMenu.book) !== "read"}
          <button class="ctx-item" data-testid="lib-ctx-mark-read" onclick={() => { markReadState(ctxMenu!.book, "read"); ctxMenu = null; }}><span class="ci"><IconCircleCheck size={16} stroke={1.5} /></span>{t("library.ctx.markRead")}</button>
        {/if}
        {#if effectiveState(ctxMenu.book) !== "unread"}
          <button class="ctx-item" data-testid="lib-ctx-mark-unread" onclick={() => { markReadState(ctxMenu!.book, "unread"); ctxMenu = null; }}><span class="ci"><IconCircle size={16} stroke={1.5} /></span>{t("library.ctx.markUnread")}</button>
        {/if}
        <button class="ctx-item" data-testid="lib-ctx-reveal" onclick={() => { api.revealInExplorer(ctxMenu!.book.path); ctxMenu = null; }}><span class="ci"><IconFolder size={16} stroke={1.5} /></span>{t("library.ctx.reveal")}</button>
        <div class="ctx-sep"></div>
        <button class="ctx-item danger" data-testid="lib-ctx-remove" onclick={() => { deleteBook(ctxMenu!.book, false); ctxMenu = null; }}><span class="ci"><IconArrowBackUp size={16} stroke={1.5} /></span>{t("library.ctx.removeOnly")}</button>
        <button class="ctx-item danger" data-testid="lib-ctx-delete-file" onclick={() => { deleteBook(ctxMenu!.book, true); ctxMenu = null; }}><span class="ci"><IconTrash size={16} stroke={1.5} /></span>{t("library.ctx.deleteFile")}</button>
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
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    /* 右侧留出自绘窗口控制按钮的宽度并留自然间距（宽度由 app.css 的 --wc-reserve-w 统一定义） */
    padding-right: var(--wc-reserve-w);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.02), 0 6px 16px -12px rgba(23, 25, 30, 0.35);
    flex-shrink: 0;
    position: relative;
    z-index: var(--z-raised);
  }

  .logo {
    font-weight: 700;
    font-size: var(--fs-lg);
    color: var(--accent);
    margin-right: var(--sp-3);
    letter-spacing: -0.02em;
    user-select: none;
  }

  .search {
    width: 220px;
  }

  /* 搜索框外壳：给 .fr-clear 提供 position: relative 的定位参照。
     总宽必须在这里显式给成 220px（与包壳前的裸 input 同宽）：包一层之后外壳成了
     .toolbar 那行 flex 里的 item，宽度一旦由内容决定，右侧那排图标按钮与
     自绘窗口控制按钮就会整体横向移位。内部的 input 再占满外壳，两者相加仍是一个 220px。
     padding-right 是 .fr-clear 的容器契约（= 键宽 20 + inset 4 + 余量 2），
     少了它长文本会滑到 × 底下；配方与这个数值都在 app.css 的 .fr-clear 处。 */
  .search-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
    width: 220px;
    flex-shrink: 0;
  }

  .search-wrap .search {
    width: 100%;
    padding-right: 26px;
  }

  .spacer { flex: 1; }

  /* ---------- 分类筛选 ---------- */
  .filter-wrap {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }

  /* 「分类」按钮的选中配色走 app.css 的「选中态配方」一组。 */
  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: var(--radius-pill);
    padding: 4px 10px;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .filter-chip .fx {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    line-height: 1;
    opacity: 0.85;
  }
  .filter-chip:hover { background: color-mix(in srgb, var(--accent) 18%, transparent); }

  .filter-panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
    animation: fr-menu-in var(--dur-fast) var(--ease-in) both;
  }

  .fp-group {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
  }

  .fp-label {
    flex-shrink: 0;
    width: 56px;
    padding-top: 5px;
    font-size: var(--fs-xs);
    color: var(--fg2);
    user-select: none;
  }

  .fp-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }

  .fp-empty { color: var(--fg2); font-size: var(--fs-xs); padding-top: 5px; }

  /* 筛选面板里的格式/状态标签：形状在下面，配色走 app.css 的「选中态配方」。 */
  .fp-tags .tag {
    border: 1px solid var(--line);
    background: var(--panel2);
    color: var(--fg2);
    border-radius: var(--radius-pill);
    padding: 4px 12px;
    font-size: var(--fs-xs);
    cursor: pointer;
    transition: background var(--dur) var(--ease), color var(--dur) var(--ease), border-color var(--dur) var(--ease);
  }
  .fp-tags .tag:hover { background: var(--hover-bg); color: var(--fg); }

  .fp-clear {
    align-self: flex-start;
    border: none;
    background: transparent;
    color: var(--fg2);
    font-size: var(--fs-xs);
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .fp-clear:hover { background: var(--hover-bg); color: var(--fg); }

  /* ---------- 文件夹胶囊 ---------- */
  /* 上下左右给一致的留白：原先底部贴死 --bg，胶囊下半截悬在网格上缘，缺一层缓冲。 */
  .folders {
    display: flex;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    flex-wrap: wrap;
  }

  .folder-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    background: var(--panel2);
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    padding: 4px 8px 4px 12px;
    font-size: var(--fs-xs);
    font-family: inherit;
    color: var(--fg2);
    max-width: 420px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
    transition: background var(--dur) var(--ease), color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .folder-chip:hover { background: var(--hover-bg); color: var(--fg); }

  /* 选中态的配色走 app.css 的「选中态配方」一组（与筛选芯片 / 分段控件同源）。 */

  /* 胶囊里的移除按钮：全项目唯一一个没走 button.icon 归一化的关闭按钮，也是观感上
     最突兀的一个 —— 12px 的胶囊文字里塞着一个 ≥30px 高、24px 宽的 X，描边却只有 1.5，
     于是读成「块很大但线很细」。三处修正缺一不可：
       ① min-height 必须显式重置 —— 全局 button 带 min-height: var(--ctl-h)(30px)，
          旧规则重置了 border/background/padding/color/border-radius 唯独漏了它，
          整颗胶囊因此被撑到近 38px；
       ② 图标落到 --icon-xs（14px，清除类按钮那一档，见 app.css 的图标阶梯）；
       ③ 默认 opacity .55 降噪，悬停胶囊 / 键盘聚焦时才到 1。
     刻意不用 opacity: 0（曾用过，见下）：0 会让元素看不见却照样接收指针事件 ——
     书库 X 还叠了 pointer-events: none，点它事件落到胶囊上就是「选中文件夹」而不是移除，
     而搜索弹层那颗更糟：它整个头部都是拖拽把手，事件穿透会直接启动拖拽、弹层位移。
     两套办法都会制造「看不见却在那儿」的隐形命中区，opacity .55 既保留可见性与命中区，
     又把对比压低，才是「不抢眼」的正确做法。全项目四颗 × 统一这一档。
     flex-shrink: 0 是第四件：胶囊有 max-width + overflow，长路径下 X 会被横向压成一条竖线。 */
  .folder-chip .x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--ctl-h-xs);
    min-height: var(--ctl-h-xs);
    flex-shrink: 0;
    border: none;
    background: transparent;
    padding: 0;
    cursor: pointer;
    color: var(--fg2);
    border-radius: var(--radius-pill);
    opacity: 0.55;
    transition: background var(--dur) var(--ease), color var(--dur) var(--ease),
      opacity var(--dur) var(--ease);
  }

  .folder-chip:hover .x,
  .folder-chip .x:focus-visible {
    opacity: 1;
  }

  .folder-chip .x:hover {
    background: var(--hover-bg);
    color: var(--fg);
  }

  /* ---------- 网格 ---------- */
  .grid {
    flex: 1;
    min-height: 0; /* flex 列中可滚动子项必须显式置 0，否则纵向不受视口约束、末行被窗口边缘裁切 */
    overflow-y: auto;
    overflow-x: hidden; /* 列宽用 minmax(0,1fr) 已不溢出；禁掉横向滚动条，避免其占位挤动纵向布局 */
    /* 左右 20px 与 gridPadX 推导一致；底部多留一屏呼吸空间 */
    padding: 16px 20px 32px;
    position: relative;
  }

  /* 左键拉框期间禁止文本选中 */
  .grid.boxing {
    user-select: none;
    -webkit-user-select: none;
  }

  /* 行偏移全部由内联 top 写死（.grid-row 是 position:absolute + top），
     这两个容器上没有任何 transform —— 常驻 will-change: transform 只会白占一块
     合成层（虚拟滚动里每屏几十张卡，全屏尺寸的图层是真实开销）。 */
  .grid-inner {
    position: relative;
    width: 100%;
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

  /* 悬停抬升：克制档 —— 底色 + 2px 位移，投影只由 .cover 那一层承担（--shadow-sm → --shadow-md），
     封面放大也只到 1.02。此前是四重叠加（底色 + -4px + 卡片自身 --shadow-md + 封面 1.055），
     在一屏几十张卡的网格里等于满屏都在跳；两个参考系统的卡片 hover 都只到
     「box-shadow 0 → 0 2px 8px / 4%」这一档（minimalist-ui 的克制、Linear 的低幅抬升）。
     用 premium 落位曲线模拟质量感（skill: haptic depth）。 */
  .card {
    width: 100%;
    box-sizing: border-box;
    cursor: pointer;
    border-radius: var(--radius-lg);
    padding: var(--sp-2);
    /* 刻意不transition box-shadow：卡片自身不再有投影变化，留着只会多跑一条无用过渡 */
    transition: background var(--dur-lift) var(--ease-premium),
      transform var(--dur-lift) var(--ease-premium);
  }

  .card:hover {
    background: var(--hover-bg);
    transform: translateY(-2px);
  }

  .card:active {
    transform: translateY(-1px) scale(0.99);
    transition-duration: var(--dur-fast);
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

  /* 封面：外层 8px 圆角容器，内层图片裁切；极淡描边 + 顶缘高光营造嵌入托盘的实体感 */
  .cover {
    position: relative;
    width: 100%;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: var(--panel2);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm), inset 0 1px 0 rgba(255, 255, 255, 0.14);
    transition: box-shadow var(--dur-lift) var(--ease-premium);
  }

  .card:hover .cover {
    box-shadow: var(--shadow-md), inset 0 1px 0 rgba(255, 255, 255, 0.14);
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform var(--dur-lift) var(--ease-premium);
  }

  .card:hover .cover img {
    transform: scale(1.02);
  }

  /* 格式角标：半透明深底 + 白色小字，10px 字距 +0.015em。
     注意：不用 backdrop-blur —— 卡片处于虚拟滚动网格内，滚动中模糊层会持续触发 GPU 重绘、
     在低端机上掉帧（perf: 禁止对滚动内容用 backdrop-filter）。改用实心半透明底达到同等观感。 */
  .fmt {
    position: absolute;
    right: 8px;
    top: 8px;
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    font-size: var(--fs-2xs);
    font-weight: 500;
    line-height: 1;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    letter-spacing: var(--ls-xs);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18);
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

  /* ---------- 列表 ---------- */
  /* 同 .grid-inner：没有 transform，不需要常驻合成层 */
  .list-inner {
    position: relative;
    width: 100%;
  }

  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 64px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-sm);
    cursor: pointer;
    /* box-shadow 追加在末尾，用于 .row.selected 的选中环渐显。
       时长/曲线跟随本行原有的 --dur / --ease，不跟 .card 的 --dur-lift：
       列表行的交互反馈本来就比卡片轻。 */
    transition: background var(--dur) var(--ease), box-shadow var(--dur) var(--ease);
  }

  .row:hover { background: var(--hover-row); }

  /* 列表行封面：32×48 = 1.5，与网格封面、书籍详情（aspect-ratio 2/3）同一比例。
     高度必须保持 48px —— .row 高 64px、上下 padding 各 8px，内容盒正好 48px，
     动它会与虚拟滚动的 ROW_H=66 失配（行内几何不得变动）。 */
  .row-cover {
    width: 32px;
    height: 48px;
    object-fit: cover;
    border-radius: var(--radius-xs);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm);
  }

  /* 列表行封面容器：为进度环提供定位上下文（不改变 32×48 布局尺寸）。 */
  .row-cover-wrap {
    position: relative;
    flex-shrink: 0;
    width: 32px;
    height: 48px;
    display: block;
  }

  .row-meta { flex: 1; min-width: 0; }
  .row-meta .t { font-size: var(--fs-md); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-meta .a { font-size: var(--fs-xs); color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-size, .row-date { color: var(--fg2); font-size: var(--fs-xs); width: 90px; text-align: right; font-variant-numeric: tabular-nums; }

  /* ---------- 空状态 ---------- */
  /* 结构照 WebdavBrowser 的 .wd-empty-cfg / .wd-empty-ico / .wd-empty-t / .wd-empty-d：
     图标 + 标题 + 描述三段。此前这里只有一行灰字，「导入书籍」这个唯一可执行的动作
     与说明文字挤在同一段里，用户既不知道发生了什么、也不知道下一步该点哪。
     min-height: 100% + justify-content: center 借 .grid 的确定高度把它撑满并垂直居中
     （同 Reader.svelte 里 .empty-state 的做法），不给它写死 vh 数值。 */
  .lib-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    min-height: 100%;
    padding: var(--sp-4);
    text-align: center;
    text-wrap: pretty;
  }

  .lib-empty-ico {
    line-height: 1;
    margin-bottom: var(--sp-1);
    color: var(--fg2);
    opacity: 0.7;
  }

  .lib-empty-t {
    margin: 0;
    font-size: var(--fs-md);
    letter-spacing: var(--ls-md);
    font-weight: 600;
    color: var(--fg);
  }

  .lib-empty-d {
    margin: 0;
    max-width: 42ch;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    line-height: 1.8;
    color: var(--fg2);
    white-space: pre-line;
  }

  /* ---------- 状态栏 ---------- */
  .statusbar {
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

  .statusbar .build {
    font-size: var(--fs-xs);
    opacity: 0.55;
  }

  .lib-tb-group {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
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

  /* 当前视图高亮：中性选中态（--bg-elev 白实体 + --fg 深字），与 app.css 的选中态配方同源。
     accent 只留给小面积强调，「我在哪个视图」用主色会把两枚图标按钮读成两枚标签。
     ⚠️ 悬停必须单独兜：全局 button.icon:hover:not(:disabled) 是 (0,3,1)，本条只有
     (0,3,0) —— 直接写 :hover 是 (0,4,0) 仍比它低一位，会被「鼠标一停上去白底就没了」。
     故按 app.css 选中态组同样的手法加 body 锚点：(0,5,1) 稳压 (0,3,1)。 */
  .view-btn.active {
    background: var(--bg-elev);
    color: var(--fg);
    opacity: 1;
  }

  :global(body) .view-btn.active:hover:not(:disabled) {
    background: var(--bg-elev);
    color: var(--fg);
    opacity: 1;
  }

  /* ---------- 刷新进度 ---------- */
  /* 刷新按钮 + 长按菜单 */
  .refresh-wrap { position: relative; display: inline-flex; }

  .refresh-menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    animation: fr-menu-in var(--dur-fast) var(--ease-in) both;
  }

  .refresh-menu .rm-item {
    border: none;
    background: transparent;
    box-shadow: none;
    text-align: left;
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    font-size: var(--fs-sm);
    color: var(--fg);
  }

  .refresh-menu .rm-item:hover:not(:disabled) { background: var(--hover-bg); border-color: transparent; }

  .rm-backdrop { position: fixed; inset: 0; z-index: var(--z-drawer); }

  .scan-progress {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 180px;
  }

  /* 进度条在状态栏里横向撑满（轨道的粗细/圆角/配色由 app.css 的全局进度条给） */
  .scan-progress .scan-bar {
    flex: 1;
  }

  /* 此处只留本组件特有的不确定态与元信息进度弱化 */
  .meta-progress .meta-fill { background: var(--accent); opacity: 0.7; }

  /* 不确定进度（total<=0，如添加目录阶段）：来回滑动动画条。
     keyframes 定义在全局 app.css（Svelte 会重命名组件内 keyframes，导致 app.css 的
     reduce-motion 覆盖无法引用），此处仅引用其名字。 */
  .scan-fill.indeterminate {
    width: 35%;
    transition: none;
    animation: scan-indeterminate 1.1s ease-in-out infinite;
  }

  .scan-text {
    white-space: nowrap;
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
  }

  /* ---------- 右键菜单 ---------- */
  /* .ctx-backdrop / .ctx-menu / .ctx-item / .ctx-sep 全部走 app.css 的全局规格。
     本组件特有的是「fixed + 视口坐标定位 + 菜单是遮罩的子节点」这一件事。
     保留 .ctx 这个类名是因为自动化脚本用它作为「菜单已出现」的等待信号。 */
  .ctx-menu {
    position: fixed;
    z-index: var(--z-popover);
  }

  /* ---------- 多选（长按 / 右键拉框） ---------- */
  /* 选中态描边用 box-shadow 而不是 outline：outline-color 在多数引擎里不插值，
     transition 无效。inset box-shadow 画在边框内侧，视觉与 outline-offset:-2px 等价且可过渡。 */
  .card.selected,
  .row.selected {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .card.selected .cover {
    box-shadow: var(--shadow-md), inset 0 1px 0 rgba(255, 255, 255, 0.14);
  }

  .sel-badge {
    position: absolute;
    left: 6px;
    top: 6px;
    width: var(--ctl-h-xs);
    height: var(--ctl-h-xs);
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-pill);
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    font-weight: 700;
    line-height: 1;
    color: #fff;
    background: color-mix(in srgb, var(--panel) 82%, transparent);
    border: 1.5px solid color-mix(in srgb, var(--fg2) 50%, transparent);
    box-shadow: var(--shadow-sm);
    z-index: var(--z-raised);
    pointer-events: none;
  }

  .sel-badge.on {
    background: var(--accent);
    border-color: var(--accent);
  }


  .row-badge {
    left: 6px;
    top: 6px;
    width: var(--ctl-h-xs);
    height: var(--ctl-h-xs);
    font-size: var(--fs-2xs);
    letter-spacing: var(--ls-2xs);
  }

  .sel-count {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
    user-select: none;
  }

  .sel-note {
    font-size: var(--fs-xs);
    color: var(--fg2);
    white-space: nowrap;
  }

  .toolbar button.danger {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
  }

  .toolbar button.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--danger);
  }

  /* 常驻元素（避免拉框时增删节点）：位置走 transform，尺寸由 rAF 写入；opacity 控制显隐。 */
  .sel-box {
    position: fixed;
    top: 0;
    left: 0;
    width: 0;
    height: 0;
    z-index: var(--z-drawer);
    pointer-events: none;
    border: 1px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-radius: var(--radius-xs);
    opacity: 0;
    /* 只有 opacity 补过渡：marquee 的 transform 每帧由 rAF 改写，
       加 transform 过渡会和拖拽手感打架（每帧都在追一个滞后的目标值）。 */
    transition: opacity var(--dur-fast) var(--ease);
    will-change: transform;
  }

  .sel-box.on {
    opacity: 1;
  }
</style>
