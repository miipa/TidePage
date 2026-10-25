<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  /** B7：PDF（含 pdfjs 主模块）按需加载，避免进入书库首屏包。 */
  let pdfModPromise: Promise<typeof import("../lib/pdf")> | null = null;
  function pdfMod(): Promise<typeof import("../lib/pdf")> {
    return (pdfModPromise ??= import("../lib/pdf"));
  }
  import { COVER_FAIL_KEY } from "../lib/openError";
  import {
    createPdfCoverQueue,
    enqueueIdle,
    enqueueVisible,
    pickIdleBatch,
    takeNext,
  } from "../lib/pdfCoverQueue";
  import { api, coverUrl, bumpCoverBust, type TagInfo } from "../lib/api";
  import { emit } from "@tauri-apps/api/event";
  import {
    deserializeShelves,
    stampShelfUpdates,
    serializeShelves,
    shelfFilter,
    shelfFromFilter,
    uniqueShelfName,
    type Shelf,
  } from "../lib/libraryShelves";
  import { appConfirm, appChoose, config, convertState, libSession, openBookInNewWindow, pendingOpen, saveConfig, toast, ui, APP_VERSION, LIBRARY_TOPBAR_IDS, type LibraryTopbarId } from "../lib/stores.svelte";
  import { dateLocale, errText, t, type MessageKey } from "../lib/i18n/index.svelte";
  import { shouldRemindMaintenance, estimateVacuumTemp, formatBytes } from "../lib/maintenance";
  import { updateState } from "../lib/update.svelte";
  import { dragSort } from "../lib/dragSort";
