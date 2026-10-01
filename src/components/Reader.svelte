<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { fade } from "svelte/transition";
  import { listen, emit } from "@tauri-apps/api/event";
  import { api } from "../lib/api";
  import {
    config,
    confirmBox,
    convertState,
    pendingOpen,
    saveConfig,
    toast,
    ui,
    THEMES,
    appConfirm,
    BIND_DEFS,
    comboOf,
    bindKeyMatches,
    toggleAi,
    openPanelWindow,
    ZONE_DEFS,
    copyText,
    stripNonTextHtml,
    requestOpenBook,
    SCROLL_BOUNCE_PRESETS,
    READER_TOPBAR_IDS,
    type ReaderTopbarId,
    type BindAction,
    type ZoneKey,
    type StatusItemId,
    type ThemeKey,
  } from "../lib/stores.svelte";
  import { getBookOverride, setBookOverride, type BookOverride } from "../lib/bookcfg.svelte";
  import { dragSort } from "../lib/dragSort";
  import { dragOutHandle } from "../lib/windowSnap";
  import { applyReadFix, clearReadFix } from "../lib/readFix";
  import { SmoothScroller } from "../lib/smoothScroll";
  import { preloadReaderFonts } from "../lib/fontfaces";
  import {
    charRangeToRange,
    clearTtsMark,
    findTextRange,
    paintHighlights,
    paintTtsMark,
    selectionOffsets,
  } from "../lib/highlights";
  import {
    segIndexAt,
    splitSegments,
    ttsNext,
    ttsPauseResume,
    ttsPrev,
    ttsSetCallbacks,
    ttsSetSegments,
    ttsStart,
    ttsState,
    ttsStop,
  } from "../lib/tts.svelte";
  import type { FlatSegment } from "../lib/tts.svelte";
  import type { PopupAnchor } from "../lib/placePopup";
  import type { Icon as TablerIcon } from "@tabler/icons-svelte";
  import {
    IconAdjustmentsHorizontal,
    IconArrowLeft,
    IconArrowsExchange,
    IconBook2,
    IconBookmarkPlus,
    IconBookmarks,
    IconBooks,
    IconChartBar,
    IconCloudDownload,
    IconCloudUp,
    IconHighlight,
    IconList,
    IconPalette,
    IconPhoto,
    IconPhotoOff,
    IconPlayerPause,
    IconPlayerPlay,
    IconPlayerStop,
    IconSettings,
    IconSparkles,
  } from "@tabler/icons-svelte";
  import AiPanel from "./AiPanel.svelte";
  import TocPanel from "./TocPanel.svelte";
  import BookStatsModal from "./BookStatsModal.svelte";
  import SelectionMenu from "./SelectionMenu.svelte";
  import BookSearchPopup from "./BookSearchPopup.svelte";
  import QuickAiPopup from "./QuickAiPopup.svelte";
  import HighlightsPanel from "./HighlightsPanel.svelte";
  import HighlightMenu from "./HighlightMenu.svelte";
  import TtsBar from "./TtsBar.svelte";
  import TtsStartMenu from "./TtsStartMenu.svelte";
  import ImageViewer from "./ImageViewer.svelte";
  import type { BookImage, ChapterData, HighlightRow, OpenResult, Progress } from "../lib/types";

  let { standalone = false }: { standalone?: boolean } = $props();

  let book: OpenResult | null = $state(null);
  let chapter: ChapterData | null = $state(null);
  let loadingChapter = $state(false);
  let chapterNum = $state(0);

  let scrollWrap: HTMLDivElement | undefined = $state();
  let scrollArea: HTMLDivElement | undefined = $state();
  let pageWrap: HTMLDivElement | undefined = $state();
  let contentEl: HTMLDivElement | undefined = $state();
  let bodyEl: HTMLDivElement | undefined = $state();
  let thumbEl: HTMLDivElement | undefined = $state();
  let scrollbarEl: HTMLDivElement | undefined = $state();

  // A?/A＋ 悬浮控件：默认完全隐藏（pointer-events:none），仅指针进入右下角热区才淡入，
  // 避免透明热区常驻拦截正文框选/右下角翻页点击。
  let fontCtlShow = $state(false);
  let hoverRaf = 0;
  let hoverX = 0;
  let hoverY = 0;

  // 沉浸阅读：空闲后淡出顶栏/底栏；任何指针移动或按键立即恢复并重新计时。
  let chromeHidden = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  function pokeChrome() {
    if (chromeHidden) chromeHidden = false;
    if (!config.reader.immersive) return;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (chromeHidden = true), 2600);
  }
  $effect(() => {
    if (!config.reader.immersive) {
      chromeHidden = false;
      clearTimeout(idleTimer);
    }
  });
  onDestroy(() => clearTimeout(idleTimer));

  // 顶栏实测高度写入全局 CSS 变量：供固定在右上角的 WindowControls 让自身高度=顶栏高度，
  // 按钮垂直居中即与顶栏图标中心线重合，自动对齐（Reader 顶栏比 Library 矮，靠此自适应）。
  let tbH = $state(0);
  $effect(() => {
    if (tbH > 0) {
      document.documentElement.style.setProperty("--fr-titlebar-h", tbH + "px");
    }
  });

  // 当前章变化时向其它窗口广播（带 bookId，供独立目录窗口按 book 过滤后跟随定位）。
  // 用统一 $effect 广播，避免在 loadChapter 的多个分支里漏发；主窗内嵌抽屉靠 current 已实时、不受影响。
  $effect(() => {
    if (!book) return;
    void emit("tidepage-chapter-changed", { bookId: book.book.id, index: chapterNum });
  });

  let scrollTop = $state(0);
  let maxScroll = $state(0);

  let page = $state(0);
  let pageCount = $state(1);
  let colW = 0;
  let gap = 48;
  let stepPx = 0;
  let contentW = $state(0);
  /** 图片最大高度（px）：页模式=单列高，滚动模式=可视区高；用于图片自适应屏幕。 */
  let imgMaxH = $state(0);
  /** 图片查看器状态：null=关闭；否则为当前图片索引（-1 表示列表未就绪）。 */
  let viewerIndex = $state<number | null>(null);
  /** 全书图片列表（打开查看器时按需加载）。 */
  let bookImages: BookImage[] | null = $state(null);

  // ===== 单本覆盖（按书持久化）=====
  // 这些函数读取 bookOverrides（$state）→ 在模板/effect 中调用即为响应式；
  // 未设置时回退到全局 config，保证与既有行为一致。
  function bookOv(): BookOverride {
    return getBookOverride(book?.book.id);
  }
  /** 本书是否单独套用了「阅读配色方案」（任一 scheme* 字段存在即视为已套用）。 */
  function bookScheme(): BookOverride | null {
    const o = bookOv();
    return o.schemeTheme !== undefined || o.schemeBg !== undefined || o.schemeFg !== undefined ? o : null;
  }
  /** 有效阅读配色：优先单本方案，其次全局。 */
  function effTheme(): ThemeKey {
    return (bookScheme()?.schemeTheme as ThemeKey) ?? config.reader.theme;
  }
  function effBgColor(): string {
    return bookScheme()?.schemeBg ?? config.reader.bgColor;
  }
  function effFgColor(): string {
    return bookScheme()?.schemeFg ?? config.reader.fgColor;
  }
  function effFontFamily(): string {
    return bookScheme()?.schemeFontFamily ?? config.reader.fontFamily;
  }
  function effLatinFont(): string {
    return bookScheme()?.schemeLatinFont ?? config.reader.latinFont;
  }
  function effLineHeight(): number {
    return bookScheme()?.schemeLineHeight ?? config.reader.lineHeight;
  }
  function effParaSpacing(): number {
    return bookScheme()?.schemeParaSpacing ?? config.reader.paraSpacing;
  }
  function effIndent(): string {
    return bookScheme()?.schemeIndent ?? config.reader.indent;
  }
  function effTextAlign(): string {
    return (bookScheme()?.schemeTextAlign as string) ?? config.reader.textAlign;
  }
  function effFontWeight(): number {
    return bookScheme()?.schemeFontWeight ?? config.reader.fontWeight;
  }
  function effCssExtra(): string {
    return bookScheme()?.schemeCssExtra ?? config.reader.cssExtra;
  }
  function effFontSize(): number {
    const o = bookOv();
    return o.fontSize ?? config.reader.fontSize;
  }
  function effOverrideCss(): boolean {
    const o = bookOv();
    return o.overrideBookCss ?? config.reader.overrideBookCss;
  }
  /** 是否处于漫画模式（自动检测结果，或用户手动切换）。 */
  function isMangaMode(): boolean {
    const o = bookOv();
    return o.mangaMode ?? book?.is_manga ?? false;
  }
  /** 漫画横向（横向连续阅读）：仅漫画模式下的「横向」布局。 */
  function mangaHorizontal(): boolean {
    return isMangaMode() && config.reader.manga.layout === "horizontal";
  }
  /** 漫画瀑布流：漫画模式下选择「瀑布流」（纵向连续滚动）。 */
  function mangaFlow(): boolean {
    return isMangaMode() && config.reader.manga.layout === "flow";
  }
  /** 当前是否采用纵向滚动（瀑布流）渲染：漫画模式由漫画显示方式决定，普通书跟随全局翻页模式。 */
  function scrollLayout(): boolean {
    if (isMangaMode()) return config.reader.manga.layout === "flow";
    return config.reader.mode === "scroll";
  }
  /** 从右到左（日漫）阅读方向：仅横向布局有意义。 */
  function mangaRtl(): boolean {
    return mangaHorizontal() && config.reader.manga.direction === "rtl";
  }
  function effImgScale(): number {
    const o = bookOv();
    if (o.imgScale != null) return o.imgScale;
    return isMangaMode() ? config.reader.manga.imgScale : 1;
  }
  function effFitWidth(): boolean {
    const o = bookOv();
    return o.fitWidth ?? (isMangaMode() ? config.reader.manga.fitWidth : false);
  }
  function effFitHeight(): boolean {
    const o = bookOv();
    return o.fitHeight ?? (isMangaMode() ? config.reader.manga.fitHeight : false);
  }
  function effShowImages(): boolean {
    return bookOv().showImages ?? true;
  }
  function blockedImages(): string[] {
    return bookOv().blockedImages ?? [];
  }
  function setOv(patch: Partial<BookOverride>) {
    if (book) setBookOverride(book.book.id, patch);
  }

  // ===== 顶栏按钮注册表（可拖拽排序 / 可隐藏）=====
  interface TbBtn {
    id: ReaderTopbarId;
    label: string;
    title: string | (() => string);
    /** 返回 Tabler 图标组件；用函数以便随状态切换（如播放/暂停）。 */
    icon: () => TablerIcon;
    /** 可选：激活态（按钮高亮）。 */
    on?: () => boolean;
    visible: () => boolean;
    run: () => void;
    /** 可选：长按（约 550ms）触发（如弹出菜单）；长按已触发则不再执行短按 run。 */
    longPress?: () => void;
  }
  const tbTitle = (b: TbBtn): string => (typeof b.title === "function" ? b.title() : b.title);
  const TOPBAR_BTNS: TbBtn[] = [
    { id: "webdav", label: "上传进度", title: "短按上传进度到云端，长按更多（上传书籍文件）", icon: () => IconCloudUp, visible: () => config.webdav.enabled, longPress: () => toggleWdMenu(), run: () => uploadProgress() },
    { id: "webdavDownload", label: "下载进度", title: "从云端更新阅读进度（需确认）", icon: () => IconCloudDownload, visible: () => config.webdav.enabled, run: () => webdavDownload() },
    { id: "convert", label: "转换格式", title: "转换格式", icon: () => IconArrowsExchange, visible: () => book?.book.format === "txt", run: () => convertThis() },
    { id: "bookmarkAdd", label: "添加书签", title: "添加书签", icon: () => IconBookmarkPlus, visible: () => true, run: () => addBookmark() },
    { id: "bookmarkList", label: "书签列表", title: "书签列表", icon: () => IconBookmarks, visible: () => true, run: () => loadBookmarks() },
    { id: "ttsPlay", label: "朗读", title: "朗读（可选中文字或从本页/本章开始）", icon: () => (ttsState.playing ? IconPlayerPause : IconPlayerPlay), visible: () => true, run: () => toggleTts() },
    { id: "ttsStop", label: "停止朗读", title: "停止朗读", icon: () => IconPlayerStop, visible: () => true, run: () => stopTts() },
    { id: "highlight", label: "划线 / 批注", title: "划线 / 批注", icon: () => IconHighlight, visible: () => true, run: () => setRight("highlights") },
    { id: "ai", label: "AI 助手", title: "AI 助手", icon: () => IconSparkles, visible: () => true, run: () => toggleAiPanel() },
    { id: "stats", label: "本书统计", title: "本书统计", icon: () => IconChartBar, visible: () => true, run: () => setRight("stats") },
    { id: "toc", label: "目录", title: "目录 (F3)", icon: () => IconList, visible: () => true, run: () => toggleToc() },
    { id: "manga", label: "漫画模式", title: () => (isMangaMode() ? "退出漫画模式（本书）" : "切换到漫画模式（本书）"), icon: () => (isMangaMode() ? IconBooks : IconBook2), on: () => isMangaMode(), visible: () => true, run: () => toggleMangaMode() },
    { id: "manager", label: "顶栏按钮管理", title: "顶栏按钮管理", icon: () => IconAdjustmentsHorizontal, visible: () => true, run: () => toggleManager() },
    { id: "images", label: "本书图片", title: "本书图片显示", icon: () => (effShowImages() ? IconPhoto : IconPhotoOff), visible: () => true, run: () => toggleImgMenu() },
    { id: "css", label: "全局样式开关", title: "使用阅读器全局样式（覆盖书籍排版，仅当前书）", icon: () => IconPalette, visible: () => true, run: () => toggleCss() },
    { id: "settings", label: "设置", title: "设置", icon: () => IconSettings, visible: () => true, run: () => (ui.settingsOpen = true) },
  ];
  const topbarBtn = (id: string): TbBtn | undefined => TOPBAR_BTNS.find((b) => b.id === id);
  const visibleTopbarBtns = $derived(
    config.reader.topbarOrder
      .map((id) => topbarBtn(id))
      .filter((b): b is TbBtn => !!b && b.visible() && (b.id === "manager" || !config.reader.topbarHidden.includes(b.id))),
  );

  /** 拖拽排序：把 from 插入到 target 之前/之后（在完整顺序数组内重排，隐藏项位置一并保留）。 */
  function tbDrop(from: string, target: string, place: "before" | "after") {
    if (!from || from === target) return;
    const arr = [...config.reader.topbarOrder];
    const fi = arr.indexOf(from);
    if (fi < 0) return;
    arr.splice(fi, 1);
    const ti = arr.indexOf(target);
    if (ti < 0) return;
    arr.splice(place === "after" ? ti + 1 : ti, 0, from);
    config.reader.topbarOrder = arr;
    saveConfig();
  }

  /** 顶栏「漫画模式」按钮：把普通书切成漫画模式，或把漫画切回普通模式（按书持久化）。 */
  function toggleMangaMode() {
    const next = !isMangaMode();
    setOv({ mangaMode: next });
    toast(next ? "已切换到漫画模式（本书）" : "已切换到普通模式（本书）");
  }

  /** 首次漫画引导：拖动滑块直接改全局默认倍率 → 正文图片即时跟随预览。 */
  function onMangaGuideInput(e: Event) {
    mangaGuideScale = Number((e.currentTarget as HTMLInputElement).value);
    config.reader.manga.imgScale = mangaGuideScale;
    saveConfig();
  }

  /** 完成引导：写入全局标记（之后打开任何漫画都不再弹出），保留当前预览值。 */
  function closeMangaGuide() {
    config.reader.mangaGuideDone = true;
    saveConfig();
    mangaGuideOpen = false;
  }

  function resetMangaGuide() {
    mangaGuideScale = 1;
    config.reader.manga.imgScale = 1;
    saveConfig();
  }

  /** 顶栏按钮显示/隐藏（持久化；重新显示回到 topbarOrder 中的原位置）。 */
  function toggleTopbarHidden(id: string, show: boolean) {
    if (id === "manager") return; // 管理按钮固定显示，否则会把自己藏起来无法恢复
    const set = new Set(config.reader.topbarHidden);
    if (show) set.delete(id);
    else set.add(id);
    config.reader.topbarHidden = [...set];
    saveConfig();
  }

  /** 长按连续触发动作（初始延迟 450ms，之后每 80ms 一次）。 */
  function repeatAction(node: HTMLElement, fn: () => void) {
    let t: ReturnType<typeof setTimeout> | undefined;
    let iv: ReturnType<typeof setInterval> | undefined;
    const stop = () => {
      if (t !== undefined) clearTimeout(t);
      if (iv !== undefined) clearInterval(iv);
      t = undefined;
      iv = undefined;
    };
    const down = (e: PointerEvent) => {
      if (e.button !== 0) return;
      stop();
      fn();
      t = setTimeout(() => {
        iv = setInterval(fn, 80);
      }, 450);
    };
    node.addEventListener("pointerdown", down);
    node.addEventListener("pointerup", stop);
    node.addEventListener("pointerleave", stop);
    node.addEventListener("pointercancel", stop);
    return {
      destroy() {
        stop();
        node.removeEventListener("pointerdown", down);
        node.removeEventListener("pointerup", stop);
        node.removeEventListener("pointerleave", stop);
        node.removeEventListener("pointercancel", stop);
      },
    };
  }

  function toggleCss() {
    const next = !effOverrideCss();
    setOv({ overrideBookCss: next });
    toast(next ? "已使用阅读器全局样式（本书）" : "已启用书籍自带样式（本书）");
  }

  function toggleAllImages() {
    const next = !effShowImages();
    setOv({ showImages: next });
    toast(next ? "已全部开启本书图片" : "已全部关闭本书图片");
  }

  function startBlockPick() {
    imgMenuOpen = false;
    blockPickMode = true;
    toast("屏蔽选择模式：点击正文中的图片即可屏蔽（Esc 取消）");
  }

  function unblockImage(src: string) {
    setOv({ blockedImages: blockedImages().filter((s) => s !== src) });
  }

  /** 点击正文图片 → 打开图库并定位到该图。图片列表随 open_book 一起返回，无需再发命令。 */
  function openViewerForImage(img: HTMLImageElement) {
    const src = img.getAttribute("src") || "";
    if (!src || !book) return;
    const list = book.images ?? [];
    if (!list.length) {
      // 没有图片清单时，至少显示被点的这一张。
      bookImages = [{ url: src, chapter_index: chapterNum, chapter_title: chapter?.title ?? "" }];
      viewerIndex = 0;
      return;
    }
    bookImages = list;
    const i = list.findIndex((b) => b.url === src);
    viewerIndex = i >= 0 ? i : 0;
  }

  let scroller: SmoothScroller | null = null;

  let tocOpen = $state(false);
  // 右侧互斥面板的单一状态源（left:0 的目录 tocOpen 不参与）
  let rightPanel = $state<null | "bookmarks" | "highlights" | "stats" | "ai">(null);
  let bookmarksOpen = $derived(rightPanel === "bookmarks");
  let statsOpen = $derived(rightPanel === "stats");
  let aiDrawerOpen = $derived(rightPanel === "ai");
  let bookmarks: { id: number; chapter_index: number; title: string; excerpt: string }[] = $state([]);

  // 顶栏按钮管理 / 图片开关 / 漫画引导 / 拖拽排序
  let tbManagerOpen = $state(false);
  let imgMenuOpen = $state(false);
  let wdMenuOpen = $state(false);
  // WebDAV 长按菜单的 fixed 定位（相对视口），打开时按被长按按钮的 rect 计算。
  let wdMenuPos = $state({ top: 52, right: 12 });
  // 菜单根元素引用：用于外点判定（点击菜单之外即关闭）。
  let wdMenuEl = $state<HTMLElement | undefined>(undefined);
  let blockedListOpen = $state(false);
  let blockPickMode = $state(false);
  let mangaGuideOpen = $state(false);
  let mangaGuideScale = $state(1);

  let highlights: HighlightRow[] = $state([]);
  let highlightsOpen = $derived(rightPanel === "highlights");
  let hlVersion = $state(0);
  let pendingHighlightJump: HighlightRow | null = null;

  type SelMenuState = { text: string; start: number; end: number; anchor: PopupAnchor };
  let selMenu: SelMenuState | null = $state(null);
  let hlMenu: { h: HighlightRow; anchor: PopupAnchor } | null = $state(null);
  let searchPopup: { query: string; anchor: PopupAnchor } | null = $state(null);
  let quickAiPopup: { text: string; anchor: PopupAnchor; highlightId?: number; sel?: { start: number; end: number; text: string } } | null = $state(null);

  /* ================= 右侧面板互斥（单一状态源 rightPanel） =================
   * rightPanel 是唯一可写来源；bookmarksOpen/statsOpen/highlightsOpen/aiDrawerOpen 全部由它 $derived。
   * 所有 toggle 只改 rightPanel，互斥自动成立（同侧任意时刻最多一个）。
   * 左侧目录 tocOpen（left:0）为独立 $state，不参与本互斥。
   * 抽屉显隐彻底不读 ui.aiOpen，也不再有任何监听 ui.aiOpen 的 $effect。 */
  function setRight(panel: "bookmarks" | "highlights" | "stats" | "ai") {
    // 互斥：打开任一右侧面板/AI 时，关闭顶栏浮层面板（管理/图片/已屏蔽/漫画引导）。
    closeTbPanels();
    rightPanel = rightPanel === panel ? null : panel;
  }

  /** 关闭全部「顶栏浮层面板」（顶栏按钮管理 / 本书图片 / WebDAV 同步菜单 / 已屏蔽图片 / 漫画引导）。 */
  function closeTbPanels() {
    tbManagerOpen = false;
    imgMenuOpen = false;
    wdMenuOpen = false;
    blockedListOpen = false;
    mangaGuideOpen = false;
  }

  /** 顶栏「按钮管理」开合：打开时关闭其它面板（右侧面板 / 目录 / 其它顶栏浮层），互斥。 */
  function toggleManager() {
    const willOpen = !tbManagerOpen;
    closeTbPanels();
    if (willOpen) {
      rightPanel = null;
      tocOpen = false;
      tbManagerOpen = true;
    }
  }

  /** 顶栏「本书图片」开合：互斥同上。 */
  function toggleImgMenu() {
    const willOpen = !imgMenuOpen;
    closeTbPanels();
    if (willOpen) {
      rightPanel = null;
      tocOpen = false;
      imgMenuOpen = true;
    }
  }

  /** 顶栏 WebDAV 按钮长按弹出的「上传书籍」菜单开合：互斥同上；打开时定位到按钮正下方。 */
  function toggleWdMenu() {
    const willOpen = !wdMenuOpen;
    closeTbPanels();
    if (willOpen) {
      rightPanel = null;
      tocOpen = false;
      // 取被长按的按钮 rect 做定位（不给按钮加包裹层，避免破坏 dragSort 的 parentElement 约束）。
      const btn = document.querySelector<HTMLElement>('.tb-r [data-drag-id="webdav"]');
      if (btn) {
        const r = btn.getBoundingClientRect();
        wdMenuPos = { top: r.bottom + 6, right: Math.max(8, window.innerWidth - r.right) };
      }
      wdMenuOpen = true;
    }
  }

  // 菜单外点关闭：窗口捕获阶段监听 pointerdown，点击菜单之外即关闭（项目既有弹层范式，无需遮罩元素）。
  // 以 wdMenuOpen 为依赖：无关状态变化不会重复绑定；关闭/卸载时移除监听。
  $effect(() => {
    if (!wdMenuOpen) return;
    const h = (e: PointerEvent) => {
      if (wdMenuEl && !wdMenuEl.contains(e.target as Node)) wdMenuOpen = false;
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  // 顶栏按钮长按识别（约 550ms）：仅长按已触发的按钮跳过其短按动作，与 Library 刷新按钮同款模式。
  let tbLpTimer: ReturnType<typeof setTimeout> | undefined;
  let tbLpFired = false;
  function tbPointerDown(b: TbBtn, e: PointerEvent) {
    if (!b.longPress || e.button !== 0) return;
    tbLpFired = false;
    if (tbLpTimer !== undefined) clearTimeout(tbLpTimer);
    tbLpTimer = setTimeout(() => {
      tbLpFired = true;
      b.longPress!();
    }, 550);
  }
  function tbPointerUp() {
    if (tbLpTimer !== undefined) {
      clearTimeout(tbLpTimer);
      tbLpTimer = undefined;
    }
  }
  function tbClick(b: TbBtn) {
    if (tbLpFired) {
      tbLpFired = false; // 长按已触发菜单，不再执行短按动作
      return;
    }
    b.run();
  }

  /** 目录开合：打开目录时关闭顶栏浮层面板，互斥。 */
  function toggleToc() {
    const willOpen = !tocOpen;
    closeTbPanels();
    tocOpen = willOpen;
  }

  /** AI 抽屉/独立窗切换：drawer 模式并入右侧互斥（rightPanel==='ai'）；
   *  window 模式交由 store.toggleAi() 管理独立窗，建窗失败(返回 false)时直接 setRight('ai') 回退抽屉，
   *  不经 ui.aiOpen 中转、不靠 effect 拉起。 */
  async function toggleAiPanel() {
    // 互斥：打开 AI 时关闭顶栏浮层面板。
    closeTbPanels();
    let mode: string | null = null;
    try { mode = localStorage.getItem("fr.ai.mode"); } catch { /* ignore */ }
    if (mode === "window") {
      const shownAsWindow = await toggleAi();
      if (!shownAsWindow) setRight("ai");            // 建窗失败 → 直接回退抽屉
      else if (rightPanel === "ai") rightPanel = null; // 独立窗呈现时确保抽屉不并存
      return;
    }
    setRight("ai");
  }

  /** 任务 5：把右侧/目录面板拉出为独立窗口，并从互斥组回收（tocOpen/rightPanel 置空）。 */
  async function popOutPanel(kind: "toc" | "bookmarks" | "highlights" | "stats") {
    if (!book) return;
    const ok = await openPanelWindow(kind, book.book.id, book.book.title || "TidePage");
    if (!ok) { toast("弹出窗口失败", "error"); return; }
    // 互斥回收：拉出为独立窗后从右侧互斥/目录移除，互斥逻辑不再管它
    if (kind === "toc") tocOpen = false;
    else rightPanel = null;
  }

  // ② 独立划线窗实时同步：本窗划线集变化时广播，独立窗订阅后重载
  $effect(() => {
    void hlVersion;
    void emit("tidepage-hl-changed", {});
  });

  /* ================= F11 全屏切换（任务 8） =================
   * keydown 命中 F11 先 preventDefault（挡掉 WebView2 自带全屏），再切 Tauri 窗口全屏。
   * isFullscreen 跟随 setFullscreen 后的实际结果，连按不错位。 */
  let isFullscreen = $state(false);
  let showFsHint = $state(false);
  let fsHintTimer: ReturnType<typeof setTimeout> | undefined;

  async function toggleFullscreen() {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const next = !isFullscreen;
      await win.setFullscreen(next);
      // 跟随实际切换结果（避免连按状态错位）
      let actual = next;
      try { actual = await win.isFullscreen(); } catch { /* 读取失败则用目标值兜底 */ }
      isFullscreen = actual;
      clearTimeout(fsHintTimer);
      if (isFullscreen) {
        // 进入全屏：淡入大字号浮层提示，2.2s 后自动淡出（不长期遮挡正文）
        showFsHint = true;
        fsHintTimer = setTimeout(() => { showFsHint = false; }, 2200);
      } else {
        // 退出全屏：提示立即淡出
        showFsHint = false;
      }
    } catch (e) {
      toast(`全屏切换失败：${e}`, "error");
    }
  }


  // 搜索结果跳转历史：可逐步/一键返回跳转前的位置
  type NavPos = { chapter_index: number; ratio: number };
  let searchNavStack: NavPos[] = $state([]);
  let searchOrigin: NavPos | null = null;

  // 朗读（TTS）
  let ttsCurrent: FlatSegment | null = $state(null);
  let ttsStartChoice: { anchor: PopupAnchor } | null = $state(null);
  /** 本章可朗读文本总长 / 当前朗读到的比例（用于精确保存朗读进度） */
  let ttsTextLen = 0;
  let ttsRatio = 0;
  let ttsSavedAt = 0;

  let statsTimer: ReturnType<typeof setInterval> | undefined;
  let progressSaveTimer: ReturnType<typeof setTimeout> | undefined;
  // 用户是否已在本次打开中「真实翻页/滚动」过：仅用户主动移动时置真，
  // 用于在保存真实进度时释放手动状态覆盖（read_state!==0）。初次打开的定位保存不置真，
  // 避免仅「打开」就把手动「未读/已读」覆盖误清除。
  let progressUserMoved = false;
  let measureRaf = 0;
  let openError = $state("");
  let handledNonce = 0;

  let sessionId = 0;
  let lastPos = 0;
  let lastFlushTs = 0;
  // 最近一次用户活动时间戳（挂机剔除用）：pointermove/pointerdown/keydown/wheel/scroll/touchstart 触发时刷新。
  let lastActivityTs = Date.now();
  // 计时是否暂停（窗口隐藏时停表）：tick 与 flushSession 据此跳过累加。
  let statsPaused = false;

  // ===== 章节缓存（LRU）+ 预取 + 竞态控制 =====
  // 目标：翻多快都即时显示——命中缓存零等待；未命中时先显示最接近的已缓存章节，
  // 后台补齐后无缝替换；重复请求复用同一次 in-flight，不产生重复 IPC。
  const CHAPTER_CACHE_MAX = 64;
  const chapterCache = new Map<number, ChapterData>();
  const inflight = new Map<number, Promise<ChapterData>>();
  /** 加载代号：快速翻页时递增，旧请求结果不得覆盖新页面。 */
  let loadGeneration = 0;
  /** 字体是否已确保加载过（只等一次，之后不再每章 await）。 */
  let fontsEnsured = false;

  /** 记录缓存访问顺序，超过上限按 LRU 淘汰（保留当前章不淘汰）。 */
  function cacheGet(idx: number): ChapterData | undefined {
    const v = chapterCache.get(idx);
    if (v) {
      chapterCache.delete(idx);
      chapterCache.set(idx, v);
    }
    return v;
  }

  function cachePut(idx: number, ch: ChapterData) {
    chapterCache.delete(idx);
    chapterCache.set(idx, ch);
    while (chapterCache.size > CHAPTER_CACHE_MAX) {
      const oldest = chapterCache.keys().next().value;
      if (oldest === undefined || oldest === idx) break;
      chapterCache.delete(oldest);
    }
  }

  /** 取章节：命中缓存同步返回；否则发起（去重）请求并缓存结果。 */
  function fetchChapter(idx: number): Promise<ChapterData> {
    const hit = cacheGet(idx);
    if (hit) return Promise.resolve(hit);
    const pending = inflight.get(idx);
    if (pending) return pending;
    const p = api
      .getChapter(book!.book.id, idx, !effOverrideCss())
      .then((c) => {
        cachePut(idx, c);
        return c;
      })
      .finally(() => {
        inflight.delete(idx);
      });
    inflight.set(idx, p);
    return p;
  }

  /** 预取窗口：以当前章为中心，前后若干章；受限并发避免打爆 IPC/内存。 */
  const PREFETCH_INFLIGHT_MAX = 3;
  let prefetchActive = 0;
  let prefetchQueue: number[] = [];
  let prefetchScheduled = false;

  function prefetchAround(center: number) {
    if (!book) return;
    const total = book.toc.length;
    // 大书窗口更大（来回翻页/跳读更顺），但仍设上限，避免内存/IO 过载。
    const span = total > 400 ? 12 : total > 120 ? 10 : 6;
    const list: number[] = [];
    for (let d = 1; d <= span; d++) {
      if (center + d < total) list.push(center + d);
      if (center - d >= 0) list.push(center - d);
    }
    for (const n of list) {
      if (!chapterCache.has(n) && !inflight.has(n) && !prefetchQueue.includes(n)) {
        prefetchQueue.push(n);
      }
    }
    schedulePrefetch();
  }

  function schedulePrefetch() {
    if (prefetchScheduled) return;
    prefetchScheduled = true;
    queueMicrotask(() => {
      prefetchScheduled = false;
      pumpPrefetch();
    });
  }

  function pumpPrefetch() {
    while (prefetchActive < PREFETCH_INFLIGHT_MAX && prefetchQueue.length) {
      const n = prefetchQueue.shift()!;
      if (!book || n < 0 || n >= book.toc.length) continue;
      if (chapterCache.has(n) || inflight.has(n)) continue;
      prefetchActive++;
      api
        .getChapter(book.book.id, n, !effOverrideCss())
        .then((c) => cachePut(n, c))
        .catch(() => {})
        .finally(() => {
          prefetchActive--;
          if (prefetchQueue.length) schedulePrefetch();
        });
    }
  }


  $effect(() => {
    if (pendingOpen.nonce > handledNonce) {
      handledNonce = pendingOpen.nonce;
      open(pendingOpen.bookId);
    }
  });

  export async function open(bookId: number) {
    openError = "";
    try {
      chapterCache.clear();
      inflight.clear();
      prefetchQueue = [];
      prefetchActive = 0;
      loadGeneration++;
      mangaWindow = [];
      lastMangaAnchorTop = -1;
      // 换书：重置漫画模式/布局追踪，避免切书时误触发多余的窗口重建（首章加载由 applyChapter 负责）。
      lastMangaOn = null;
      lastMangaLayout = null;
      bookImages = null;
      viewerIndex = null;
      progressUserMoved = false; // 新书打开：清空「用户已移动」标记（打开本身不释放手动状态覆盖）
      book = await api.openBook(bookId);
      ui.currentBookId = book.book.id;
      if (book.encoding_warning) console.log(book.encoding_warning);
      // 漫画书首次引导：全局只提示一次（写入 config.reader.mangaGuideDone），
      // 之后打开任何漫画都不再弹出；拖动时实时预览（直接改全局 manga.imgScale）。
      blockPickMode = false;
      imgMenuOpen = false;
      tbManagerOpen = false;
      blockedListOpen = false;
      if (book.is_manga && !config.reader.mangaGuideDone) {
        mangaGuideScale = config.reader.manga.imgScale;
        mangaGuideOpen = true;
      } else {
        mangaGuideOpen = false;
      }
      let restore: Progress | null = null;
      try {
        restore = await api.loadProgress(bookId);
      } catch { /* ignore */ }
      chapterNum = restore && restore.chapter_index < book.toc.length ? restore.chapter_index : 0;
      await loadChapter(chapterNum, restore ?? undefined);
      refreshHighlights();
      {
        const openedBookId = book.book.id;
        // 延后到首屏稳定后再建索引：避免与首章渲染/预取争 CPU 与 DB 锁。
        // index_book_fts 内部一次遍历同时写 fts 与 chapter_text；仅当无 FTS 能力（返回 0）时兜底补 chapter_text。
        const runIndex = () => {
          api
            .indexBookFts(openedBookId)
            .then((n) => { if (n === 0) api.ensureBookText(openedBookId).catch(() => {}); })
            .catch(() => {});
        };
        if (typeof requestIdleCallback === "function") requestIdleCallback(runIndex, { timeout: 4000 });
        else setTimeout(runIndex, 1500);
      }
      if (pendingHighlightJump && pendingHighlightJump.book_id === book.book.id) {
        const ph = pendingHighlightJump;
        pendingHighlightJump = null;
        jumpToHighlight(ph);
      }
      if (statsTimer) clearInterval(statsTimer);
      const bid = bookId;
      try {
        sessionId = await api.startSession(bid);
        lastPos = bookPos();
        lastFlushTs = Date.now();
        lastActivityTs = Date.now();
        statsPaused = document.visibilityState !== "visible";
      } catch { sessionId = 0; }
      statsTimer = setInterval(() => {
        // 隐藏时已在 visibilitychange 结算并停表；这里再兜底门控一次。
        if (!statsPaused && document.visibilityState === "visible") flushSession();
      }, 30_000);
      if (config.webdav.enabled && config.webdav.syncOnOpen) {
        webdavSync(restore)
          .then((restored) => { if (restored) toast("已从 WebDAV 恢复阅读进度"); })
          .catch(() => {});
      }
    } catch (e) {
      const s = String(e);
      const friendly = s.includes("AWZ3EncryptedError")
        ? "检测到 DRM 保护，无法直接阅读。请移除 DRM 后重试。"
        : s.includes("ParseTimeoutOrCorrupted")
          ? "文件畸形或过大，解析已中止（防卡死保护）。"
          : s;
      openError = friendly;
      toast(friendly, "error");
    }
  }

  export async function close() {
    stopTts();
    webdavPush();
    await doSaveProgress();
    await flushSession(true);
    sessionId = 0;
    if (statsTimer) clearInterval(statsTimer);
    scroller?.destroy();
    book = null;
    chapter = null;
    mangaWindow = [];
    ui.currentBookId = 0;
  }

  // 退出书籍时把本地进度推送远端（同一软件多端同步，新的覆盖旧的）
  async function webdavPush() {
    if (!book || !chapter || !config.webdav.enabled || !config.webdav.syncOnOpen) return;
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) return; // 配置不完整：视为不执行同步，静默跳过
    const bid = book.book.id;
    const bpath = book.book.path;
    const p: Progress = {
      chapter_index: chapterNum,
      chapter_ratio: currentRatio(),
      pct: totalPct(),
      updated_at: Date.now(),
      device: "pc",
    };
    try {
      await api.webdavSyncProgress(w.base, w.user, w.pass, bid, bpath, p);
      toast("退出时已同步进度到云端");
    } catch (e) {
      toast(`退出时同步进度失败：${e}`, "error");
    }
  }

  async function webdavSync(localP: Progress | null): Promise<boolean> {
    if (!book) return false;
    const w = config.webdav;
    const remote = await api.webdavSyncProgress(
      w.base, w.user, w.pass, book.book.id, book.book.path, localP,
    );
    let restored = false;
    if (remote.chapter_index !== chapterNum && remote.chapter_index < book.toc.length) {
      await loadChapter(remote.chapter_index, remote);
      restored = true;
    }
    if (config.webdav.uploadBooks) {
      api.webdavUploadBook(w.base, w.user, w.pass, book.book.path).catch(() => {});
    }
    return restored;
  }

  /** 当前内存中的本地进度快照（用于同步/对比）。 */
  function localProgress(): Progress {
    return {
      chapter_index: chapterNum,
      chapter_ratio: currentRatio(),
      pct: totalPct(),
      updated_at: Date.now(),
      device: "pc",
    };
  }

  /** 「上传进度」：直接把当前本地进度上传云端（本地 updated_at=now 必为较新，等价于直接上传）。 */
  async function uploadProgress() {
    if (!book) return;
    if (!config.webdav.enabled) { toast("未启用 WebDAV", "error"); return; }
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) { toast("请先在 设置→WebDAV 填写服务器与账号", "error"); return; }
    try {
      await api.webdavSyncProgress(w.base, w.user, w.pass, book.book.id, book.book.path, localProgress());
      toast("进度已上传");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  /** 「下载进度」：要先弹确认框（会改动本地），再从云端更新本地进度。 */
  async function webdavDownload() {
    if (!book) return;
    if (!config.webdav.enabled) { toast("未启用 WebDAV", "error"); return; }
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) { toast("请先在 设置→WebDAV 填写服务器与账号", "error"); return; }
    let remote: Progress | null = null;
    try {
      remote = await api.webdavReadProgress(w.base, w.user, w.pass, book.book.path);
    } catch (e) {
      toast(String(e), "error");
      return;
    }
    if (!remote) { toast("云端暂无进度"); return; }
    const ok = await appConfirm(
      `云端进度：第 ${remote.chapter_index + 1} 章 ${remote.pct.toFixed(0)}%（${fmtProgressTime(remote.updated_at)}）\n本地进度：第 ${chapterNum + 1} 章 ${totalPct().toFixed(0)}%（${fmtProgressTime(Date.now())}）\n\n确定用云端进度更新本地吗？`,
    );
    if (!ok) return;
    try {
      if (remote.chapter_index < book.toc.length) {
        await loadChapter(remote.chapter_index, remote);
      }
      toast("已从云端更新进度");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  /** 长按菜单：上传当前书籍文件到云端（手动，不受 uploadBooks 自动开关影响）。 */
  async function uploadBook() {
    wdMenuOpen = false;
    if (!book) return;
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) { toast("请先在 设置→WebDAV 填写服务器与账号", "error"); return; }
    try {
      await api.webdavUploadBook(w.base, w.user, w.pass, book.book.path);
      toast("已上传书籍到云端");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function fmtProgressTime(ms: number): string {
    if (!ms) return "无";
    const d = new Date(ms);
    const p = (x: number) => String(x).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  function aiCtxHtml(h: string): string {
    const s = stripNonTextHtml(h);
    return s.length > 400_000 ? s.slice(0, 400_000) : s;
  }

  async function loadChapter(
    idx: number,
    restore?: Progress,
    entry?: { at: "head" | "tail"; extra?: number }
  ) {
    if (!book) return;
    if (ttsState.playing) stopTts(); // 手动切换章节时停止朗读（自动续读在 onTtsChapterEnd 中处理）
    // 竞态：快速连续翻页时，只有最后一次请求的结果才允许落地。
    const gen = ++loadGeneration;

    const cached = cacheGet(idx);
    // 命中缓存：立即同步切换，不显示任何 loading。
    if (cached) {
      loadingChapter = false;
      await applyChapter(idx, cached, restore, entry, gen);
      prefetchAround(idx);
      return;
    }

    // 未命中但邻近章已缓存：先立即显示最接近的已缓存内容，避免整页空白，
    // 后台补齐后再无缝替换（可用时）。
    const nearest = nearestCached(idx);
    if (nearest !== null && nearest !== idx) {
      const near = chapterCache.get(nearest)!;
      await applyChapter(idx, near, restore, entry, gen, true);
    }
    // 请求在途期间保持 loadingChapter=true：handleScrollEdge 因此不再继续跨章，
    // 避免连续滚动时不断 ++loadGeneration，把已返回的章节结果全部当作过期丢弃
    // （表现为滚到底后卡住、不再显示后续章节）。占位内容仍照常显示，不空白。
    loadingChapter = true;

    try {
      const ch = await fetchChapter(idx);
      if (gen !== loadGeneration) return; // 已被更新的翻页取代
      loadingChapter = false;
      await applyChapter(idx, ch, restore, entry, gen);
      prefetchAround(idx);
    } catch (e) {
      if (gen === loadGeneration) {
        loadingChapter = false;
        toast(String(e), "error");
      }
    }
  }

  /** 找距离 idx 最近的已缓存章节（用于未命中时的即时占位，避免空白）。 */
  function nearestCached(idx: number): number | null {
    if (!book) return null;
    for (let d = 1; d <= 12; d++) {
      const a = idx - d;
      const b = idx + d;
      if (a >= 0 && chapterCache.has(a)) return a;
      if (b < book.toc.length && chapterCache.has(b)) return b;
    }
    return null;
  }

  /** 把章节内容应用为当前页（含滚动/翻页定位）。placeholder=true 表示临时占位内容。 */
  async function applyChapter(
    idx: number,
    ch: ChapterData,
    restore: Progress | undefined,
    entry: { at: "head" | "tail"; extra?: number } | undefined,
    gen: number,
    placeholder = false,
  ) {
    // 字体就绪只在首次等待（之后直接跳过，避免每章额外延迟）。
    if (!fontsEnsured) {
      fontsEnsured = true;
      await preloadReaderFonts();
    }
    if (gen !== loadGeneration && !placeholder) return;
    chapter = ch;
    chapterNum = idx;
    // 漫画（横向 / 瀑布流）：重建跨章连续窗口，跳过普通单章定位逻辑。
    if (isMangaMode()) {
      await loadMangaWindow(idx, restore);
      return;
    }
    await tick();
    if (!fontsEnsured) return;
    fontsReady();
    scroller?.reset();
    resetScroll();
    // 横向布局优先；否则按 scrollLayout()（普通滚动模式）走纵向滚动。
    if (scrollLayout()) {
      measureScroll();
      let pos = 0;
      if (entry) pos = entry.at === "tail" ? maxScroll - (entry.extra || 0) : entry.extra || 0;
      else if (restore) pos = restore.chapter_ratio * maxScroll;
      pos = Math.max(0, Math.min(pos, maxScroll));
      scroller?.jumpTo(pos, true);
      if (restore && restore.chapter_ratio > 0) {
        setTimeout(() => {
          if (gen !== loadGeneration) return;
          measureScroll();
          const p2 = restore.chapter_ratio * maxScroll;
          scroller?.jumpTo(Math.max(0, Math.min(p2, maxScroll)), true);
        }, 300);
      }
      if (entry?.at === "tail") {
        requestAnimationFrame(() => pinToEnd());
      }
    } else {
      measurePages();
      let pg = restore ? Math.round(restore.chapter_ratio * Math.max(1, pageCount - 1)) : 0;
      if (entry) pg = entry.at === "tail" ? Math.max(0, pageCount - 1) : 0;
      pendingPageRatio = restore && !entry ? restore.chapter_ratio : null;
      gotoPage(Math.max(0, Math.min(pg, Math.max(0, pageCount - 1))), true);
      setTimeout(() => {
        if (gen === loadGeneration) recheckPages();
      }, 350);
      if (entry?.at === "tail") {
        requestAnimationFrame(() => pinToEnd());
      }
    }
    if (placeholder) return;
    // AI 上下文写入移出换章热路径：仅在启用 AI 时写，且延后到空闲（html 上限 40 万字符，
    // 同步 JSON.stringify + aiCtxHtml 会阻塞主线程导致换章顿挫）。
    if (config.ai.enabled) {
      const ctxTitle = `${book?.book.title ?? ""} · ${ch.title}`;
      const ctxHtml = ch.html;
      const writeCtx = () => {
        try {
          localStorage.setItem("fr.ai.ctx", JSON.stringify({ title: ctxTitle, html: aiCtxHtml(ctxHtml) }));
        } catch { /* ignore */ }
      };
      if (typeof requestIdleCallback === "function") requestIdleCallback(writeCtx, { timeout: 2000 });
      else setTimeout(writeCtx, 0);
    }
    maybeAutoAi();
    saveProgressDebounced(false); // 章节定位/初次打开：保存进度但不算「用户移动」，不释放手动状态覆盖
  }

  async function fontsReady() {
    try {
      await Promise.race([
        document.fonts.ready,
        new Promise((r) => setTimeout(r, 500)),
      ]);
    } catch { /* ignore */ }
  }

  function recheckPages() {
    if (!pageWrap || scrollLayout()) return;
    const before = pageCount;
    computePageCount();
    if (pageCount !== before) {
      // 字体/图片等延迟加载后页数变化：若在恢复进度，按原比例重新落到正确页，
      // 否则保持当前页（避免恢复章节时被夹到第 0 页）。
      if (pendingPageRatio != null) {
        const target = Math.round(pendingPageRatio * Math.max(1, pageCount - 1));
        gotoPage(Math.max(0, Math.min(target, pageCount - 1)), true);
      } else {
        applyPage();
      }
    }
    pendingPageRatio = null;
  }

  function nextChapter(fromEnd = false) {
    if (book && chapterNum < book.toc.length - 1) loadChapter(chapterNum + 1);
    else if (fromEnd) toast("已是最后一章");
  }

  function prevChapter(toEnd = false) {
    console.log("Prev Chapter Triggered, toEnd:", toEnd, "chapterNum:", chapterNum);
    if (book && chapterNum > 0) {
      // 回溯语义统一为"接着看上一章的结尾"（翻页与显式上一章均落章末）
      loadChapter(chapterNum - 1, undefined, toEnd ? { at: "tail", extra: 0 } : undefined);
    }
  }

  function maybeAutoAi() {
    if (config.ai.enabled && config.ai.autoOnChapter && chapter) {
      const ch = { title: chapter.title, html: chapter.html };
      window.dispatchEvent(
        new CustomEvent("tidepage-ai-chapter", {
          detail: { title: ch.title, html: aiCtxHtml(ch.html) },
        }),
      );
      // 跨窗口广播：AI 独立窗口也能收到
      import("@tauri-apps/api/event")
        .then((m) =>
          m.emit("tidepage-ai-chapter", {
            title: ch.title,
            html: aiCtxHtml(ch.html),
          })
        )
        .catch(() => {});
      try {
        localStorage.setItem(
          "fr.ai.ctx",
          JSON.stringify({
            title: `${book?.book.title ?? ""} · ${ch.title}`,
            html: aiCtxHtml(ch.html),
          }),
        );
      } catch { /* ignore */ }
    }
  }

  function saveProgressDebounced(userMoved = true) {
    // 除「初次打开/换章定位」外的调用都视为用户真实移动，记入标记以便释放手动状态覆盖。
    if (userMoved) progressUserMoved = true;
    clearTimeout(progressSaveTimer);
    progressSaveTimer = setTimeout(doSaveProgress, 1200);
  }

  function currentRatio(): number {
    // 漫画横向：按「当前章块内部」计算（而非整条窗口），保证恢复/进度对应到具体某章某位置。
    if (mangaHorizontal()) {
      const el = mangaCanvas;
      const block = el?.querySelector<HTMLElement>(`.manga-block[data-index="${chapterNum}"]`);
      if (el && block && block.offsetWidth > 0) {
        const W = el.offsetWidth;
        const semStart = mangaRtl() ? W - (block.offsetLeft + block.offsetWidth) : block.offsetLeft;
        return Math.max(0, Math.min(1, (mangaPosX - semStart) / block.offsetWidth));
      }
      return mangaMaxX > 0 ? Math.min(1, mangaPosX / mangaMaxX) : 0;
    }
    // 漫画瀑布流：进度/比例按「当前章块内部」计算（而非整条窗口），保证跨会话恢复落回同一章同一位置。
    if (scrollLayout() && isMangaMode()) {
      const vp = mangaFlowViewport;
      const block = vp?.querySelector<HTMLElement>(`.manga-block[data-index="${chapterNum}"]`);
      if (block && block.offsetHeight > 0) {
        const within = ((vp?.scrollTop ?? 0) - block.offsetTop) / block.offsetHeight;
        return Math.max(0, Math.min(1, within));
      }
      return 0;
    }
    if (scrollLayout()) return maxScroll > 0 ? Math.min(1, scrollTop / maxScroll) : 0;
    return pageCount > 1 ? Math.min(1, page / (pageCount - 1)) : 0;
  }

  function totalPct(ratio = currentRatio()): number {
    if (!book) return 0;
    const n = book.toc.length || 1;
    return Math.min(100, ((chapterNum + ratio) / n) * 100);
  }

  async function doSaveProgress(ratioOverride?: number) {
    if (!book) return;
    const ratio = Math.max(0, Math.min(1, ratioOverride ?? currentRatio()));
    const p: Progress = {
      chapter_index: chapterNum,
      chapter_ratio: ratio,
      pct: totalPct(ratio),
      updated_at: Date.now(),
      device: "pc",
    };
    try {
      await api.saveProgress(book.book.id, p);
      // 用户产生真实阅读进度后，若该书存在手动状态覆盖（已读/未读），释放为「自动」，
      // 使有效状态回归由真实进度推导（不再需要手动「恢复自动」）。
      if (progressUserMoved && book.book.read_state !== 0) {
        book.book.read_state = 0;
        try {
          await api.setReadState(book.book.id, "auto");
        } catch { /* ignore */ }
      }
    } catch { /* ignore */ }
  }

  function bookPos(): number {
    if (!book) return 0;
    let before = 0;
    for (let i = 0; i < chapterNum && i < book.toc.length; i++) before += book.toc[i].chars || 0;
    const cur = book.toc[chapterNum]?.chars || 0;
    return before + cur * currentRatio();
  }

  // 跳转类操作完成后对齐统计基准：把 lastPos 设为当前位置，使被跳过的整段字数不进入下次 flush。
  // 仅用于目录/进度条/书签/划线/搜索等显式跳转；顺序翻章(next/prevChapter)是连续阅读，不调用此函数。
  function alignStatsBaseline() {
    lastPos = bookPos();
  }

  async function flushSession(force = false) {
    if (!book || !sessionId) return;
    const now = Date.now();
    const elapsed = (now - lastFlushTs) / 1000;
    if (!force && elapsed < 15) return;
    // 挂机剔除：距上次用户活动超过 IDLE_LIMIT(120s) 视为放置不读，
    // 只对齐基准、跳过本次累加；force 结算路径（关闭/隐藏）不受此拦截，保证最后有效片段落库。
    const IDLE_LIMIT = 120_000;
    if (!force && now - lastActivityTs > IDLE_LIMIT) {
      lastFlushTs = now;
      lastPos = bookPos();
      return;
    }
    const pos = bookPos();
    const dCharsRaw = Math.max(0, Math.round(pos - lastPos));
    // 跳读/跳转灌字保护：按本次时长换算，若字数增量超过人类阅读速度上限(MAX_CPM 字/分)，
    // 判为目录/进度条/搜索等大跳，本次不计字数（时长仍照常计）。连续翻章增量小、不受影响。
    const MAX_CPM = 3000;
    const minutes = Math.max(elapsed, 1) / 60;
    const dChars = dCharsRaw <= MAX_CPM * minutes ? dCharsRaw : 0;
    lastPos = pos;
    lastFlushTs = now;
    const secs = Math.min(Math.round(elapsed), 600);
    if (secs < 3 && dChars === 0) return;
    try {
      await api.updateSession(sessionId, secs, dChars);
      // 广播给独立“统计”窗口（按 bookId 过滤后重新拉取时长/字数）
      void emit("tidepage-stats-changed", { bookId: book.book.id });
    } catch { /* ignore */ }
  }

  // 活动检测：任何用户交互都刷新 lastActivityTs，供 flushSession 判定挂机。
  // 与「隐藏 UI」的 idleTimer 相互独立，互不影响。
  $effect(() => {
    const bump = () => { lastActivityTs = Date.now(); };
    const evs: (keyof WindowEventMap)[] = [
      "pointermove", "pointerdown", "keydown", "wheel", "scroll", "touchstart",
    ];
    for (const ev of evs) window.addEventListener(ev, bump, { passive: true });
    return () => {
      for (const ev of evs) window.removeEventListener(ev, bump);
    };
  });

  // 窗口隐藏/显示：隐藏时立即结算并停表（隐藏期一律不计）；恢复时重置基准并复表，
  // 消除「恢复瞬间 elapsed 一大段被 600s 截断误计」。用 $effect 管理监听，卸载时清理。
  $effect(() => {
    const onVis = () => {
      if (document.visibilityState === "hidden") {
        void flushSession(true); // 结算最后有效片段
        statsPaused = true;       // 停表
      } else {
        lastFlushTs = Date.now();     // 从当前时刻重新计
        lastActivityTs = Date.now();
        statsPaused = false;          // 复表
      }
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  });

  /* ================= scroll mode ================= */
  let lastEdgeAt = 0;

  function measureScroll() {
    if (!contentEl || !scrollArea) return;
    // 滚动模式采用原生滚动容器（.scroll-area 程序化 scrollTop）：正文由浏览器分块绘制，
    // 不再把超长正文整体提升为合成层用 transform 平移，从根本上避免超大图层撕裂/空白。
    maxScroll = Math.max(0, scrollArea.scrollHeight - scrollArea.clientHeight);
    // 图片最大高度=可视区高，保证滚动模式下大图也完整适配一屏，不撑出超长内容。
    const h = scrollArea.clientHeight;
    if (h > 0) imgMaxH = h;
    scroller?.setMax(maxScroll);
  }

  /// 递归 rAF 钉到章末：直到（图片等延迟加载后）滚动位置真正落底/页数稳定才停止。
  /// scroll：scrollTop + clientHeight >= scrollHeight - 5；page：钉住最后一页且 pageCount 不再变。
  function pinToEnd(r = 0) {
    if (r > 180) return;
    let done = false;
    if (scrollLayout()) {
      measureScroll();
      scroller?.jumpTo(maxScroll, true);
      done =
        !contentEl ||
        !scrollArea ||
        maxScroll - scrollRender <= 5 ||
        scrollRender + scrollArea.clientHeight >= scrollArea.scrollHeight - 5;
    } else {
      const before = pageCount;
      recheckPages();
      gotoPage(Number.MAX_SAFE_INTEGER, true);
      done = before === pageCount && (pageCount <= 1 || page >= pageCount - 1);
    }
    if (done) return;
    requestAnimationFrame(() => pinToEnd(r + 1));
  }

  /** 每帧更新的纯逻辑滚动位置（非响应式，供 pinToEnd/保存进度即时读取）。可短暂越界（回弹）。 */
  let scrollRender = 0;
  /** 上一次写入 .scroll-area 的回弹 transform，避免每帧重复写。 */
  let lastBounce = "";
  /** 响应式 UI（进度百分比等）上次同步时间，节流到 ~150ms。 */
  let lastUiSync = 0;

  function applyScroll(renderPos: number) {
    scrollRender = renderPos;
    const area = scrollArea;
    // 主体位置走原生滚动（scrollTop）：浏览器按需分块绘制超长正文，无巨型合成层、无撕裂/空白。
    const clamped = renderPos < 0 ? 0 : renderPos > maxScroll ? maxScroll : renderPos;
    const p = Math.round(clamped);
    if (area) {
      if (area.scrollTop !== p) area.scrollTop = p;
      // 越界部分（回弹）只在视口大小的 .scroll-area 上做微小 transform，图层恒为一屏，绝不超纹理上限。
      // 仅在值真正变化时写 style：避免每帧重复写 transform 触发样式失效/无谓合成。
      const over = renderPos - clamped; // <0 顶部回弹 / >0 底部回弹
      const tf = over === 0 ? "" : `translateY(${(-over).toFixed(2)}px)`;
      if (tf !== lastBounce) {
        lastBounce = tf;
        area.style.transform = tf;
      }
    }
    if (thumbEl) {
      const trackH = (scrollArea?.clientHeight || 0) - 28;
      const ratio = maxScroll > 0 ? Math.max(0, Math.min(1, p / maxScroll)) : 0;
      if (trackH > 0) thumbEl.style.transform = `translate3d(0,${(ratio * trackH * 0.92).toFixed(1)}px,0)`;
    }
    // 响应式 UI 节流更新；滚动 settle 时会精确同步一次。
    const now = performance.now();
    if (now - lastUiSync > 150) {
      lastUiSync = now;
      if (scrollTop !== p) scrollTop = p;
    }
  }

  /* ===== 自定义滚动条拖动 ===== */
  let sbDragPid = -1;
  let sbDragging = $state(false);
  // 拖动期间缓存的轨道几何：避免每次 pointermove 都读 getBoundingClientRect（写 scrollTop 后再读会触发布局抖动）。
  let sbTop = 0;
  let sbH = 0;

  /** 按轨道内 Y 比例即时跳到目标位置（点击轨道 / 拖动 thumb 共用）。 */
  function sbSeek(clientY: number) {
    if (maxScroll <= 0 || sbH <= 0) return;
    const ratio = (clientY - sbTop) / sbH;
    const pos = Math.max(0, Math.min(1, ratio)) * maxScroll;
    scroller?.jumpTo(pos, true);
    applyScroll(pos); // 立即同步 thumb 位置
  }

  function onSbPointerDown(e: PointerEvent) {
    // 先阻止冒泡：否则滚动条上的按下会冒泡到 .content 触发翻章/区域动作（内容不满屏、maxScroll=0 时也不能例外）。
    e.stopPropagation();
    e.preventDefault();
    if (e.button !== 0 || maxScroll <= 0) return;
    const el = scrollbarEl;
    const r = el?.getBoundingClientRect();
    if (!el || !r || r.height <= 0) return;
    sbTop = r.top;
    sbH = r.height;
    el.setPointerCapture?.(e.pointerId);
    sbDragPid = e.pointerId;
    sbDragging = true;
    sbSeek(e.clientY);
  }

  function onSbPointerMove(e: PointerEvent) {
    if (e.pointerId !== sbDragPid) return;
    e.stopPropagation();
    sbSeek(e.clientY);
  }

  function onSbPointerUp(e: PointerEvent) {
    if (e.pointerId !== sbDragPid) return;
    e.stopPropagation();
    sbDragPid = -1;
    sbDragging = false;
    if (scrollbarEl?.hasPointerCapture?.(e.pointerId)) scrollbarEl.releasePointerCapture(e.pointerId);
    saveProgressDebounced();
  }

  function onSbPointerCancel(e: PointerEvent) {
    if (e.pointerId !== sbDragPid) return;
    sbDragPid = -1;
    sbDragging = false;
    if (scrollbarEl?.hasPointerCapture?.(e.pointerId)) scrollbarEl.releasePointerCapture(e.pointerId);
  }

  /** 章节切换时把滚动容器复位到顶部并清除回弹位移。 */
  function resetScroll() {
    if (scrollArea) {
      scrollArea.scrollTop = 0;
      scrollArea.style.transform = "";
    }
    lastBounce = "";
    if (thumbEl) thumbEl.style.transform = "";
    // 漫画双页：复位横向/纵向平移
    mangaPosX = 0;
    mangaPosY = 0;
    if (mangaCanvas) mangaCanvas.style.transform = "translate3d(0,0,0)";
  }

  function handleScrollEdge(dir: number, carry: number) {
    if (!book) return;
    // 漫画瀑布流：连续窗口在两端自动续章，不重建窗口（保持视觉连续）。
    if (isMangaMode()) {
      if (dir > 0) void extendMangaWindow(1);
      else void extendMangaWindow(-1);
      return;
    }
    const now = Date.now();
    if (now - lastEdgeAt < 320 || loadingChapter) return;
    if (dir > 0) {
      if (chapterNum >= book.toc.length - 1) return;
      lastEdgeAt = now;
      loadChapter(chapterNum + 1, undefined, { at: "head", extra: config.reader.scrollBounce ? 0 : carry });
    } else {
      if (chapterNum <= 0) return;
      lastEdgeAt = now;
      loadChapter(chapterNum - 1, undefined, { at: "tail", extra: config.reader.scrollBounce ? 0 : carry });
    }
  }

  /* ================= page mode ================= */
  let screenCols = 1;
  /** 待恢复的阅读比例：页数在首次测量后仍可能变化，用它把目标页重新落位。 */
  let pendingPageRatio: number | null = null;

  function computePageCount() {
    if (!pageWrap || !(colW > 0)) return;
    const totalCols = Math.max(1, Math.round((pageWrap.scrollWidth + gap) / (colW + gap)));
    pageCount = Math.max(1, Math.ceil(totalCols / screenCols));
    page = Math.min(page, pageCount - 1);
  }

  /** 有效左右边距：漫画模式用独立边距；否则 maxLineChars>0 时忽略 marginH 按「每行 N 字」居中。 */
  function effMarginH(): number {
    if (isMangaMode()) return config.reader.manga.marginH;
    const mlc = config.reader.maxLineChars;
    const w = contentW || contentEl?.clientWidth || 0;
    if (mlc > 0 && w > 0) return Math.max(0, (w - mlc * effFontSize()) / 2);
    return config.reader.marginH;
  }

  /** 有效上下边距：漫画模式用独立边距。 */
  function effMarginV(): number {
    return isMangaMode() ? config.reader.manga.marginV : config.reader.marginV;
  }

  function measurePages() {
    if (!contentEl || !pageWrap || !chapter) return;
    const mh = effMarginH();
    const mv = effMarginV();
    const W = contentEl.clientWidth;
    const H = contentEl.clientHeight;
    // 容器尚未完成布局（宽高为 0）时不要测量：否则会算出 colW=0/pageCount=1，
    // 并把内容按错误步长平移出屏（页模式空白的直接原因）。等下一帧再试。
    if (W <= 0 || H <= 0) {
      cancelAnimationFrame(measureRaf);
      measureRaf = requestAnimationFrame(() => measurePages());
      return;
    }
    // 可用宽度不足时兜底（窄窗口 + 大边距）：保证列宽为正。
    const avail = Math.max(160, W - mh * 2);
    const vh = Math.max(200, H - mv * 2);
    gap = config.reader.colGap;
    screenCols = config.reader.duo ? 2 : 1;
    colW = Math.max(40, Math.floor((avail - (screenCols - 1) * gap) / screenCols));
    const effAvail = screenCols * colW + (screenCols - 1) * gap;
    stepPx = screenCols * (colW + gap);
    // 图片最大高度=单列可视高，保证任何图片都不超过一页/一列（自适应屏幕）。
    imgMaxH = vh;
    const st = pageWrap.style;
    st.left = "0px";
    st.top = `${mv}px`;
    st.width = `${effAvail}px`;
    st.height = `${vh}px`;
    st.columnWidth = `${colW}px`;
    st.columnGap = `${gap}px`;
    cancelAnimationFrame(measureRaf);
    // 先同步算一次页数（读取 scrollWidth 会触发一次布局），保证恢复进度时
    // gotoPage 的夹取使用的是真实页数，而不是上一章的旧值/初始值 1。
    computePageCount();
    applyPage();
    measureRaf = requestAnimationFrame(() => {
      computePageCount();
      applyPage();
    });
  }

  function reducedMotion(): boolean {
    return typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
  }

  /** 页面偏移统一由 transform 表达（GPU 合成）；animate=true 时仅用户主动翻页才启用过渡。 */
  let lastPageX = Number.NaN;
  function applyPage(animate = false) {
    if (!pageWrap) return;
    // 步长非法时不要平移（否则会把内容推出可视区）；等测量完成后再定位。
    const x = stepPx > 0 ? -Math.round(page * stepPx) : 0;
    if (!animate && x === lastPageX) return; // 位置未变：跳过，避免无谓的强制同步布局
    lastPageX = x;
    if (animate && config.reader.anim && !reducedMotion()) {
      pageWrap.style.transform = `translate3d(${x}px,0,0)`;
      return;
    }
    // 立即跳转：临时禁用过渡 → 落位 → 强制 reflow → 恢复常驻 transition，
    // 避免恢复进度/章节切换/relayout 复用旧动画。
    pageWrap.style.transition = "none";
    pageWrap.style.transform = `translate3d(${x}px,0,0)`;
    void pageWrap.offsetWidth;
    pageWrap.style.transition = "";
  }

  function gotoPage(n: number, silent = false) {
    page = Math.max(0, Math.min(n, pageCount - 1));
    applyPage(!silent);
    if (!silent) saveProgressDebounced();
  }

  function nextPage() {
    if (page < pageCount - 1) gotoPage(page + 1);
    else nextChapter(true);
  }

  function prevPage() {
    if (page > 0) gotoPage(page - 1);
    else if (chapterNum > 0) prevChapter(true); // 首页回翻 → 上一章【末尾】，而非章首
  }

  /** 阅读页内快捷字号调整（单本生效，覆盖全局字号）。 */
  function bumpFont(delta: number) {
    const cur = effFontSize();
    const n = Math.max(10, Math.min(64, cur + delta));
    if (n !== cur) setOv({ fontSize: n });
  }

  /** 右下「图−/图＋」调整图片大小。
   *  漫画模式：改为调整全局漫画倍率（0.3~3，步长 0.1），并清除本书 imgScale 覆盖使其跟随全局；
   *  普通模式：保持单本覆盖行为（以有效值为基准，0.1~8，可超过默认）。 */
  function bumpImgScale(delta: number) {
    if (isMangaMode()) {
      const g = config.reader.manga.imgScale;
      const n = Math.max(0.3, Math.min(3, Math.round((g + delta) * 10) / 10));
      if (n !== g) {
        config.reader.manga.imgScale = n;
        saveConfig();
      }
      // 清除本书单本覆盖，使当前书即时跟随新的全局倍率。
      if (book) setBookOverride(book.book.id, { imgScale: undefined });
    } else {
      const cur = effImgScale();
      const n = Math.max(0.1, Math.min(8, Math.round((cur + delta) * 20) / 20));
      if (n !== cur) setOv({ imgScale: n });
    }
  }

  /** 单本：图片适应阅读器宽度开关。 */
  function toggleFitWidth() {
    const on = !effFitWidth();
    setOv({ fitWidth: on, fitHeight: false });
  }

  /** 单本：图片适应阅读器高度开关（与适宽互斥）。 */
  function toggleFitHeight() {
    const on = !effFitHeight();
    setOv({ fitHeight: on, fitWidth: false });
  }

  /* ================= 漫画双页（横向连续阅读） ================= */
  // 视口固定（不滚动整页），内容以 transform 平移；图片按阅读方向横向排列；
  // 拖动平移 vs 点击翻页/打开图库用 6px 手势阈值区分。
  let mangaViewport: HTMLDivElement | undefined = $state();
  let mangaCanvas: HTMLDivElement | undefined = $state();
  let mangaFlowViewport: HTMLDivElement | undefined = $state();
  let mangaFlowCanvas: HTMLDivElement | undefined = $state();
  /** 跨章连续滚动窗口（按序连续章节区间）：漫画两种布局共用。 */
  let mangaWindow = $state<{ index: number; html: string }[]>([]);
  /** 下拉追加/前插的节流时间戳，避免连续滚动时反复触发 IPC。 */
  let mangaExtendLock = 0;
  let mangaPosX = 0; // 横向位置：0=阅读起点（与阅读方向无关的语义坐标）
  let mangaPosY = 0; // 纵向位置（放大后超出屏幕时可上下平移）
  let mangaMaxX = 0;
  let mangaMaxY = 0;
  let mangaPid = -1;
  let mangaPanning = false;
  let mangaStartX = 0;
  let mangaStartY = 0;
  let mangaOriginX = 0;
  let mangaOriginY = 0;
  let mangaDownTarget: EventTarget | null = null;
  let mangaWheelAcc = 0;
  let mangaWheelLock = 0;

  function clampManga() {
    mangaPosX = Math.max(0, Math.min(mangaPosX, mangaMaxX));
    mangaPosY = Math.max(0, Math.min(mangaPosY, mangaMaxY));
  }

  function resetManga() {
    mangaPosX = 0;
    mangaPosY = 0;
    if (mangaCanvas) mangaCanvas.style.transform = "translate3d(0,0,0)";
  }

  /** 把图片轴（视口内容盒）在视口内居中：固定视口层承载边距，内容变形层只做居中 + 平移。
   *  单图/内容不满屏时主轴居中；超过时由 applyMangaPan 的平移查看。 */
  function applyCanvasCentering() {
    const el = mangaCanvas;
    if (el) {
      const vp = mangaViewport;
      const mh = effMarginH();
      const vh = vp?.clientHeight ?? 0;
      const w = el.offsetWidth;
      const ty = widthCentered && vh > 0 ? Math.max(0, (vh - el.offsetHeight) / 2) : 0;
      el.style.transform = `translate(${Math.max(0, (vp?.clientWidth ?? w) / 2 - mh - w / 2)}px, ${ty}px)`;
    }
    const fel = mangaFlowCanvas;
    if (fel) {
      const vp = mangaFlowViewport;
      const mh = effMarginH();
      const w = fel.offsetWidth;
      fel.style.transform = `translateX(${Math.max(0, (vp?.clientWidth ?? w) / 2 - mh - w / 2)}px)`;
    }
  }

  /** 应用平移：LTR 起点在左、RTL 起点在右；无论方向，语义坐标 0 都是“阅读起点”。 */
  function applyMangaPan(smooth = false) {
    const el = mangaCanvas;
    if (!el) return;
    const tx = mangaRtl() ? -(mangaMaxX - mangaPosX) : -mangaPosX;
    const ty = -mangaPosY;
    if (smooth && !reducedMotion()) {
      el.style.transition = "transform 180ms ease";
      el.style.transform = `translate3d(${Math.round(tx)}px, ${Math.round(ty)}px, 0)`;
      return;
    }
    el.style.transition = "none";
    el.style.transform = `translate3d(${Math.round(tx)}px, ${Math.round(ty)}px, 0)`;
    void el.offsetWidth;
    el.style.transition = "";
  }

  /** 横向布局下，内容宽度是否小于可用宽度（单图不满屏 → 主轴居中）。 */
  let widthCentered = false;

  /** 测量横向内容尺寸，计算可平移范围；进入/换章/窗口变化/图片加载后调用。 */
  function measureManga() {
    const vp = mangaViewport;
    const el = mangaCanvas;
    if (!vp) return;
    const vw = vp.clientWidth;
    const vh = vp.clientHeight;
    if (vw <= 0 || vh <= 0) {
      requestAnimationFrame(() => { if (mangaHorizontal()) measureManga(); });
      return;
    }
    // 上下边距只保留这一条路径：作为固定视口 padding 后，基准图高 = 视口高 - 上下边距。
    // 删除原先 imgMaxH = vh - mv*2 与 align-items:center、applyMangaPan 居中三者并存的打架。
    imgMaxH = Math.max(120, vh - effMarginV() * 2);
    if (el) {
      el.style.setProperty("--mg-vieww", `${Math.max(80, vw - effMarginH() * 2)}px`);
      mangaMaxX = Math.max(0, el.offsetWidth - vw);
      mangaMaxY = Math.max(0, el.offsetHeight - vh);
      widthCentered = el.offsetWidth <= vw + 1;
    }
    applyCanvasCentering();
    clampManga();
    applyMangaPan();
  }

  /** 瀑布流（纵向连续窗口）：测滚动范围并同步滚动条。 */
  function measureFlow() {
    const vp = mangaFlowViewport;
    if (!vp) return;
    const vh = vp.clientHeight;
    if (vh > 0) imgMaxH = Math.max(120, vh - effMarginV() * 2);
    applyCanvasCentering();
    maxScroll = Math.max(0, vp.scrollHeight - vp.clientHeight);
    scroller?.setMax(maxScroll);
  }

  /* ================= 漫画跨章连续窗口 ================= */
  // 漫画两种布局都在一个容器里按序渲染「一段连续章节」的图片，滚到接近两端时续章、
  // 裁掉远端，形成连续刷图体验（「一张图一章」不再卡住）。普通分页模式不走这里。
  // 窗口大小按「图片数量」而非章数限定：对「一张图一章」的书 ≈ MANGA_IMG_INIT 章（体验不变），
  // 对「每章很多图」的书则可能只含 1~2 章（DOM 立刻从几千降到百级）。
  const MANGA_IMG_INIT = 15; // 初始窗口累计到约这么多张图即停（至少含当前章）
  const MANGA_IMG_BUDGET = 50; // 续章后窗口累计图片预算上限，超出则从离屏远端裁掉整章

  /** 粗略统计一段 html 里的图片数量（用于按图片数而非章数限定窗口）。 */
  function imgCount(html: string): number {
    return html ? (html.match(/<img/gi)?.length ?? 0) : 0;
  }

  /** 仅漫画路径：给没有 loading 的 <img 补 loading="lazy"（保留后端已加的 decoding="async"），
      使单章即便几百张图也只解码视口附近几张。页模式（普通阅读）绝不调用此函数。 */
  function injectMangaLazy(html: string): string {
    if (!html) return html;
    return html.replace(/<img\b(?![^>]*\bloading=)/gi, '<img loading="lazy"');
  }

  /** 当前窗口累计图片数（预算裁剪的判据）。 */
  function windowImgTotal(): number {
    let n = 0;
    for (const it of mangaWindow) n += imgCount(it.html);
    return n;
  }

  function mangaWindowTrim(pos: { top: number; height: number }) {
    const vp = mangaFlowViewport;
    if (!vp) return pos;
    const blocks = Array.from(vp.querySelectorAll<HTMLElement>(".manga-block"));
    let removed = 0;
    while (mangaWindow.length > 1) {
      const first = blocks.shift();
      if (!first) break;
      const nextTop = blocks[0]?.offsetTop ?? 0;
      const nextH = blocks[0]?.offsetHeight ?? 0;
      if (nextTop + nextH < pos.top) {
        mangaWindow = mangaWindow.slice(1);
        removed += first.offsetHeight;
        continue;
      }
      break;
    }
    while (mangaWindow.length > 1) {
      const last = blocks.pop();
      if (!last) break;
      const prev = blocks[blocks.length - 1];
      const prevBottom = (prev?.offsetTop ?? 0) + (prev?.offsetHeight ?? 0);
      if (prevBottom > pos.top + pos.height) {
        mangaWindow = mangaWindow.slice(0, -1);
        continue;
      }
      break;
    }
    if (removed) {
      pos.top -= removed;
      if (scrollArea) scrollArea.scrollTop = Math.max(0, pos.top);
      scroller?.jumpTo(Math.max(0, pos.top), true);
    }
    return pos;
  }

  /** 按当前章重建窗口（换章/跳转/恢复进度/退出进入漫画模式时调用；同步首屏，保证定位准确）。 */
  async function loadMangaWindow(idx: number, restore?: Progress) {
    if (!book) return;
    const gen = ++loadGeneration;
    loadingChapter = true;
    const total = book.toc.length;
    // 按「图片数量」建窗：先取当前章保证至少含它；不足初值则向两侧交替扩，累计图片达 MANGA_IMG_INIT 即停。
    // 一张图一章 → 约 MANGA_IMG_INIT 章；每章很多图 → 当前章一张就超初值 → 窗口仅 1~2 章。
    const htmls = new Map<number, string>();
    const objs = new Map<number, ChapterData | null>();
    const take = (i: number) =>
      fetchChapter(i)
        .catch(() => null)
        .then((ch) => {
          objs.set(i, ch);
          htmls.set(i, injectMangaLazy(ch?.html ?? ""));
          return htmls.get(i) ?? "";
        });
    // 先占位当前章，首屏即出结构。
    mangaWindow = [{ index: idx, html: "" }];
    chapterNum = idx;
    chapter = cacheGet(idx) ?? null;
    await tick();
    let imgs = imgCount(await take(idx));
    if (gen !== loadGeneration) return;
    let lo = idx;
    let hi = idx;
    if (imgs < MANGA_IMG_INIT) {
      // 并行预取两侧候选（最多各 MANGA_IMG_INIT 章，够一张图一章的书），再按序累加到达初值。
      const loMin = Math.max(0, idx - MANGA_IMG_INIT);
      const hiMax = Math.min(total - 1, idx + MANGA_IMG_INIT);
      const cand: number[] = [];
      for (let i = loMin; i <= hiMax; i++) if (i !== idx) cand.push(i);
      await Promise.all(cand.map(take));
      if (gen !== loadGeneration) return;
      let goLeft = true;
      // 只在已预取的候选范围内扩，避免某些章无图时把整本书纳入空窗口。
      while (imgs < MANGA_IMG_INIT && (lo > loMin || hi < hiMax)) {
        const canL = lo > loMin;
        const canR = hi < hiMax;
        const left = canL && (!canR || goLeft);
        goLeft = !goLeft;
        if (left) lo--;
        else hi++;
        imgs += imgCount(htmls.get(left ? lo : hi) ?? "");
      }
    }
    const idxs: number[] = [];
    for (let i = lo; i <= hi; i++) idxs.push(i);
    mangaWindow = idxs.map((i) => ({ index: i, html: htmls.get(i) ?? "" }));
    chapter = objs.get(idx) ?? chapter;
    loadingChapter = false;
    await tick();
    if (gen !== loadGeneration) return;
    if (scrollLayout()) {
      measureFlow();
      // 定位到目标章块的顶部，再按章内比例下移：保证恢复/跳转落回「该章」而非窗口比例的任意位置。
      const vp = mangaFlowViewport;
      const block = vp?.querySelector<HTMLElement>(`.manga-block[data-index="${idx}"]`);
      let pos = block ? block.offsetTop : 0;
      if (restore && block && block.offsetHeight > 0) {
        pos += Math.max(0, Math.min(1, restore.chapter_ratio)) * block.offsetHeight;
      }
      pos = Math.max(0, Math.min(pos, maxScroll));
      scroller?.jumpTo(pos, true);
      applyScroll(pos);
      lastMangaAnchorTop = block?.offsetTop ?? -1;
    } else if (mangaHorizontal()) {
      measureManga();
      // 定位到目标章块的「阅读起点」（LTR=左缘 / RTL=右缘的语义坐标），再按章内比例推进。
      const el = mangaCanvas;
      const block = el?.querySelector<HTMLElement>(`.manga-block[data-index="${idx}"]`);
      let semX = 0;
      if (el && block) {
        const W = el.offsetWidth;
        semX = mangaRtl() ? W - (block.offsetLeft + block.offsetWidth) : block.offsetLeft;
        if (restore && block.offsetWidth > 0) {
          semX += Math.max(0, Math.min(1, restore.chapter_ratio)) * block.offsetWidth;
        }
      }
      mangaPosX = Math.max(0, Math.min(semX, mangaMaxX));
      mangaPosY = 0;
      applyMangaPan();
    }
    prefetchAround(idx);
    saveProgressDebounced(false);
  }

  /** 窗口向后/前追加一章（滚动接近两端时调用）；prepend 用滚动补偿避免跳动。 */
  async function extendMangaWindow(dir: 1 | -1) {
    if (!book || !mangaWindow.length) return;
    const now = Date.now();
    if (now < mangaExtendLock) return;
    mangaExtendLock = now + 260;
    const gen = loadGeneration;
    if (dir > 0) {
      const cur = mangaWindow[mangaWindow.length - 1].index;
      const next = cur + 1;
      if (next >= book.toc.length) return;
      const ch = await fetchChapter(next).catch(() => null);
      if (gen !== loadGeneration) return;
      // 期间可能已由别处裁剪/重建：确认尾章仍是 cur 再追加。
      if (!mangaWindow.length || mangaWindow[mangaWindow.length - 1].index !== cur) return;
      mangaWindow = [...mangaWindow, { index: next, html: injectMangaLazy(ch?.html ?? "") }];
    } else {
      const cur = mangaWindow[0].index;
      const prev = cur - 1;
      if (prev < 0) return;
      const ch = await fetchChapter(prev).catch(() => null);
      if (gen !== loadGeneration) return;
      if (!mangaWindow.length || mangaWindow[0].index !== cur) return;
      mangaWindow = [{ index: prev, html: injectMangaLazy(ch?.html ?? "") }, ...mangaWindow];
      await tick();
      if (mangaHorizontal()) {
        // 横向 prepend：新插入章排在阅读起点前，所有既有内容的语义坐标整体后移 newWidth，
        // 用 measureManga 重算后把 mangaPosX 加上新章宽度，保持当前画面不跳。
        const el = mangaCanvas;
        const first = el?.querySelector<HTMLElement>(".manga-block");
        const w = first?.offsetWidth ?? 0;
        const gap = config.reader.manga.imgGap;
        measureManga();
        mangaPosX = Math.max(0, Math.min(mangaMaxX, mangaPosX + w + gap));
        applyMangaPan();
        prefetchAround(chapterNum);
        await mangaBudgetTrim();
        return;
      }
      // 瀑布流 prepend 后补偿滚动位置，视觉不跳。
      const vp = mangaFlowViewport;
      if (vp) {
        const first = vp.querySelector<HTMLElement>(".manga-block");
        const h = first?.offsetHeight ?? 0;
        if (scrollArea) scrollArea.scrollTop = Math.max(0, scrollArea.scrollTop + h);
        scroller?.jumpTo(Math.max(0, (scroller?.targetPos ?? 0) + h), true);
      }
    }
    await tick();
    if (scrollLayout()) measureFlow();
    else if (mangaHorizontal()) measureManga();
    await mangaBudgetTrim();
  }

  /** 预算裁剪：窗口累计图片超 MANGA_IMG_BUDGET 时，从「已完全离屏」的远端裁掉整章块，带滚动/位移补偿；
      永不裁掉视口内的章（故与瀑布流的 mangaWindowTrim 互补，横向模式则由它独自保证窗口有界）。 */
  async function mangaBudgetTrim() {
    if (mangaWindow.length <= 1 || windowImgTotal() <= MANGA_IMG_BUDGET) return;
    if (scrollLayout()) {
      const vp = mangaFlowViewport;
      if (!vp) return;
      const blocks = Array.from(vp.querySelectorAll<HTMLElement>(".manga-block"));
      let removedTop = 0;
      // 头部：完全在视口上方的整章
      while (mangaWindow.length > 1 && blocks.length > 1 && windowImgTotal() > MANGA_IMG_BUDGET) {
        const first = blocks[0];
        if (first.offsetTop + first.offsetHeight <= vp.scrollTop) {
          removedTop += first.offsetHeight;
          blocks.shift();
          mangaWindow = mangaWindow.slice(1);
        } else break;
      }
      // 尾部：完全在视口下方的整章
      while (mangaWindow.length > 1 && blocks.length > 1 && windowImgTotal() > MANGA_IMG_BUDGET) {
        const last = blocks[blocks.length - 1];
        if (last.offsetTop >= vp.scrollTop + vp.clientHeight) {
          blocks.pop();
          mangaWindow = mangaWindow.slice(0, -1);
        } else break;
      }
      if (removedTop) {
        if (scrollArea) scrollArea.scrollTop = Math.max(0, scrollArea.scrollTop - removedTop);
        scroller?.jumpTo(Math.max(0, (scroller?.targetPos ?? 0) - removedTop), true);
      }
      await tick();
      measureFlow();
    } else if (mangaHorizontal()) {
      const el = mangaCanvas;
      if (!el) return;
      const vw = mangaViewport?.clientWidth ?? 0;
      const gap = config.reader.manga.imgGap;
      const blocks = Array.from(el.querySelectorAll<HTMLElement>(".manga-block"));
      let removedHeadW = 0;
      let tailRemoved = false;
      // 头部（阅读起点、已读过并离屏）：裁掉并把 mangaPosX 回退该章宽度（prepend 补偿的逆操作）
      while (mangaWindow.length > 1 && blocks.length > 1 && windowImgTotal() > MANGA_IMG_BUDGET) {
        const first = blocks[0];
        const w = first.offsetWidth;
        // 该章在「去掉已裁头部」后的语义末端 = w，仍 ≤ 当前读位（同样减去已裁宽度）才算完全离屏
        if (w <= mangaPosX - removedHeadW) {
          removedHeadW += w + gap;
          blocks.shift();
          mangaWindow = mangaWindow.slice(1);
        } else break;
      }
      // 尾部（阅读终点之后、离屏）：裁掉，无需补偿
      while (mangaWindow.length > 1 && blocks.length > 1 && windowImgTotal() > MANGA_IMG_BUDGET) {
        const last = blocks[blocks.length - 1];
        const semStart = mangaRtl()
          ? el.offsetWidth - (last.offsetLeft + last.offsetWidth)
          : last.offsetLeft;
        if (semStart >= mangaPosX + vw) {
          blocks.pop();
          mangaWindow = mangaWindow.slice(0, -1);
          tailRemoved = true;
        } else break;
      }
      if (removedHeadW || tailRemoved) {
        await tick();
        measureManga();
        if (removedHeadW) {
          mangaPosX = Math.max(0, Math.min(mangaMaxX, mangaPosX - removedHeadW));
          applyMangaPan();
        }
      }
    }
  }

  /** 滚动接近两端时续章（按视口高/宽倍数提前预载），并裁剪远端保持窗口有界。 */
  function checkMangaWindowExtend() {
    const vp = mangaFlowViewport;
    if (!vp || !mangaWindow.length) return;
    const top = vp.scrollTop;
    const height = vp.clientHeight;
    const full = vp.scrollHeight;
    const pos = mangaWindowTrim({ top, height });
    updateChapterFromWindow();
    // 预加载提前：按视口高度（约 1.5 屏）续章，使「一图一章」的书到达旧底/顶前 maxScroll 已增长，消除停顿/回弹。
    const lead = height * 1.5;
    if (pos.top <= lead) void extendMangaWindow(-1);
    if (full - (pos.top + height) <= lead) void extendMangaWindow(1);
  }

  /** 根据视口顶部所在章更新章号（保证 TOC 高亮 / 进度 / 朗读起点正确）。 */
  function updateChapterFromWindow() {
    const vp = mangaFlowViewport;
    if (!vp || !mangaWindow.length) return;
    const blocks = vp.querySelectorAll<HTMLElement>(".manga-block");
    const target = vp.scrollTop + vp.clientHeight * 0.35;
    let best = mangaWindow[0].index;
    for (const b of blocks) {
      const i = Number(b.dataset.index);
      if (!Number.isFinite(i)) continue;
      if (b.offsetTop <= target) best = i;
      else break;
    }
    if (best !== chapterNum) {
      chapterNum = best;
      chapter = chapterCache.get(best) ?? chapter;
      saveProgressDebounced();
    }
  }

  /** 横向：根据当前平移位置落在哪一章更新章号（语义坐标，LTR/RTL 均正确）。 */
  function updateChapterFromHorizontal() {
    const el = mangaCanvas;
    if (!el || !mangaWindow.length) return;
    const W = el.offsetWidth;
    const center = mangaPosX + (mangaViewport?.clientWidth ?? 0) / 2;
    const blocks = el.querySelectorAll<HTMLElement>(".manga-block");
    let best = mangaWindow[0].index;
    let bestStart = -Infinity;
    for (const b of blocks) {
      const i = Number(b.dataset.index);
      if (!Number.isFinite(i)) continue;
      // 块的「阅读起点」语义坐标：LTR=左缘，RTL=右缘（随阅读顺序递增）。
      const semStart = mangaRtl() ? W - (b.offsetLeft + b.offsetWidth) : b.offsetLeft;
      if (semStart <= center && semStart >= bestStart) {
        bestStart = semStart;
        best = i;
      }
    }
    if (best !== chapterNum) {
      chapterNum = best;
      chapter = chapterCache.get(best) ?? chapter;
      saveProgressDebounced();
    }
  }

  /** 一屏横向步长（约 90% 视口宽，保留少量重叠）。 */
  function mangaStep(): number {
    const vw = mangaViewport?.clientWidth ?? 0;
    return Math.max(80, Math.round(vw * 0.9));
  }

  /** 阅读前进一屏；连续窗口下滚过头会自动续章。 */
  function nextManga() {
    if (mangaPosX < mangaMaxX - 2) {
      mangaPosX = Math.min(mangaMaxX, mangaPosX + mangaStep());
      applyMangaPan(true);
      updateChapterFromHorizontal();
      saveProgressDebounced();
      if (mangaMaxX - mangaPosX < (mangaViewport?.clientWidth ?? 0) * 1.2) void extendMangaWindow(1);
    } else if (chapterNum < (book?.toc.length ?? 1) - 1) {
      // 窗口尚未加载下一章时兜底：加载下一章并重建窗口。
      loadMangaWindow(chapterNum + 1);
    }
  }

  /** 阅读后退一屏；到起点则续上一章。 */
  function prevManga() {
    if (mangaPosX > 2) {
      mangaPosX = Math.max(0, mangaPosX - mangaStep());
      applyMangaPan(true);
      updateChapterFromHorizontal();
      saveProgressDebounced();
      if (mangaPosX < (mangaViewport?.clientWidth ?? 0) * 1.2) void extendMangaWindow(-1);
    } else if (chapterNum > 0) {
      loadMangaWindow(chapterNum - 1);
    }
  }

  function onMangaWheel(e: WheelEvent) {
    e.preventDefault();
    const now = Date.now();
    const delta = e.deltaX !== 0 ? e.deltaX : (e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY);
    if (Math.abs(delta) < 1) return;
    if (now < mangaWheelLock) return;
    const before = mangaPosX;
    mangaPosX = Math.max(0, Math.min(mangaMaxX, mangaPosX + delta));
    if (mangaPosX !== before) {
      applyMangaPan();
      updateChapterFromHorizontal();
      saveProgressDebounced();
      mangaWheelAcc = 0;
      // 连续滚动：接近两端时自动续章（按视口宽约 1.2 屏提前，扩展后 maxX 变大可继续滚）。
      const lead = (mangaViewport?.clientWidth ?? 0) * 1.2;
      if (mangaMaxX - mangaPosX < lead) void extendMangaWindow(1);
      else if (mangaPosX < lead) void extendMangaWindow(-1);
      return;
    }
    // 已到横向两端：滚轮继续推动即续章。
    const forward = delta > 0;
    const atForwardEdge = forward && mangaPosX >= mangaMaxX - 2;
    const atBackEdge = !forward && mangaPosX <= 2;
    if (atForwardEdge || atBackEdge) {
      if (forward) void extendMangaWindow(1);
      else void extendMangaWindow(-1);
      saveProgressDebounced();
      updateChapterFromHorizontal();
    } else {
      mangaWheelAcc = 0;
    }
  }

  function suppressMangaClick() {
    const swallow = (ev: Event) => {
      ev.stopPropagation();
      ev.preventDefault();
    };
    window.addEventListener("click", swallow, true);
    setTimeout(() => window.removeEventListener("click", swallow, true), 400);
  }

  function onMangaPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    mangaPid = e.pointerId;
    mangaStartX = e.clientX;
    mangaStartY = e.clientY;
    mangaOriginX = mangaPosX;
    mangaOriginY = mangaPosY;
    mangaPanning = false;
    mangaDownTarget = e.target;
    mangaViewport?.setPointerCapture?.(e.pointerId);
    window.getSelection()?.removeAllRanges();
  }

  function onMangaPointerMove(e: PointerEvent) {
    if (e.pointerId !== mangaPid) return;
    const dx = e.clientX - mangaStartX;
    const dy = e.clientY - mangaStartY;
    if (!mangaPanning) {
      if (Math.hypot(dx, dy) < 6) return;
      mangaPanning = true;
      if (mangaViewport) mangaViewport.style.cursor = "grabbing";
    }
    // 内容跟随指针 1:1 平移（拖右→看到左侧内容）。
    mangaPosX = Math.max(0, Math.min(mangaMaxX, mangaOriginX - dx));
    mangaPosY = Math.max(0, Math.min(mangaMaxY, mangaOriginY - dy));
    applyMangaPan();
  }

  function onMangaPointerUp(e: PointerEvent) {
    if (e.pointerId !== mangaPid) return;
    mangaPid = -1;
    if (mangaViewport) mangaViewport.style.cursor = "";
    if (mangaPanning) {
      mangaPanning = false;
      suppressMangaClick(); // 进入过平移：松手不触发点击翻页/打开图库
      saveProgressDebounced();
      return;
    }
    const el = mangaViewport;
    if (!el) return;
    const t = mangaDownTarget as HTMLElement | null;
    const img = t?.closest?.("img") as HTMLImageElement | null;
    if (img && el.contains(img)) {
      openViewerForImage(img);
      return;
    }
    const r = el.getBoundingClientRect();
    const x = (e.clientX - r.left) / r.width;
    if (x < 0.3) prevManga();
    else if (x > 0.7) nextManga();
  }

  function onMangaPointerCancel(e: PointerEvent) {
    if (e.pointerId !== mangaPid) return;
    mangaPid = -1;
    mangaPanning = false;
    if (mangaViewport) mangaViewport.style.cursor = "";
  }

  /** 指针是否位于 .body 右下角热区（约 150×130），用 rAF 节流后短算。 */
  function onReaderMove(e: MouseEvent) {
    hoverX = e.clientX;
    hoverY = e.clientY;
    pokeChrome();
    if (hoverRaf) return;
    hoverRaf = requestAnimationFrame(() => {
      hoverRaf = 0;
      const el = bodyEl;
      if (!el) return;
      const r = el.getBoundingClientRect();
      // 控件行现有 5 个按钮，热区相应加宽，保证移到最左侧按钮时不会突然消失。
      const inCorner = hoverX >= r.right - 230 && hoverY >= r.bottom - 130;
      if (inCorner !== fontCtlShow) fontCtlShow = inCorner;
    });
  }

  /** 上次已知的可视区尺寸；用于区分「窗口尺寸变化」与「内容高度变化（图片加载）」。 */
  let lastVpW = -1;
  let lastVpH = -1;

  function relayout() {
    if (!chapter) return;
    // 漫画横向：重测窗口尺寸。内容窗口宽度可能因图片加载/缩放/边距变化而改变，
    // 以横向绝对位置占内容总宽的比例恢复，避免重排后跳到别处。
    if (mangaHorizontal()) {
      const vp = mangaViewport;
      const vw = vp?.clientWidth ?? 0;
      const prevSpanX = mangaMaxX + vw; // 旧内容总宽（measureManga 前仍是旧值）
      const fx = prevSpanX > 0 ? mangaPosX / prevSpanX : 0;
      measureManga(); // 重算 --mg-vieww / mangaMaxX / centered 居中
      mangaPosX = Math.max(0, Math.min(fx * (mangaMaxX + vw), mangaMaxX));
      applyMangaPan();
      return;
    }
    // 漫画瀑布流：重测滚动范围；窗口内容在视口上方变高时按锚点补偿 scrollTop，避免跳动。
    if (scrollLayout() && isMangaMode()) {
      const vp = mangaFlowViewport;
      const anchorIndex = chapterNum;
      const before = vp?.scrollTop ?? 0;
      measureFlow();
      const vp2 = mangaFlowViewport;
      if (vp2) {
        const block = vp2.querySelector<HTMLElement>(`.manga-block[data-index="${anchorIndex}"]`);
        // 保持「锚点章节相对视口的位置」不变：用其 offsetTop 变化量补偿。
        if (block && lastMangaAnchorTop >= 0) {
          const delta = block.offsetTop - lastMangaAnchorTop;
          if (delta !== 0) {
            const p = Math.max(0, before + delta);
            scroller?.jumpTo(p, true);
            applyScroll(p);
          }
        }
        lastMangaAnchorTop = block?.offsetTop ?? -1;
      }
      return;
    }
    // 滚动模式下先同步节流的响应式位置，保证重排后按真实位置恢复
    if (scrollLayout()) scrollTop = Math.round(scrollRender);
    if (scrollLayout()) {
      const vpW = contentEl?.clientWidth ?? 0;
      const vpH = contentEl?.clientHeight ?? 0;
      const viewportChanged = vpW !== lastVpW || vpH !== lastVpH;
      lastVpW = vpW;
      lastVpH = vpH;
      if (viewportChanged) {
        // 可视区变化（窗口缩放）：按比例恢复，保持阅读位置。
        const ratio = currentRatio();
        measureScroll();
        scroller?.jumpTo(ratio * maxScroll, true);
      } else {
        // 仅内容高度变化（图片延迟加载/字体等）：保持当前绝对位置，不做比例重定位，
        // 否则会因内容在视口上方变高而整体跳一下（滚动中的可见跳动）。
        measureScroll();
      }
    } else {
      const ratio = currentRatio();
      measurePages();
      gotoPage(Math.round(ratio * (pageCount - 1)), true);
    }
  }

  /** 漫画瀑布流重排的锚点章相对视口的 offsetTop（-1=未记录）。 */
  let lastMangaAnchorTop = -1;

  let relayoutTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleRelayout() {
    clearTimeout(relayoutTimer);
    relayoutTimer = setTimeout(relayout, 160);
  }

  /* ================= input: wheel / keys / zones ================= */
  let wheelAcc = 0;
  let wheelLockUntil = 0;

  function onWheel(e: WheelEvent) {
    if (!contentEl) return;
    if (mangaHorizontal()) {
      onMangaWheel(e);
      return;
    }
    const d = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    if (!scrollLayout()) {
      if (!config.reader.wheelTurnPage) return;
      e.preventDefault();
      const now = Date.now();
      if (now < wheelLockUntil) {
        wheelAcc = 0;
        return;
      }
      wheelAcc += d;
      if (wheelAcc >= 100) {
        nextPage();
        wheelAcc = 0;
        wheelLockUntil = now + 240;
      } else if (wheelAcc <= -100) {
        prevPage();
        wheelAcc = 0;
        wheelLockUntil = now + 240;
      }
      return;
    }
    e.preventDefault();
    if (maxScroll <= 1 || !scroller) {
      if (d > 0) handleScrollEdge(1, 0);
      else if (d < 0) handleScrollEdge(-1, 0);
      return;
    }
    const res = scroller.wheelBy(d * 1.4);
    if (res.edge !== 0) handleScrollEdge(res.edge, res.carry);
  }

  function scrollStep(dy: number) {
    if (!scroller || maxScroll <= 1) {
      if (dy > 0) handleScrollEdge(1, 0);
      else if (dy < 0) handleScrollEdge(-1, 0);
      return;
    }
    const res = scroller.wheelBy(dy);
    if (res.edge !== 0) handleScrollEdge(res.edge, res.carry);
  }

  function runAction(a: BindAction, e?: { preventDefault(): void }) {
    e?.preventDefault();
    const vh = contentEl ? contentEl.clientHeight * 0.9 : 480;
    switch (a) {
      case "next":
        if (mangaHorizontal()) nextManga();
        else if (scrollLayout()) scrollStep(vh);
        else nextPage();
        break;
      case "prev":
        if (mangaHorizontal()) prevManga();
        else if (scrollLayout()) scrollStep(-vh);
        else prevPage();
        break;
      case "scrollDown":
        if (mangaHorizontal()) { mangaPosX = Math.min(mangaMaxX, mangaPosX + vh * 0.4); clampManga(); applyMangaPan(); }
        else scrollStep(vh * 0.4);
        break;
      case "scrollUp":
        if (mangaHorizontal()) { mangaPosX = Math.max(0, mangaPosX - vh * 0.4); clampManga(); applyMangaPan(); }
        else scrollStep(-vh * 0.4);
        break;
      case "chapterNext":
        if (mangaHorizontal()) nextChapter(true);
        else if (scrollLayout()) handleScrollEdge(1, 0);
        else nextChapter(true);
        break;
      case "chapterPrev":
        if (mangaHorizontal() && chapterNum > 0) loadMangaWindow(chapterNum - 1);
        else prevChapter(true);
        break;
      case "toc":
        tocOpen = !tocOpen;
        break;
      case "addBookmark":
        addBookmark();
        break;
      case "bookmarkList":
        loadBookmarks();
        break;
      case "ai":
        toggleAiPanel();
        break;
      case "bookStats":
        setRight("stats");
        break;
      case "ttsPlayPause":
        toggleTts();
        break;
      case "ttsStop":
        stopTts();
        break;
      case "ttsNext":
        ttsNext();
        break;
      case "ttsPrev":
        ttsPrev();
        break;
      case "settings":
        ui.settingsOpen = true;
        break;
      case "close":
        if (tbManagerOpen || imgMenuOpen || blockedListOpen || mangaGuideOpen) {
          tbManagerOpen = false;
          imgMenuOpen = false;
          blockedListOpen = false;
          mangaGuideOpen = false;
        } else if (blockPickMode) {
          blockPickMode = false;
        } else if (tocOpen || bookmarksOpen || statsOpen || highlightsOpen || selMenu || searchPopup || quickAiPopup || hlMenu) {
          tocOpen = false;
          rightPanel = null;
          selMenu = null;
          searchPopup = null;
          quickAiPopup = null;
          hlMenu = null;
        } else {
          goBack();
        }
        break;
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    pokeChrome();
    // F11 全屏：置于所有早退之前——先 preventDefault 挡掉 WebView2 默认全屏，再切 Tauri 全屏。
    // 焦点在输入框/文本区时仍触发（并吃掉默认行为），符合交付说明的处理方式。
    if (e.key === "F11") {
      e.preventDefault();
      void toggleFullscreen();
      return;
    }
    const tgt = e.target as HTMLElement | null;
    if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA" || tgt.isContentEditable)) return;

    // 任何弹窗/抽屉/确认框打开时都不响应阅读快捷键（Esc 关闭弹窗由 +page 全局处理）
    if (ui.settingsOpen || ui.searchOpen || ui.statsOpen || ui.webdavOpen || convertState.open || confirmBox.active) return;
    // 图库打开时，按键只由 ImageViewer 处理（否则 Esc 会同时触发 Reader 的“返回书库”）
    if (viewerIndex !== null) return;

    const combo = comboOf(e);
    const b = config.reader.bindings;
    for (const def of BIND_DEFS) {
      const keys = b[def.id];
      if (keys && keys.some((k) => bindKeyMatches(k, combo))) {
        if (!config.reader.pageTurnKeys &&
          ["next", "prev", "scrollDown", "scrollUp", "chapterNext", "chapterPrev"].includes(def.id)) {
          return;
        }
        runAction(def.id, e);
        return;
      }
    }
  }

  let ptr: { x: number; y: number; t: number; id: number } | null = null;

  function onContentPointerDown(e: PointerEvent) {
    if (mangaHorizontal()) return; // 漫画双页：手势由横向视口单独处理
    if (e.button !== 0) {
      ptr = null;
      return;
    }
    ptr = { x: e.clientX, y: e.clientY, t: Date.now(), id: e.pointerId };
    // 新手势开始前清掉旧选区：避免「点空白处既保留上次选区、又把它扩成一大段」。
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) sel.removeAllRanges();
  }

  function onContentPointerUp(e: PointerEvent) {
    if (mangaHorizontal()) return; // 漫画双页：手势由横向视口单独处理
    if (!ptr || ptr.id !== e.pointerId) {
      ptr = null;
      return;
    }
    const moved = Math.hypot(e.clientX - ptr.x, e.clientY - ptr.y);
    const quick = Date.now() - ptr.t < 450;
    ptr = null;
    // 只有「主动拖拽出的选区」才弹划词菜单（划线 / 全文 / 浏览器 / AI）；
    // 纯点击（未拖动）一律不弹，直接走点击区域动作。
    if (contentEl && moved > 4) {
      const sel = window.getSelection();
      if (sel && !sel.isCollapsed && sel.rangeCount) {
        const root = contentRoot();
        const range = sel.getRangeAt(0);
        if (root && root.contains(range.commonAncestorContainer)) {
          const text = sel.toString().replace(/\s+/g, " ").trim();
          const offs = selectionOffsets(root, range);
          if (text && offs && offs.end > offs.start) {
            showSelectionMenu(text, offs, range.getBoundingClientRect());
            return;
          }
        }
      }
      return; // 拖拽但没形成有效选区：不当作点击翻页
    }
    // 纯点击：清掉可能残留的选区（含浏览器在空白处“整块选中”），避免误选与菜单复现。
    window.getSelection()?.removeAllRanges();
    // 点击已有划线 → 打开划线设置（改颜色 / 改样式 / 删除）
    const markEl = (e.target as HTMLElement | null)?.closest?.("mark.fr-hl") as HTMLElement | null;
    if (markEl && contentEl && contentEl.contains(markEl)) {
      const id = Number(markEl.dataset.hlid);
      const h = highlights.find((x) => x.id === id);
      if (h) {
        hlMenu = { h, anchor: rectAnchor(markEl.getBoundingClientRect()) };
        return;
      }
    }
    if (moved > 10 || !quick || !contentEl) return;
    // 纯点击正文图片 → 屏蔽选择模式下屏蔽该图；否则打开全书图片查看器（不触发翻页/区域动作）。
    const imgEl = (e.target as HTMLElement | null)?.closest?.("img") as HTMLImageElement | null;
    if (imgEl && contentEl.contains(imgEl)) {
      e.preventDefault();
      e.stopPropagation();
      if (blockPickMode) {
        const src = imgEl.getAttribute("src") || "";
        if (src) {
          const list = blockedImages();
          if (!list.includes(src)) setOv({ blockedImages: [...list, src] });
          toast("已屏蔽该图片（可在「已屏蔽图片」中恢复）");
        }
        blockPickMode = false;
        return;
      }
      openViewerForImage(imgEl);
      return;
    }
    const rect = contentEl.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width;
    const y = (e.clientY - rect.top) / rect.height;
    let zone: ZoneKey;
    if (Math.abs(x - 0.5) < 0.16 && Math.abs(y - 0.5) < 0.21) zone = "c";
    else if (x < 0.5 && y < 0.5) zone = "tl";
    else if (x >= 0.5 && y < 0.5) zone = "tr";
    else if (x < 0.5) zone = "bl";
    else zone = "br";
    const action = config.reader.zones[zone];
    if (!action || action === "none") return;
    runAction(action);
  }

  function onBodyClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest?.("a") as HTMLAnchorElement | null;
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    const href = a.getAttribute("href") || "";
    if (!href || href.startsWith("#")) return;
    if (/^https?:/i.test(href)) {
      copyText(href).then(() => toast("外部链接地址已复制到剪贴板"));
      return;
    }
    const clean = decodeURIComponent(href.split("#")[0]);
    const base = (clean.split(/[\\/]/).pop() || "").toLowerCase();
    if (!base || !base.includes(".")) return;
    const idx = book?.docs?.findIndex((d) => d.toLowerCase() === base) ?? -1;
    if (idx >= 0) {
      loadChapter(idx);
    } else {
      toast(`跳转失败：未找到 ${base}`);
    }
  }

  function goBack() {
    stopTts();
    if (standalone) {
      try {
        doSaveProgress();
        flushSession(true);
      } catch { /* ignore */ }
      import("@tauri-apps/api/window").then((m) => m.getCurrentWindow().close()).catch(() => { try { window.close(); } catch { /* ignore */ } });
    } else {
      close().then(() => (ui.view = "library"));
    }
  }

  onMount(() => {
    // 开发期调试钩子：便于自动化验证搜索框/笔记交互（仅 DEV 构建，release 不含）。
    if (import.meta.env.DEV) {
      (window as any).__frDebug = {
        openSearch: (q = "的") => {
          searchPopup = { query: q, anchor: { top: 120, bottom: 121, cx: window.innerWidth / 2 } };
        },
        /** 打开某条划线的菜单（H1：删除/添加感想）。 */
        openHighlightMenu: async (hlId: number) => {
          await refreshHighlights();
          await tick();
          paintNow();
          const h = highlights.find((x) => x.id === hlId);
          if (!h) return false;
          hlMenu = { h, anchor: { top: 200, bottom: 201, cx: window.innerWidth / 2 } };
          return true;
        },
        /** 悬停某条划线（H2）。 */
        hoverHighlight: async (hlId: number) => {
          await refreshHighlights();
          await tick();
          paintNow();
          const root = contentRoot();
          const mark = root?.querySelector(`mark.fr-hl[data-hlid="${hlId}"]`) as HTMLElement | null;
          const h = highlights.find((x) => x.id === hlId);
          if (!mark || !h || !h.note.trim()) {
            hoverNote = null;
            return false;
          }
          const r = mark.getBoundingClientRect();
          hoverNote = { note: h.note, text: h.text, x: r.left + r.width / 2, y: r.top };
          return true;
        },
        clearHover: () => { hoverNote = null; },
        /** 打开某条划线的 AI 释义小窗（H3：可添加到笔记）。 */
        openAiForHighlight: async (hlId: number) => {
          await refreshHighlights();
          await tick();
          const h = highlights.find((x) => x.id === hlId);
          if (!h) return false;
          quickAiPopup = { text: h.text, anchor: { top: 200, bottom: 201, cx: window.innerWidth / 2 }, highlightId: h.id };
          return true;
        },
        /** 当前 hoverNote 文本（供断言）。 */
        hoverNoteText: () => hoverNote?.note ?? null,
      };
    }
    ttsSetCallbacks({
      onSegment: (seg) => onTtsSegment(seg),
      onEnd: () => { void onTtsChapterEnd(); },
    });
    scroller = new SmoothScroller(applyScroll);
    scroller.onSettled(() => {
      // settle 时精确同步一次响应式位置（滚动中仅节流更新）
      const p = Math.round(scroller!.pos);
      lastUiSync = performance.now();
      scrollRender = p;
      if (scrollTop !== p) scrollTop = p;
      saveProgressDebounced();
    });
    const onJump = (e: Event) => {
      const idx = (e as CustomEvent<number>).detail;
      if (book && idx >= 0 && idx < book.toc.length) loadChapter(idx);
    };
    const onUnload = () => {
      doSaveProgress();
      flushSession(true);
    };
    window.addEventListener("tidepage-jump", onJump);
    window.addEventListener("pagehide", onUnload);
    // 独立 AI 窗口「⇲ 停靠」回主窗口：以 rightPanel 打开抽屉（不经 ui.aiOpen）。
    const aiDockUn = listen("ai-dock", () => { setRight("ai"); });
    // 任务 5：独立面板窗口的跳转请求（跨窗 Tauri 事件）→ 主窗执行导航
    const gotoChUn = listen<{ index: number }>("tidepage-goto-chapter", (e) => {
      const i = e.payload?.index;
      if (book && typeof i === "number" && i >= 0 && i < book.toc.length) loadChapter(i);
    });
    const gotoHlUn = listen<{ id: number }>("tidepage-goto-highlight", async (e) => {
      const id = e.payload?.id;
      if (typeof id !== "number") return;
      await refreshHighlights();
      const h = highlights.find((x) => x.id === id);
      if (h) jumpToHighlight(h);
    });
    return () => {
      ttsStop();
      scroller?.destroy();
      if (statsTimer) clearInterval(statsTimer);
      window.removeEventListener("tidepage-jump", onJump);
      window.removeEventListener("pagehide", onUnload);
      aiDockUn.then((un) => un()).catch(() => {});
      gotoChUn.then((un) => un()).catch(() => {});
      gotoHlUn.then((un) => un()).catch(() => {});
      clearTimeout(fsHintTimer);
      doSaveProgress();
      const el = document.getElementById("tidepage-dynamic-css");
      if (el) el.remove();
      styleTag = null;
    };
  });

  let styleTag: HTMLStyleElement | null = null;

  $effect(() => {
    if (scroller) {
      const p = SCROLL_BOUNCE_PRESETS[config.reader.scrollBounceLevel] ?? SCROLL_BOUNCE_PRESETS.medium;
      scroller.bounce = config.reader.scrollBounce;
      scroller.strength = p.strength;
      scroller.threshold = p.threshold;
    }
  });

  $effect(() => {
    const w = pageWrap;
    const sw = mangaFlowCanvas ?? scrollWrap;
    const c = contentEl;
    if (!w && !sw && !c) return;
    const updateW = () => { if (c) contentW = c.clientWidth; };
    updateW();
    const ro = new ResizeObserver(() => { updateW(); scheduleRelayout(); });
    if (c) ro.observe(c);
    if (w) ro.observe(w);
    if (sw) ro.observe(sw);
    const onWin = () => scheduleRelayout();
    window.addEventListener("resize", onWin);
    // 按需/懒加载图片在渲染后才解码，加载完成会改变排版：捕获阶段监听 img load 重新测量。
    const onImgLoad = (ev: Event) => {
      const t = ev.target as HTMLElement | null;
      if (t && t.tagName === "IMG") scheduleRelayout();
    };
    if (c) c.addEventListener("load", onImgLoad, true);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onWin);
      if (c) c.removeEventListener("load", onImgLoad, true);
    };
  });

  function fontStack(raw: string, latinFirst?: string): string {
    const norm = (list: string[]) => list
      .map((s) => s.trim().replace(/^["']|["']$/g, ""))
      .filter(Boolean)
      .map((s) => (/^[A-Za-z][A-Za-z0-9-]*$/.test(s) ? s : `"${s}"`))
      .join(", ");
    const base = norm((raw || "").split(","));
    const latin = norm([latinFirst || ""]);
    if (!latin) return base || '"Microsoft YaHei", "Segoe UI", sans-serif';
    return base ? `${latin}, ${base}` : latin;
  }

  function readerVars(): string {
    // 有效阅读配色：优先本书单独套用的方案，其次全局。主题预设取有效主题。
    const t = THEMES[effTheme()];
    // 漫画模式可独立设置背景色（空则跟随普通主题/自定义背景）。
    const mangaBg = isMangaMode() && config.reader.manga.bgColor ? config.reader.manga.bgColor : "";
    const bg = mangaBg || effBgColor() || t.bg;
    const fg = effFgColor() || t.fg;
    return [
      `--rdr-bg:${bg}`,
      `--rdr-fg:${fg}`,
      `--rdr-font:${fontStack(effFontFamily(), effLatinFont())}`,
      `--rdr-fs:${effFontSize()}px`,
      `--rdr-lh:${effLineHeight()}`,
      `--rdr-ps:${effParaSpacing()}em`,
      `--rdr-mh:${effMarginH()}px`,
      `--rdr-img-h:${imgMaxH > 0 ? imgMaxH + "px" : "80vh"}`,
      `--rdr-img-scale:${effImgScale()}`,
      `--rdr-img-gap:${config.reader.manga.imgGap}px`,
      `--rdr-mv:${effMarginV()}px`,
      `--rdr-indent:${effIndent()}`,
      `--rdr-align:${effTextAlign()}`,
      `--rdr-fw:${effFontWeight()}`,
      `--rdr-sel:${t.sel}`,
      `--hl-color:${config.reader.highlightColor || "#c0392b"}`,
      `--rdr-top:${config.reader.topbarColor || "color-mix(in srgb, " + bg + " 90%, black 10%)"}`,
      `--rdr-stat:${config.reader.statusColor || t.fg}`,
      `--rdr-stat-align:${config.reader.statusPos === "none" ? "center" : config.reader.statusPos}`,
    ].join(";");
  }

  function buildDynamicCss(): string {
    const r = config.reader;
    const overrideRule = effOverrideCss()
      ? `
#chapter-content { color: var(--rdr-fg) !important; font-family: var(--rdr-font) !important; text-align: var(--rdr-align) !important; }
#chapter-content p { font-size: var(--rdr-fs) !important; text-indent: var(--rdr-indent) !important; margin: var(--rdr-ps) 0 !important; line-height: var(--rdr-lh) !important; font-weight: var(--rdr-fw) !important; }
#chapter-content :is(h1,h2,h3,h4,h5,h6) { font-size: 1.2em !important; font-weight: 600 !important; text-align: center !important; margin: 0.4em 0 0.8em !important; }
#chapter-content sup { font-size: 0.7em !important; }
`
      : "";
    // 图片缩放/适应宽度：无论是否覆盖书籍排版都生效（单本图片控件）。
    // 基础：宽度/高度上限都随 --rdr-img-scale 缩放（否则只改 max-height 时，宽度受限的图不变）。
    // .imgw（漫画书，或用户已对本书缩放）：直接按阅读器宽度比例设置宽度，保证「漫画图片大小」可见生效。
    // zoom 缩放实际尺寸；max-width 上限随 zoom 一起放大 → 允许右下角缩放突破阅读器宽度/漫画默认大小。
    const imgRules = `
#chapter-content :is(img,svg,video) { height: auto !important; width: auto !important; max-width: calc(100% * var(--rdr-img-scale, 1)) !important; object-fit: contain !important; max-height: calc(var(--rdr-img-h, 80vh) - 1.6em) !important; zoom: var(--rdr-img-scale, 1); }
.reader.manga #chapter-content :is(img,svg,video) { margin-top: calc(var(--rdr-img-gap, 6px) / 2) !important; margin-bottom: calc(var(--rdr-img-gap, 6px) / 2) !important; }
.reader.fitw #chapter-content :is(img,svg,video) { width: 100% !important; max-width: 100% !important; height: auto !important; max-height: none !important; zoom: 1; }
`;
    // 漫画两种布局的图片规则统一在组件样式区的 .manga-h-* / .manga-flow-* 中定义
    // （固定视口层承载边距 + 内容变形层平移/滚动），不再依赖会随 transform 移动的 #chapter-content padding。
    const hideTextRule = isMangaMode() && r.manga.hideText
      ? `
/* 漫画模式「仅图片」：隐藏不含图片的文本块 */
.reader.manga #chapter-content :is(p,div,span,h1,h2,h3,h4,h5,h6,li,blockquote,figcaption):not(:has(img,svg,video)) { display: none !important; }
`
      : "";
    return `
${overrideRule}
${imgRules}
${hideTextRule}
${effCssExtra() || ""}
/* 硬编码根治发糊：正文统一禁止合成加粗/斜体（与 .ai-mark 一致），避免第三方字体假粗假斜发糊 */
:root #chapter-content { font-synthesis: none !important; }
`;
  }

  $effect(() => {
    const r = config.reader;
    void effOverrideCss();
    void isMangaMode();
    void effCssExtra();
    void effFitHeight();
    void r.manga.hideText;
    void r.manga.layout;
    void r.manga.imgGap;
    const css = buildDynamicCss();
    if (!styleTag) {
      styleTag = document.createElement("style");
      styleTag.id = "tidepage-dynamic-css";
      document.head.appendChild(styleTag);
    }
    styleTag.textContent = css;
  });

  // 旧式 MOBI <font face/size/color> 硬修复：章节进入 DOM 后剥离属性并注入通用覆盖 CSS。
  // 依赖：章节内容 + 五项排版设置 + 是否覆盖书籍排版。幂等（固定 id=__read_fix，替换 textContent）。
  $effect(() => {
    const override = effOverrideCss();
    void chapter;
    const opts = {
      fontFamily: fontStack(effFontFamily(), effLatinFont()),
      monoFont: "Consolas, \"Courier New\", monospace",
      fontSize: effFontSize(),
      lineHeight: effLineHeight(),
      color: effFgColor() || THEMES[effTheme()].fg,
      background: effBgColor() || THEMES[effTheme()].bg,
    };
    void tick().then(() => {
      if (!override) {
        clearReadFix();
        return;
      }
      applyReadFix(opts);
    });
  });

  $effect(() => {
    const r = config.reader;
    r.mode;
    r.duo;
    r.colGap;
    r.fontSize;
    r.fontWeight;
    r.lineHeight;
    r.paraSpacing;
    r.marginH;
    r.maxLineChars;
    r.marginV;
    r.indent;
    r.textAlign;
    r.fontFamily;
    r.latinFont;
    r.cssExtra;
    r.overrideBookCss;
    r.customFonts;
    r.highlightColors;
    r.highlightColor;
    r.highlightStyle;
    saveConfig();
    tick().then(scheduleRelayout);
  });

  // 「覆盖书籍排版」切换会改变章节 HTML（是否内联原书 CSS）：清缓存并重载当前章，
  // 保证原书样式开关立即生效，而不是沿用已缓存的旧渲染。
  let lastOverrideCss: boolean | null = null;
  $effect(() => {
    const v = effOverrideCss();
    if (lastOverrideCss === null) {
      lastOverrideCss = v;
      return;
    }
    if (lastOverrideCss === v) return;
    lastOverrideCss = v;
    chapterCache.clear();
    inflight.clear();
    if (book) {
      loadGeneration++;
      loadChapter(chapterNum);
    }
  });

  /** 单本图片显示/屏蔽：章节渲染后把被关闭/被屏蔽的图片隐藏（不改变后端缓存 HTML）。 */
  function applyImageVisibility() {
    const root = contentEl?.querySelector("#chapter-content") as HTMLElement | null;
    if (!root) return;
    const show = effShowImages();
    const blocked = new Set(blockedImages());
    root.querySelectorAll("img").forEach((el) => {
      const img = el as HTMLImageElement;
      const src = img.getAttribute("src") || "";
      const hide = !show || blocked.has(src);
      if (img.style.display !== (hide ? "none" : "")) img.style.display = hide ? "none" : "";
    });
    root.querySelectorAll("image").forEach((el) => {
      const im = el as SVGImageElement;
      const href = im.getAttribute("href") || im.getAttributeNS("http://www.w3.org/1999/xlink", "href") || "";
      const hide = !show || (!!href && blocked.has(href));
      const s = (im as unknown as HTMLElement).style;
      if (s.display !== (hide ? "none" : "")) s.display = hide ? "none" : "";
    });
  }

  $effect(() => {
    void chapter;
    void effShowImages();
    void blockedImages();
    tick().then(() => applyImageVisibility());
  });

  // 漫画横向：窗口内容变化/切换「仅图片」时，测量横向窗口尺寸。
  $effect(() => {
    void mangaWindow;
    const on = mangaHorizontal();
    void config.reader.manga.hideText;
    if (!on) return;
    tick().then(() => {
      if (!mangaHorizontal()) return;
      measureManga();
    });
  });

  // 单本字号/图片缩放/适应宽度/适应高度/方案变化后重新测量（页模式页数、滚动模式高度、漫画窗口宽度。
  $effect(() => {
    void effFontSize();
    void effImgScale();
    void effFitWidth();
    void effFitHeight();
    void effShowImages();
    void effFontFamily();
    void effLatinFont();
    void effLineHeight();
    void effParaSpacing();
    void effIndent();
    void effTextAlign();
    void effFontWeight();
    void effCssExtra();
    void config.reader.manga.marginH;
    void config.reader.manga.marginV;
    void config.reader.manga.imgGap;
    void config.reader.manga.layout;
    tick().then(() => scheduleRelayout());
  });

  // 漫画模式开关 / 布局切换：
  //  - 进入漫画模式：按当前章重建跨章连续窗口（否则 mangaWindow 为空 → 空白）。
  //  - 退出漫画模式：恢复普通单章渲染（重测分页/滚动）。
  //  - 布局切换（flow ⇄ horizontal）：两种容器不同，需按当前章重建窗口。
  // 初次打开由 applyChapter 直接调用 loadMangaWindow，这里用 lastMangaOn 初始化避免重复加载。
  let lastMangaLayout: string | null = null;
  let lastMangaOn: boolean | null = null;
  $effect(() => {
    const lay = config.reader.manga.layout;
    const on = isMangaMode();
    if (lastMangaOn === null) {
      lastMangaOn = on;
      lastMangaLayout = lay;
      return;
    }
    const modeChanged = on !== lastMangaOn;
    const layoutChanged = on && lay !== lastMangaLayout;
    lastMangaOn = on;
    lastMangaLayout = lay;
    if (!book) return;
    if (modeChanged) {
      if (on) loadMangaWindow(chapterNum);
      else loadChapter(chapterNum);
      return;
    }
    if (layoutChanged) loadMangaWindow(chapterNum);
  });

  const bgImageUrl = $derived(
    (bookScheme()?.schemeBgImage ?? config.reader.bgImage)
      ? `http://tidepage.localhost/bg/${bookScheme()?.schemeBgImage ?? config.reader.bgImage}`
      : ""
  );

  function zoneName(k: ZoneKey): string {
    const a = config.reader.zones[k];
    if (!a || a === "none") return "无";
    return BIND_DEFS.find((d) => d.id === a)?.name ?? a;
  }

  async function loadBookmarks() {
    if (!book) return;
    bookmarks = await api.listBookmarks(book.book.id);
    setRight("bookmarks");
  }

  async function addBookmark() {
    if (!book || !chapter) return;
    const sel = window.getSelection()?.toString().slice(0, 80) ?? "";
    await api.addBookmark(book.book.id, chapterNum, chapter.title, sel);
    toast("书签已添加");
    if (bookmarksOpen) bookmarks = await api.listBookmarks(book.book.id);
    // 广播给独立“书签”窗口（按 bookId 过滤后重拉列表）
    void emit("tidepage-bookmarks-changed", { bookId: book.book.id });
  }

  async function deleteBookmark(id: number) {
    try {
      await api.deleteBookmark(id);
      bookmarks = bookmarks.filter((b) => b.id !== id);
      toast("书签已删除");
      // 广播给独立“书签”窗口（按 bookId 过滤后重拉列表）
      if (book) void emit("tidepage-bookmarks-changed", { bookId: book.book.id });
    } catch (e) {
      toast(String(e), "error");
    }
  }

  /* ================= 划线 / 划词菜单 / 检索 ================= */

  const chapterHighlights = $derived(highlights.filter((h) => h.chapter_index === chapterNum));

  $effect(() => {
    void chapter;
    void chapterNum;
    void highlights;
    void hlVersion;
    tick().then(() => paintNow());
  });

  function contentRoot(): HTMLElement | null {
    return (contentEl?.querySelector("#chapter-content") as HTMLElement | null) ?? null;
  }

  function paintNow() {
    const root = contentRoot();
    if (!root) return;
    paintHighlights(root, chapterHighlights);
  }

  async function refreshHighlights() {
    if (!book) return;
    try {
      highlights = await api.listHighlights(book.book.id);
      hlVersion++;
    } catch { /* ignore */ }
  }

  function closeSelectionMenu() {
    selMenu = null;
  }

  function rectAnchor(rect: DOMRect): PopupAnchor {
    return { top: rect.top, bottom: rect.bottom, cx: rect.left + rect.width / 2 };
  }

  function showSelectionMenu(text: string, offs: { start: number; end: number }, rect: DOMRect) {
    selMenu = { text, start: offs.start, end: offs.end, anchor: rectAnchor(rect) };
  }

  async function onHighlight(style: string, color: string) {
    const s = selMenu;
    selMenu = null;
    if (!s || !book || !chapter) return;
    try {
      const id = await api.addHighlight({
        bookId: book.book.id,
        chapterIndex: chapterNum,
        chapterTitle: chapter.title,
        start: s.start,
        end: s.end,
        text: s.text,
        style,
        color,
        note: "",
      });
      highlights.push({
        id,
        book_id: book.book.id,
        book_title: book.book.title,
        chapter_index: chapterNum,
        chapter_title: chapter.title,
        pos_start: s.start,
        pos_end: s.end,
        text: s.text,
        style,
        color,
        note: "",
        created_at: Date.now(),
      });
      hlVersion++;
      window.getSelection()?.removeAllRanges();
      await tick();
      paintNow();
      toast("已划线");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function closeHlMenu() {
    hlMenu = null;
  }



  async function hlSetColor(h: HighlightRow, color: string) {
    try {
      await api.updateHighlight(h.id, h.style, color, h.note);
      h.color = color;
      hlVersion++;
      await tick();
      paintNow();
      if (hlMenu) hlMenu = { ...hlMenu, h };
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function hlSetStyle(h: HighlightRow, style: string) {
    try {
      await api.updateHighlight(h.id, style, h.color, h.note);
      h.style = style;
      hlVersion++;
      await tick();
      paintNow();
      if (hlMenu) hlMenu = { ...hlMenu, h };
    } catch (e) {
      toast(String(e), "error");
    }
  }

  /** H2：悬停显示感想（仅对有 note 的划线）。 */
  let hoverNote = $state<{ note: string; text: string; x: number; y: number } | null>(null);

  function onContentHover(e: MouseEvent) {
    const markEl = (e.target as HTMLElement | null)?.closest?.("mark.fr-hl") as HTMLElement | null;
    if (!markEl) {
      if (hoverNote) hoverNote = null;
      return;
    }
    const id = Number(markEl.dataset.hlid);
    const h = highlights.find((x) => x.id === id);
    if (!h || !h.note || !h.note.trim()) {
      if (hoverNote) hoverNote = null;
      return;
    }
    const r = markEl.getBoundingClientRect();
    hoverNote = { note: h.note, text: h.text, x: r.left + r.width / 2, y: r.top };
  }

  /** H1：保存感想（写入 highlight.note；空串=取消笔记、回到纯划线）。 */
  async function hlSetNote(h: HighlightRow, note: string) {
    try {
      await api.setHighlightNote(h.id, note);
      h.note = note;
      hlVersion++;
      if (hlMenu) hlMenu = { ...hlMenu, h };
      await tick();
      paintNow();
      toast(note.trim() ? "已保存感想" : "已取消笔记");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  /** H3：把 AI 输出追加到某条划线的 note（不覆盖已有感想）。 */
  async function hlAppendNote(h: HighlightRow, addition: string) {
    try {
      const next = await api.appendHighlightNote(h.id, addition);
      h.note = next;
      hlVersion++;
      toast("已添加到笔记");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function hlDelete(h: HighlightRow) {
    const ok = await appConfirm(`删除这条划线？\n${h.text.slice(0, 60)}`);
    if (!ok) return;
    try {
      await api.deleteHighlight(h.id);
      const i = highlights.findIndex((x) => x.id === h.id);
      if (i >= 0) highlights.splice(i, 1);
      hlMenu = null;
      hlVersion++;
      await tick();
      paintNow();
      toast("已删除划线");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function onSearchSelection() {
    const s = selMenu;
    selMenu = null;
    if (!s) return;
    searchOrigin = { chapter_index: chapterNum, ratio: currentRatio() };
    searchNavStack = [];
    searchPopup = { query: s.text, anchor: s.anchor };
  }

  async function onBrowserSelection() {
    const s = selMenu;
    selMenu = null;
    if (!s) return;
    const tpl = config.reader.browserSearchUrl || "https://www.baidu.com/s?wd=%s";
    const url = tpl.includes("%s")
      ? tpl.replace("%s", encodeURIComponent(s.text))
      : tpl + encodeURIComponent(s.text);
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch {
      try { window.open(url, "_blank"); } catch { /* ignore */ }
    }
  }

  function onAiSelection() {
    const s = selMenu;
    selMenu = null;
    if (!s) return;
    // 携带选区信息：QuickAiPopup「添加到感想」在新划选来源时据此创建划线
    quickAiPopup = { text: s.text, anchor: s.anchor, sel: { start: s.start, end: s.end, text: s.text } };
  }

  /** 判重（口径对齐 src-tauri/src/db.rs::append_highlight_note）：
   *  db 拼接形式为 `format!("{}\n\n{}", cur.trim_end(), addition.trim())`，首段为 addition.trim()，
   *  各段之间恰以 "\n\n" 相连。故按「以 \n\n 为边界、匹配完整 add 块」判重（add 内部可含空行不误判）。 */
  function noteHasSegment(note: string | undefined, addition: string): boolean {
    const add = addition.trim();
    if (!add) return false;
    const c = (note ?? "").trim();
    if (!c) return false;
    if (c === add) return true;                          // 唯一一段即等
    if (c.startsWith(add + "\n\n")) return true;         // 位于开头块
    if (c.endsWith("\n\n" + add)) return true;           // 位于末尾块
    if (c.includes("\n\n" + add + "\n\n")) return true;  // 位于中间块
    return false;
  }

  /** 任务 7：把 AI 释义结果添加到「感想」(highlight.note)。
   *  来源①已有划线 → appendHighlightNote（幂等判重）；
   *  来源②新划选（无 highlightId）→ 先按 onHighlight 方式创建划线拿 id，再写入 note。
   *  返回 true 表示已处理（添加/已存在，按钮置「已添加」）；false 表示失败/无选区（按钮保持可点）。 */
  async function quickAiAddNote(output: string): Promise<boolean> {
    const q = quickAiPopup;
    const add = output.trim();
    if (!q || !add) {
      toast("无可添加内容", "error");
      return false;
    }
    // 来源①：已有划线
    if (q.highlightId != null) {
      const h = highlights.find((x) => x.id === q.highlightId);
      if (h && noteHasSegment(h.note, add)) {
        toast("感想中已有相同内容");
        return true;
      }
      try {
        const next = await api.appendHighlightNote(q.highlightId, add);
        if (h) h.note = next;
        hlVersion++;
        await tick();
        paintNow();
        toast("已添加到感想");
        return true;
      } catch (e) {
        toast(String(e), "error");
        return false;
      }
    }
    // 来源②：新划选 → 先建划线（note 空）拿 id，再写入 note；写 note 失败则回滚删除，杜绝无 note 残留划线
    const s = q.sel;
    if (!s || !book || !chapter) {
      toast("无有效选区，无法添加到感想", "error");
      return false;
    }
    const style = config.reader.highlightStyle || "underline";
    const color = config.reader.highlightColor || config.reader.highlightColors[0] || "#c0392b";
    let id: number;
    try {
      id = await api.addHighlight({
        bookId: book.book.id,
        chapterIndex: chapterNum,
        chapterTitle: chapter.title,
        start: s.start,
        end: s.end,
        text: s.text,
        style,
        color,
        note: "",
      });
    } catch (e) {
      toast(`创建划线失败：${e}`, "error");
      return false;
    }
    // 本地先插入（note 空），便于回滚时精确移除
    highlights.push({
      id,
      book_id: book.book.id,
      book_title: book.book.title,
      chapter_index: chapterNum,
      chapter_title: chapter.title,
      pos_start: s.start,
      pos_end: s.end,
      text: s.text,
      style,
      color,
      note: "",
      created_at: Date.now(),
    });
    hlVersion++;
    try {
      await api.setHighlightNote(id, add);
    } catch (e) {
      // 写 note 失败 → 尝试回滚删除刚建的划线；据删除真实结果给一致文案，绝不谎称"已回滚"
      let deleted = false;
      try { await api.deleteHighlight(id); deleted = true; } catch { /* 删除也失败 */ }
      const ri = highlights.findIndex((x) => x.id === id);
      if (deleted) {
        // 服务端已删 → 本地一并移除，状态一致
        if (ri >= 0) highlights.splice(ri, 1);
        hlVersion++;
        await tick();
        paintNow();
        toast(`写入感想失败，已回滚新建划线：${e}`, "error");
      } else {
        // 服务端未删成功 → 保留本地行（note 空，与服务端一致），如实提示可能残留
        hlVersion++;
        await tick();
        paintNow();
        toast(`写入感想失败，且划线可能残留（自动删除未成功），请在划线面板手动删除`, "error");
      }
      return false;
    }
    const nh = highlights.find((x) => x.id === id);
    if (nh) nh.note = add;
    hlVersion++;
    // 重复点击时走「已有划线」判重分支，避免重复创建
    quickAiPopup = { ...q, highlightId: id };
    window.getSelection()?.removeAllRanges();
    await tick();
    paintNow();
    toast("已创建划线并添加到感想");
    return true;
  }

  function scrollRangeIntoView(range: Range) {
    const sel = window.getSelection();
    if (sel) {
      sel.removeAllRanges();
      sel.addRange(range);
    }
    scrollToRange(range);
  }

  /** 仅滚动到指定 Range（不改动当前选区），供朗读跟随使用。 */
  function scrollToRange(range: Range) {
    if (mangaHorizontal()) return; // 横向布局无纵向滚动的跟随语义
    if (scrollLayout()) {
      if (!contentEl || !scroller) return;
      const contentRect = contentEl.getBoundingClientRect();
      const r = range.getBoundingClientRect();
      const target = scrollRender + (r.top - contentRect.top) - contentRect.height * 0.3;
      scroller.jumpTo(Math.max(0, Math.min(target, maxScroll)), true);
    } else {
      if (!pageWrap) return;
      const wrapRect = pageWrap.getBoundingClientRect();
      const r = range.getBoundingClientRect();
      const left = r.left - wrapRect.left;
      const pg = Math.max(0, Math.min(pageCount - 1, Math.floor(left / (stepPx || 1))));
      gotoPage(pg, true);
    }
  }

  async function jumpToChapterText(chapterIndex: number, needle: string) {
    if (!book) return;
    // 记录跳转前位置，便于返回；搜索框保持打开
    searchNavStack = [...searchNavStack, { chapter_index: chapterNum, ratio: currentRatio() }];
    if (chapterIndex !== chapterNum || !chapter) {
      await loadChapter(chapterIndex);
    }
    await tick();
    paintNow();
    const root = contentRoot();
    if (!root) { alignStatsBaseline(); return; }
    const range = findTextRange(root, needle);
    if (!range) {
      toast("已跳转到章节（未精确定位到文字）");
      alignStatsBaseline(); // 搜索跳转落点：对齐统计基准
      return;
    }
    scrollRangeIntoView(range);
    alignStatsBaseline(); // 搜索跳转落点：对齐统计基准
  }

  async function gotoNavPos(p: NavPos) {
    if (p.chapter_index !== chapterNum || !chapter) {
      await loadChapter(p.chapter_index);
      await tick();
    }
    applyRatio(p.ratio);
    alignStatsBaseline(); // 搜索“返回”跳转落点：对齐统计基准
  }

  /** 返回上一次跳转前的位置；已在最初位置则返回搜索开始前的位置。 */
  async function searchGoBack() {
    const prev = searchNavStack.length
      ? searchNavStack[searchNavStack.length - 1]
      : searchOrigin;
    if (!prev) return;
    if (searchNavStack.length) searchNavStack = searchNavStack.slice(0, -1);
    await gotoNavPos(prev);
  }

  async function jumpToHighlight(h: HighlightRow) {
    if (rightPanel === "highlights") rightPanel = null;
    if (!book) return;
    if (h.book_id !== book.book.id) {
      pendingHighlightJump = h;
      requestOpenBook(h.book_id);
      return;
    }
    if (h.chapter_index !== chapterNum) {
      await loadChapter(h.chapter_index);
      await tick();
    } else {
      await tick();
    }
    paintNow();
    const root = contentRoot();
    if (!root) return;
    const range = charRangeToRange(root, h.pos_start, h.pos_end);
    if (range) scrollRangeIntoView(range);
    else toast("未定位到划线位置");
    alignStatsBaseline(); // 划线跳转落点：对齐统计基准
  }

  /* ================= 语音朗读（TTS） ================= */

  /** 朗读跟随：高亮当前句并滚动到可见位置。 */
  function onTtsSegment(seg: FlatSegment | null) {
    ttsCurrent = seg;
    const root = contentRoot();
    if (seg && ttsTextLen > 0) {
      ttsRatio = Math.max(0, Math.min(1, seg.start / ttsTextLen));
    }
    if (root) {
      if (!seg || !config.tts.highlight) {
        clearTtsMark(root);
      } else {
        paintTtsMark(root, seg.start, seg.end);
        const range = charRangeToRange(root, seg.start, seg.end);
        if (range) scrollToRange(range);
      }
    }
    // 朗读时按「读到的字符位置」保存进度（节流），关掉软件重开可回到朗读处。
    if (seg) {
      const now = Date.now();
      if (now - ttsSavedAt > 1500) {
        ttsSavedAt = now;
        void doSaveProgress(ttsRatio);
      }
    }
  }

  /** 任意 Range 起点在 root 纯文本中的字符偏移（对元素/文本节点都稳健）。 */
  function offsetOfRange(root: Node, range: Range): number | null {
    try {
      const r = document.createRange();
      r.selectNodeContents(root);
      r.setEnd(range.startContainer, range.startOffset);
      return r.toString().length;
    } catch {
      return null;
    }
  }

  /** 视口内某点对应的章节文本偏移。 */
  function offsetAtPoint(root: HTMLElement, x: number, y: number): number | null {
    const doc = document as Document & {
      caretRangeFromPoint?: (x: number, y: number) => Range | null;
      caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
    };
    let range: Range | null = null;
    if (doc.caretRangeFromPoint) {
      range = doc.caretRangeFromPoint(x, y);
    } else if (doc.caretPositionFromPoint) {
      const p = doc.caretPositionFromPoint(x, y);
      if (p) {
        range = document.createRange();
        range.setStart(p.offsetNode, p.offset);
      }
    }
    if (!range || !root.contains(range.startContainer)) return null;
    return offsetOfRange(root, range);
  }

  /** 朗读定位用的「可见文本框」：翻页模式是 .clip（左右已除外边距的正文窗口），
   *  滚动模式是整个 .content。注意不能用 .content 作为翻页模式的判定框：它包含
   *  两侧留白，会把被裁掉的相邻栏也判为“可见”，导致从本页开始时定位到更靠前的页。
   */
  function visibleTextBox(): DOMRect | null {
    if (!scrollLayout() && !mangaHorizontal()) {
      const clip = contentEl?.querySelector(".clip") as HTMLElement | null;
      if (clip) return clip.getBoundingClientRect();
    }
    return contentEl ? contentEl.getBoundingClientRect() : null;
  }

  /** 偏移对应的实际渲染矩形是否落在可见文本框内。 */
  function offsetVisibleInBox(root: HTMLElement, off: number, vr: DOMRect): boolean {
    const r = charRangeToRange(root, off, off + 1);
    if (!r) return false;
    const rc = r.getBoundingClientRect();
    return rc.bottom > vr.top + 1 && rc.top < vr.bottom - 1 && rc.right > vr.left + 1 && rc.left < vr.right - 1;
  }

  /** 第一个「可见」文本节点对应的章节文本偏移。以真实可见文本框判定，
   *  并返回可见片段起始字符的精确偏移（文本节点可能跨栏/跨页）。 */
  function firstVisibleOffset(root: HTMLElement): number | null {
    const vr = visibleTextBox();
    if (!vr) return null;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let acc = 0;
    let n = walker.nextNode() as Text | null;
    while (n) {
      const val = n.nodeValue ?? "";
      const len = val.length;
      if (len && val.trim()) {
        const r = document.createRange();
        r.selectNodeContents(n);
        const rects = r.getClientRects();
        for (let i = 0; i < rects.length; i++) {
          const rc = rects[i];
          const visX = rc.right > vr.left + 2 && rc.left < vr.right - 2;
          const visY = rc.bottom > vr.top + 2 && rc.top < vr.bottom - 2;
          if (visX && visY) {
            const px = Math.max(rc.left + 2, vr.left + 2);
            const py = Math.min(Math.max(rc.top + 2, vr.top + 2), vr.bottom - 2, rc.bottom - 2);
            const exact = offsetAtPoint(root, px, py);
            if (exact != null && offsetVisibleInBox(root, exact, vr)) return exact;
            return acc;
          }
        }
      }
      acc += len;
      n = walker.nextNode() as Text | null;
    }
    return null;
  }

  /** 当前屏幕顶部（可见页）起始处的章节文本偏移，用于「从本页开始朗读」。 */
  function offsetAtViewportTop(root: HTMLElement): number | null {
    const walkerHit = firstVisibleOffset(root);
    if (walkerHit != null) return walkerHit;
    // 兜底：在真实可见文本框内按光标点取偏移（并校验该偏移确实可见）。
    const vr = visibleTextBox();
    if (!vr) return null;
    const xs = [0.02, 0.05, 0.15, 0.3, 0.5, 0.7, 0.85];
    const ys = [4, 10, 18, 28, 40, 56, 78, 104, 140];
    for (const dy of ys) {
      const y = Math.min(vr.bottom - 2, vr.top + dy);
      if (y <= vr.top + 1) continue;
      for (const fx of xs) {
        const off = offsetAtPoint(root, vr.left + vr.width * fx, y);
        if (off != null && offsetVisibleInBox(root, off, vr)) return off;
      }
    }
    return 0;
  }

  type TtsStartMode = "auto" | "page" | "chapter";

  /**
   * 开始朗读。
   * auto=优先选中处，其次本页顶部；page=本页顶部；chapter=章节开头。
   */
  function startTts(mode: TtsStartMode = "auto") {
    if (!ttsState.supported) {
      toast("当前环境不支持系统语音朗读", "error");
      return;
    }
    const root = contentRoot();
    if (!root || !chapter) return;
    const text = root.textContent ?? "";
    const segs = splitSegments(text);
    if (!segs.length) {
      toast("本章没有可朗读的文字");
      return;
    }
    let off: number | null = null;
    if (mode === "auto") {
      const sel = window.getSelection();
      if (sel && !sel.isCollapsed && sel.rangeCount) {
        const o = selectionOffsets(root, sel.getRangeAt(0));
        if (o) off = o.start;
      }
    }
    if (mode !== "chapter" && off == null) off = offsetAtViewportTop(root);
    const idx = off != null ? segIndexAt(segs, off) : 0;
    ttsTextLen = text.length;
    ttsRatio = ttsTextLen > 0 ? Math.max(0, Math.min(1, segs[idx].start / ttsTextLen)) : 0;
    ttsSavedAt = Date.now();
    ttsSetSegments(segs);
    ttsStart(idx);
  }

  /** 当前是否处于本章第一页/顶部。 */
  function atChapterStart(): boolean {
    if (mangaHorizontal()) return mangaPosX <= 24;
    if (scrollLayout()) return scrollRender <= 24;
    return page <= 0;
  }

  /** 点朗读时：不在章节开头则先问从本页还是从章节开头。 */
  function beginTts() {
    if (ttsState.playing) {
      ttsPauseResume();
      return;
    }
    if (atChapterStart()) {
      startTts("auto");
      return;
    }
    const r = (contentEl ?? bodyEl)?.getBoundingClientRect();
    const anchor: PopupAnchor = r
      ? { top: r.top, bottom: r.top + 1, cx: r.left + r.width / 2 }
      : { top: 80, bottom: 81, cx: window.innerWidth / 2 };
    ttsStartChoice = { anchor };
  }

  function ttsStartFromPage() {
    ttsStartChoice = null;
    startTts("page");
  }

  function ttsStartFromChapter() {
    ttsStartChoice = null;
    startTts("chapter");
  }

  /** 停止朗读并清除高亮。 */
  function stopTts() {
    const wasActive = ttsState.playing || ttsState.paused;
    const ratio = ttsRatio;
    ttsStop();
    const root = contentRoot();
    if (root) clearTtsMark(root);
    ttsCurrent = null;
    // 仅当确实在朗读时才落盘，避免“未朗读点停止”把已有阅读进度覆盖成 0。
    if (wasActive && book) void doSaveProgress(ratio);
  }

  /** 本章读完：按设置自动续读下一章。 */
  async function onTtsChapterEnd() {
    if (!config.tts.autoNextChapter || !book) return;
    // 本章已读完，先把进度记到章末，重开时不会退回本章开头。
    void doSaveProgress(1);
    if (chapterNum >= book.toc.length - 1) {
      toast("已朗读到最后一章");
      return;
    }
    // 此时 ttsState.playing 已为 false，loadChapter 不会误杀朗读
    await loadChapter(chapterNum + 1);
    await tick();
    startTts("chapter");
  }

  function toggleTts() {
    beginTts();
  }

  function convertThis() {
    if (!book) return;
    convertState.book = {
      id: book.book.id,
      title: book.book.title,
      path: book.book.path,
      format: book.book.format,
    };
    convertState.open = true;
  }

  let progDrag = $state(false);
  let progLastRatio = 0;
  let progBusy = false;
  let progReq = false;

  function applyRatio(r: number) {
    if (mangaHorizontal()) {
      mangaPosX = r * mangaMaxX;
      clampManga();
      applyMangaPan();
    } else if (scrollLayout()) scroller?.jumpTo(r * maxScroll, true);
    else gotoPage(Math.round(r * (pageCount - 1)), true);
  }

  function chapterFromRatio(ratio: number) {
    if (!book) return { idx: 0, cr: 0 };
    const n = book.toc.length;
    const scaled = ratio * n;
    const idx = Math.max(0, Math.min(n - 1, Math.floor(scaled)));
    const cr = Math.max(0, Math.min(1, scaled - idx));
    return { idx, cr };
  }

  async function progSeek(ratio: number) {
    if (!book) return;
    progLastRatio = ratio;
    const { idx, cr } = chapterFromRatio(ratio);
    if (idx === chapterNum) {
      applyRatio(cr);
      alignStatsBaseline(); // 同章内进度条拖拽落点也对齐基准
      return;
    }
    if (progBusy) {
      progReq = true;
      return;
    }
    progBusy = true;
    do {
      progReq = false;
      const t = chapterFromRatio(progLastRatio);
      await loadChapter(t.idx);
      applyRatio(t.cr);
    } while (progReq);
    progBusy = false;
    alignStatsBaseline(); // 进度条跳转落点：对齐统计基准，避免大跳位移被计为已读字数
  }

  function progFromEvent(e: PointerEvent) {
    const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect();
    return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  }

  function progDown(e: PointerEvent) {
    progDrag = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    progSeek(progFromEvent(e));
  }

  function progMove(e: PointerEvent) {
    if (progDrag) progSeek(progFromEvent(e));
  }

  /** 进度条键盘操作：←/→ 微调 1%，Shift 加速 5%，Home/End 到首尾。 */
  function progKey(e: KeyboardEvent) {
    let delta = 0;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") delta = e.shiftKey ? 5 : 1;
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") delta = e.shiftKey ? -5 : -1;
    else if (e.key === "Home") delta = -Infinity;
    else if (e.key === "End") delta = Infinity;
    else return;
    e.preventDefault();
    const next = Math.max(0, Math.min(100, totalPct() + delta)) / 100;
    void progSeek(next).then(() => saveProgressDebounced());
  }

  /* ================= 顶栏 / 底栏 状态项 ================= */

  let nowTs = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (nowTs = Date.now()), 20000);
    return () => clearInterval(t);
  });

  let readSpeed = $state(0);
  let speedBuf: { t: number; pos: number }[] = [];
  $effect(() => {
    const t = setInterval(() => {
      if (!book) return;
      const now = Date.now();
      speedBuf.push({ t: now, pos: bookPos() });
      if (speedBuf.length > 60) speedBuf.splice(0, speedBuf.length - 60);
      const recent = speedBuf.filter((s) => s.t >= now - 45000);
      if (recent.length >= 2) {
        const dPos = recent[recent.length - 1].pos - recent[0].pos;
        const dtMin = (recent[recent.length - 1].t - recent[0].t) / 60000;
        readSpeed = dtMin > 0 ? Math.max(0, Math.round(dPos / dtMin)) : 0;
      } else {
        readSpeed = 0;
      }
    }, 4000);
    return () => clearInterval(t);
  });

  /** 全书总页数（按当前章“每页多少字”估算；仅翻页模式有效）。 */
  function estTotalPages(): number {
    if (!book || scrollLayout() || mangaHorizontal()) return 0;
    const chars = chapter?.chars || 0;
    if (pageCount <= 0 || chars <= 0) return 0;
    const cpp = chars / pageCount;
    const total = book.toc.reduce((s, t) => s + (t.chars || 0), 0);
    if (cpp <= 0 || total <= 0) return 0;
    return Math.max(1, Math.round(total / cpp));
  }

  function statusItemText(id: StatusItemId): string {
    if (!book) return "";
    switch (id) {
      case "bookTitle":
        return book.book.title || "";
      case "chapterName":
        return chapter?.title || "";
      case "chapterIndex":
        return `第${chapterNum + 1}章`;
      case "chapterCount":
        return `共${book.toc.length}章`;
      case "chapterPages":
        return !scrollLayout() && !mangaHorizontal()
          ? `${page + 1}/${pageCount}页`
          : `${Math.round(currentRatio() * 100)}%`;
      case "bookPages": {
        const tp = estTotalPages();
        return tp ? `共约${tp}页` : "";
      }
      case "pagePercent":
        return `${Math.round(currentRatio() * 100)}%`;
      case "bookProgress":
        return `${totalPct().toFixed(1)}%`;
      case "chars": {
        const n = book.toc[chapterNum]?.chars || 0;
        return n ? `${n}字` : "";
      }
      case "clock": {
        const d = new Date(nowTs);
        return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      }
      case "speed":
        return readSpeed > 0 ? `${readSpeed}字/分` : "";
      case "mode":
        if (mangaHorizontal()) return "横向";
        if (mangaFlow()) return "上下滚动";
        return config.reader.mode === "page" ? (config.reader.duo ? "双栏" : "翻页") : "滚动";
      default:
        return "";
    }
  }

  const enabledStatusItems = $derived(config.reader.statusItems.filter((i) => i.enabled));

  function footerItems(ids: StatusItemId[]): { id: StatusItemId; text: string }[] {
    if (!book) return [];
    return ids.map((id) => ({ id, text: statusItemText(id) })).filter((x) => x.text);
  }

  /** 状态项拖拽排序（与顶栏按钮共用 dragSort 视觉）。 */
  function statusDrop(from: string, target: string, place: "before" | "after") {
    if (!from || from === target) return;
    const arr = config.reader.statusItems.map((x) => ({ ...x }));
    const fi = arr.findIndex((x) => x.id === from);
    if (fi < 0) return;
    const [m] = arr.splice(fi, 1);
    const ti = arr.findIndex((x) => x.id === target);
    if (ti < 0) return;
    arr.splice(place === "after" ? ti + 1 : ti, 0, m);
    config.reader.statusItems = arr;
    saveConfig();
  }



  function progUp(e: PointerEvent) {
    if (!progDrag) return;
    progDrag = false;
    progSeek(progFromEvent(e)).then(() => saveProgressDebounced());
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<div
  class="reader"
  class:ovr={effOverrideCss()}
  class:fitw={effFitWidth()}
  class:fith={effFitHeight()}
  class:manga={isMangaMode()}
  class:manga-h={mangaHorizontal()}
  class:hide-text={isMangaMode() && config.reader.manga.hideText}
  class:no-images={!effShowImages()}
  style={readerVars()}
  onwheel={onWheel}
  onmousemove={onReaderMove}
  onmouseleave={() => (fontCtlShow = false)}
>
  {#if bgImageUrl}
    <div class="bg-layer" style="background-image:url('{bgImageUrl}'); opacity:{bookScheme()?.schemeBgOpacity ?? config.reader.bgOpacity};"></div>
  {/if}
  {#if showFsHint}
    <div class="fs-hint" transition:fade={{ duration: 220 }}>按 F11 退出全屏</div>
  {/if}
  <div class="topbar" class:dim={chromeHidden} style="background:var(--rdr-top);" data-tauri-drag-region bind:clientHeight={tbH}>
    <div class="tb-zone tb-l" data-tauri-drag-region>
      <button
        class="icon tb-btn tip-right"
        aria-label={standalone ? "关闭" : "返回"}
        data-tip={standalone ? "关闭 (Esc)" : "返回 (Esc)"}
        onclick={goBack}
      >
        <IconArrowLeft size={20} stroke={1.75} />
      </button>
      <div class="btitle" data-tauri-drag-region>{book?.book.title ?? ""}</div>
    </div>
    {#if config.reader.statusPos !== "none" && enabledStatusItems.length}
      <div
        class="tb-zone tb-c"
        data-tauri-drag-region
        use:dragSort={{ onDrop: statusDrop, axis: "x" }}
        style="justify-content:{config.reader.statusPos === 'left' ? 'flex-start' : config.reader.statusPos === 'right' ? 'flex-end' : 'center'}"
      >
        {#each enabledStatusItems as it, i (it.id)}
          {#if i > 0}<span class="stat-sep">·</span>{/if}
          <span
            class="stat-item"
            data-drag-id={it.id}
            style="color:var(--rdr-stat)"
          >{statusItemText(it.id)}</span>
        {/each}
      </div>
    {/if}
    <div class="tb-zone tb-r" use:dragSort={{ onDrop: tbDrop }}>
      {#each visibleTopbarBtns as b (b.id)}
        {@const Icon = b.icon()}
        {@const label = tbTitle(b)}
        <button
          class="icon tb-btn tip-left"
          class:on={b.on?.()}
          data-drag-id={b.id}
          aria-label={label}
          data-tip={label}
          onpointerdown={(e) => tbPointerDown(b, e)}
          onpointerup={tbPointerUp}
          onpointerleave={tbPointerUp}
          onpointercancel={tbPointerUp}
          onclick={() => tbClick(b)}
        >
          <Icon size={20} stroke={1.75} />
        </button>
      {/each}
    </div>
  </div>

  {#if tbManagerOpen}
    <div class="tb-panel tb-manager" role="dialog" aria-label="顶栏按钮管理">
      <div class="tb-panel-head"><span>顶栏按钮</span><button class="icon" title="关闭" onclick={() => (tbManagerOpen = false)}>×</button></div>
      <div class="tb-panel-body">
        <div class="tb-chip-grid">
          {#each config.reader.topbarOrder as id (id)}
            {#if topbarBtn(id)}
              {@const b = topbarBtn(id)!}
              {@const shown = id === "manager" || !config.reader.topbarHidden.includes(id)}
              <label class="tb-chip" class:on={shown} class:fixed={id === "manager"} class:na={id !== "manager" && !b.visible()}>
                <input
                  type="checkbox"
                  checked={shown}
                  disabled={id === "manager"}
                  onchange={(e) => toggleTopbarHidden(id, e.currentTarget.checked)}
                />
                <span class="tb-chip-label">{b.label}</span>
              </label>
            {/if}
          {/each}
        </div>
        <p class="tb-panel-hint">在顶栏直接拖动按钮可调整顺序；隐藏的按钮重新显示后回到其保存的顺序位置。</p>
      </div>
    </div>
  {/if}

  {#if imgMenuOpen}
    <div class="tb-panel" role="menu" aria-label="本书图片">
      <div class="tb-panel-head"><span>本书图片（仅当前书）</span><button class="icon" title="关闭" onclick={() => (imgMenuOpen = false)}>×</button></div>
      <div class="tb-panel-body">
        <button class="tb-menu-item" onclick={toggleAllImages}>{effShowImages() ? "全部关闭图片" : "全部开启图片"}</button>
        <button class="tb-menu-item" onclick={startBlockPick}>屏蔽选中图片</button>
        <button class="tb-menu-item" onclick={() => { imgMenuOpen = false; blockedListOpen = true; }}>已屏蔽图片（{blockedImages().length}）</button>
      </div>
    </div>
  {/if}

  {#if wdMenuOpen}
    <!-- 关闭依赖窗口捕获阶段的外点监听（见下方 $effect），不再使用全屏遮罩元素。 -->
    <div class="tb-panel wd-menu" bind:this={wdMenuEl} role="menu" aria-label="WebDAV 上传书籍" style="top:{wdMenuPos.top}px; right:{wdMenuPos.right}px">
      <div class="tb-panel-head"><span>WebDAV（本书）</span><button class="icon" title="关闭" aria-label="关闭" onclick={() => (wdMenuOpen = false)}>×</button></div>
      <div class="tb-panel-body">
        <button class="tb-menu-item" onclick={uploadBook} disabled={!book}>上传书籍到云端</button>
      </div>
    </div>
  {/if}

  {#if blockedListOpen}
    <div class="tb-panel" role="dialog" aria-label="已屏蔽图片">
      <div class="tb-panel-head"><span>已屏蔽图片（仅当前书）</span><button class="icon" title="关闭" onclick={() => (blockedListOpen = false)}>×</button></div>
      <div class="tb-panel-body">
        {#if blockedImages().length === 0}
          <div class="tb-panel-empty">暂无已屏蔽图片</div>
        {:else}
          {#each blockedImages() as src (src)}
            <div class="tb-block-row">
              <img class="tb-block-thumb" src={src} alt="" />
              <button class="tb-menu-item" onclick={() => unblockImage(src)}>取消屏蔽</button>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  {/if}

  {#if mangaGuideOpen}
    <div class="tb-panel manga-guide" role="dialog" aria-label="漫画图片大小">
      <div class="tb-panel-head"><span>检测到漫画书籍</span><button class="icon" title="关闭" onclick={closeMangaGuide}>×</button></div>
      <div class="tb-panel-body">
        <p class="tb-panel-hint">拖动即可**实时预览**本书图片大小。默认值对所有漫画生效（仅首次打开漫画时提示一次），之后可在设置「漫画模式」或阅读界面右下角按单本单独覆盖。</p>
        <div class="manga-slider">
          <span>小</span>
          <input
            type="range"
            min="0.3"
            max="3"
            step="0.1"
            value={mangaGuideScale}
            oninput={onMangaGuideInput}
          />
          <span>大</span>
          <span class="manga-val">{mangaGuideScale.toFixed(1)}×</span>
        </div>
        <div class="manga-actions">
          <button class="tb-menu-item" onclick={closeMangaGuide}>完成</button>
          <button class="tb-menu-item" onclick={resetMangaGuide}>恢复默认（1.0×）</button>
        </div>
      </div>
    </div>
  {/if}

  <div class="body" bind:this={bodyEl}>
    {#if openError}
      <div class="empty error-box">
        <div>
          <p>打开失败</p>
          <p class="err-detail">{openError}</p>
          <button onclick={() => (ui.view = "library")}>返回书库</button>
        </div>
      </div>
    {:else if !book}
      <div class="empty">正在打开…</div>
    {:else}
      <div class="stage">
        <div
          class="content"
          bind:this={contentEl}
          onclick={onBodyClick}
          onpointerdown={onContentPointerDown}
          onpointerup={onContentPointerUp}
          onmouseover={onContentHover}
          onmouseleave={() => (hoverNote = null)}
        >
          {#if mangaHorizontal()}
            <div
              class="manga-h-viewport"
              bind:this={mangaViewport}
              onpointerdown={onMangaPointerDown}
              onpointermove={onMangaPointerMove}
              onpointerup={onMangaPointerUp}
              onpointercancel={onMangaPointerCancel}
              ondragstart={(e) => e.preventDefault()}
            >
              <div class="manga-h-canvas" bind:this={mangaCanvas} class:rtl={mangaRtl()}>
                {#each mangaWindow as item (item.index)}
                  <div class="manga-block" data-index={item.index}>{@html item.html}</div>
                {/each}
              </div>
            </div>
          {:else if mangaFlow()}
            <!-- 漫画瀑布流：跨章连续窗口（纵向），渲染 mangaWindow；原生竖向滚动 + SmoothScroller 共用同一元素。 -->
            <div
              class="scroll-area manga-flow-viewport"
              bind:this={mangaFlowViewport}
              bind:this={scrollArea}
              onscroll={checkMangaWindowExtend}
            >
              <div class="manga-flow-canvas" bind:this={mangaFlowCanvas}>
                {#each mangaWindow as item (item.index)}
                  <div class="manga-block" data-index={item.index}>{@html item.html}</div>
                {/each}
              </div>
            </div>
            <!-- 漫画瀑布流只用原生滚动条（.manga-flow-viewport overflow-y:auto），
                 不渲染为 transform 滚动设计的自绘 .scrollbar，避免右侧出现两条滚动条。 -->
          {:else if scrollLayout()}
            <!-- 普通（非漫画）滚动模式：维持原有单章渲染 + SmoothScroller（overflow:hidden）。不改动。 -->
            <div class="scroll-area" bind:this={scrollArea}>
              <div class="col-scroll">
                <div id="chapter-content">{@html chapter?.html ?? ""}</div>
              </div>
            </div>
            <div
              class="scrollbar"
              class:dragging={sbDragging}
              bind:this={scrollbarEl}
              onpointerdown={onSbPointerDown}
              onpointermove={onSbPointerMove}
              onpointerup={onSbPointerUp}
              onpointercancel={onSbPointerCancel}
              onclick={(e) => e.stopPropagation()}
            >
              <div class="thumb" bind:this={thumbEl} style="height:8%"></div>
            </div>
          {:else}
            <div class="clip">
              <div class="col-page" bind:this={pageWrap}>
                <div id="chapter-content">{@html chapter?.html ?? ""}</div>
              </div>
            </div>
          {/if}
          {#if loadingChapter && !chapter}
            <div class="loading" aria-label="加载中" aria-busy="true">
              <span class="sk sk-title"></span>
              <span class="sk"></span>
              <span class="sk"></span>
              <span class="sk sk-short"></span>
              <span class="sk"></span>
              <span class="sk sk-short"></span>
            </div>
          {/if}
          {#if config.reader.zoneHints}
            <div class="zone-hints">
              {#each ZONE_DEFS as z (z.id)}
                <div class="zh zh-{z.id}">{zoneName(z.id)}</div>
              {/each}
            </div>
          {/if}
        </div>
      </div>
      <div class="footer" class:dim={chromeHidden}>
        <span
          class="finfo finfo-l"
          role="button"
          tabindex="0"
          aria-label="上一章"
          onclick={() => prevChapter()}
          onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); prevChapter(); } }}
        >
          {#each footerItems(config.reader.footerLeft) as x, i (x.id)}
            {#if i > 0}<span class="fsep">·</span>{/if}<span class="fitem">{x.text}</span>
          {/each}
        </span>
        <div
          class="progress-track"
          class:dragging={progDrag}
          role="slider"
          tabindex="0"
          aria-label="阅读进度"
          aria-valuenow={Math.round(totalPct())}
          aria-valuemin={0}
          aria-valuemax={100}
          onpointerdown={progDown}
          onpointermove={progMove}
          onpointerup={progUp}
          onpointercancel={progUp}
          onkeydown={progKey}
        >
          <div class="progress-fill" style="transform:scaleX({totalPct() / 100})"></div>
          <div class="progress-knob" style="left:{totalPct()}%"></div>
        </div>
        <span
          class="finfo finfo-r"
          role="button"
          tabindex="0"
          aria-label="下一章"
          onclick={() => nextChapter()}
          onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); nextChapter(); } }}
        >
          {#each footerItems(config.reader.footerRight) as x, i (x.id)}
            {#if i > 0}<span class="fsep">·</span>{/if}<span class="fitem">{x.text}</span>
          {/each}
        </span>
      </div>
    {/if}

    {#if tocOpen && book}
      <TocPanel toc={book.toc} current={chapterNum} onpick={(i) => { tocOpen = false; void loadChapter(i).then(alignStatsBaseline); }} onclose={() => (tocOpen = false)} onpopout={() => popOutPanel("toc")} />
    {/if}

    {#if bookmarksOpen}
      <div class="sidepanel">
        <div class="sp-head" use:dragOutHandle={() => popOutPanel("bookmarks")} title="按住标题栏拖出面板外可弹为独立窗口"><span>书签</span><div style="display:flex; gap:6px; align-items:center;"><button class="icon" title="弹出为独立窗口" onclick={() => popOutPanel("bookmarks")}>⧉</button><button class="icon" title="关闭" onclick={() => (rightPanel = null)}>×</button></div></div>
        <div class="sp-list">
          {#each bookmarks as bm (bm.id)}
            <div class="sp-item bm-item">
              <div class="bm-main" onclick={() => { void loadChapter(bm.chapter_index).then(alignStatsBaseline); rightPanel = null; }}>
                <div class="bm-title">{bm.title || "未命名"}</div>
                {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
              </div>
              <button class="icon bm-del" title="删除书签" onclick={() => deleteBookmark(bm.id)}>×</button>
            </div>
          {:else}
            <div class="sp-empty">暂无书签，点 ?? 添加</div>
          {/each}
        </div>
      </div>
    {/if}

    {#if highlightsOpen && book}
      <HighlightsPanel
        bookId={book.book.id}
        version={hlVersion}
        onjump={jumpToHighlight}
        onclose={() => (rightPanel = null)}
        onchanged={refreshHighlights}
        onpopout={() => popOutPanel("highlights")}
      />
    {/if}

    {#if selMenu}
      <SelectionMenu
        anchor={selMenu.anchor}
        onhighlight={onHighlight}
        onsearch={onSearchSelection}
        onbrowser={onBrowserSelection}
        onai={onAiSelection}
        onclose={closeSelectionMenu}
      />
    {/if}

    {#if quickAiPopup}
      <QuickAiPopup
        text={quickAiPopup.text}
        anchor={quickAiPopup.anchor}
        targetHighlightId={quickAiPopup.highlightId ?? null}
        onaddnote={quickAiAddNote}
        onadded={() => { hlVersion++; void refreshHighlights(); }}
        onclose={() => (quickAiPopup = null)}
      />
    {/if}

    {#if hlMenu}
      <HighlightMenu
        highlight={hlMenu.h}
        anchor={hlMenu.anchor}
        oncolor={(c) => hlSetColor(hlMenu!.h, c)}
        onstyle={(s) => hlSetStyle(hlMenu!.h, s)}
        onnote={(n) => hlSetNote(hlMenu!.h, n)}
        onai={() => {
          const h = hlMenu!.h;
          const a = hlMenu!.anchor;
          hlMenu = null;
          quickAiPopup = { text: h.text, anchor: a, highlightId: h.id };
        }}
        ondelete={() => hlDelete(hlMenu!.h)}
        onclose={closeHlMenu}
      />
    {/if}

    {#if ttsState.playing || ttsState.paused}
      <TtsBar current={ttsCurrent} />
    {/if}

    {#if ttsStartChoice}
      <TtsStartMenu
        anchor={ttsStartChoice.anchor}
        onpage={ttsStartFromPage}
        onchapter={ttsStartFromChapter}
        onclose={() => (ttsStartChoice = null)}
      />
    {/if}

    {#if aiDrawerOpen}
      <AiPanel chapterTitle={chapter?.title ?? ""} chapterHtml={chapter?.html ?? ""} onclose={() => (rightPanel = null)} />
    {/if}

    {#if statsOpen && book}
      <BookStatsModal bookId={book.book.id} bookTitle={book.book.title} onclose={() => (rightPanel = null)} onpopout={() => popOutPanel("stats")} />
    {/if}

    {#if book && !openError}
      <div class="font-ctl" class:show={fontCtlShow}>
        <div class="ctl-row">
          <button class="icon" title="减小字号（长按连续）" use:repeatAction={() => bumpFont(-1)}>A−</button>
          <button class="icon" title="增大字号（长按连续）" use:repeatAction={() => bumpFont(1)}>A＋</button>
        </div>
        <div class="ctl-row">
          <button class="icon" title="缩小图片（长按连续）" use:repeatAction={() => bumpImgScale(-0.1)}>图−</button>
          <button class="icon" title="放大图片（长按连续）" use:repeatAction={() => bumpImgScale(0.1)}>图＋</button>
          <button class="icon" class:on={effFitWidth()} title="图片适应阅读器宽度" onclick={toggleFitWidth}>适宽</button>
          <button class="icon" class:on={effFitHeight()} title="图片适应阅读器高度" onclick={toggleFitHeight}>适高</button>
        </div>
      </div>
    {/if}

  </div>

  <!-- 图库放在 .body 之外：.body 有 z-index 形成层叠上下文，放里面会被顶栏(z-index:30)盖住，
       导致右上角 × 点不到（用户以为没有关闭按钮）。放在 .reader 下才能盖住顶栏。 -->
  {#if viewerIndex !== null && book}
    <ImageViewer
      images={bookImages ?? []}
      index={viewerIndex}
      onindex={(i) => (viewerIndex = i)}
      onclose={() => (viewerIndex = null)}
    />
  {/if}

  <!-- H2：悬停划线的感想浮层（仅 note 非空时） -->
  {#if hoverNote}
    <div class="fr-note-tip" style="left:{Math.round(hoverNote.x)}px; top:{Math.round(hoverNote.y)}px;">
      <div class="fr-note-tip-quote">{hoverNote.text.length > 40 ? hoverNote.text.slice(0, 40) + "…" : hoverNote.text}</div>
      <div class="fr-note-tip-body">{hoverNote.note}</div>
    </div>
  {/if}

  <!-- 全书搜索框同样放在 .body 之外：.body 形成层叠上下文会限制其层级，
       放外面才可自由拖到阅读区域之外、并可盖住顶栏。 -->
  {#if searchPopup && book}
    <BookSearchPopup
      bookId={book.book.id}
      initialQuery={searchPopup.query}
      anchor={searchPopup.anchor}
      canBack={searchNavStack.length > 0 || !!searchOrigin}
      onpick={jumpToChapterText}
      onback={searchGoBack}
      onclose={() => { searchPopup = null; searchNavStack = []; searchOrigin = null; }}
    />
  {/if}
</div>


<style>
  .reader {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--rdr-bg, #f5f1e8);
    color: var(--rdr-fg, #3b3630);
    position: relative;
  }

  .bg-layer {
    position: absolute;
    inset: 0;
    background-size: cover;
    background-position: center;
    background-repeat: no-repeat;
    pointer-events: none;
    z-index: 0;
  }

  /* 任务 8：进入全屏时的大字号、居中淡入浮层提示（transition:fade 负责淡入/淡出） */
  .fs-hint {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 9999;
    padding: 22px 40px;
    font-size: 30px;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: #fff;
    background: rgba(0, 0, 0, 0.62);
    border-radius: 14px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.45);
    pointer-events: none;
    white-space: nowrap;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 12px;
    /* 右侧留出自绘窗口控制按钮（约 3×30 + 间距 + 右边距 ≈ 100px）的宽度并留自然间距 */
    padding-right: 112px;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 90%, black 10%);
    z-index: 30;
    position: relative;
    user-select: none;
    /* 顶栏下方 1px 用顶栏背景色补齐：快速滚动时滚动层与顶栏交界处不会露出背景形成亮线/接缝 */
    box-shadow: 0 1px 0 0 var(--rdr-top, #ece7dd);
    /* 让顶栏成为独立稳定合成层，避免滚动中边框/交界被单独合成产生一帧闪烁 */
    transform: translateZ(0);
    transition: opacity var(--dur) var(--ease);
  }

  /* 沉浸阅读：空闲后顶栏/底栏淡出（只动 opacity，不触发布局） */
  .topbar.dim {
    opacity: 0;
    pointer-events: none;
  }

  .footer.dim {
    opacity: 0;
    pointer-events: none;
  }

  /* 分隔线画在顶栏自身图层内部（不用 border-bottom，避免合成层边界抗锯齿闪烁） */
  .topbar::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 1px;
    background: rgba(128, 128, 128, 0.15);
    pointer-events: none;
  }

  .btitle {
    font-size: 13px;
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 300px;
  }

  .tb-zone {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .tb-l {
    gap: 6px;
    flex-shrink: 0;
    max-width: 36%;
  }

  .tb-c {
    flex: 1;
    min-width: 0;
    padding: 0 10px;
    overflow: hidden;
  }

  .tb-r {
    gap: 6px;
    flex-shrink: 0;
    margin-left: auto;
  }

  .stat-item {
    font-size: 13px;
    opacity: 0.85;
    white-space: nowrap;
    cursor: grab;
    border-radius: 4px;
    padding: 0 2px;
  }

  .stat-item:hover {
    background: rgba(128, 128, 128, 0.16);
  }

  .stat-item.dragging {
    opacity: 0.4;
  }

  .stat-sep {
    font-size: 13px;
    opacity: 0.45;
    margin: 0 6px;
  }

  .spacer { flex: 1; }

  /* 顶栏图标按钮：统一为 30×30 方形命中区，图标居中、颜色随 currentColor。 */
  .tb-btn {
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

  /* 顶栏按钮管理 / 本书图片 / 已屏蔽列表 / 漫画引导 弹出面板 */
  .tb-panel {
    position: absolute;
    top: 52px;
    right: 12px;
    width: 290px;
    max-height: 70vh;
    overflow: auto;
    z-index: 60;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 96%, black 4%);
    color: var(--rdr-fg, #333);
    border: 1px solid rgba(128, 128, 128, 0.25);
    border-radius: var(--radius-md);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.1), 0 20px 48px rgba(0, 0, 0, 0.18);
    padding: 8px;
    transform-origin: top right;
    animation: tb-panel-in 0.22s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes tb-panel-in {
    from { opacity: 0; transform: translateY(-8px) scale(0.97); }
    to { opacity: 1; transform: none; }
  }

  /* WebDAV 长按菜单：覆盖 .tb-panel 的 absolute，改用内联 fixed top/right 定位到按钮正下方 */
  .wd-menu {
    position: fixed;
    width: 240px;
    max-height: 60vh;
  }

  .tb-panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-weight: 600;
    font-size: 13px;
    padding: 4px 6px 8px;
  }

  .tb-panel-body { display: flex; flex-direction: column; gap: 2px; }

  /* 顶栏按钮管理：横排芯片式（替代竖排清单），窄面板自动换行；比清单更紧凑直观 */
  .tb-manager { width: 340px; }
  .tb-chip-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
    padding: 2px 2px 4px;
  }
  .tb-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 11px 5px 9px;
    border-radius: var(--radius-pill, 999px);
    border: 1px solid rgba(128, 128, 128, 0.28);
    background: transparent;
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
    user-select: none;
    transition: background var(--dur, 0.2s) var(--ease), border-color var(--dur, 0.2s) var(--ease), opacity var(--dur, 0.2s) var(--ease);
  }
  .tb-chip:hover { background: rgba(128, 128, 128, 0.12); }
  .tb-chip.on {
    background: color-mix(in srgb, var(--accent, #4a6b8a) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent, #4a6b8a) 42%, transparent);
    color: var(--accent, #4a6b8a);
  }
  .tb-chip.fixed { cursor: default; opacity: 0.75; }
  .tb-chip.na { opacity: 0.42; }
  .tb-chip input { width: 13px; height: 13px; margin: 0; flex-shrink: 0; }
  .tb-chip-label { white-space: nowrap; }

  .tb-menu-item {
    text-align: left;
    background: transparent;
    border: none;
    color: inherit;
    padding: 8px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }

  .tb-menu-item:hover { background: rgba(128, 128, 128, 0.14); }

  .tb-panel-hint { font-size: 12px; opacity: 0.6; margin: 8px 6px 2px; line-height: 1.5; }
  .tb-panel-empty { font-size: 13px; opacity: 0.5; padding: 10px 8px; }
  .tb-block-row { display: flex; align-items: center; gap: 8px; padding: 4px 6px; }
  .tb-block-thumb {
    width: 44px;
    height: 44px;
    object-fit: cover;
    border-radius: 4px;
    background: rgba(128, 128, 128, 0.15);
  }

  .manga-slider { display: flex; align-items: center; gap: 8px; padding: 6px; font-size: 12px; }
  .manga-slider input[type="range"] { flex: 1; }
  .manga-val { min-width: 38px; text-align: right; opacity: 0.75; }
  .manga-actions { display: flex; gap: 8px; padding: 6px; }

  .font-ctl .icon.on { color: var(--accent, #4a6b8a); font-weight: 700; }

  .body {
    flex: 1;
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    z-index: 1;
  }

  .stage {
    flex: 1;
    display: flex;
    position: relative;
    min-height: 0;
  }

  .content {
    flex: 1;
    position: relative;
    overflow: hidden;
    min-width: 0;
    /* 空白区/边距/滚动条不允许选中：避免点击空白处误选一大段文字 */
    user-select: none;
    -webkit-user-select: none;
  }

  /* 仅正文文本可被选中（用于划词/划线） */
  .content :global(#chapter-content),
  .content :global(#chapter-content *) {
    user-select: text;
    -webkit-user-select: text;
  }

  .content :global(::selection) {
    background: var(--rdr-sel, #e8e0cd);
  }

  /* 单本「全部关闭图片」：隐藏正文里的图片/SVG 图 */
  .reader.no-images #chapter-content :global(img),
  .reader.no-images #chapter-content :global(svg) {
    display: none !important;
  }

  .font-ctl {
    position: absolute;
    right: 14px;
    bottom: 46px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    z-index: 25;
    opacity: 0;
    pointer-events: none;
    transform: translateY(4px);
    transition: opacity 0.18s, transform 0.18s;
  }

  .font-ctl .ctl-row {
    display: flex;
    gap: 4px;
    justify-content: flex-end;
  }

  .font-ctl.show,
  .font-ctl:focus-within {
    opacity: 1;
    pointer-events: auto;
    transform: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .font-ctl {
      transition: none;
      transform: none;
    }
    .col-page {
      transition: none;
    }
  }

  .clip {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--rdr-mh, 56px);
    right: var(--rdr-mh, 56px);
    overflow: hidden;
  }

  .col-page {
    position: absolute;
    left: 0;
    column-fill: auto;
    /* 翻页只走合成：常驻 transition，立即跳转时用内联 transition:none 短路 */
    will-change: transform;
    transition: transform var(--dur-page) var(--ease-page);
  }

  .scroll-area {
    position: absolute;
    inset: 0;
    /* 程序化滚动容器：通过 scrollTop 定位；overflow:hidden 隐藏原生滚动条（滚动由 SmoothScroller 驱动）。 */
    overflow: hidden;
    /* 关闭浏览器滚动锚定：图片延迟加载使内容变高时，滚动锚定会自动改 scrollTop，
       与 SmoothScroller 每帧写入的绝对位置互相打架，产生「跳一下再弹回」的可见闪烁。 */
    overflow-anchor: none;
    /* 不再常驻 will-change: transform：合成型滚动容器 + 主线程写 scrollTop 会在滚动时出现
       一帧的空白/撕裂（闪屏）。回弹位移的 transform 仍会按需合成，但不再把滚动容器长期提升为图层。 */
  }

  .col-scroll {
    /* 正常文档流：高度=正文高度，作为 .scroll-area 的可滚动内容，由浏览器按需分块绘制。
       不再 position:absolute + will-change:transform 把整章提升为超长合成层（撕裂/空白的根因）。 */
    padding: var(--rdr-mv, 48px) var(--rdr-mh, 56px);
    box-sizing: border-box;
  }

  /* 漫画横向连续阅读：固定视口层只承载上下边距（左右不加 marginH）；
     内容变形层横向排列以 transform 平移，交叉轴（纵向）由 align-items:center 居中。 */
  .manga-h-viewport {
    position: absolute;
    inset: 0;
    overflow: hidden;
    cursor: grab;
    touch-action: none;
    background: var(--rdr-bg, #f5f1e8);
    padding: var(--rdr-mv, 0) 0;
    box-sizing: border-box;
  }

  .manga-h-canvas {
    position: absolute;
    left: 0;
    top: 0;
    width: max-content;
    min-height: 100%;
    will-change: transform;
  }

  .reader.manga-h .manga-h-canvas {
    display: flex !important;
    flex-wrap: nowrap !important;
    align-items: center !important;
  }

  .reader.manga-h .manga-h-canvas.rtl {
    flex-direction: row-reverse !important;
  }

  .reader.manga-h .manga-h-canvas :global(.manga-block) {
    flex: 0 0 auto !important;
    margin: 0 var(--rdr-img-gap, 0) 0 0 !important;
    padding: 0 !important;
    width: auto !important;
  }

  .reader.manga-h .manga-h-canvas.rtl :global(.manga-block) {
    margin: 0 0 0 var(--rdr-img-gap, 0) !important;
  }

  .reader.manga-h .manga-h-canvas :global(.manga-block:last-child) {
    margin: 0 !important;
  }

  .reader.manga-h .manga-h-canvas :global(img),
  .reader.manga-h .manga-h-canvas :global(svg),
  .reader.manga-h .manga-h-canvas :global(video) {
    height: calc(var(--rdr-img-h, 80vh) * var(--rdr-img-scale, 1)) !important;
    width: auto !important;
    max-height: none !important;
    max-width: none !important;
    object-fit: contain !important;
    margin: 0 !important;
    padding: 0 !important;
    display: block !important;
    zoom: 1 !important;
  }

  .reader.fitw.manga-h .manga-h-canvas :global(img),
  .reader.fitw.manga-h .manga-h-canvas :global(svg),
  .reader.fitw.manga-h .manga-h-canvas :global(video) {
    width: var(--mg-vieww, 100vw) !important;
    height: auto !important;
    max-width: none !important;
  }

  .reader.fith.manga-h .manga-h-canvas :global(img),
  .reader.fith.manga-h .manga-h-canvas :global(svg),
  .reader.fith.manga-h .manga-h-canvas :global(video) {
    height: var(--rdr-img-h, 80vh) !important;
    width: auto !important;
    max-height: none !important;
  }

  /* 漫画瀑布流（纵向连续窗口）：固定视口层承载左右边距，内容块纵向顺排，原生竖向滚动。
     canvas 占满宽度 + 每个 .manga-block 内部水平居中（漫画窗口里无 #chapter-content 包裹层）。 */
  .manga-flow-viewport {
    padding: 0 var(--rdr-mh, 0);
    box-sizing: border-box;
    overflow-y: auto !important;
    overflow-x: hidden !important;
    overflow-anchor: none;
  }

  .manga-flow-canvas {
    width: 100%;
    will-change: transform;
  }

  .reader.manga .manga-flow-canvas :global(.manga-block) {
    margin: 0 !important;
    padding: 0 !important;
    width: 100% !important;
    display: flex !important;
    flex-direction: column !important;
    align-items: center !important;
  }

  .reader.manga .manga-flow-canvas :global(img),
  .reader.manga .manga-flow-canvas :global(svg),
  .reader.manga .manga-flow-canvas :global(video) {
    max-width: 100% !important;
    max-height: calc(var(--rdr-img-h, 80vh) * var(--rdr-img-scale, 1)) !important;
    height: auto !important;
    width: auto !important;
    object-fit: contain !important;
    display: block !important;
    margin: calc(var(--rdr-img-gap, 6px) / 2) auto !important;
    zoom: var(--rdr-img-scale, 1) !important;
  }

  .reader.fitw.manga .manga-flow-canvas :global(img),
  .reader.fitw.manga .manga-flow-canvas :global(svg),
  .reader.fitw.manga .manga-flow-canvas :global(video) {
    width: 100% !important;
    max-width: 100% !important;
    height: auto !important;
    max-height: none !important;
    zoom: 1 !important;
  }

  .reader.fith.manga .manga-flow-canvas :global(img),
  .reader.fith.manga .manga-flow-canvas :global(svg),
  .reader.fith.manga .manga-flow-canvas :global(video) {
    height: var(--rdr-img-h, 80vh) !important;
    width: auto !important;
    max-width: 100% !important;
    max-height: var(--rdr-img-h, 80vh) !important;
    zoom: 1 !important;
  }

  .tb-btn.on {
    color: var(--accent, #4a6b8a);
    font-weight: 700;
  }

  #chapter-content {
    font-family: var(--rdr-font);
    font-synthesis: none;
    overflow-wrap: break-word;
  }

  /* 阅读版式覆盖仅在「覆盖书籍排版」开启时生效；关闭后由书内样式决定字号/行距/颜色等 */
  .reader.ovr #chapter-content {
    font-size: var(--rdr-fs);
    line-height: var(--rdr-lh);
    color: var(--rdr-fg);
    letter-spacing: 0.05em;
    text-align: var(--rdr-align, justify);
    text-justify: inter-ideograph;
  }

  .reader.ovr #chapter-content :global(p) {
    text-indent: var(--rdr-indent, 2em);
    margin: var(--rdr-ps, 0.7em) 0;
    font-weight: var(--rdr-fw, 400);
  }

  .reader.ovr #chapter-content :global(h1),
  .reader.ovr #chapter-content :global(h2),
  .reader.ovr #chapter-content :global(h3),
  .reader.ovr #chapter-content :global(h4) {
    font-size: 1.2em;
    margin: 0.4em 0 0.8em;
    font-weight: 600;
    text-align: center;
    /* 标题避免末行孤字 */
    text-wrap: balance;
  }

  /* 正文段落：优先避免难看的断行与孤行（浏览器支持时生效） */
  .reader.ovr #chapter-content :global(p) {
    text-wrap: pretty;
    orphans: 2;
    widows: 2;
  }

  #chapter-content :global(img) {
    max-width: 100%;
    max-height: calc(var(--rdr-img-h, 80vh) - 1.6em);
    width: auto;
    height: auto;
    object-fit: contain;
    display: block;
    margin: 0.6em auto;
  }

  #chapter-content :global(blockquote) {
    margin: 0.8em 1.5em;
    opacity: 0.85;
  }

  #chapter-content :global(a) {
    color: inherit;
    text-decoration: none;
  }

  #chapter-content :global(table) {
    border-collapse: collapse;
    margin: 0.8em 0;
    max-width: 100%;
  }

  #chapter-content :global(td),
  #chapter-content :global(th) {
    border: 1px solid rgba(128, 128, 128, 0.3);
    padding: 4px 8px;
  }

  #chapter-content :global(mark.fr-hl) {
    background: transparent;
    color: inherit;
    padding: 0;
    cursor: pointer;
  }

  #chapter-content :global(mark.fr-tts) {
    background: color-mix(in srgb, var(--accent, #4a6b8a) 32%, transparent);
    color: inherit;
    border-radius: 3px;
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent, #4a6b8a) 45%, transparent);
  }

  #chapter-content :global(mark.fr-hl-underline) {
    text-decoration-line: underline;
    text-decoration-color: var(--hl-color, #c0392b);
    text-decoration-thickness: 2px;
    text-underline-offset: 2px;
  }

  #chapter-content :global(mark.fr-hl-wavy) {
    text-decoration-line: underline;
    text-decoration-style: wavy;
    text-decoration-color: var(--hl-color, #c0392b);
    text-decoration-thickness: 2px;
    text-underline-offset: 3px;
  }

  #chapter-content :global(mark.fr-hl-dotted) {
    text-decoration-line: underline;
    text-decoration-style: dotted;
    text-decoration-color: var(--hl-color, #c0392b);
    text-decoration-thickness: 2px;
    text-underline-offset: 2px;
  }

  #chapter-content :global(mark.fr-hl-double) {
    text-decoration-line: underline;
    text-decoration-style: double;
    text-decoration-color: var(--hl-color, #c0392b);
    text-decoration-thickness: 2px;
    text-underline-offset: 2px;
  }

  #chapter-content :global(mark.fr-hl-strike) {
    text-decoration-line: line-through;
    text-decoration-color: var(--hl-color, #c0392b);
    text-decoration-thickness: 2px;
  }

  #chapter-content :global(mark.fr-hl-background) {
    background: color-mix(in srgb, var(--hl-color, #c0392b) 38%, transparent);
    border-radius: 3px;
  }

  /* H2：有感想(note)的划线加一个小圆点标记，提示可悬停查看 */
  #chapter-content :global(mark.fr-hl.fr-has-note) {
    position: relative;
  }
  #chapter-content :global(mark.fr-hl.fr-has-note::after) {
    content: "";
    display: inline-block;
    width: 3px;
    height: 3px;
    margin-left: 2px;
    border-radius: 50%;
    background: var(--hl-color, #c0392b);
    vertical-align: super;
  }

  /* H2 感想浮层 */
  .fr-note-tip {
    position: fixed;
    z-index: 990;
    transform: translate(-50%, calc(-100% - 8px));
    max-width: 320px;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 92%, #202020 8%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 10px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16), 0 20px 44px rgba(0, 0, 0, 0.24);
    padding: 8px 10px;
    pointer-events: none;
    font-size: 13px;
  }
  .fr-note-tip-quote {
    font-size: 12px;
    opacity: 0.6;
    margin-bottom: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fr-note-tip-body { line-height: 1.6; white-space: pre-wrap; }

  #chapter-content :global(mark.fr-hl-box) {
    box-shadow: 0 0 0 1.5px var(--hl-color, #c0392b);
    border-radius: 3px;
  }

  .zone-hints {
    position: absolute;
    inset: 0;
    z-index: 22;
    pointer-events: none;
  }

  .zh {
    position: absolute;
    font-size: 12px;
    color: rgba(127, 127, 127, 0.65);
    background: rgba(127, 127, 127, 0.10);
    border: 1px dashed rgba(127, 127, 127, 0.4);
    border-radius: 6px;
  }

  .zh-tl { left: 2%; top: 2%; height: 46%; width: 46%; display: grid; place-items: center; }
  .zh-tr { right: 2%; top: 2%; height: 46%; width: 46%; display: grid; place-items: center; }
  .zh-bl { left: 2%; bottom: 2%; height: 46%; width: 46%; display: grid; place-items: center; }
  .zh-br { right: 2%; bottom: 2%; height: 46%; width: 46%; display: grid; place-items: center; }
  .zh-c {
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 30%;
    height: 56%;
    display: grid;
    place-items: center;
    background: rgba(127, 127, 127, 0.14);
  }

  /* 可视宽约 5px，但用 13px 透明命中区（左右各 4px padding），thumb 居中便于拖动。 */
  .scrollbar {
    position: absolute;
    right: 1px;
    top: 14px;
    bottom: 14px;
    width: 13px;
    padding: 0 4px;
    box-sizing: border-box;
    z-index: 20;
    cursor: pointer;
    touch-action: none;
  }

  .thumb {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    width: 5px;
    margin: 0 auto;
    background: rgba(128, 128, 128, 0.35);
    border-radius: 3px;
    will-change: transform;
    transition: background 0.12s ease, width 0.12s ease;
  }

  .scrollbar:hover .thumb,
  .scrollbar.dragging .thumb {
    width: 8px;
    background: rgba(128, 128, 128, 0.6);
  }

  /* 加载骨架：用与正文同形的灰条替代「加载中…」药丸，减少版面跳动。 */
  .loading {
    position: absolute;
    top: 12%;
    left: 50%;
    transform: translateX(-50%);
    width: min(680px, 78%);
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: 14px;
    pointer-events: none;
  }

  .sk {
    display: block;
    height: 1em;
    border-radius: 4px;
    background: rgba(128, 128, 128, 0.14);
    background-image: linear-gradient(90deg, transparent, rgba(128, 128, 128, 0.2), transparent);
    background-size: 200% 100%;
    animation: sk-shimmer 1.2s ease-in-out infinite;
  }

  .sk-title { height: 1.4em; width: 46%; margin: 0 auto 8px; }
  .sk-short { width: 68%; }

  @keyframes sk-shimmer {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
  }

  .footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 18px;
    font-size: 12px;
    opacity: 0.8;
    position: relative;
    z-index: 5;
    user-select: none;
    transition: opacity var(--dur) var(--ease);
  }

  .progress-track {
    flex: 1;
    height: 16px;
    display: flex;
    align-items: center;
    cursor: pointer;
    position: relative;
    touch-action: none;
  }

  .progress-track::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    background: rgba(128, 128, 128, 0.15);
    pointer-events: none;
  }

  /* 填充条用 scaleX 走合成（不再动 width，拖动时不触发布局）。 */
  .progress-fill {
    position: absolute;
    left: 0;
    top: 50%;
    width: 100%;
    height: 4px;
    margin-top: -2px;
    background: var(--accent, #4a6b8a);
    border-radius: 2px;
    transform-origin: left center;
    z-index: 1;
    pointer-events: none;
    transition: transform var(--dur) var(--ease);
  }

  .progress-knob {
    position: absolute;
    top: 50%;
    left: 0;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent, #4a6b8a);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.32), 0 1px 1px rgba(0, 0, 0, 0.18), inset 0 1px 0 rgba(255, 255, 255, 0.35);
    transform: translate(-50%, -50%);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.15s, left var(--dur) var(--ease);
    z-index: 2;
  }

  .progress-track:hover .progress-knob,
  .progress-track:focus-visible .progress-knob,
  .progress-track.dragging .progress-knob {
    opacity: 1;
  }

  .progress-track.dragging {
    cursor: grabbing;
  }

  .progress-track.dragging .progress-fill {
    transition: none;
  }

  .progress-track.dragging .progress-knob {
    transition: opacity 0.15s;
  }

  .finfo {
    white-space: nowrap;
    user-select: none;
    /* 空信息时也要有可点热区（左=上一章 / 右=下一章）：给最小宽度与内边距 */
    min-width: 88px;
    padding: 2px 8px;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background var(--dur) var(--ease), opacity var(--dur) var(--ease), transform var(--dur) var(--ease);
  }
  .finfo:hover {
    background: var(--hover-bg);
    opacity: 1;
  }
  .finfo:active {
    background: var(--active-bg);
    transform: scale(0.96);
  }
  .finfo-l { text-align: left; }
  .finfo-r { text-align: right; }
  .fitem { opacity: 0.85; }
  .fsep { opacity: 0.45; margin: 0 4px; }

  .empty {
    flex: 1;
    display: grid;
    place-items: center;
    opacity: 0.5;
  }

  .error-box {
    opacity: 1;
    text-align: center;
  }

  .error-box p {
    margin: 6px 0;
    font-weight: 600;
  }

  .err-detail {
    font-size: 12px;
    font-weight: 400;
    opacity: 0.7;
    max-width: 480px;
    word-break: break-all;
    margin-bottom: 16px;
  }

  .sidepanel {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 320px;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 94%, black 6%);
    border-left: 1px solid rgba(128, 128, 128, 0.2);
    z-index: 40;
    display: flex;
    flex-direction: column;
    box-shadow: -6px 0 18px rgba(0, 0, 0, 0.08), -18px 0 48px rgba(0, 0, 0, 0.14);
    animation: slide-in 0.34s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes slide-in {
    from { transform: translateX(24px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .sp-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    font-weight: 600;
    border-bottom: 1px solid rgba(128, 128, 128, 0.15);
  }

  .sp-list { flex: 1; overflow-y: auto; }

  .sp-item {
    padding: 10px 16px;
    cursor: pointer;
    border-bottom: 1px solid rgba(128, 128, 128, 0.08);
  }

  .sp-item:hover { background: rgba(128, 128, 128, 0.1); }

  .bm-item { display: flex; align-items: flex-start; gap: 8px; }
  .bm-main { flex: 1; min-width: 0; cursor: pointer; }
  .bm-del { opacity: 0; flex-shrink: 0; transition: opacity 0.15s; }
  .bm-item:hover .bm-del { opacity: 1; }

  .bm-title { font-size: 13px; font-weight: 500; }
  .bm-excerpt { font-size: 12px; opacity: 0.6; margin-top: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-empty { padding: 24px; text-align: center; opacity: 0.5; }
</style>