import { reorderByDrop } from "../lib/reorder";
  import { BOOK_EXTENSIONS } from "../lib/bookExt";
  import { effectiveState, type ReadState } from "../lib/readState";
  import { bookListKey, resolveListTotal, shouldCountTotal } from "../lib/bookList";
  import { HIDDEN_CACHE_KEY, hiddenIdsFromGroups, parseHiddenCache, serializeHiddenCache, type DupDisplayGroup } from "../lib/dupHiddenCache";
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
    IconBooks,
    IconCopy,
    IconPlus,
    IconX,
  } from "@tabler/icons-svelte";
  /** B7：详情弹窗按需加载（打开时/空闲预取），不进入书库首屏 chunk。 */
  let maintReminder = $state<{ freelist: number; need: number } | null>(null);
  let maintBusy = $state(false);
  /** 横幅内直接整理数据库：复用维护页同一 vacuum 命令与确认文案；不改 7 天关闭逻辑。 */
  async function vacuumFromBanner() {
    const ok = await appConfirm(t("settings.maintenance.vacuumConfirm"));
    if (!ok) return;
    maintBusy = true;
    try {
      const freed = await api.vacuumDb();
      toast(t("settings.maintenance.vacuumDone", { size: formatBytes(freed) }));
      maintReminder = null;
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      maintBusy = false;
    }
  }
  $effect(() => {
    const timer = setTimeout(() => {
      void api.storageStats().then((s) => {
        if (shouldRemindMaintenance(s.freelist_bytes, s.db_bytes, config.maintenance.reminderDismissedUntil, Date.now())) {
          maintReminder = { freelist: s.freelist_bytes, need: estimateVacuumTemp(s.db_bytes, s.temp_same_disk).requiredBytes };
        }
      }).catch(() => undefined);
    }, 3000);
    return () => clearTimeout(timer);
  });
  let detailModPromise: Promise<typeof import("./BookDetailModal.svelte")> | null = null;
  function detailMod(): Promise<typeof import("./BookDetailModal.svelte")> {
    return (detailModPromise ??= import("./BookDetailModal.svelte"));
  }
  import DuplicateBooksModal from "./DuplicateBooksModal.svelte";
  import ProgressRing from "./ProgressRing.svelte";
  import { outDissolve, outFade } from "../lib/motion";
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
  // 主书库默认视图去重（只影响显示，不删文件）：hidden 为每组除代表书外的 book id。
  const cachedHidden = (() => {
    try { return parseHiddenCache(localStorage.getItem(HIDDEN_CACHE_KEY)); } catch { return null; }
  })();
  function metaFromGroups(groups: DupDisplayGroup[]): Map<number, { copies: number }> {
    const m = new Map<number, { copies: number }>();
    for (const g of groups) for (const id of g.memberIds) m.set(id, { copies: g.copies });
    return m;
  }
  let hiddenIds = new Set<number>(cachedHidden ? hiddenIdsFromGroups(cachedHidden.groups) : []);
  let dupMetaById = $state<Map<number, { copies: number }>>(cachedHidden ? metaFromGroups(cachedHidden.groups) : new Map());
  let dupRepIds = $state<Set<number>>(new Set(cachedHidden?.groups.map((g) => g.repId) ?? []));
  let dupGroupCount = $state(cachedHidden?.groups.length ?? 0);
  /** 「重复分组」筛选：只看有重复的书；无筛选时每组 1 本，有筛选时显示范围内全部副本。 */
  let duplicateView = $state(false);
  // 「重复」胶囊可隐藏（本地偏好，同步读取避免首屏闪烁）；空白右键可恢复。
  const DUP_CHIP_HIDDEN_KEY = "tp.lib.dupChipHidden";
  let dupChipHidden = $state((() => { try { return localStorage.getItem(DUP_CHIP_HIDDEN_KEY) === "1"; } catch { return false; } })());
  let dupChipMenu = $state<{ x: number; y: number } | null>(null);

  /** 点「重复」：从关闭进入时先清掉搜索/目录/格式/状态/标签范围，显示全库重复组；
   *  已开启时再点则退出。之后用户可再点目录/搜索/筛选在重复视图内收窄。 */
  function toggleDuplicateView() {
    if (duplicateView) {
      duplicateView = false;
      reload();
      return;
    }
    exitSelect();
    libSession.queryInput = "";
    libSession.query = "";
    clearTimeout(searchTimer);
    searchComposing = false;
    selectedFolder = null;
    saveFolder();
    filterFormats = [];
    filterStates = [];
    filterTags = [];
    saveFilter();
    filterPanelOpen = false;
    duplicateView = true;
    invalidateListTotal();
    reload();
  }

  function saveDupChipHidden(v: boolean) {
    dupChipHidden = v;
    try { localStorage.setItem(DUP_CHIP_HIDDEN_KEY, v ? "1" : "0"); } catch { /* ignore */ }
  }

  function hideDupChip() {
    saveDupChipHidden(true);
    if (duplicateView) {
      duplicateView = false;
      reload();
    }
  }

  function showDupChip() {
    saveDupChipHidden(false);
    dupChipMenu = null;
  }

  function openDupChipMenu(e: MouseEvent) {
    if (!dupChipHidden || dupGroupCount <= 0) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest(".folder-chip")) return;
    e.preventDefault();
    dupChipMenu = {
      x: Math.max(4, Math.min(e.clientX, window.innerWidth - 180)),
      y: Math.max(4, Math.min(e.clientY, window.innerHeight - 52)),
    };
  }
  let hiddenLoaded = false;
  let hiddenPromise: Promise<void> | null = null;
  let hiddenGen = 0;
  function isDefaultListView(): boolean {
    return !query.trim() && !selectedFolder && !hasFilter;
  }
  /** 取去重隐藏集合；失败退回显示全部，不阻断书库。库变更时 force 重新取。 */
  function loadHidden(force = false): Promise<void> {
    if (!force && hiddenLoaded) return Promise.resolve();
    if (hiddenPromise) return hiddenPromise;
    const gen = ++hiddenGen;
    hiddenPromise = api.duplicateDisplayHidden()
      .then((res) => {
        if (gen !== hiddenGen) return;
        hiddenIds = new Set(res.hiddenIds);
        dupMetaById = metaFromGroups(res.groups);
        dupRepIds = new Set(res.groups.map((g) => g.repId));
        dupGroupCount = res.groups.length;
        hiddenLoaded = true;
        try { localStorage.setItem(HIDDEN_CACHE_KEY, serializeHiddenCache(res.groups, Date.now())); } catch { /* ignore */ }
        if (indexMode && isDefaultListView() && idIndex) {
          // hidden 晚于首屏索引到达：默认视图重建一次，让总数/列表按去重结果收敛。
          void rebuildIndex(topAnchor());
        }
      })
      .catch(() => {
        if (gen !== hiddenGen) return;
        // 失败保留旧 hiddenIds/缓存，不清空；下次库变更/刷新会重取。
        hiddenLoaded = true;
      })
      .finally(() => {
        if (gen === hiddenGen) hiddenPromise = null;
      });
    return hiddenPromise;
  }
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
  // 搜索词放模块级共享状态：进书会销毁本组件，返回时输入框与检索结果都保留。
  let queryInput = $derived(libSession.queryInput);
  let query = $derived(libSession.query);
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
  /** 长按菜单「重新扫描书库」进行中：防止重复触发后台全量扫描。 */
  let rescanBusy = $state(false);
  /** 刷新扫描进度（由后端 refresh-progress 事件驱动）。 */
  let refreshProgress = $state<{ phase: string; done: number; total: number } | null>(null);
  /** 添加目录扫描进度（由后端 folder-scan-progress 事件驱动；只统计本次目录的条目）。 */
  let folderScan = $state<{
    running: boolean;
    phase: "walk" | "diff" | "insert" | "done";
    scanned: number;
    added: number;
  } | null>(null);
  /** 移除目录进度（由后端 remove-folder-progress 事件驱动；delete→clean→done）。 */
  let removeProgress = $state<{
    running: boolean;
    phase: "delete" | "clean" | "done";
    done: number;
    total: number;
  } | null>(null);
  /** 封面/元数据后台生成进度（由后端 meta-progress 事件驱动；不阻塞其它操作）。 */
  let metaProgress = $state<{ phase: string; done: number; total: number } | null>(null);
  /**
   * 上次扫描的异常态（`last_scan_stale`，后端每次扫描结束重算）：首屏常驻提示条的唯一来源。
   * 为什么不复用 `stale_pending_*`：那张表只存"待确认删除"的 path，目录没走完的路径
   * 根本不进它（被 plan_stale 剔进 skipped），从它读会得到"目录改名时一切正常"的错误结论。
   */
  let scanStale = $state<{
    refused: boolean;
    pending: number;
    skipped: number;
    unfinishedDirs: number;
    dirs: string[];
    at: number;
  } | null>(null);

  // DEV 探针：定位「进书返回后滚动位置丢失」。生产构建由 import.meta.env.DEV 摇掉。
  function scrollProbe(tag: string, data: Record<string, unknown>): void {
    if (!import.meta.env.DEV) return;
    try {
      const w = window as unknown as { __tpLibProbe?: unknown[] };
      const arr = w.__tpLibProbe ?? [];
      arr.push({ tag, t: Date.now(), ...data });
      const last = arr.slice(-24);
      w.__tpLibProbe = last;
      console.warn("[tp-lib-probe]", tag, JSON.stringify(data));
      void api.setSetting("debug.scrollProbe", JSON.stringify(last)).catch(() => {});
    } catch { /* ignore */ }
  }

  async function loadScanStale() {
    try {
      const raw = await api.getLastScanStale();
      if (!raw) {
        scanStale = null;
        return;
      }
      const v = JSON.parse(raw);
      scanStale = v && v.state === "stale" ? { refused: !!v.refused, pending: v.pending ?? 0, skipped: v.skipped ?? 0, unfinishedDirs: v.unfinishedDirs ?? 0, dirs: Array.isArray(v.dirs) ? v.dirs : [], at: v.at ?? 0 } : null;
    } catch {
      scanStale = null;
    }
  }
  /** 让最后一帧进度（N/N）短暂可见后再隐藏，避免一闪而过看不到。 */
  let metaHideTimer: ReturnType<typeof setTimeout> | undefined;
  /** library-changed 防抖：移除大目录等场景可能短时多次触发，合并为一次全量刷新，
   *  避免叠加多次整库索引重建把主线程压死。 */
  let libChangedTimer: ReturnType<typeof setTimeout> | undefined;
  /** 长按刷新弹出的菜单（普通点击=检查增删；长按=重新扫描书库）。 */
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
    { id: "dupes", labelKey: "library.topbar.dupes.label", titleKey: "library.topbar.dupes.title", icon: IconCopy, run: () => (dupesOpen = true) },
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
    const next = reorderByDrop(config.library.topbarOrder, from, target, place, (s) => s);
    if (!next) return;
    config.library.topbarOrder = next;
    saveConfig();
  }

  const refreshPhaseText = (p: { phase: string; done: number; total: number }): string => {
    if (p.phase === "check") return t("library.refresh.phase.check", { done: p.done, total: p.total });
    if (p.phase === "scan") return t("library.refresh.phase.scan", { done: p.done, total: p.total });
    return t("library.refresh.phase.done");
  };
  /** 目录扫描底栏文案：按 phase 选键；done 在 running:false 时已清空，归入遍历分支兜底。 */
  const folderScanText = (p: { phase: "walk" | "diff" | "insert" | "done"; scanned: number; added: number }): string => {
    if (p.phase === "diff") return t("library.statusbar.scanDiff");
    if (p.phase === "insert") return t("library.statusbar.scanInsert", { count: p.added });
    return t("library.statusbar.scanWalk", { count: p.scanned });
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
          tags: Array.isArray(o.tags) ? o.tags.filter((x: unknown) => typeof x === "string") : [],
        };
      }
    } catch { /* ignore */ }
    return { formats: [] as string[], states: [] as string[], tags: [] as string[] };
  })();
  let availableFormats = $state<string[]>([]);
  let filterFormats = $state<string[]>(_initFilter.formats);
  let filterStates = $state<string[]>(_initFilter.states as ReadState[]);
  let filterTags = $state<string[]>(_initFilter.tags);
  /** D3：标签列表 + 书籍标签筛选。 */
  let tags: TagInfo[] = $state([]);
  /** D3-3：智能书架（settings JSON，多窗口事件刷新）。 */
  let shelves: Shelf[] = $state([]);
  let shelfNameInput = $state("");
  const SHELVES_KEY = "library.shelves.v1";
  let filterPanelOpen = $state(false);
  // 任一维度有选中即视为「已筛选」。空数组=不约束（回完整书库）。
  const hasFilter = $derived(
    filterFormats.length > 0 || filterStates.length > 0 || filterTags.length > 0,
  );

  function saveFilter() {
    try {
      localStorage.setItem(
        "tidepage.lib.filter",
        JSON.stringify({ formats: filterFormats, states: filterStates, tags: filterTags }),
      );
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
    // 作用域变化：旧选中可能已不可见，先退出多选，避免批量删除误伤。
    exitSelect();
    selectedFolder = f;
    saveFolder();
    reload();
  }

  /** 进度环显示信息：显示条件统一为 effective_read_state != 'unread'（Task E）。 */
  function ringInfo(b: BookRow): { show: boolean; full: boolean; pct: number } {
    const st = effectiveState(b);
    if (st === "unread") return { show: false, full: false, pct: 0 };
    const full = b.read_state === 1; // 手动已读 → 满环 + ✓，不显示真实 read_pct
    return { show: true, full, pct: full ? 100 : b.read_pct || 0 };
  }

  function toggleFormat(f: string) {
    // 作用域变化：先退出多选，避免批量删除作用到当前不可见的旧选中书。
    exitSelect();
    filterFormats = filterFormats.includes(f) ? filterFormats.filter((x) => x !== f) : [...filterFormats, f];
    saveFilter();
    reload();
  }
  function toggleState(s: ReadState) {
    // 作用域变化：先退出多选，避免批量删除作用到当前不可见的旧选中书。
    exitSelect();
    filterStates = filterStates.includes(s) ? filterStates.filter((x) => x !== s) : [...filterStates, s];
    saveFilter();
    reload();
  }
  async function loadTags() {
    try {
      tags = await api.listTags();
    } catch {
      tags = [];
    }
  }

  function toggleTag(name: string) {
    exitSelect();
    filterTags = filterTags.includes(name)
      ? filterTags.filter((x) => x !== name)
      : [...filterTags, name];
    saveFilter();
    invalidateListTotal();
    reload();
  }

  async function loadShelves() {
    try {
      const raw = await api.getSetting(SHELVES_KEY);
      shelves = deserializeShelves(raw);
    } catch {
      shelves = [];
    }
  }

  async function persistShelves(next: Shelf[]) {
    // B9：保存/重命名时按内容变化补 updatedAt（供同步按时间裁决）。
    next = stampShelfUpdates(next, shelves, Date.now());
    shelves = next;
    try {
      await api.setSetting(SHELVES_KEY, serializeShelves(next));
      await emit("library-shelves-changed");
    } catch {
      toast(t("library.shelf.saveFailed"), "error");
    }
  }

  function saveCurrentShelf() {
    const name = shelfNameInput.trim();
    if (!name) {
      toast(t("library.shelf.nameRequired"), "error");
      return;
    }
    const shelf = shelfFromFilter(
      name,
      {
        folder: selectedFolder,
        formats: filterFormats,
        states: filterStates as string[],
        tags: filterTags,
        query,
      },
      shelves,
    );
    void persistShelves([...shelves, shelf]);
    shelfNameInput = "";
    toast(t("library.shelf.saved"));
  }

  function applyShelf(s: Shelf) {
    const f = shelfFilter(s);
    if (!f) return;
    exitSelect();
    selectedFolder = f.folder;
    filterFormats = f.formats;
    filterStates = f.states as ReadState[];
    filterTags = f.tags;
    libSession.queryInput = f.query;
    saveFolder();
    saveFilter();
    invalidateListTotal();
    reload();
  }

  async function removeShelf(s: Shelf) {
    const ok = await appConfirm(t("library.shelf.deleteConfirm", { name: s.name }));
    if (ok) {
      // B9：书架删除写同步墓碑（归一名称与 D5 shelf key 一致），供跨设备传播删除。
      void api.recordSyncTombstone("shelf", s.name.trim().toLowerCase(), Date.now()).catch(() => undefined);
      void persistShelves(shelves.filter((x) => x.id !== s.id));
    }
  }

  // D3：书架/标签的内联编辑（不用 window.prompt）。
  let shelfRenameId = $state<string | null>(null);
  let shelfRenameName = $state("");
  function startShelfRename(s: Shelf) {
    shelfRenameId = s.id;
    shelfRenameName = s.name;
  }
  function confirmShelfRename(s: Shelf) {
    const name = shelfRenameName.trim();
    if (!name) {
      toast(t("library.shelf.nameRequired"), "error");
      return;
    }
    const others = shelves.filter((x) => x.id !== s.id);
    const uniq = uniqueShelfName(name, others);
    shelfRenameId = null;
    void persistShelves(shelves.map((x) => (x.id === s.id ? { ...x, name: uniq } : x)));
  }

  let tagManageOpen = $state(false);
  let tagEditId = $state<number | null>(null);
  let tagEditName = $state("");
  function startTagRename(tag: TagInfo) {
    tagEditId = tag.id;
    tagEditName = tag.name;
  }
  async function confirmTagRename(tag: TagInfo) {
    const name = tagEditName.trim();
    if (!name) {
      toast(t("library.tags.nameRequired"), "error");
      return;
    }
    try {
      await api.renameTag(tag.id, name);
      tagEditId = null;
      await emit("library-tags-changed");
    } catch (e) {
      toast(errText(e), "error");
    }
  }
  async function removeTagGlobal(tag: TagInfo) {
    const ok = await appConfirm(
      t("library.tags.deleteConfirm", { name: tag.name, count: tag.book_count }),
    );
    if (!ok) return;
    try {
      await api.deleteTag(tag.id);
      await emit("library-tags-changed");
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  function clearFilter() {
    if (!hasFilter) return;
    // 作用域变化：先退出多选，避免批量删除作用到当前不可见的旧选中书。
    exitSelect();
    filterFormats = [];
    filterStates = [];
    filterTags = [];
    saveFilter();
    reload();
  }

  /** 拉取库内实际格式并动态生成标签；顺带清理「已选中但已不存在」的格式（失效清理）。 */
  async function loadFormats() {
    void api.bootStage("formats_start");
    try {
      availableFormats = await api.listFormats();
      void api.bootStage("formats_done");
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
      document.documentElement.style.setProperty("--tp-titlebar-h", tbH + "px");
    }
  });

  let ctxMenu = $state<{ x: number; y: number; book: BookRow } | null>(null);
  let ctxEl: HTMLDivElement | undefined = $state();
  let ctxPos = $state({ left: 0, top: 0 });
  let detailBook: BookRow | null = $state(null);
  let dupesOpen = $state(false);

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

  /** 上一帧画过的框。width/height 是会作废布局的属性，boxLoop 里 paintBox() 紧跟着
   *  updateBoxHits() 的逐卡 getBoundingClientRect() —— 每帧无条件写三个 style 就等于
   *  每帧逼出一次强制重排（默认几何下窗口内约 90 张卡）。拖拽中途指针停住的那部分帧
   *  框根本没变，跳过写入后那些帧读到的就是干净的布局缓存。
   *  初值全 NaN，保证首帧必写（NaN 与任何数都不相等）。 */
  let boxPainted = { x0: NaN, y0: NaN, x1: NaN, y1: NaN };

  function paintBox() {
    if (!boxEl) return;
    const { x0, y0, x1, y1 } = clampedBox();
    const p = boxPainted;
    if (x0 === p.x0 && y0 === p.y0 && x1 === p.x1 && y1 === p.y1) return;
    p.x0 = x0; p.y0 = y0; p.x1 = x1; p.y1 = y1;
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
    // 新一次拉框从「未绘制」状态开始：否则上一次拖拽最后一帧的坐标若与本次起手相同，
    // paintBox 会误判为「无变化」而跳过写入。
    boxPainted = { x0: NaN, y0: NaN, x1: NaN, y1: NaN };
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
    if (e.key === "Escape" && dupChipMenu) {
      e.preventDefault();
      dupChipMenu = null;
      return;
    }
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

  /** R11-2：回落档总数缓存——查询/筛选/文件夹/排序不变时，滚动窗口只取行（include_total=false），
   *  不重复全表 COUNT；key 变化或 reload（库变更）时重新计数。 */
  let listTotalKey = "";
  let listTotalCached = -1;
  function invalidateListTotal() {
    listTotalKey = "";
    listTotalCached = -1;
  }

  async function loadRange(offset: number, limit = FETCH) {
    const seq = ++generation;
    const listKey = bookListKey({
      query,
      sort: sortKey,
      formats: filterFormats,
      states: filterStates,
      folder: selectedFolder,
      desc,
      tags: filterTags,
    });
    const needCount = shouldCountTotal(listTotalKey, listKey) || listTotalCached < 0;
    try {
      const r = await api.listBooksPage(
        query,
        sortKey,
        offset,
        limit,
        filterFormats,
        filterStates,
        desc,
        selectedFolder ?? undefined,
        needCount,
        filterTags,
      );
      if (seq !== generation) return;
      const resolved = resolveListTotal(r.total, listTotalCached);
      if (needCount && r.total >= 0) {
        listTotalCached = r.total;
        listTotalKey = listKey;
      }
      total = resolved;
      rangeStart = offset;
      // 回落档（超长结果/搜索）：同样按重复成员过滤窗口；总数仍取后端 total，状态栏可能偏大（报告说明）。
      booksWindow = duplicateView && !isDefaultListView() ? r.rows.filter((b) => dupMetaById.has(b.id)) : r.rows;
    } catch (e) {
      if (seq === generation) toast(errText(e), "error");
    } finally {
      if (seq === generation) loading = false;
    }
  }

  /** 回落档（搜索/超限）：窗口未覆盖 [start,end) 时按 FETCH 对齐拉取，跨段时一次取够整段。 */
  function ensureRange(start: number, end: number) {
    if (total === 0 || end <= start) return;
    const rangeEnd = rangeStart + booksWindow.length;
    if (booksWindow.length && rangeStart <= start && rangeEnd >= end) return;
    const off = Math.max(0, Math.floor(start / FETCH) * FETCH);
    const need = Math.max(FETCH, end - off);
    loadRange(off, need);
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
    // 首屏行已进 rowMap：向后端报一次「本轮启动走到了书库」（连续启动失败 → 下轮跳过封面重建）。
    void api.bootStage("first_rows_done");
    signalLibraryReady();
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
        // 覆盖写取数失败：先删掉本批 id 再重试。ensureWindowIndex 只补「rowMap 缺什么」，
        // 旧行留在 map 里会让重试直接空转，更新永久丢失；成功批次不动。
        for (const id of chunk) rowMap.delete(id);
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
    const domBefore = gridEl.scrollTop;
    gridEl.scrollTop = top;
    scrollTop = top;
    scrollProbe("scroll-to-rank", {
      rank: r,
      cols,
      cellH,
      top,
      domBefore,
      domAfter: gridEl.scrollTop,
      total,
    });
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
      void api.bootStage("order_index_start");
      const buf = await api.listOrderIndex(
        sortKey,
        query,
        filterFormats,
        filterStates,
        desc,
        selectedFolder ?? undefined,
        filterTags,
      );
      if (seq !== generation) return;
      const raw = new BigInt64Array(buf);
      const keep = (id: number): boolean => {
        if (duplicateView) {
          // 无筛选：每组只留代表；有搜索/目录/格式/状态/标签：显示该范围内全部重复副本。
          return isDefaultListView() ? dupRepIds.has(id) : dupMetaById.has(id);
        }
        if (isDefaultListView() && hiddenIds.size > 0) return !hiddenIds.has(id);
        return true;
      };
      if (duplicateView || (isDefaultListView() && hiddenIds.size > 0)) {
        const kept: bigint[] = [];
        for (let i = 0; i < raw.length; i++) if (keep(Number(raw[i]))) kept.push(raw[i]);
        idIndex = BigInt64Array.from(kept);
      } else {
        idIndex = raw;
      }
      void api.bootStage("order_index_done");
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
    invalidateListTotal();
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

  /** R12-1：PDF 后台补封面（仅「自动」模式）。
   *  根因：旧实现 onMount 立即从全库缺封面 DB 顺序取前 120 本连续渲染，且 releaseAll 的
   *  doc.destroy() 不等待 → pdf.js worker 堆积、启动首屏被抢占。
   *  现在：可见区优先队列 + 串行 + 每本后 await 销毁完成 + 空闲小批次（间隔 1.5s）。 */
  let pdfCoverToken = 0;
  let pdfCoverRunning = false;
  let pdfCoverTimer: ReturnType<typeof setTimeout> | undefined;
  /** 空闲补充批次大小（每批最多取这么多 id，处理完隔 1.5s 再取）。 */
  const PDF_COVER_IDLE_BATCH = 12;
  /** 空闲批次间隔 / 启动延后。 */
  const PDF_COVER_IDLE_GAP_MS = 1500;
  const PDF_COVER_START_DELAY_MS = 300;
  /** 本次会话里已试过的书：失败的不再重挑（避免 132 字节空壳 PDF 无限重试）。 */
  const pdfCoverTried = new Set<number>();
  const pdfCoverQueue = createPdfCoverQueue();

  /** 当前是否允许继续补封面：auto、书库可见、窗口可见、令牌未被换书/卸载作废。 */
  function pdfCoverAllowed(token: number): boolean {
    return (
      token === pdfCoverToken &&
      config.library.coverScan === "auto" &&
      ui.view === "library" &&
      document.visibilityState === "visible"
    );
  }

  /** R12-1 补丁：前 3 个失败在 DEV 下打 warn（bookId + 阶段 + 错误），便于 CDP 诊断。 */
  let pdfCoverFailWarns = 0;
  function warnPdfCoverFail(id: number, stage: string, e: unknown) {
    if (pdfCoverFailWarns >= 3) return;
    pdfCoverFailWarns++;
    console.warn("[pdf-cover] failed", { bookId: id, stage, error: String(e) });
  }

  /** 最近一次补封面的书：队列收尾时据此按 bookId 守卫释放，避免无条件销毁 Reader 会话。 */
  let lastPdfCoverBookId = -1;

  /** 渲染一本并写回封面；返回是否成功。 */
  async function processPdfCover(id: number): Promise<void> {
    lastPdfCoverBookId = id;
    let dataUrl = "";
    let rendered = false;
    try {
      const pdf = await pdfMod();
      dataUrl = await pdf.renderCoverImage(id, 320);
      rendered = true;
      const b64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
      dataUrl = ""; // 立刻放掉整幅 data URL（可能数 MB），只把 base64 段交给后端
      if (b64) {
        try {
          await api.setBookCoverImage(id, b64);
          pdf.pdfCoverStats.wrote++;
        } catch (e) {
          pdf.pdfCoverStats.failed++;
          pdf.pdfCoverStats.lastError = "write: " + String(e);
          warnPdfCoverFail(id, "setBookCoverImage", e);
        }
      }
    } catch (e) {
      /* 单本失败静默：封面是锦上添花，不弹错、也不拖住后面的书。
       * renderCoverImage 包装层已经统计过 failed/lastError（含阶段前缀），这里不再重复计数。 */
      dataUrl = "";
      warnPdfCoverFail(id, rendered ? "write-prepare" : "renderCoverImage", e);
    }
    pdfCoverTried.add(id); // 无论成败都记账
    // R12-1 补丁：不再无条件 release——用户可能刚打开一本 PDF（Reader 会话），
    // 无条件 releaseAll 会销毁 Reader 的文档。串行场景下下一本 renderCoverImage→ensurePdf
    // 已在打开前 releaseAll()+await pendingDestroy，自然等上一本销毁；最后一本留 1 个空闲 worker。
    // 让出事件循环：canvas / PNG 字符串在这一拍后可回收。
    await new Promise((r) => setTimeout(r, 0));
  }

  async function backfillPdfCovers() {
    if (pdfCoverRunning) return;
    const token = ++pdfCoverToken;
    pdfCoverRunning = true;
    let more = false;
    try {
      // ① 先清空可见区优先队列。
      for (;;) {
        if (!pdfCoverAllowed(token)) return;
        const id = takeNext(pdfCoverQueue);
        if (id === null) break;
        if (pdfCoverTried.has(id)) continue; // 已试过/失败的 id 不再派发（attempted 不会重复增长）
        await processPdfCover(id);
      }
      // ② 可见队列空：取一小批空闲补充（从全库缺封面列表，跳过 tried/已入队）。
      if (!pdfCoverAllowed(token)) return;
      const ids = await api.pdfBooksMissingCover().catch(() => null);
      if (!ids || !pdfCoverAllowed(token)) return;
      const batch = pickIdleBatch(ids, pdfCoverTried, pdfCoverQueue.seen, PDF_COVER_IDLE_BATCH);
      if (batch.length) {
        enqueueIdle(pdfCoverQueue, batch);
        more = ids.length > batch.length;
        for (;;) {
          if (!pdfCoverAllowed(token)) return;
          const id = takeNext(pdfCoverQueue);
          if (id === null) break;
          if (pdfCoverTried.has(id)) continue;
          await processPdfCover(id);
        }
      }
    } finally {
      if (token === pdfCoverToken) pdfCoverRunning = false;
    }
    // 还有剩余：隔 1.5s 排下一轮（每次只补一小批，避免启动冲刺）。
    if (more && pdfCoverAllowed(token)) {
      clearTimeout(pdfCoverTimer);
      pdfCoverTimer = setTimeout(() => void backfillPdfCovers(), PDF_COVER_IDLE_GAP_MS);
      return;
    }
    // 收尾：队列已空/本轮结束，若当前会话仍是最后补封面那本且书库可见，按 bookId 守卫释放，
    // 使 idle 后 active 回落到 0~1（不触碰 Reader 的其它书会话）。
    if (pdfCoverAllowed(token) && lastPdfCoverBookId >= 0) {
      try {
        await (await pdfMod()).releasePdfSessionIfBook(lastPdfCoverBookId);
      } catch {
        /* ignore */
      }
    }
  }
  /** 起一轮补封面（重复调用安全）；关闭/切手动后令牌失效，循环自行退出。 */
  function kickPdfCovers() {
    if (config.library.coverScan !== "auto") return;
    if (pdfCoverRunning) return;
    void backfillPdfCovers();
  }
  /** 首屏就绪后延后启动（requestIdleCallback 优先），不占首屏关键路径。 */
  function schedulePdfCovers(delay = PDF_COVER_START_DELAY_MS) {
    clearTimeout(pdfCoverTimer);
    pdfCoverTimer = setTimeout(() => {
      if (config.library.coverScan !== "auto" || ui.view !== "library") return;
      kickPdfCovers();
    }, delay);
  }
  // 离开书架（组件卸载）即停止：令牌 +1 让在跑的循环在下一次检查点退出。
  // 同时记下顶部锚点与滚动位置：进书后返回书库时据此还原视线。
  onDestroy(() => {
    pdfCoverToken++;
    clearTimeout(pdfCoverTimer);
    libSession.anchor = topAnchor() ?? null;
    libSession.scrollTop = scrollTop;
    scrollProbe("save", {
      query: libSession.query,
      anchor: libSession.anchor,
      scrollTop: libSession.scrollTop,
      indexMode,
      view,
      cols,
      cellH,
      gridW,
      gridH,
      total,
    });
  });

  // R11-3：书库可见时恢复后台封面扫描；进入阅读/离开书库时暂停，避免与阅读抢 I/O。
  onMount(() => {
    void api.setScanPaused(false).catch(() => {});
    // B7：空闲预取详情弹窗 chunk，点开时命中缓存；失败不影响书库。
    const detailPrefetch = setTimeout(() => { void detailMod().catch(() => undefined); }, 2500);
    const onVis = () => {
      if (ui.view !== "library") return;
      void api.setScanPaused(document.visibilityState !== "visible").catch(() => {});
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      clearTimeout(detailPrefetch);
      document.removeEventListener("visibilitychange", onVis);
    };
  });
  onDestroy(() => {
    void api.setScanPaused(true).catch(() => {});
  });

  /** 首屏取到行后向后端报一次「已进入书库」：启动日志据此判定上一轮是否崩溃，
   *  连续启动失败时下一次启动自动跳过封面重建。只报一次。 */
  let readySignaled = false;
  function signalLibraryReady() {
    if (readySignaled) return;
    readySignaled = true;
    api.libraryReady().catch(() => {});
    // R12-1：首屏关键路径之后再启动 PDF 封面队列（可见区优先由窗口 effect 入队）。
    schedulePdfCovers(PDF_COVER_START_DELAY_MS);
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
  // 200ms 防抖；依赖视口 index 区间 + 索引/窗口版本（排序/查询变化后重建索引也要补发优先项）。
  // ids 数量 = 渲染窗口行数（O(W)），与滚动位置无关（约束 2）。
  let prioTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    // R11-3：idIndex/winVersion 纳入依赖——rebuildIndex 后 total 可能不变、但索引内容变了，
    // 不依赖它们会导致排序/查询变化后可见区优先项不再补发。
    void idIndex;
    void winVersion;
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
      // R12-1：可见区 PDF（缺封面）插到补封面队首；滚动/窗口变化即优先处理。
      const pdfIds: number[] = [];
      for (let i = s; i < e; i++) {
        const b = bookAt(i);
        if (b && b.format === "pdf" && !b.cover_path) pdfIds.push(b.id);
      }
      if (pdfIds.length) {
        enqueueVisible(pdfCoverQueue, pdfIds, pdfCoverTried);
        if (config.library.coverScan === "auto" && ui.view === "library") {
          schedulePdfCovers(0);
        }
      }
    }, 200);
  });

  // R11-2：输入法组合输入期间不触发搜索；组合结束后再按防抖触发。
  let searchComposing = $state(false);

  // 搜索 250ms 防抖后再请求（输入不触发全量过滤）。
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const q = queryInput;
    void searchComposing;
    clearTimeout(searchTimer);
    if (searchComposing) return;
    searchTimer = setTimeout(() => {
      if (q !== query) {
        // 搜索作用域变化：先退出多选，避免旧选中书被批量删除。
        exitSelect();
        libSession.query = q;
        reload();
      }
    }, 250);
    return () => clearTimeout(searchTimer);
  });

  function onSortChange() {
    // 排序变化会重建结果集：先退出多选，避免旧选中书被批量删除。
    exitSelect();
    saveSort();
    reload();
  }

  /** 反向排序：翻转方向并持久化，重新加载当前列表。 */
  function toggleReverse() {
    // 排序变化会重建结果集：先退出多选，避免旧选中书被批量删除。
    exitSelect();
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
    void api.bootStage("folders_start");
    try {
      folders = await api.listFolders();
      void api.bootStage("folders_done");
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
    // pickFolder 也可能 reject（对话框/权限异常）：一并纳入 try，失败必须给出可读提示。
    try {
      const p = await api.pickFolder(t("library.dialog.pickFolder"));
      if (!p) return; // 用户取消
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
    try {
      await api.removeFolder(p);
      await loadFolders();
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  function openBook(b: BookRow) {
    if (config.reader.multiWindow) {
      openBookInNewWindow({ id: b.id, title: b.title });
    } else {
      pendingOpen.bookId = b.id;
      pendingOpen.nonce++;
      // B7：点击开书即预加载 Reader chunk（与 idle 预取双保险），不阻塞当前帧。
      void import("../components/Reader.svelte").catch(() => undefined);
      ui.view = "reader";
    }
  }

  // 对账刷新的执行器（两个入口共用）：先跑一次不带确认的对账。
  // - 后端返回 needs_confirm ⇒ 一条都没删，这里用多行确认框把「数量 + 主要目录」告诉用户，
  //   确认后再以 confirm=true 调一次（此时才真正删除）。
  // - 返回 null 表示用户取消或发生错误（错误已 toast）。
  async function runReconcile(): Promise<{ removed: number; added: number; failed: number; pending: number; skipped: number } | null> {
    try {
      const r = await api.refreshLibrary();
      if (!r.needs_confirm) return { removed: r.removed, added: r.added, failed: r.failed, pending: r.pending, skipped: r.skipped };
      // 确认框：第一行放数量（`word-break: break-all` 会断路径，关键数字必须在第一行）
      const lines = [t("library.refresh.confirmCount", { count: r.pending })];
      const dirs = r.dirs.slice(0, 3);
      if (dirs.length) {
        lines.push(t("library.refresh.confirmDirs", { dirs: dirs.map((d) => `${d.dir}（${d.count}）`).join("、") }));
      }
      lines.push(t("library.refresh.confirmAsk"));
      const ok = await appConfirm(lines.join("\n"));
      if (!ok) return null;
      const r2 = await api.refreshLibrary(true);
      return { removed: r2.removed, added: r2.added, failed: r2.failed, pending: r2.pending, skipped: r2.skipped };
    } catch (e) {
      toast(errText(e), "error");
      return null;
    }
  }

  // 任务3：打开入口先查文件是否存在；不存在则弹确认刷新，绝不进入空白阅读。
  // 确认 → 刷新目录后停下（不继续打开、不触碰旧 reader 状态）；取消 → 直接返回。
  // 三态：ok=打开；missing=弹"文件缺失"确认；unreachable=只提示"暂时无法访问"，不进删除确认。
  async function tryOpenBook(b: BookRow) {
    let state: "ok" | "missing" | "unreachable" = "ok";
    try {
      state = await api.fileExists(b.path);
    } catch {
      state = "ok"; // 检查调用本身失败时不阻断（交由打开流程自行报错）
    }
    if (state === "ok") {
      openBook(b);
      return;
    }
    if (state === "unreachable") {
      toast(t("toast.library.unreachable"));
      return;
    }
    const ok = await appConfirm(t("library.book.missingConfirm", { path: b.path }));
    if (!ok) return;
    const r = await runReconcile();
    if (r) {
      if (r.skipped > 0 && r.removed === 0) toast(t("toast.library.unreachable"));
      else toast(t("toast.library.refreshed", { added: r.added, removed: r.removed }));
    }
    // 无论该书是否仍在索引，都停在这里（不进入空白阅读）
  }

  // 任务2：刷新按钮 → 按记录目录重扫（一次遍历，无实时监听）。
  // 16 万本下 IO 密集且耗时：立即给「进行中」提示并禁用按钮，完成后只增量刷新当前窗口。
  // 注意：普通刷新**只做对账增删**；后台全量重扫走「长按 → 重新扫描书库」菜单（rescanLibrary）。
  async function refreshAll() {
    if (refreshing) return;
    refreshing = true;
    refreshProgress = { phase: "check", done: 0, total: 0 };
    try {
      const r = await runReconcile();
      if (r) {
        if (r.skipped > 0 && r.removed === 0) toast(t("toast.library.unreachable"));
        else toast(t("toast.library.refreshed", { added: r.added, removed: r.removed }));
        if (r.failed > 0) toast(t("toast.library.refreshPartial", { count: r.failed }), "error");
        await reloadWindow();
        void loadHidden(true);
      }
    } finally {
      refreshing = false;
      refreshProgress = null;
    }
  }

  /** 长按菜单：后台全量重新扫描书库（独立于单击的对账刷新）。 */
  async function rescanLibrary() {
    refreshMenu = false;
    if (rescanBusy) return;
    rescanBusy = true;
    try {
      await api.rescan(); // 后端 rescan 命令：后台全量扫描
      toast(t("toast.library.rescanStarted"));
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      rescanBusy = false;
    }
  }

  // 刷新按钮：单击=检查增删；长按（550ms）=重新扫描书库。用 Pointer Events 兼容鼠标/触屏。
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
      const b = await api.randomBookFiltered(
      query,
      sortKey,
      filterFormats,
      filterStates,
      selectedFolder ?? undefined,
      filterTags,
    );
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
      const name = b.format === "pdf" ? await (await pdfMod()).regeneratePdfCover(b.id, 320) : await api.reextractCover(b.id);
      // 换新对象身份（不就地改属性），令封面 <img> 重挂即时刷新。
      replaceBook(b.id, { cover_path: name, cover_ver: (b.cover_ver ?? 0) + 1 });
      toast(t("toast.library.coverReextracted"));
    } catch {
      // R13-A4：封面生成失败用统一短文案，不暴露底层异常名/路径。
      toast(t(COVER_FAIL_KEY), "error");
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
      if (BOOK_EXTENSIONS.includes((p.split(".").pop() || "").toLowerCase())) {
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
    void api.bootStage("lib_mount");
    // 首屏提示条：读上次扫描的最终判定（多窗口同一份；重启后仍在）。
    void loadScanStale();
    // DEV 调试钩子：便于自动化验证删书弹窗（release 剥离）。
    if (import.meta.env.DEV) {
      (window as any).__tpLibDebug = {
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
    void loadTags();
    void loadShelves();
    void loadHidden(true);
    // 进书返回时若有保存的锚点或滚动位置：按锚点重建后复原视线；否则照旧全量重载回顶。
    if (libSession.anchor || libSession.scrollTop > 0) {
      loading = true;
      total = 0;
      scrollProbe("restore-start", {
        anchor: libSession.anchor,
        savedScrollTop: libSession.scrollTop,
        query: libSession.query,
        view,
        gridW,
        gridH,
        cols,
        cellH,
      });
      void (async () => {
        await rebuildIndex(libSession.anchor ?? undefined);
        scrollProbe("restore-after-rebuild", {
          indexMode,
          total,
          scrollTop,
          domScroll: gridEl ? gridEl.scrollTop : null,
          cols,
          cellH,
          gridW,
          gridH,
        });
        // 挂载时 total=0、虚拟列表占位高度尚未建立，重建期间设的 scrollTop 会被浏览器夹回 0；
        // 等 total 写入、DOM 高度建立后再设一次：回落档没有锚点，全靠这一步复原原位置。
        await tick();
        if (indexMode) {
          // 索引档：重建时 scrollToRank 已按锚点算出目标行，这里把同一个值重新落到已建立高度的 DOM。
          if (gridEl) gridEl.scrollTop = scrollTop;
        } else {
          if (gridEl) gridEl.scrollTop = libSession.scrollTop;
          scrollTop = libSession.scrollTop;
        }
        scrollProbe("restore-after-apply", {
          indexMode,
          total,
          scrollTop,
          domScroll: gridEl ? gridEl.scrollTop : null,
          cols,
          cellH,
          gridW,
          gridH,
        });
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            scrollProbe("restore-final", {
              indexMode,
              total,
              scrollTop,
              domScroll: gridEl ? gridEl.scrollTop : null,
              cols,
              cellH,
              gridW,
              gridH,
            });
          });
        });
      })();
    } else {
      reload();
    }
    // R12-1：启动期不再立即补 PDF 封面；首屏行就绪后由 signalLibraryReady 延后 300ms 启动。
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
    const uTags = listen("library-tags-changed", () => {
      void (async () => {
        await loadTags();
        // 标签被删除后，已选 filterTags 里可能残留旧名字：剔除并失效总数，避免空结果。
        const alive = new Set(tags.map((x) => x.name));
        const kept = filterTags.filter((n) => alive.has(n));
        if (kept.length !== filterTags.length) {
          filterTags = kept;
          saveFilter();
        }
        invalidateListTotal();
        if (indexMode) void rebuildIndex(topAnchor());
        else void reloadWindow();
      })();
    });
    const uShelves = listen("library-shelves-changed", () => void loadShelves());
    const u1 = listen("library-changed", () => {
      bumpCoverBust();
      // 防抖合并：移除大目录/批量删除时该事件可能与其它事件叠加，120ms 内只做一次全量刷新，
      // 避免多次整库索引重建叠加冻结 UI。计时打点①：从收到事件到窗口刷新完成的总耗时。
      clearTimeout(libChangedTimer);
      libChangedTimer = setTimeout(() => {
        console.time("[tidepage] library-changed→refresh");
        // R11-2：库变更（增删书）后总数缓存失效，回落档下一次取窗口要重新 COUNT。
        invalidateListTotal();
        // 增删书改变 N 与顺序：索引档按锚点重建，回落档重拉窗口。
        // 列表先刷新；hidden 后台更新，绝不阻塞 library-changed 刷新链路。
        void reloadWindow().finally(() => {
          console.timeEnd("[tidepage] library-changed→refresh");
        });
        void loadHidden(true);
        // 格式可能新增/消失：刷新格式列表并清理失效选中。
        loadFormats();
        kickPdfCovers(); // 新导入的 PDF 也要能自动出封面（本组件只处理 pdf）
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
    // `origin` 分流：只有用户动作触发的扫描才显示底栏与提示；`timer` 触发的扫描两者都不显示。
    const u8 = listen<{
      running: boolean;
      phase: "walk" | "diff" | "insert" | "done";
      scanned: number;
      added: number;
      origin?: string;
    }>("folder-scan-progress", (e) => {
      const p = e.payload;
      if (!p) return;
      const userScan = p.origin === "user";
      if (p.running) {
        if (userScan) folderScan = p;
        return;
      }
      if (userScan) folderScan = null;
      if (!userScan) return;
      toast(p.added > 0 ? t("toast.library.folderScanAdded", { count: p.added }) : t("toast.library.folderScanNone"));
      // 扫描完成后按当前排序全量刷新：resetScroll() + 按 sortKey/query/desc 重建 id 索引，
      // 确保新书出现在其排序位置并回到顶部（reloadWindow 会锚定原顶部 id，新书可能不在可视区）。
      if (p.added > 0) reload();
    });
    // 扫描删行护栏（后端 folder-scan-stale）：目录没走完或失效数超阈值时后端拒绝自动删除，只登记不清理。
    // `origin="user"` 才弹提示（用户动作，弹了才知道结果）；`timer` 一律静默，
    // 异常态改由首屏常驻提示条呈现（`last_scan_stale`），数据安全信号不会因为静默而消失。
    const u9 = listen<{
      refused: boolean;
      pending: number;
      skipped: number;
      unfinishedDirs: number;
      origin?: string;
    }>("folder-scan-stale", (e) => {
      const p = e.payload;
      if (!p) return;
      if (p.origin === "user") {
        if (p.refused) toast(t("toast.library.scanStaleRefused", { count: p.pending }));
        else if (p.unfinishedDirs > 0) toast(t("toast.library.scanUnfinished"));
      }
      void loadScanStale();
    });
    // 扫描最终态（每次扫描结束都发，含恢复为正常）：首屏提示条据此出现/消失。
    // 与 `folder-scan-stale` 的区别：那个只在异常时发，恢复时没有事件。
    const u10 = listen<{
      state: string;
      refused: boolean;
      pending: number;
      skipped: number;
      unfinishedDirs: number;
      dirs?: string[];
      at?: number;
    }>("scan-stale-state", (e) => {
      const p = e.payload;
      if (!p) return;
      scanStale =
        p.state === "stale"
          ? { refused: !!p.refused, pending: p.pending ?? 0, skipped: p.skipped ?? 0, unfinishedDirs: p.unfinishedDirs ?? 0, dirs: Array.isArray(p.dirs) ? p.dirs : [], at: p.at ?? 0 }
          : null;
    });
    // 移除目录进度（后端 remove_folder 发 delete/clean/done 三相位）。
    const u12 = listen<{
      running: boolean;
      phase: "delete" | "clean" | "done";
      done: number;
      total: number;
    }>("remove-folder-progress", (e) => {
      const p = e.payload;
      if (!p) return;
      removeProgress = p.running ? p : null;
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
    // 任何窗口打开书（含独立书窗）后端都会广播：主窗停在书库时重拉当前窗口，
    // 让「最近阅读」排序即时反映刚打开的书。
    const u11 = listen("book-opened", () => {
      // 主窗自己开书时事件也会到，此刻正在离开书库，无需刷新。
      if (ui.view !== "library") return;
      void reloadWindow();
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
      u11.then((f) => f());
      u12.then((f) => f());
      u9.then((f) => f());
      u10.then((f) => f());
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
    <button class="primary ico" data-testid="lib-btn-add-folder" onclick={addFolder}><IconPlus />{t("library.toolbar.addFolder")}</button>
    <div class="refresh-wrap">
      <button
        class="ico"
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
        <div class="refresh-menu" role="menu" out:outFade={{ from: 0.96 }}>
          <button class="rm-item" data-testid="lib-btn-rescan" onclick={rescanLibrary} disabled={rescanBusy}>{t("library.toolbar.rescan")}</button>
        </div>
        <div class="rm-backdrop" out:outDissolve onclick={() => (refreshMenu = false)}></div>
      {/if}
    </div>
    <div class="search-wrap">
      <input
      class="search"
      data-testid="lib-search"
      placeholder={t("library.toolbar.searchPlaceholder")}
      bind:value={libSession.queryInput}
      oncompositionstart={() => (searchComposing = true)}
      oncompositionend={() => (searchComposing = false)}
    />
      {#if libSession.queryInput}
        <button
          class="search-clear tp-x"
          data-testid="lib-search-clear"
          title={t("library.toolbar.clearSearch")}
          aria-label={t("library.toolbar.clearSearch")}
          onclick={() => (libSession.queryInput = "")}
        ><IconX size={16} stroke={1.5} /></button>
      {/if}
    </div>
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
        <button class="filter-chip" data-testid="lib-filter-chip" title={t("library.toolbar.clearFilterTip")} aria-label={t("library.toolbar.clearFilterTip")} onclick={clearFilter}>
          {t("library.toolbar.filtered")} <span class="fx tp-x" aria-hidden="true"><IconX size={14} stroke={1.8} /></span>
        </button>
      {/if}
    </div>
    <div class="spacer" data-tauri-drag-region></div>
    <div class="lib-tb-group" use:dragSort={{ onDrop: libDrop }}>
      {#each orderedLibBtns as b (b.id)}
        {@const Icon = b.icon}
        <button
          class="icon lib-tb-btn tip-left"
          class:has-update={b.id === "settings" && !!updateState.latest}
          data-drag-id={b.id}
          data-testid={"lib-btn-" + b.id}
          aria-label={b.title}
          data-tip={b.title}
          onclick={b.run}
        >
          <Icon size={20} stroke={1.5} />
          {#if b.id === "settings" && updateState.latest}
            <span class="update-dot abs" aria-hidden="true"></span>
          {/if}
        </button>
      {/each}
    </div>
    {/if}
  </div>

  {#if maintReminder}
    <div class="maint-banner" data-testid="lib-maint-reminder">
      <div class="maint-text">
        <span>{t("settings.maintenance.reminder", { size: formatBytes(maintReminder.freelist), need: formatBytes(maintReminder.need) })}</span>
        <span class="maint-sub">{t("settings.maintenance.reminderHint")}</span>
      </div>
      <div class="maint-actions">
        <button data-testid="lib-maint-vacuum" disabled={maintBusy} onclick={() => void vacuumFromBanner()}>
          {maintBusy ? t("settings.maintenance.vacuumRunning") : t("settings.maintenance.reminderAction")}
        </button>
        <button disabled={maintBusy} onclick={() => { config.maintenance.reminderDismissedUntil = Date.now(); saveConfig(); maintReminder = null; }}>{t("settings.maintenance.dismiss7")}</button>
      </div>
    </div>
  {/if}

  {#if filterPanelOpen}
    <div class="filter-panel" out:outFade={{ from: 0.98, up: false }}>
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
        <span class="fp-label">{t("library.filter.tags")}</span>
        <div class="fp-tags">
          {#each tags as tag (tag.id)}
            <button class="tag" class:active={filterTags.includes(tag.name)} onclick={() => toggleTag(tag.name)}>
              {tag.name}{#if tag.book_count}<span class="fp-count"> · {tag.book_count}</span>{/if}
            </button>
          {/each}
          {#if !tags.length}<span class="fp-empty">{t("library.filter.noTags")}</span>{/if}
          {#if tags.length}
            <button class="tag" data-testid="lib-tag-manage" onclick={() => (tagManageOpen = !tagManageOpen)}>
              {tagManageOpen ? t("library.tags.collapse") : t("library.tags.manage")}
            </button>
          {/if}
        </div>
        {#if tagManageOpen}
          <div class="fp-tags" data-testid="lib-tag-manage-list">
            {#each tags as tag (tag.id)}
              <span class="tag">
                {#if tagEditId === tag.id}
                  <input
                    class="search"
                    style="max-width:120px"
                    bind:value={tagEditName}
                    onkeydown={(e) => { if (e.key === "Enter") void confirmTagRename(tag); if (e.key === "Escape") tagEditId = null; }}
                  />
                  <button class="tag" onclick={() => void confirmTagRename(tag)}>✓</button>
                  <button class="tag" onclick={() => (tagEditId = null)}>×</button>
                {:else}
                  {tag.name}{#if tag.book_count}<span class="fp-count"> · {tag.book_count}</span>{/if}
                  <button class="tag" title={t("library.tags.renameTip")} onclick={() => startTagRename(tag)}>✎</button>
                  <button class="tag tp-x tp-x-sm" title={t("library.tags.deleteTip")} aria-label={t("library.tags.deleteTip")} onclick={() => void removeTagGlobal(tag)}><IconX size={12} stroke={1.8} /></button>
                {/if}
              </span>
            {/each}
          </div>
        {/if}
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
      <div class="fp-group">
        <span class="fp-label">{t("library.shelf.title")}</span>
        <div class="fp-tags">
          <input
            class="search"
            style="max-width:180px"
            placeholder={t("library.shelf.namePlaceholder")}
            bind:value={shelfNameInput}
          />
          <button class="tag" onclick={saveCurrentShelf}>{t("library.shelf.saveCurrent")}</button>
        </div>
      </div>
      {#if hasFilter}
        <button class="fp-clear" data-testid="lib-filter-clear" onclick={clearFilter}>{t("library.filter.clear")}</button>
      {/if}
    </div>
  {/if}

  {#if scanStale}
    <!-- 上次扫描未走完/已跳过清理：首屏常驻提示条（不可折叠进设置）。
         数据安全信号不能因为"后台扫描不弹提示"而消失；措辞与刷新确认框同一套口径。 -->
    <div class="scan-stale" data-testid="lib-scan-stale">
      <div class="ss-line1">
        {scanStale.refused
          ? t("toast.library.scanStaleRefused", { count: scanStale.pending })
          : t("toast.library.scanUnfinished")}
      </div>
      {#if scanStale.dirs.length}
        <div class="ss-line2">{t("library.refresh.confirmDirs", { dirs: scanStale.dirs.join("、") })}</div>
      {/if}
    </div>
  {/if}

  {#if folders.length || foldersFailed || dupGroupCount > 0}
    <div class="folders" oncontextmenu={openDupChipMenu}>
      <button
        class="folder-chip"
        class:active={selectedFolder === null}
        data-testid="lib-folder-all"
        title={t("library.folder.allTip")}
        onclick={() => selectFolder(null)}
      >{t("library.folder.all")}</button>
      {#if dupGroupCount > 0 && !dupChipHidden}
        <span
          class="folder-chip"
          class:active={duplicateView}
          role="button"
          tabindex="0"
          data-testid="lib-folder-duplicates"
          title={t("library.folders.duplicatesTip")}
          onclick={toggleDuplicateView}
          onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggleDuplicateView(); } }}
        >
          <IconCopy size={14} stroke={1.8} /> {t("library.folders.duplicates")} · {t("library.folders.duplicatesCount", { n: dupGroupCount })}
          <button
            class="x tp-x"
            data-testid="lib-folder-duplicates-hide"
            title={t("library.folders.hideDuplicates")}
            aria-label={t("library.folders.hideDuplicates")}
            onclick={(e) => { e.stopPropagation(); hideDupChip(); }}
            onkeydown={(e) => e.stopPropagation()}
            onkeyup={(e) => e.stopPropagation()}
            onpointerdown={(e) => e.stopPropagation()}
          ><IconX size={14} stroke={1.8} /></button>
        </span>
      {/if}
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
          <IconFolder /> {f}
          <button class="x tp-x" title={t("library.folder.removeTip")} aria-label={t("library.folder.removeTip")} onclick={(e) => { e.stopPropagation(); removeFolder(f); }}><IconX size={14} stroke={1.8} /></button>
        </span>
      {/each}
      {#if foldersFailed}<span class="fp-empty" data-testid="lib-folders-failed">{t("library.folders.failed")}</span>{/if}
    </div>
  {/if}
  {#if dupChipMenu}
    <div class="ctx-backdrop" out:outDissolve onclick={() => (dupChipMenu = null)} oncontextmenu={(e) => { e.preventDefault(); dupChipMenu = null; }}>
      <div class="ctx ctx-menu" style="left:{dupChipMenu.x}px; top:{dupChipMenu.y}px;" out:outFade={{ from: 0.96 }}>
        <button class="ctx-item" data-testid="lib-folder-duplicates-show" onclick={showDupChip}>{t("library.folders.showDuplicates")}</button>
      </div>
    </div>
  {/if}

  {#if shelves.length}
    <div class="folders" data-testid="lib-shelves">
      {#each shelves as s (s.id)}
        {#if shelfRenameId === s.id}
          <span class="folder-chip">
            <input
              class="search"
              style="max-width:140px"
              bind:value={shelfRenameName}
              onkeydown={(e) => { if (e.key === "Enter") confirmShelfRename(s); if (e.key === "Escape") shelfRenameId = null; }}
            />
            <button class="x" title={t("library.shelf.renameSave")} onclick={() => confirmShelfRename(s)}>✓</button>
            <button class="x" title={t("library.shelf.renameCancel")} onclick={() => (shelfRenameId = null)}><IconX /></button>
          </span>
        {:else}
          <span
            class="folder-chip"
            role="button"
            tabindex="0"
            title={t("library.shelf.applyTip", { name: s.name })}
            onclick={() => applyShelf(s)}
            onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); applyShelf(s); } }}
          >
            {s.name}
            <button class="x" title={t("library.shelf.renameTip")} aria-label={t("library.shelf.renameTip")} onclick={(e) => { e.stopPropagation(); startShelfRename(s); }}>✎</button>
            <button class="x" title={t("library.shelf.deleteTip")} aria-label={t("library.shelf.deleteTip")} onclick={(e) => { e.stopPropagation(); void removeShelf(s); }}><IconX /></button>
          </span>
        {/if}
      {/each}
    </div>
  {/if}

  <div
    class="grid"
    class:list-view={view === "list"}
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
    {#if (loading || refreshing || folderScan?.running) && total === 0}
      <div class="empty-state" data-testid="lib-hint-loading">{t("library.empty.loading")}</div>
    {:else if total === 0}
      {#if query.trim() || hasFilter || selectedFolder}
        <div class="empty-state" data-testid="lib-hint-empty">{t("library.empty.noMatch")}</div>
      {:else}
        {@const blank = t("library.empty.blank").split("\n")}
        <div class="lib-start">
          <!-- 图标尺寸归到既有令牌：28 = --icon-lg（大号目标），不新增尺寸值。
               本仓库有一条 icon_size_probe 全量普查，任何不在 14/16/20/28 里的 svg 都会被点名。 -->
          <span class="start-mark"><IconBooks size={28} stroke={1.5} /></span>
          <!-- 文案仍是 library.empty.blank 一条键（三行由 \n 拆开），故 textContent 的前缀
               与旧实现逐字一致，i18n 探针的 /^书库为空/ 断言不受影响。 -->
          <div class="empty-state lib-blank start-copy" data-testid="lib-hint-empty">
            <p class="start-title">{blank[0]}</p>
            <p class="start-sub">{blank[1]}</p>
          </div>
          <!-- Button-in-button：文字在左，图标单独坐在右侧的圆形承托里（不与文字并排裸露）。
               圆圈在 hover 时向右上「被吸出去」一点，形成按钮内部的动能张力。 -->
          <button class="primary start-cta" data-testid="lib-empty-add-folder" onclick={addFolder}>
            <span class="cta-label">{t("library.toolbar.addFolder")}</span>
            <span class="cta-icon" aria-hidden="true"><IconPlus size={16} stroke={1.5} /></span>
          </button>
          <p class="start-formats">{blank[2]}</p>
        </div>
      {/if}
    {:else if view === "grid"}
      <!-- 视图切换 / 重载时整块重建，入场用一次上浮淡入收住。
           只挂 .grid-inner / .list-inner（每次重建恰好一个），不给每张卡加动画：
           虚拟窗口随滚动增删行，逐卡入场会在滚动中不断重播，看起来像闪烁。 -->
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
                    class:dup-card={dupMetaById.has(b.id) && config.library.duplicateEffect}
                    data-book-id={b.id}
                    style="max-width:{CARD_MAX}px;"
                    tabindex="0"
                    role="button"
                    aria-label={b.title}
                    onkeydown={(e) => { if (e.key === "Enter") onCardDblClick(e as unknown as MouseEvent, b); }}
                    onpointerdown={(e) => cardPointerDown(e, b)}
                    onclick={(e) => onCardClick(e, b)}
                    ondblclick={(e) => onCardDblClick(e, b)}
                    oncontextmenu={(e) => openCtx(e, b)}
                  >
                    <div class="cover" style="height:{coverH}px;">
                      {#if dupMetaById.has(b.id) && config.library.duplicateEffect}
                        <span class="dup-ghost g2" aria-hidden="true"><img src={cu} alt="" loading="lazy" decoding="async" draggable="false" /></span>
                        <span class="dup-ghost g1" aria-hidden="true"><img src={cu} alt="" loading="lazy" decoding="async" draggable="false" /></span>
                      {/if}
                      <div class="cover-frame">
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
                        {#if dupMetaById.has(b.id) && config.library.duplicateEffect}
                          <span class="dup-badge" data-testid="dup-badge" title={t("library.duplicateBadgeTitle", { n: dupMetaById.get(b.id)?.copies ?? 0 })}>{t("library.duplicateCopies", { n: dupMetaById.get(b.id)?.copies ?? 0 })}</span>
                        {/if}
                      </div>
                    </div>
                    <div class="meta">
                      <div class="t" title={b.title}>{b.title}</div>
                      <div class="a" title={b.author}>{b.author}</div>
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
              tabindex="0"
              role="button"
              aria-label={b.title}
              onkeydown={(e) => { if (e.key === "Enter") onCardDblClick(e as unknown as MouseEvent, b); }}
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
                <div class="t" title={b.title}>{b.title}{#if dupMetaById.has(b.id) && config.library.duplicateEffect}<span class="dup-list-badge">{t("library.duplicateCopiesShort", { n: dupMetaById.get(b.id)?.copies ?? 0 })}</span>{/if}</div>
                <div class="a" title={b.author}>{b.author}</div>
              </div>
              <!-- 纯视觉的进度条（信息与网格视图的进度环同源），故对读屏隐藏，不引入新文案键 -->
              <div class="row-prog" aria-hidden="true">
                <div class="rp-track"><div class="rp-fill" class:full={ri.full} style="width:{ri.show ? ri.pct : 0}%"></div></div>
                {#if ri.show}<span class="rp-num">{Math.round(ri.pct)}%</span>{/if}
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
      <div class="scan-progress" title={t("library.statusbar.refreshScanTip")} out:outDissolve>
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
      <div class="scan-progress" title={t("library.statusbar.folderScanTip")} out:outDissolve>
        <div class="scan-bar"><div class="scan-fill indeterminate"></div></div>
        <span class="scan-text" data-testid="lib-statusbar-folderscan">{folderScanText(folderScan)}</span>
      </div>
    {/if}
    {#if removeProgress}
      <div class="scan-progress" title={t("library.statusbar.removeTip")} out:outDissolve>
        <div class="scan-bar"><div class="scan-fill indeterminate"></div></div>
        <span class="scan-text" data-testid="lib-statusbar-removefolder">
          {removeProgress.phase === "clean"
            ? t("library.statusbar.removeClean", { done: removeProgress.done, total: removeProgress.total })
            : t("library.statusbar.removeDelete", { done: removeProgress.done, total: removeProgress.total })}
        </span>
      </div>
    {/if}
    {#if metaProgress && metaProgress.phase !== "done" && metaProgress.total > 0}
      <div class="scan-progress meta-progress" title={t("library.statusbar.metaTip")} out:outDissolve>
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
    <div class="ctx-backdrop" out:outDissolve onclick={() => (ctxMenu = null)} oncontextmenu={(e) => { e.preventDefault(); ctxMenu = null; }}>
      <div class="ctx ctx-menu" bind:this={ctxEl} style="left:{ctxPos.left}px; top:{ctxPos.top}px;" out:outFade={{ from: 0.96 }}>
        <button class="ctx-item" data-testid="lib-ctx-open" onclick={() => { tryOpenBook(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconBook2 size={16} stroke={1.5} /></span>{t("library.ctx.open")}</button>
        <button class="ctx-item" data-testid="lib-ctx-detail" onclick={() => { void detailMod().catch(() => undefined); detailBook = ctxMenu!.book; ctxMenu = null; }}><span class="ci"><IconInfoCircle size={16} stroke={1.5} /></span>{t("library.ctx.detail")}</button>
        {#if ctxMenu.book.format === "txt"}
          <button class="ctx-item" data-testid="lib-ctx-convert" onclick={() => { openConvert(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconArrowsExchange size={16} stroke={1.5} /></span>{t("library.ctx.convert")}</button>
        {/if}
        <div class="ctx-sep"></div>
        <button class="ctx-item" data-testid="lib-ctx-cover" onclick={() => { setCover(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconPhoto size={16} stroke={1.5} /></span>{t("library.ctx.setCover")}</button>
        {#if ctxMenu.book.format !== "pdf"}
          <button class="ctx-item" data-testid="lib-ctx-reextract" onclick={() => { reextract(ctxMenu!.book); ctxMenu = null; }}><span class="ci"><IconRefresh size={16} stroke={1.5} /></span>{t("library.ctx.reextract")}</button>
        {/if}
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

  {#if dupesOpen}
    <DuplicateBooksModal onclose={() => (dupesOpen = false)} />
  {/if}

  {#if detailBook}
    {#await detailMod()}
      <div style="padding:16px;color:var(--fg2);text-align:center">{t("common.loading")}</div>
    {:then { default: C }}
      <C
        book={detailBook}
        onclose={() => { detailBook = null; touchWindow(); }}
        onupdated={(title, author) => {
          const b = detailBook;
          if (!b) return;
          const next = { ...b, title, author };
          detailBook = next;
          rowMap.set(b.id, next);
          winVersion++;
          touchWindow();
        }}
      />
    {:catch}
      <div style="padding:16px;color:var(--fg2);text-align:center">{t("boot.dynamicFailed")}</div>
    {/await}
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
  .maint-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
    gap: var(--sp-3);
    margin: var(--sp-3) var(--sp-4) 0;
    padding: 8px 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    background: var(--panel);
    color: var(--fg2);
    font-size: var(--fs-xs);
  }
  .maint-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .maint-banner span {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .maint-sub {
    font-size: var(--fs-2xs);
    color: var(--fg2);
    opacity: 0.8;
  }
  .maint-actions {
    display: flex;
    gap: var(--sp-2);
    flex: none;
  }
  .maint-banner button {
    flex: none;
    font-size: var(--fs-2xs);
    white-space: nowrap;
  }
  .library {
    height: 100%;
    display: flex;
    flex-direction: column;
    /* 环境光：一层极淡的径向提亮，光源固定在左上（at 18% 0），与封面、卡片投影同一方向。
       纯色平铺在 1792px 宽窗口下会显得空且平；这层给底色一点纵深。
       刻意用 --bg-elev 混透明而不是写死白色：浅色下它提亮，暗色下同样提亮，
       一条配方两种主题都成立，不需要第二套规则。
       静态、不参与滚动、不产生合成层（只是一条 background-image）。 */
    background:
      radial-gradient(120% 80% at 18% 0%, color-mix(in srgb, var(--bg-elev) 60%, transparent), transparent 70%),
      var(--bg);
  }

  /* ---------- 顶栏 ---------- */
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
    row-gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    /* 右侧留出自绘窗口控制按钮（约 3×30 + 间距 + 右边距 ≈ 100px）的宽度并留自然间距 */
    padding-right: 112px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    /* 刻意不给投影：顶栏是**静态表面**，本仓库的层级纪律是「静态表面只靠发丝边 + 面色差，
       投影只留给 popover/drawer/dialog/卡片 hover 四类」。原先这里写了一条硬编码纯黑阴影
       （rgba(23,25,30,.35)），既违反上述纪律，又在暗色外壳下完全不可见（黑阴影落在
       #1e2126 上读不出来），等于只在浅色模式生效的一半装饰。层级改由 --line 承担。 */
    flex-shrink: 0;
    position: relative;
    z-index: var(--z-raised);
  }

  /* 品牌字标：换成衬线字族。这是一个**读书**应用，衬线字标比无衬线多一层「书卷气」，
     也与正文默认的衬线阅读方案同源；同时把无衬线的工具栏按钮群与品牌区分开。
     Noto Serif SC 是随机器安装的可变字重衬线（见 app.css 的字族注释），Georgia 兜底拉丁。 */
  .logo {
    font-family: "Noto Serif SC", Georgia, "Times New Roman", serif;
    font-weight: 700;
    font-size: 17px;
    color: var(--accent);
    margin-right: var(--sp-3);
    letter-spacing: -0.01em;
    user-select: none;
  }

  .search {
    width: 220px;
    padding-right: 26px;
  }

  .search-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }

  .search-clear {
    position: absolute;
    right: 3px;
    top: 50%;
    transform: translateY(-50%);
    width: 20px;
    height: 20px;
    min-height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    color: var(--fg2);
    cursor: pointer;
  }

  .search-clear:hover {
    background: var(--hover-bg);
    color: var(--fg);
  }
  /* 全局 button:active 的位移优先级更高，会把按钮从居中位置拉下来；按下时保持居中。 */
  .search-clear:active:not(:disabled) {
    transform: translateY(-50%);
  }
  .search-clear :global(svg) {
    flex: none;
  }

  /* 中文按钮/下拉不允许逐字竖排：窄窗口靠 flex-wrap 折行，而不是压窄文字。 */
  .toolbar button,
  .toolbar .cat-btn,
  .toolbar select {
    white-space: nowrap;
  }

  .spacer { flex: 1; }

  /* ---------- 分类筛选 ---------- */
  .filter-wrap {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .cat-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--accent-soft);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: var(--radius-pill);
    padding: 4px 10px;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .filter-chip .fx {
    font-weight: 700;
    line-height: 1;
    opacity: 0.9;
  }
  .filter-chip .fx :global(svg) {
    width: 14px;
    height: 14px;
    stroke-width: 1.8;
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
    animation: tp-menu-in var(--dur-fast) var(--ease-in) both;
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
  .fp-tags .tag.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }

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
  .folders {
    display: flex;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4) 0;
    flex-wrap: wrap;
  }

  /* 上次扫描异常态常驻提示条：不折叠、不进设置，只陈述数量与目录。 */
  .scan-stale {
    margin: var(--sp-3) var(--sp-4) 0;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel2);
    color: var(--fg);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-line;
  }
  .scan-stale .ss-line2 {
    color: var(--fg-dim, var(--fg));
    opacity: 0.75;
    word-break: break-all;
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

  /* 选中态：仿分类标签 .fp-tags .tag.active。 */
  .folder-chip.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }

  /* 移除按钮：默认隐藏，指针移到本卡片才显形（卡片上一排 × 一直挂着很吵，
     而移除是低频动作，不该和「选这个文件夹」抢视觉层级）。
     只用 opacity 隐藏、**不**动 width/padding —— 元素始终占位，
     所以显形时卡片不会横向抖一下。
     pointer-events: none 是必须的：只藏 opacity 会留下「看不见却点得到」的幽灵热区，
     点卡片右侧会误触移除。
     :focus-within 让键盘 Tab 到它时也能看见，不只靠 hover。 */
  .folder-chip .x {
    opacity: 0;
    pointer-events: none;
    border: none;
    background: transparent;
    padding: 0 4px;
    cursor: pointer;
    color: var(--fg2);
    border-radius: var(--radius-sm);
    flex-shrink: 0;
    transition: opacity var(--dur-fast) var(--ease), background var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  .folder-chip:hover .x,
  .folder-chip:focus-within .x {
    opacity: 1;
    pointer-events: auto;
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

  /* 刻意不加 will-change: transform：虚拟滚动改的是各行的 style="top"
     （.grid-row 绝对定位），这两个容器自身从不发生 transform 变化，提示买不到任何
     加速；反而会请求合成层按容器高度分配纹理 —— 超大书库下这里可达数万像素高，
     远超 GPU 纹理上限，引擎要么拒绝提升、要么退化成平铺光栅化。 */
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

  .card {
    width: 100%;
    box-sizing: border-box;
    cursor: pointer;
    border-radius: var(--radius-lg);
    padding: var(--sp-2);
    /* 双层嵌套（Double-Bezel）：.card 是外层托盘，.cover 是嵌在其中的内芯。
       外壳给一层发丝边 + 极轻投影，托着封面这块「实体」；不这样处理的话卡片
       静置时完全没有边界（.card 无 border / 无 shadow），整屏只剩封面的投影，
       网格读起来是「一堆浮空的图片」而不是「一排卡片」。 */
    position: relative; /* 让 hover 的 z-index 生效，见下方 .card:hover */
    box-shadow: var(--edge-hi), var(--shadow-sm);
    /* 悬停抬升：柔和抬起 + 分层扩散阴影；用 premium 落位曲线模拟质量感 */
    transition: background var(--dur-lift) var(--ease-premium),
      transform var(--dur-lift) var(--ease-premium),
      box-shadow var(--dur-lift) var(--ease-premium);
  }

  .card:hover {
    background: var(--hover-bg);
    transform: translateY(-4px);
    box-shadow: var(--edge-hi), var(--shadow-md);
    /* 必须显式提层：.grid-row 是依次绘制的兄弟节点，卡片抬起的 4px 会伸进
       下一行的 12px 间隙里，不提层就会被后画的行盖住阴影。 */
    z-index: var(--z-raised);
  }

  .card:active {
    transform: translateY(-1px) scale(0.99);
    transition-duration: var(--dur-fast);
  }

  /* 键盘可达：卡片此前是纯 div、无 tabindex，键盘用户完全够不到书库。
     focus-visible 的外圈走全局规则（app.css 只给 button/summary/[tabindex]）。 */
  .card:focus-visible,
  .row:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 62%, transparent);
    outline-offset: 1px;
  }

  /* 未加载完成的占位槽：只占位保持列对齐，不响应交互、无悬停效果。
     此前它与已加载卡片同貌（同底色 + 同投影 + 同高度的空 .meta），看上去
     就是「一本没有封面的书」。这里压到极低对比并扫一道微光，明确是骨架。 */
  .card-slot {
    cursor: default;
    pointer-events: none;
  }
  .card-slot:hover {
    background: transparent;
    transform: none;
    box-shadow: none;
  }
  .card-slot .cover {
    background: color-mix(in srgb, var(--fg2) 8%, transparent);
    box-shadow: none;
    background-image: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--fg2) 14%, transparent),
      transparent
    );
    background-size: 200% 100%;
    animation: sk-shimmer 1.2s var(--ease) infinite;
  }

  /* 封面外层：只负责尺寸与叠影溢出（不裁切）；内层 .cover-frame 承接圆角/描边/图片裁切。 */
  .cover {
    position: relative;
    width: 100%;
    overflow: visible;
    border-radius: calc(var(--radius-lg) - var(--sp-2));
  }

  /* 内芯：原 .cover 的圆角/描边/裁切/阴影移到这里，尺寸由外层决定。 */
  .cover-frame {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: calc(var(--radius-lg) - var(--sp-2));
    overflow: hidden;
    background: var(--panel2);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm);
    transition: box-shadow var(--dur-lift) var(--ease-premium);
  }

  .card:hover .cover-frame {
    box-shadow: var(--shadow-md);
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform var(--dur-lift) var(--ease-premium);
  }

  .card:hover .cover img {
    transform: scale(1.055);
  }

  /* ---------- 重复书叠影卡组 ---------- */
  .dup-ghost {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    overflow: hidden;
    background: var(--panel2);
    outline: 1px solid var(--line);
    outline-offset: -1px;
    pointer-events: none;
    z-index: 0;
    opacity: 0.55;
    filter: saturate(0.45) brightness(0.82);
    transition: transform var(--dur-lift) var(--ease-premium), opacity var(--dur-lift) var(--ease-premium);
  }
  .dup-ghost img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .dup-ghost.g2 { transform: translate(4px, 4px) rotate(1.4deg); opacity: 0.3; }
  .dup-ghost.g1 { transform: translate(2px, 2px) rotate(0.7deg); opacity: 0.45; }
  .card.dup-card .cover-frame { z-index: 1; }
  .card.dup-card:hover .dup-ghost.g2 { transform: translate(15px, 10px) rotate(2.4deg); opacity: 0.42; }
  .card.dup-card:hover .dup-ghost.g1 { transform: translate(8px, 5px) rotate(1.2deg); opacity: 0.6; }
  .card:hover .dup-ghost img { transform: none; }
  .dup-badge {
    position: absolute;
    right: 8px;
    bottom: 8px;
    z-index: 2;
    padding: 2px 7px;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--accent-fg, #fff);
    font-size: var(--fs-2xs);
    font-weight: 600;
    line-height: 1.4;
    pointer-events: none;
    box-shadow: var(--shadow-sm);
  }
  .dup-list-badge {
    margin-left: 6px;
    padding: 0 5px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    font-size: var(--fs-2xs);
    font-weight: 600;
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: reduce) {
    .dup-ghost { transition: none; }
  }

  /* 格式角标：半透明深底 + 白色小字，10px 字距 +0.015em。
     注意：不用 backdrop-blur —— 卡片处于虚拟滚动网格内，滚动中模糊层会持续触发 GPU 重绘、
     在低端机上掉帧（perf: 禁止对滚动内容用 backdrop-filter）。改用实心半透明底达到同等观感。 */
  .fmt {
    position: absolute;
    /* 内移到 14px：封面新增的书框线在 SVG 坐标 13/320 处（180px 宽的封面上约 7px），
       角标停在 8px 会正好压在框线上、把书框右上角切掉，读起来像碰撞而不是设计。
       14px 让角标整块落在框内，两种卡片尺寸（120–216px 宽）下都留得住 5px 以上间隙。 */
    right: 14px;
    top: 14px;
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

/* 标题/作者依次上浮落位（错峰 60ms）。
     为什么逐卡挂动画在虚拟滚动里不会「闪烁」：虚拟窗口有 ±3 行 overscan，
     新行是在进入视口【之前】挂载的，动画在屏幕外就跑完了 —— 用户看到它们时
     早已是终态。全量重载（排序/筛选/搜索）时整批重挂，这时才有肉眼可见的错峰落位。
     挂在 .meta 的子元素上而非卡片本身：卡片身上已有 340ms 的 hover 抬升过渡，
     同一元素再叠一个 transform 动画，两者会互相覆盖。 */
  .meta .t {
    font-size: var(--fs-md);
    line-height: 20px;
    font-weight: 500;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    animation: tp-meta-rise var(--dur-lift) var(--ease-premium) both;
  }

  .meta .a {
    font-size: var(--fs-xs);
    line-height: 18px;
    color: var(--fg2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-top: 1px;
    animation: tp-meta-rise var(--dur-lift) var(--ease-premium) 60ms both;
  }

  /* ---------- 列表 ---------- */
  /* 同 .grid-inner：容器自身从不 transform，故不挂 will-change（理由见上）。 */
  .list-inner {
    position: relative;
    width: 100%;
    /* 限宽：书库网格用满宽是对的（列数随之增加），列表行用满宽是错的 ——
       1792px 窗口下书名贴左、大小/日期贴右，中间空出 1400px 的真空带，
       两端的字段读不出「同一行」的关系。限到 1040px 后整行才成为一个可扫视的整体。
       刻意**不居中**（不写 margin-inline: auto）：网格卡片与顶栏都从左侧 20px 起排，
       居中的列表会自成一块浮在中间，与上面的工具栏对不上边。 */
    max-width: 1040px;
  }

  /* 视图切换的入场：整块淡入上浮。
     只做 opacity + translateY（不碰 filter/box-shadow）：.grid-inner/.list-inner
     在超大书库下可高达数万像素，任何大面积 blur 或阴影重绘都会在切视图那一帧
     卡住；虚拟窗口逐行增删，若给每张卡挂动画则滚动中会不断重播（像闪烁）。 */
  .grid-inner,
  .list-inner {
    animation: tp-content-in var(--dur-lift) var(--ease-premium) both;
  }

  @keyframes tp-content-in {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }

  /* 列表行此前只有 4% 的整行底色，比网格卡片平一整档 —— 同一个应用两个视图
     像两种设计。补上抬升 + 封面缩放 + 按下反馈，与卡片同源。 */
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
    /* transform 只在 hover/active 出现，静止态不占合成层（虚拟滚动下
       上千个 .row 若都提层会把显存吃满）。 */
    transition: background var(--dur) var(--ease), box-shadow var(--dur) var(--ease),
      transform var(--dur) var(--ease-premium);
  }

  .row:hover {
    background: var(--hover-row);
    transform: translateY(-1px);
    box-shadow: var(--shadow-sm);
    z-index: var(--z-raised);
  }

  .row:active {
    transform: translateY(0) scale(0.996);
    transition-duration: var(--dur-fast);
  }

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
    transition: transform var(--dur) var(--ease-premium), box-shadow var(--dur) var(--ease);
  }

  /* 行悬停时封面轻微放大，与网格卡片的 scale(1.055) 同一套语言；
     1.08 而非 1.055：列表封面只有 32px 宽，放大同样比例几乎看不出来。 */
  .row:hover .row-cover {
    transform: scale(1.08);
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

  /* 行内阅读进度：网格视图有封面进度环，列表视图此前**完全没有**进度信息，
     行中段又是一片空白。补一条与进度环同源（同一个 ringInfo）的细进度轨 + 百分比，
     把空白换成有用信息，两个视图的信息量也就对齐了。
     轨道粗细/圆角/配色沿用全局进度条令牌（--bar-h / --radius-pill / --accent）。 */
  .row-prog {
    flex-shrink: 0;
    width: 132px;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .row-prog .rp-track {
    flex: 1;
    height: var(--bar-h);
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--fg2) 22%, transparent);
    overflow: hidden;
  }

  .row-prog .rp-fill {
    height: 100%;
    border-radius: var(--radius-pill);
    background: var(--accent);
    transition: width var(--dur) var(--ease);
  }

  /* 已读完用成功色，与网格进度环的「满环」语义一致 */
  .row-prog .rp-fill.full {
    background: var(--success);
  }

  .row-prog .rp-num {
    flex-shrink: 0;
    width: 34px;
    text-align: right;
    font-size: var(--fs-2xs);
    letter-spacing: var(--ls-2xs);
    color: var(--fg2);
    font-variant-numeric: tabular-nums;
  }
  .row-meta .t { font-size: var(--fs-md); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-meta .a { font-size: var(--fs-xs); color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-size, .row-date { color: var(--fg2); font-size: var(--fs-xs); width: 90px; text-align: right; font-variant-numeric: tabular-nums; }

  /* ---------- 空状态 ---------- */
  /* 加载中：给一个旋转指示器。此前只有「加载中…」三个字，且组件从不 import
     reducedMotion.ts（JS 反馈未做动效门控），这里靠 CSS 的全局 reduce-motion
     块静止。 */
  .empty-state[data-testid="lib-hint-loading"] {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
  }
  .empty-state[data-testid="lib-hint-loading"]::before {
    content: "";
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    border-radius: 50%;
    border: 1.5px solid color-mix(in srgb, var(--fg2) 28%, transparent);
    border-top-color: var(--accent);
    animation: tp-spin 0.9s linear infinite;
  }

  /* 书库为空 / 无匹配结果：文案里的 \n 需要真实换行才能分行显示。
     .empty-state 是全局基类（多处共用），不能给 pre-line —— 别的调用方里
     没有需要保留的换行，加了反而会在别处出现意外空行。故只在本组件的空态上加。 */
  .lib-blank {
    white-space: pre-line;
  }

  /* ---------- 空书库的「开始」视图 ----------
     此前只有一段靠顶对齐的灰字（3 行、48px 上内边距），落在一整屏底色上读起来像渲染失败。
     现在给一个居中的组合：品牌标记 + 标题 + 说明 + 主行动按钮 + 支持格式。
     文案仍全部取自 library.empty.blank 一条键（拆 \n 渲染），不新增 i18n 键。 */
  .lib-start {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-4);
    padding: var(--sp-4);
    text-align: center;
    /* 容器不吃指针：.grid 上挂着框选的 pointer 处理，空态不该挡住它。
       唯一可交互的是主行动按钮。 */
    pointer-events: none;
  }

  .lib-start .start-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 64px;
    height: 64px;
    border-radius: var(--radius-xl);
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: var(--edge-hi);
  }

  .lib-start .start-copy {
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    max-width: 36ch;
  }

  .lib-start .start-title {
    margin: 0;
    font-size: 19px;
    font-weight: 600;
    letter-spacing: var(--ls-tight);
    color: var(--fg);
  }

  .lib-start .start-sub {
    margin: 0;
    font-size: var(--fs-sm);
    line-height: 1.7;
    color: var(--fg2);
  }

  /* 主行动按钮：整圆药丸 + 内部圆形图标承托（button-in-button）。
     工具栏里的控件仍是 6px 圆角矩形 —— 药丸只留给「唯一的主行动」，层级才唯一。 */
  .lib-start .start-cta {
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: 10px;
    min-height: 38px;
    padding: 0 6px 0 20px;
    border-radius: var(--radius-pill);
    font-size: var(--fs-sm);
    font-weight: 600;
  }

  .lib-start .start-cta .cta-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    /* --accent-fg 是「强调底上的前景色」：浅色主题下为白、暗色下为近黑，
       故这圈承托在两种主题下都与 accent 底形成同向对比。 */
    background: color-mix(in srgb, var(--accent-fg) 16%, transparent);
    transition: transform var(--dur-lift) var(--ease-premium), background var(--dur) var(--ease);
  }

  /* 磁吸：hover 时图标圈向右上位移 + 微放大，制造按钮内部的动能张力 */
  .lib-start .start-cta:hover .cta-icon {
    transform: translate(1px, -1px) scale(1.06);
    background: color-mix(in srgb, var(--accent-fg) 26%, transparent);
  }

  /* 空态逐项错峰落位：标记 → 标题/说明 → 按钮 → 格式。
     这个状态一次挂载只出现一次（不在虚拟滚动里），所以逐项入场不会像书库卡片那样
     在滚动中重播 —— 那正是 app.css 里 tp-meta-rise 注释警告过的场景。
     填充模式用 backwards 而**不是** both：both 会在动画结束后继续把 transform 钉在
     none 上，压过按钮自己的 :active 按下反馈（动画填充优先于普通声明）。 */
  .lib-start > * {
    animation: tp-meta-rise var(--dur-lift) var(--ease-premium) backwards;
  }
  .lib-start > *:nth-child(2) { animation-delay: 60ms; }
  .lib-start > *:nth-child(3) { animation-delay: 120ms; }
  .lib-start > *:nth-child(4) { animation-delay: 180ms; }

  .lib-start .start-formats {
    margin: 0;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: var(--fg2);
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

  /* 版本号是次要信息，但**仍是文字**：原先叠了 opacity .55，实测有效对比度只有
     ~1.9:1（浅色）/ ~2.4:1（暗色），远低于正文 4.5:1 的门槛。次要层级改由 --fg2 表达
     （它本身就是为次要文字定义的），不再压不透明度。 */
  .statusbar .build {
    font-size: var(--fs-xs);
    color: var(--fg2);
  }

  .lib-tb-group {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    position: relative;
    margin-left: auto;
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
    background: var(--accent-soft);
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
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    animation: tp-menu-in var(--dur-fast) var(--ease-in) both;
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
    /* 进度块随事件到来凭空出现、结束又凭空消失，两端都是硬切。入场给一次
       淡入上浮（块本身是横向的横向位移读不出来，用纵向 4px），退场见模板里的
       out:outDissolve。 */
    animation: tp-rise-in var(--dur) var(--ease-premium) both;
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

  /* 右键菜单项错峰落位：此前整块菜单一起淡入，12 个条目读起来是一个整体。
     逐条 26ms 上浮后，菜单的「来路」（指针所在处）就有了方向感。
     12 × 26ms = 312ms，与抽屉入场同量级，不会拖成慢动作。 */
  .ctx-menu .ctx-item {
    animation: tp-rise-in var(--dur) var(--ease-premium) both;
  }
  .ctx-menu .ctx-item:nth-child(1) { animation-delay: 0ms; }
  .ctx-menu .ctx-item:nth-child(2) { animation-delay: 26ms; }
  .ctx-menu .ctx-item:nth-child(3) { animation-delay: 52ms; }
  .ctx-menu .ctx-item:nth-child(4) { animation-delay: 78ms; }
  .ctx-menu .ctx-item:nth-child(5) { animation-delay: 104ms; }
  .ctx-menu .ctx-item:nth-child(6) { animation-delay: 130ms; }
  .ctx-menu .ctx-item:nth-child(7) { animation-delay: 156ms; }
  .ctx-menu .ctx-item:nth-child(8) { animation-delay: 182ms; }
  .ctx-menu .ctx-item:nth-child(9) { animation-delay: 208ms; }
  .ctx-menu .ctx-item:nth-child(10) { animation-delay: 234ms; }
  .ctx-menu .ctx-item:nth-child(n + 11) { animation-delay: 260ms; }

  /* ---------- 多选（长按 / 右键拉框） ---------- */
  /* 选中态描边用 box-shadow 而不是 outline：outline-color 在多数引擎里不插值，
     transition 无效。inset box-shadow 画在边框内侧，视觉与 outline-offset:-2px 等价且可过渡。
     悬停规则必须复述这一条：.card:hover 与 .card.selected 特异性相同、后写的
     .selected 会整条覆盖 hover 的 box-shadow —— 表现为「选中卡片悬停时抬升阴影
     突然消失」，此前确实如此。 */
  .card.selected,
  .row.selected {
    box-shadow: var(--edge-hi), inset 0 0 0 2px var(--accent);
  }

  .card:hover,
  .row:hover {
    box-shadow: var(--edge-hi), var(--shadow-md);
  }

  .card.selected:hover,
  .row.selected:hover {
    /* 两条同时要：抬升阴影在外、选中环在内（inset 叠在外层阴影之上）。 */
    box-shadow: var(--edge-hi), var(--shadow-md), inset 0 0 0 2px var(--accent);
  }

  .card.selected .cover {
    box-shadow: var(--shadow-md);
  }

  /* 勾选徽标：此前无任何 transition，on/off 是瞬变的 —— 长按进入多选后，
     一批徽标同时「啪」地变色。现在底色/边框走 --dur-fast，勾号本身用弹簧
     曲线做一次轻微过冲，读作「按下去咔哒一声扣上了」。 */
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
    transition: background var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease);
  }

  .sel-badge.on {
    background: var(--accent);
    border-color: var(--accent);
    animation: sel-badge-pop var(--dur) var(--ease-spring) both;
  }

  @keyframes sel-badge-pop {
    from {
      transform: scale(0.7);
    }
    to {
      transform: scale(1);
    }
  }

  /* 勾号淡入：徽标底色已经插值过了，勾号若瞬现会读作「贴上去的图标」。
     这条必须写在全局（本文件的 <style> 是 scoped 的，匹配不到 IconCheck
     内部渲染的 <svg> —— app.css 里那批 .xxx .tabler-icon 规则同此理由）。 */
  :global(.sel-badge .tabler-icon) {
    animation: tp-fade-in var(--dur-fast) var(--ease) both;
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
