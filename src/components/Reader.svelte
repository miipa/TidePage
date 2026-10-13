<script lang="ts">
  import { errText, labelOfId, t } from "../lib/i18n/index.svelte";
  import { updateState } from "../lib/update.svelte";
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
    marginScopeOf,
    type ReaderTopbarId,
    type BindAction,
    type ZoneKey,
    type StatusItemId,
    type ThemeKey,
  } from "../lib/stores.svelte";
  import { getBookOverride, setBookOverride, type BookOverride } from "../lib/bookcfg.svelte";
  import { dragSort } from "../lib/dragSort";
import { reorderByDrop } from "../lib/reorder";
  import { dragOutHandle } from "../lib/windowSnap";
  import { applyReadFix, clearReadFix } from "../lib/readFix";
  import { SmoothScroller } from "../lib/smoothScroll";
  import {
    classifyPointerUp,
    isWheelLocal,
    mangaWheelDelta,
    normalizeWheelDelta,
    reduceWheelTurn,
    scrollKeyRoute,
    shouldDedupeLoad,
    WHEEL_TURN_IDLE,
    type WheelAxis,
    type WheelTurnState,
  } from "../lib/readerInput";
import { prefersReducedMotion } from "../lib/reducedMotion";
  import { outSlideL, outSlideR } from "../lib/motion";

  // 侧面板退场的承载层。
  //
  // 为什么需要它：Svelte 不允许 transition: 直接挂在组件上（只接受元素），
  // 而五个面板里四个（TocPanel / HighlightsPanel / BookStatsModal / AiPanel）
  // 都是组件、根元素是它自己内部的 .sidepanel/.ai-panel —— 父级拿不到那个元素，
  // 于是没法给它退场。
  //
  // 这个 div 只做两件事：铺满 .body 当定位上下文（面板本就是 absolute，
  // 锚在自己的四边上，所以视觉结果与不包裹时逐像素相同），以及承接退场位移。
  // pointer-events:none 是必须的：它铺满整个正文区，不放行会吃掉正文的点击与
  // 拖选框选；子元素显式 auto 把交互还回面板。
  //
  // ⚠️ 刻意不加 z-index / 不形成层叠上下文：面板自己的 --z-drawer 仍需直接与
  //    正文层竞争，包一层会把 z 解析域从 .body 改成 wrapper，层级就会变。
  //    .body 有 overflow:hidden，退场滑出的部分由它裁掉 —— 正是想要的效果。
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
    IconArrowsSplit2,
    IconBook2,
    IconBookmarkPlus,
    IconBookmarks,
    IconBooks,
    IconChartBar,
    IconCheck,
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
    IconTextDecrease,
    IconTextIncrease,
    IconTrash,
    IconX,
    IconZoomIn,
    IconZoomOut,
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
  /** 正在加载的目标章号（-1=无）。与 loadingChapter 成对维护：供同章去重判断复用在途请求，
   *  连按/连滚指向同一章时不再重复 ++loadGeneration（见 loadChapter）。非响应式即可。 */
  let loadingIdx = -1;
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

  // 沉浸阅读：空闲后淡出顶栏/底栏；只有光标进入顶栏/底栏所在的带状区域才唤回。
  let chromeHidden = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let topbarEl: HTMLDivElement | undefined = $state();
  let footerEl: HTMLDivElement | undefined = $state();
  let hoverTarget: Element | null = null;   // 只在 rAF 块里用于 .tb-panel 命中，非响应式
  let overChromeBar = false;                // 非响应式：写成 $state 会让上面的 $effect 每帧重跑

  /** 开始/重置「空闲淡出」计时；光标停在顶栏/底栏（或它们弹出的浮层）上时不计时。 */
  function armChromeHide() {
    clearTimeout(idleTimer);
    if (!config.reader.immersive || overChromeBar) return;
    idleTimer = setTimeout(() => (chromeHidden = true), 2600);
  }

  /** 光标是否进入「顶栏/底栏 + 其弹出浮层」：进入即唤回并暂停计时，离开再重新计时。 */
  function syncChromeHover(inChrome: boolean) {
    if (inChrome === overChromeBar) return;
    overChromeBar = inChrome;
    if (inChrome) { if (chromeHidden) chromeHidden = false; clearTimeout(idleTimer); }
    else armChromeHide();
  }

  $effect(() => {
    if (!config.reader.immersive) { chromeHidden = false; clearTimeout(idleTimer); return; }
    armChromeHide();   // 打开沉浸阅读即开始计时，不再依赖「先动一下鼠标」
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

  // 右上角 WindowControls 由 +page.svelte 渲染在 .reader 子树之外，拿不到 readerVars() 的内联
  // 令牌；阅读期间把「阅读前景色 + 图标静止不透明度」发布到 :root，让它跟随阅读主题而不是外壳
  // （浅色外壳 + 夜读时外壳 --fg 在阅读顶栏上几乎不可见）。
  //
  // 退出时**恢复**而不是删除 :root --rdr-fg：它本来就由 stores.svelte.ts 的
  // applyReaderSurfaceVars() 维护（供无 .reader 的阅读窗与 portal 浮层，只在 theme/bgColor/
  // fgColor 变化时重跑）。删掉会回落到 app.css 的静态兜底（纸色默认），丢掉全局阅读主题值。
  //
  // ⚠️ WindowControls 不能拿 --rdr-fg 当「是否在阅读」的信号：applyReaderSurfaceVars() 把
  //    --rdr-fg **常驻**写在 <html> 上，所以它在外壳界面里也有值（= 阅读主题字色，默认
  //    纸色 #2c2c2c）。WindowControls 原先写的是 var(--rdr-fg, var(--fg))，那个回退分支
  //    永远不会命中 —— 暗色外壳下窗口按钮就用了近黑的纸色字，在 #1e2126 面板上对比度
  //    只有 1.27:1，三个按钮等于隐形。故本机制改用**专属变量** --rdr-wc-fg：只有阅读期间
  //    存在，卸载即删除，外壳界面自然回落到 --fg。--rdr-wc-opacity 同理。
  $effect(() => {
    const { fg } = readerColors();
    const root = document.documentElement;
    const prev = root.style.getPropertyValue("--rdr-fg");
    root.style.setProperty("--rdr-fg", fg);
    root.style.setProperty("--rdr-wc-fg", fg);
    root.style.setProperty("--rdr-wc-opacity", "0.9");
    return () => {
      if (prev) root.style.setProperty("--rdr-fg", prev);
      else root.style.removeProperty("--rdr-fg");
      // --rdr-wc-fg / --rdr-wc-opacity 是本机制自己新增的，删除即可（无别人维护）。
      root.style.removeProperty("--rdr-wc-fg");
      root.style.removeProperty("--rdr-wc-opacity");
    };
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
  /** 是否处于漫画模式（自动检测结果，或用户手动切换）。
   *  关掉「自动识别漫画」后忽略后端判定，但按书本的手动覆盖仍然优先生效。 */
  function isMangaMode(): boolean {
    const o = bookOv();
    return o.mangaMode ?? (config.reader.manga.autoDetect ? (book?.is_manga ?? false) : false);
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
    label: string | (() => string);
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
  const tbLabel = (b: TbBtn): string => (typeof b.label === "function" ? b.label() : b.label);
  const TOPBAR_BTNS: TbBtn[] = [
    { id: "webdav", label: () => t("reader.toolbar.webdav"), title: () => t("reader.toolbar.webdavTitle"), icon: () => IconCloudUp, visible: () => config.webdav.enabled, longPress: () => toggleWdMenu(), run: () => uploadProgress() },
    { id: "webdavDownload", label: () => t("reader.toolbar.webdavDownload"), title: () => t("reader.toolbar.webdavDownloadTitle"), icon: () => IconCloudDownload, visible: () => config.webdav.enabled, run: () => webdavDownload() },
    { id: "convert", label: () => t("reader.toolbar.convert"), title: () => t("reader.toolbar.convert"), icon: () => IconArrowsExchange, visible: () => book?.book.format === "txt", run: () => convertThis() },
    { id: "bookmarkAdd", label: () => t("reader.toolbar.bookmarkAdd"), title: () => t("reader.toolbar.bookmarkAdd"), icon: () => IconBookmarkPlus, visible: () => true, run: () => addBookmark() },
    { id: "bookmarkList", label: () => t("reader.toolbar.bookmarkList"), title: () => t("reader.toolbar.bookmarkList"), icon: () => IconBookmarks, visible: () => true, run: () => loadBookmarks() },
    { id: "ttsPlay", label: () => t("reader.toolbar.ttsPlay"), title: () => t("reader.toolbar.ttsPlayTitle"), icon: () => (ttsState.playing ? IconPlayerPause : IconPlayerPlay), visible: () => true, run: () => toggleTts() },
    { id: "ttsStop", label: () => t("reader.toolbar.ttsStop"), title: () => t("reader.toolbar.ttsStop"), icon: () => IconPlayerStop, visible: () => true, run: () => stopTts() },
    { id: "highlight", label: () => t("reader.toolbar.highlight"), title: () => t("reader.toolbar.highlight"), icon: () => IconHighlight, visible: () => true, run: () => setRight("highlights") },
    { id: "ai", label: () => t("reader.toolbar.ai"), title: () => t("reader.toolbar.ai"), icon: () => IconSparkles, visible: () => true, run: () => toggleAiPanel() },
    { id: "stats", label: () => t("reader.toolbar.bookStats"), title: () => t("reader.toolbar.bookStats"), icon: () => IconChartBar, visible: () => true, run: () => setRight("stats") },
    { id: "toc", label: () => t("reader.toolbar.toc"), title: () => t("reader.toolbar.tocTitle"), icon: () => IconList, visible: () => true, run: () => toggleToc() },
    { id: "manga", label: () => t("reader.toolbar.manga"), title: () => (isMangaMode() ? t("reader.toolbar.mangaOnTitle") : t("reader.toolbar.mangaOffTitle")), icon: () => (isMangaMode() ? IconBooks : IconBook2), on: () => isMangaMode(), visible: () => true, run: () => toggleMangaMode() },
    { id: "manager", label: () => t("reader.toolbar.manager"), title: () => t("reader.toolbar.manager"), icon: () => IconAdjustmentsHorizontal, visible: () => true, run: () => toggleManager() },
    { id: "images", label: () => t("reader.toolbar.images"), title: () => t("reader.toolbar.imagesTitle"), icon: () => (effShowImages() ? IconPhoto : IconPhotoOff), visible: () => true, run: () => toggleImgMenu() },
    // 按钮 id 沿用历史值 "css"：topbarOrder / topbarHidden 里存的就是它，改名会让老用户的按钮
    // 位置与隐藏态错位、甚至整个按钮不渲染。图标与位置不变，行为已改为打开阅读主题列表。
    { id: "css", label: () => t("reader.toolbar.theme"), title: () => t("reader.toolbar.themeTitle", { name: labelOfId("theme", effTheme()) }), icon: () => IconPalette, on: () => themeMenuOpen, visible: () => true, run: () => toggleThemeMenu() },
    { id: "settings", label: () => t("reader.toolbar.settings"), title: () => t("reader.toolbar.settings"), icon: () => IconSettings, visible: () => true, run: () => (ui.settingsOpen = true) },
  ];
  const topbarBtn = (id: string): TbBtn | undefined => TOPBAR_BTNS.find((b) => b.id === id);
  const visibleTopbarBtns = $derived(
    config.reader.topbarOrder
      .map((id) => topbarBtn(id))
      .filter((b): b is TbBtn => !!b && b.visible() && (b.id === "manager" || !config.reader.topbarHidden.includes(b.id))),
  );

  /** 拖拽排序：把 from 插入到 target 之前/之后（在完整顺序数组内重排，隐藏项位置一并保留）。 */
  function tbDrop(from: string, target: string, place: "before" | "after") {
    const next = reorderByDrop(config.reader.topbarOrder, from, target, place, (s) => s);
    if (!next) return;
    config.reader.topbarOrder = next;
    saveConfig();
  }

  /** 顶栏「漫画模式」按钮：把普通书切成漫画模式，或把漫画切回普通模式（按书持久化）。 */
  function toggleMangaMode() {
    const next = !isMangaMode();
    setOv({ mangaMode: next });
    toast(next ? t("reader.toast.mangaOn") : t("reader.toast.mangaOff"));
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

  /** 应用一套阅读主题：theme 必须连带 bg/fg 一起写（bgColor/fgColor 默认非空，只改 theme 肉眼看不到变化）；
   *  当前书若单独套用过配色方案，同步写这本书的 scheme*，保证按钮在任何书上都可见生效。 */
  function applyTheme(key: ThemeKey) {
    config.reader.theme = key;
    config.reader.bgColor = THEMES[key].bg;
    config.reader.fgColor = THEMES[key].fg;
    saveConfig();
    if (book && bookScheme()) setOv({ schemeTheme: key, schemeBg: THEMES[key].bg, schemeFg: THEMES[key].fg });
    toast(t("reader.toast.theme", { name: labelOfId("theme", key) }));
  }

  /** 主题列表顺序（浅 → 深），只此一份。 */
  const THEME_ORDER: ThemeKey[] = ["paper", "white", "green", "dark", "night"];

  function toggleAllImages() {
    const next = !effShowImages();
    setOv({ showImages: next });
    toast(next ? t("reader.toast.imagesOn") : t("reader.toast.imagesOff"));
  }

  function startBlockPick() {
    imgMenuOpen = false;
    blockPickMode = true;
    toast(t("reader.toast.blockHint"));
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
  let themeMenuOpen = $state(false);
  let themeMenuEl = $state<HTMLElement | undefined>(undefined);
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
    themeMenuOpen = false;
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

  /** 顶栏「阅读主题」列表开合：互斥同上。 */
  function toggleThemeMenu() {
    const willOpen = !themeMenuOpen;
    closeTbPanels();
    if (willOpen) {
      rightPanel = null;
      tocOpen = false;
      themeMenuOpen = true;
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

  // 主题列表外点关闭：同 wdMenu 范式（窗口捕获阶段 pointerdown）。
  $effect(() => {
    if (!themeMenuOpen) return;
    const h = (e: PointerEvent) => {
      const tgt = e.target as Element | null;
      // 面板内 与 顶栏本身 都不关：后者是必须的 —— 否则再次点主题按钮时，
      // pointerdown 先把面板关掉、紧接着 click 又把 toggle 打开，表现为「点一下关不掉」。
      if (tgt?.closest?.(".topbar") || themeMenuEl?.contains(tgt as Node)) return;
      themeMenuOpen = false;
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
    if (!ok) { toast(t("reader.toast.popoutFailed"), "error"); return; }
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
      toast(t("reader.toast.fullscreenFailed", { err: String(e) }), "error");
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
      // 全书总字数只在这里算一次（见 tocCharsTotal 处的说明）
      tocCharsTotal = book.toc.reduce((s, t) => s + (t.chars || 0), 0);
      ui.currentBookId = book.book.id;
      if (book.encoding_warning) console.log(book.encoding_warning);
      // 漫画书首次引导：全局只提示一次（写入 config.reader.mangaGuideDone），
      // 之后打开任何漫画都不再弹出；拖动时实时预览（直接改全局 manga.imgScale）。
      blockPickMode = false;
      imgMenuOpen = false;
      tbManagerOpen = false;
      blockedListOpen = false;
      // 走 isMangaMode() 而不是裸 book.is_manga：关掉自动识别后不应再弹漫画引导。
      if (isMangaMode() && !config.reader.mangaGuideDone) {
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
          .then((restored) => { if (restored) toast(t("reader.toast.davRestored")); })
          .catch(() => {});
      }
    } catch (e) {
      const s = String(e);
      const friendly = s.includes("AWZ3EncryptedError")
        ? t("reader.err.drm")
        : s.includes("ParseTimeoutOrCorrupted")
          ? t("reader.err.malformed")
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
    tocCharsTotal = 0;
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
      toast(t("reader.toast.davSyncOnExit"));
    } catch (e) {
      toast(t("reader.toast.davSyncOnExitFailed", { err: String(e) }), "error");
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
    if (!config.webdav.enabled) { toast(t("reader.toast.davDisabled"), "error"); return; }
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) { toast(t("reader.toast.davNeedSetup"), "error"); return; }
    try {
      await api.webdavSyncProgress(w.base, w.user, w.pass, book.book.id, book.book.path, localProgress());
      toast(t("reader.toast.davUploaded"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  /** 「下载进度」：要先弹确认框（会改动本地），再从云端更新本地进度。 */
  async function webdavDownload() {
    if (!book) return;
    if (!config.webdav.enabled) { toast(t("reader.toast.davDisabled"), "error"); return; }
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) { toast(t("reader.toast.davNeedSetup"), "error"); return; }
    let remote: Progress | null = null;
    try {
      remote = await api.webdavReadProgress(w.base, w.user, w.pass, book.book.path);
    } catch (e) {
      toast(errText(e), "error");
      return;
    }
    if (!remote) { toast(t("reader.toast.davNoRemote")); return; }
    const ok = await appConfirm(
      t("reader.confirm.davCompare", {
        rc: remote.chapter_index + 1,
        rp: remote.pct.toFixed(0),
        rt: fmtProgressTime(remote.updated_at),
        lc: chapterNum + 1,
        lp: totalPct().toFixed(0),
        lt: fmtProgressTime(Date.now()),
      }),
    );
    if (!ok) return;
    try {
      if (remote.chapter_index < book.toc.length) {
        await loadChapter(remote.chapter_index, remote);
      }
      toast(t("reader.toast.davApplied"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  /** 长按菜单：上传当前书籍文件到云端（手动，不受 uploadBooks 自动开关影响）。 */
  async function uploadBook() {
    wdMenuOpen = false;
    if (!book) return;
    const w = config.webdav;
    if (!w.base || !w.user || !w.pass) { toast(t("reader.toast.davNeedSetup"), "error"); return; }
    try {
      await api.webdavUploadBook(w.base, w.user, w.pass, book.book.path);
      toast(t("reader.toast.davBookUploaded"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  function fmtProgressTime(ms: number): string {
    if (!ms) return t("common.none");
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
    entry?: { at: "head" | "tail"; extra?: number },
    /** 是否由「滚动到章边界」触发（handleScrollEdge）：只有这类加载才允许在落地后
     *  补触发 pendingEdge。目录/书签/搜索/朗读等显式跳章与上一轮滚动无关，若让它
     *  补触发，就会出现「跳章后自动又前进一章」。 */
    fromEdge = false
  ) {
    if (!book) return;
    // 同章在途且不带定位：直接复用这次请求（gen 语义保留给「不同章节之间」的跳转）。
    // 旧实现每次调用都 ++loadGeneration，连按 N 次就让前 N-1 次在途结果全部作废，
    // 表现为「按 N 下只前进 1 章、且每按一下都重新发一次请求」。
    // 带 entry / restore 的请求是显式定位，一律放行（见 shouldDedupeLoad）。
    if (shouldDedupeLoad(loadingChapter, loadingIdx, idx, { restore, entry })) return;
    // 通过去重检查后立刻丢弃陈旧的跨章意图：非 edge 跳章不能补触发上一轮滚动留下的意图。
    if (!fromEdge) pendingEdge = null;
    if (ttsState.playing) stopTts(); // 手动切换章节时停止朗读（自动续读在 onTtsChapterEnd 中处理）
    // 竞态：快速连续翻页时，只有最后一次请求的结果才允许落地。
    const gen = ++loadGeneration;

    const cached = cacheGet(idx);
    // 命中缓存：立即同步切换，不显示任何 loading。
    if (cached) {
      loadingChapter = false;
      loadingIdx = -1;
      await applyChapter(idx, cached, restore, entry, gen);
      prefetchAround(idx);
      afterChapterApplied(fromEdge);
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
    // 被挡下的 edge 意图改由 pendingEdge 缓存，落地后补触发（见 handleScrollEdge）。
    loadingChapter = true;
    loadingIdx = idx;

    try {
      const ch = await fetchChapter(idx);
      if (gen !== loadGeneration) return; // 已被更新的翻页取代
      loadingChapter = false;
      loadingIdx = -1;
      await applyChapter(idx, ch, restore, entry, gen);
      prefetchAround(idx);
      afterChapterApplied(fromEdge);
    } catch (e) {
      if (gen === loadGeneration) {
        loadingChapter = false;
        loadingIdx = -1;
        // 本章失败：丢掉缓存的跨章意图，避免用户稍后跳到别处时突然补跳一章。
        pendingEdge = null;
        toast(errText(e), "error");
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
    else if (fromEnd) toast(t("reader.toast.lastChapter"));
  }

  function prevChapter(toEnd = false) {
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
  /** 章节加载期间被丢弃的跨章意图（dir=±1、carry=剩余行程）。
   *  旧实现 loadingChapter 时直接 return，意图没有任何补偿 → 滚到底后必须再滚一次才有反应。
   *  这里缓存最后一次，edge 触发的章节落地后由 flushPendingEdge() 补触发（同 pendingMangaCarry 的思路）。 */
  let pendingEdge: { dir: number; carry: number } | null = null;

  /** 换章落地后补触发一次被丢弃的跨章意图（skipThrottle 跳过 320ms 节流，但仍受加载态与章边界约束）。
   *  先取走再置 null：补触发链里再次遇到 loadingChapter 时还能继续缓存，不会自锁。 */
  function flushPendingEdge() {
    const p = pendingEdge;
    pendingEdge = null;
    if (p) handleScrollEdge(p.dir, p.carry, true);
  }

  /** 章节落地后的统一收尾。只有 edge 触发的加载才补触发跨章意图；
   *  目录/书签/搜索/朗读等非 edge 跳章一律丢弃——加载期间新产生的意图也是相对旧位置的，
   *  补触发只会「跳完目标章又自动跳到下一章」。 */
  function afterChapterApplied(fromEdge: boolean) {
    if (fromEdge) flushPendingEdge();
    else pendingEdge = null;
  }

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

  function handleScrollEdge(dir: number, carry: number, skipThrottle = false) {
    if (!book) return;
    // 漫画模式（两种布局共用）：连续窗口在两端自动续章，不重建窗口（保持视觉连续）。
    // 窗口两端是「人造边界」，不让 SmoothScroller 的 overshoot 表现成回弹：清掉越界量，
    // 把越界行程留给续章（窗口补齐后由 onsettled 把 carry 补滚一次）。横向没有 scrollTop，
    // 续章本身就会拉长 mangaMaxX，故同样只需清掉越界量。只影响漫画模式，
    // 普通文本滚动的翻章/回弹行为完全不变（见下一段代码）。
    if (isMangaMode()) {
      scroller?.consumeEdge();
      if (scrollLayout() && carry > 0) pendingMangaCarry = Math.min(carry, 4000);
      void extendMangaWindow(dir > 0 ? 1 : -1);
      return;
    }
    // 加载中丢弃的意图必须留痕（补触发用），否则「滚到底」的第一下会被无声吞掉。
    if (loadingChapter) {
      pendingEdge = { dir, carry };
      return;
    }
    const now = Date.now();
    if (!skipThrottle && now - lastEdgeAt < 320) return;
    if (dir > 0) {
      if (chapterNum >= book.toc.length - 1) return;
      lastEdgeAt = now;
      // 第四个参数 fromEdge=true：落地后允许补触发 pendingEdge（连滚跨章的意图链）。
      loadChapter(chapterNum + 1, undefined, { at: "head", extra: config.reader.scrollBounce ? 0 : carry }, true);
    } else {
      if (chapterNum <= 0) return;
      lastEdgeAt = now;
      loadChapter(chapterNum - 1, undefined, { at: "tail", extra: config.reader.scrollBounce ? 0 : carry }, true);
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

  /** 有效左右边距：漫画模式用独立边距；普通阅读按当前作用域取（滚动/单栏/双栏各存一份）。 */
  function effMarginH(): number {
    if (isMangaMode()) return config.reader.manga.marginH; // 漫画仍走自己的独立边距
    return config.reader.margins[marginScopeOf(config.reader)].h;
  }

  /** 有效上下边距：漫画模式用独立边距；普通阅读按当前作用域取。 */
  function effMarginV(): number {
    return isMangaMode() ? config.reader.manga.marginV : config.reader.margins[marginScopeOf(config.reader)].v;
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

/** 页面偏移统一走 transform 合成（GPU 合成层），animate=true 时不会带动页面滚动 */
  let lastPageX = Number.NaN;
  function applyPage(animate = false) {
    if (!pageWrap) return;
    // 步长非法时不要平移（否则会把内容推出可视区）；等测量完成后再定位。
    const x = stepPx > 0 ? -Math.round(page * stepPx) : 0;
    if (!animate && x === lastPageX) return; // 位置未变：跳过，避免无谓的强制同步布局
    lastPageX = x;
    if (animate && !prefersReducedMotion()) {
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
    // 保留 1 位小数：设置里允许 32.5 这类字号，A−/A+ 不能把它抹成整数。
    const n = Math.max(10, Math.min(64, Math.round((cur + delta) * 10) / 10));
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
      // 先判断覆盖是否真的存在：setBookOverride 即便 patch 全是 undefined 也会
      // map[key] = {} 建一条空覆盖并同步写一次 localStorage（bookcfg.svelte.ts:76-77），
      // 没有覆盖时这一步纯属白写，还会给这本书留一条永久空记录。
      if (book && getBookOverride(book.book.id).imgScale !== undefined) {
        setBookOverride(book.book.id, { imgScale: undefined });
      }
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
  /** 窗口条目：imgs 预先统计好，预算/预载判据只做求和（禁止对窗口内 HTML 全量正则）。 */
  interface MangaItem { index: number; html: string; imgs: number }
  /** 跨章连续滚动窗口（按序连续章节区间）：漫画两种布局共用。 */
  let mangaWindow = $state<MangaItem[]>([]);
  /** 续章节流（rAF 合并而非墙上时钟静默丢弃）：保证每次请求要么执行、要么排队补发。 */
  let mangaExtendLock = 0;
  let mangaExtendBusy = false;
  let mangaExtendPending: 1 | -1 | 0 = 0;
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
  /** 漫画纵向：窗口边界被 consumeEdge 丢弃的越界行程，待窗口补齐后补滚一次。 */
  let pendingMangaCarry = 0;

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
   *  单图/内容不满屏时主轴居中；超过时由 applyMangaPan 的平移查看。
   *  纵向画布仅在 transform 值真正变化时写入（沿用 applyScroll 里 lastBounce 的做法），
   *  否则每次 measureFlow 都会写一次无意义的 translateX(0px) 触发样式失效。 */
  let lastFlowCenterXf = "";
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
      const xf = `translateX(${Math.max(0, (vp?.clientWidth ?? w) / 2 - mh - w / 2)}px)`;
      if (xf !== lastFlowCenterXf) {
        lastFlowCenterXf = xf;
        fel.style.transform = xf;
      }
    }
  }

  /** 应用平移：LTR 起点在左、RTL 起点在右；无论方向，语义坐标 0 都是“阅读起点”。 */
  function applyMangaPan(smooth = false) {
    const el = mangaCanvas;
    if (!el) return;
    const tx = mangaRtl() ? -(mangaMaxX - mangaPosX) : -mangaPosX;
    const ty = -mangaPosY;
    const xf = `translate3d(${Math.round(tx)}px, ${Math.round(ty)}px, 0)`;
    if (smooth && !prefersReducedMotion()) {
      el.style.transition = "transform 180ms ease";
      el.style.transform = xf;
      return;
    }
    // 非平滑路径要打断可能正在跑的 180ms 平滑过渡，得走
    // 「transition:none → 写 transform → 强制刷新 → 还原 transition」这套把戏：
    // 不刷新的话浏览器只看到最终计算值，transition 从没真正被关掉，于是会补一段动画。
    // **但只有 transition 真的还挂着（刚被上面的 smooth 路径设过）时才需要刷新。**
    // 常态（拖拽跟手、滚轮）transition 已是空串，直接写 transform 就是瞬变；
    // 原先无条件 `void el.offsetWidth` 等于每个输入事件都逼一次强制重排，
    // 而这个事件是绑在 pointermove 上的（触发频率跟鼠标轮询率走，可达数百 Hz）。
    if (el.style.transition) {
      el.style.transition = "none";
      el.style.transform = xf;
      void el.offsetWidth;
      el.style.transition = "";
    } else {
      el.style.transform = xf;
    }
  }

  /* 横向拖拽跟手：pointermove 只更新 mangaPosX/Y（普通变量，非 $state，写入无副作用），
     真正的 transform 写回合并进 rAF，至多一帧一次。
     与纵向 manga 流的 checkMangaWindowExtend 同一套路（那里也是 scroll 只记标量、
     裁剪/续章/章号统一在帧里做）。指针事件频率远高于帧率，逐事件写样式没有额外收益。 */
  let mangaPanRaf = 0;
  function scheduleMangaPan() {
    if (mangaPanRaf) return;
    mangaPanRaf = requestAnimationFrame(() => {
      mangaPanRaf = 0;
      applyMangaPan();
    });
  }

  /** 立即落盘当前平移（松手等需要当场看到最终画面、不许等到下一帧的场合）。 */
  function flushMangaPan() {
    if (mangaPanRaf) {
      cancelAnimationFrame(mangaPanRaf);
      mangaPanRaf = 0;
    }
    applyMangaPan();
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

  /** 瀑布流（纵向连续窗口）：测滚动范围并同步滚动条。滚动范围宁可偏小也绝不偏大：
   *  maxScroll 偏大会让 applyScroll 的夹取与 SmoothScroller 的越界判定都基于错值
   *  （画面不动且 edge 永不触发），偏小只多滚一下滚轮即可自愈。 */
  function measureFlow() {
    const vp = mangaFlowViewport;
    if (!vp) return;
    const vh = vp.clientHeight;
    // imgMaxH 会写进 --rdr-img-h 变量，改动即让全窗口所有图片重算 max-height：
    // 只在真的变了时才写，避免每次续章/裁剪都引发一次整窗重排。
    if (vh > 0) {
      const ih = Math.max(120, vh - effMarginV() * 2);
      if (ih !== imgMaxH) imgMaxH = ih;
    }
    applyCanvasCentering();
    maxScroll = Math.max(0, vp.scrollHeight - vh);
    scroller?.setMax(maxScroll);
  }

  /** 漫画瀑布流：重测滚动范围 + 可选按锚点章补偿滚动位置。
   *  锚点补偿后必须就地刷新 lastMangaAnchorTop，否则下一次重排会把同一份位移再算一遍。 */
  function remapFlowScroll(withAnchor = false) {
    const vp = mangaFlowViewport;
    if (!vp || !mangaFlow()) return;
    const before = vp.scrollTop;
    measureFlow();
    if (!withAnchor) return;
    const block = vp.querySelector<HTMLElement>(`.manga-block[data-index="${chapterNum}"]`);
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

  /** 滚动范围重测的 rAF 合并口：window/续章/裁剪/图片加载后调用。
   *  不复用 scheduleRelayout 的 160ms 防抖——连续懒加载时那条防抖会被 img load 无限饿死。 */
  let flowRemapRaf = 0;
  let flowRemapAnchor = false;
  function scheduleFlowRemap(anchor = false) {
    if (anchor) flowRemapAnchor = true;
    if (flowRemapRaf) return;
    flowRemapRaf = requestAnimationFrame(() => {
      flowRemapRaf = 0;
      const wantAnchor = flowRemapAnchor;
      flowRemapAnchor = false;
      remapFlowScroll(wantAnchor);
    });
  }

  /* ================= 漫画跨章连续窗口 ================= */
  // 漫画两种布局都在一个容器里按序渲染「一段连续章节」的图片，滚到接近两端时续章、
  // 裁掉远端，形成连续刷图体验（「一张图一章」不再卡住）。普通分页模式不走这里。
  // 窗口大小按「图片数量」而非章数限定：对「一张图一章」的书 ≈ MANGA_IMG_INIT 章（体验不变），
  // 对「每章很多图」的书则可能只含 1~2 章（DOM 立刻从几千降到百级）。
  const MANGA_IMG_INIT = 15; // 初始窗口累计到约这么多张图即停（至少含当前章）
  const MANGA_IMG_BUDGET = 50; // 续章后窗口累计图片预算上限，超出则从离屏远端裁掉整章
  const MANGA_FLOW_LEAD = 1.5; // 尾块进入「视口底边 + 1.5 屏」即续章
  const MANGA_EXTEND_MIN_GAP = 70; // 续章最小间隔（ms），配 rAF 合并替代 260ms 墙上时钟

  /** 粗略统计一段 html 里的图片数量（用于按图片数而非章数限定窗口）。 */
  function imgCount(html: string): number {
    return html ? (html.match(/<img/gi)?.length ?? 0) : 0;
  }

  /** 仅漫画路径：给没有 loading 的 <img 补 loading="lazy"（保留后端已加的 decoding="async"），
      使单章即便几百张图也只解码视口附近几张。页模式（普通阅读）绝不调用此函数。 */
  function injectMangaLazy(html: string): string {
    if (!html) return "";
    return html.replace(/<img\b(?![^>]*\bloading=)/gi, '<img loading="lazy"');
  }

  /** 当前窗口累计图片数：条目构建时已把该章图片数算好，这里只求和（不做正则扫描）。 */
  function windowImgTotal(): number {
    let n = 0;
    for (const it of mangaWindow) n += it.imgs;
    return n;
  }

  /** 建一个窗口条目（懒加载注入 + 一次性图片计数，供 windowImgTotal 求和）。 */
  function mangaItem(index: number, html: string): MangaItem {
    const h = injectMangaLazy(html);
    return { index, html: h, imgs: imgCount(h) };
  }

  /** 窗口尾块的真实底边（纵向续章判据一律用它，不再读裁剪前的 scrollHeight）。 */
  function flowWindowTailBottom(): number {
    const last = mangaFlowViewport?.querySelector<HTMLElement>(".manga-block:last-of-type");
    return last ? last.offsetTop + last.offsetHeight : 0;
  }

  /** 需要向上续章？「视口顶距窗口首块不足 1.5 屏」即前插一章。
   *  判据必须用「视口顶距窗口顶的绝对距离 = scrollTop」，**不能**写成
   *  `firstTop <= scrollTop + 1.5h`：前插一章时 firstTop 与 scrollTop 同时 +块高，该式恒成立、
   *  于是循环前插到书首（且 mangaExtendBusy 把向下续章全部饿死）→ 窗口最后被头裁剪吃光而卡死。
   *  与头部裁剪共用同一个阈值且一侧用 `<`、另一侧用「裁完仍 ≥ 阈值」，两者形成迟滞、不会互相打架。
   *  窗口已含全书第一章时直接返回 false（书首没有上方内容可补）。 */
  function flowNeedTop(): boolean {
    const vp = mangaFlowViewport;
    if (!vp || !mangaWindow.length) return false;
    if (mangaWindow[0].index <= 0) return false;
    return vp.scrollTop < vp.clientHeight * MANGA_FLOW_LEAD;
  }

  /** 需要向下续章？尾块底边已进入视口底 + 1.5 屏（已含全书末章同理必须返回 false）。 */
  function flowNeedBottom(): boolean {
    const vp = mangaFlowViewport;
    if (!vp || !mangaWindow.length) return false;
    if (mangaWindow[mangaWindow.length - 1].index >= (book?.toc.length ?? 1) - 1) return false;
    return flowWindowTailBottom() <= vp.scrollTop + vp.clientHeight * (1 + MANGA_FLOW_LEAD);
  }

  /** 记录「当前章块相对视口的 offsetTop」作为锚点基线。
   *  任何做了滚动位置补偿的路径（裁剪 / 前插 / 预算裁剪）都必须就地同步，否则下一次重排
   *  会把同一份位移再算一遍（relayout 里 delta = block.offsetTop - lastMangaAnchorTop）。 */
  function syncMangaAnchor(): void {
    lastMangaAnchorTop =
      mangaFlowViewport?.querySelector<HTMLElement>(`.manga-block[data-index="${chapterNum}"]`)
        ?.offsetTop ?? -1;
  }


  /** 唯一的头部裁剪者：只裁「完全离屏」的整章块；内容高度变化后立即重测滚动范围。
   *  关键不变量：**这里不做尾部裁剪**。原实现在此把「视口以下的所有预载章」删光，与
   *  loadMangaWindow 的 15 张预载、extendMangaWindow 的「提前 1.5 屏续章」直接矛盾：
   *  内容高度塌到约一屏 → 浏览器把 scrollTop 夹回底部 → 不再发 scroll 事件 → 永久卡死。
   *  尾部只由 mangaBudgetTrim 按图片预算裁（且只裁完全离屏的块），窗口因此始终有界。 */
  async function mangaWindowTrim(pos: { top: number; height: number }) {
    const vp = mangaFlowViewport;
    if (!vp || mangaWindow.length <= 1) return pos;
    const blocks = Array.from(vp.querySelectorAll<HTMLElement>(".manga-block"));
    if (blocks.length < 2) return pos;
    let removedTop = 0;
    let lo = 0;
    // 迟滞：只有「裁掉这块之后，视口顶之上仍留 ≥1.5 屏」才裁。否则每次下滚都会把刚补上的
    // 上方缓冲又削掉，与 flowNeedTop 的前插判据互相触发（窗口原地抖动、CPU 空转）。
    const lead = pos.height * MANGA_FLOW_LEAD;
    while (blocks.length - lo > 1) {
      const h = blocks[lo].offsetHeight;
      if (blocks[lo].offsetTop + h > pos.top) break;
      if (pos.top - removedTop - h < lead) break;
      removedTop += h;
      lo++;
    }
    if (lo === 0) return pos;
    mangaWindow = mangaWindow.slice(lo);
    if (removedTop) pos.top = Math.max(0, pos.top - removedTop);
    // 裁剪改变了内容高度：必须在写回 scrollTop 之前重测，否则浏览器按旧范围夹取。
    await tick();
    if (scrollLayout()) remapFlowScroll(false);
    else measureManga();
    if (removedTop) {
      if (scrollArea) scrollArea.scrollTop = pos.top;
      scroller?.jumpTo(pos.top, true);
    }
    // 锚点就地刷新（章号会在随后 updateChapterFromWindow 里更新），消除双重补偿。
    syncMangaAnchor();
    return pos;
  }

  /** 按当前章重建窗口（换章/跳转/恢复进度/退出进入漫画模式时调用；同步首屏，保证定位准确）。 */
  async function loadMangaWindow(idx: number, restore?: Progress) {
    if (!book) return;
    const gen = ++loadGeneration;
    loadingChapter = true;
    loadingIdx = idx;
    const total = book.toc.length;
    // 按「图片数量」建窗：先取当前章保证至少含它；不足初值则向两侧交替扩，累计图片达 MANGA_IMG_INIT 即停。
    // 一张图一章 → 约 MANGA_IMG_INIT 章；每章很多图 → 当前章一张就超初值 → 窗口仅 1~2 章。
    const items = new Map<number, MangaItem>();
    const objs = new Map<number, ChapterData | null>();
    const take = (i: number) =>
      fetchChapter(i)
        .catch(() => null)
        .then((ch) => {
          objs.set(i, ch);
          const it = mangaItem(i, ch?.html ?? "");
          items.set(i, it);
          return it.imgs;
        });
    // 先占位当前章，首屏即出结构。
    mangaWindow = [mangaItem(idx, "")];
    chapterNum = idx;
    chapter = cacheGet(idx) ?? null;
    await tick();
    let imgs = await take(idx);
    let lo = idx;
    let hi = idx;
    if (imgs < MANGA_IMG_INIT) {
      // 并行预取两侧候选（最多各 MANGA_IMG_INIT 章，够一张图一章的书），再按序累加到达初值。
      const loMin = Math.max(0, idx - MANGA_IMG_INIT);
      const hiMax = Math.min(total - 1, idx + MANGA_IMG_INIT);
      const cand: number[] = [];
      for (let i = loMin; i <= hiMax; i++) if (i !== idx) cand.push(i);
      await Promise.all(cand.map(take));
      let goLeft = true;
      // 只在已预取的候选范围内扩，避免某些章无图时把整本书纳入空窗口。
      while (imgs < MANGA_IMG_INIT && (lo > loMin || hi < hiMax)) {
        const canL = lo > loMin;
        const canR = hi < hiMax;
        const left = canL && (!canR || goLeft);
        goLeft = !goLeft;
        if (left) lo--;
        else hi++;
        imgs += items.get(left ? lo : hi)?.imgs ?? 0;
      }
    }
    const next: MangaItem[] = [];
    for (let i = lo; i <= hi; i++) next.push(items.get(i) ?? mangaItem(i, ""));
    mangaWindow = next;
    chapter = objs.get(idx) ?? chapter;
    loadingChapter = false;
    loadingIdx = -1; // 与 loadingChapter 成对收尾，避免去重状态残留
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
      // 首屏定位后补一次续章判定：短章节的书初始窗口可能仍贴着视口底。
      scheduleExtendCheck();
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

  /** 循环续章之间的最小间隔（把 IPC 压力摊开，同时远小于原 260ms，肉眼无「一卡一卡」）。 */
  function sleepMangaGap(): Promise<void> {
    return new Promise((r) => setTimeout(r, MANGA_EXTEND_MIN_GAP));
  }

  /** 窗口向后/前追加（滚动接近两端时调用）；prepend 用滚动补偿避免跳动。
   *  续章请求不允许丢：锁期内的请求记到 pendingDir，当前续章结束（finally）后自动补发。 */
  async function extendMangaWindow(dir: 1 | -1, bypass = false) {
    if (!book || !mangaWindow.length) return;
    if (!bypass && (Date.now() < mangaExtendLock || mangaExtendBusy)) {
      mangaExtendPending = dir;
      return;
    }
    const gen = loadGeneration;
    mangaExtendBusy = true;
    try {
      if (dir > 0) {
        const total = book.toc.length;
        const vp = mangaFlowViewport;
        const h = vp?.clientHeight ?? 0;
        // 一次补到「尾块底边 ≥ 视口底 + 2.5 屏」或书尾，消除按 260ms 粒度一卡一卡的追加，
        // 同时给「章节正在被取」这段留足滚轮跑道（触发线是视口底 + 1.5 屏）。
        while (mangaWindow.length > 0) {
          const cur = mangaWindow[mangaWindow.length - 1].index;
          const next = cur + 1;
          if (next >= total) break;
          const ch = await fetchChapter(next).catch(() => null);
          if (gen !== loadGeneration) return;
          // 期间可能已由别处裁剪/重建：确认尾章仍是 cur 再追加。
          if (!mangaWindow.length || mangaWindow[mangaWindow.length - 1].index !== cur) break;
          mangaWindow = [...mangaWindow, mangaItem(next, ch?.html ?? "")];
          await tick();
          if (gen !== loadGeneration) return;
          if (scrollLayout()) {
            // 窗口有界的唯一保证：每追加一章就按图片预算回收（章节极矮的书否则会被一次性追加几百章）。
            await mangaBudgetTrim();
            if (gen !== loadGeneration) return;
            if (next >= total - 1) break; // 已到书尾
            if (windowImgTotal() > MANGA_IMG_BUDGET) break;
            if (vp && flowWindowTailBottom() >= vp.scrollTop + h * 3.5) break;
          } else if (mangaHorizontal()) {
            measureManga();
            await mangaBudgetTrim();
            break; // 横向维持「一次一章」的原有追加节奏
          }
          await sleepMangaGap();
        }
        return;
      }
      while (mangaWindow.length > 0) {
        const cur = mangaWindow[0].index;
        const prev = cur - 1;
        if (prev < 0) break;
        const ch = await fetchChapter(prev).catch(() => null);
        if (gen !== loadGeneration) return;
        if (!mangaWindow.length || mangaWindow[0].index !== cur) break;
        mangaWindow = [mangaItem(prev, ch?.html ?? ""), ...mangaWindow];
        await tick();
        if (gen !== loadGeneration) return;
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
        // 瀑布流 prepend 后补偿滚动位置，视觉不跳（先重测范围再写回，避免浏览器按旧范围夹取）。
        const vp = mangaFlowViewport;
        if (vp) {
          remapFlowScroll(false);
          const first = vp.querySelector<HTMLElement>(".manga-block");
          const keep = Math.max(0, vp.scrollTop + (first?.offsetHeight ?? 0));
          if (scrollArea) scrollArea.scrollTop = keep;
          scroller?.jumpTo(keep, true);
        }
        syncMangaAnchor();
        await mangaBudgetTrim();
        if (gen !== loadGeneration) return;
        if (windowImgTotal() > MANGA_IMG_BUDGET) break;
        if (!flowNeedTop()) break;
        await sleepMangaGap();
      }
    } finally {
      mangaExtendBusy = false;
      mangaExtendLock = Date.now() + MANGA_EXTEND_MIN_GAP;
      const pending = mangaExtendPending;
      mangaExtendPending = 0;
      if (pending) void extendMangaWindow(pending, true);
    }
  }

  /** 预算裁剪：窗口累计图片超 MANGA_IMG_BUDGET 时，从「已完全离屏」的远端裁掉整章块，带滚动/位移补偿；
      永不裁掉视口内的章（故与瀑布流的 mangaWindowTrim 互补，横向模式则由它独自保证窗口有界）。 */
  async function mangaBudgetTrim() {
    if (mangaWindow.length <= 1 || windowImgTotal() <= MANGA_IMG_BUDGET) return;
    if (scrollLayout()) {
      const vp = mangaFlowViewport;
      if (!vp) return;
      const blocks = Array.from(vp.querySelectorAll<HTMLElement>(".manga-block"));
      // 图片总数随每次 slice 维护：避免在 while 条件里反复对窗口内全部 HTML 做正则扫描。
      let total = windowImgTotal();
      let removedTop = 0;
      let lo = 0;
      let hi = blocks.length;
      // 头部：完全在视口上方的整章
      while (hi - lo > 1 && total > MANGA_IMG_BUDGET) {
        const first = blocks[lo];
        if (first.offsetTop + first.offsetHeight <= vp.scrollTop) {
          removedTop += first.offsetHeight;
          total -= mangaWindow[lo]?.imgs ?? 0;
          lo++;
        } else break;
      }
      // 尾部：完全在视口下方的整章（纵向唯一的尾部裁剪者）
      while (hi - lo > 1 && total > MANGA_IMG_BUDGET) {
        const last = blocks[hi - 1];
        if (last.offsetTop >= vp.scrollTop + vp.clientHeight) {
          total -= mangaWindow[hi - 1]?.imgs ?? 0;
          hi--;
        } else break;
      }
      if (lo !== 0 || hi !== blocks.length) {
        mangaWindow = mangaWindow.slice(lo, hi);
        await tick();
        // 先重测滚动范围再写回 scrollTop：范围偏大会让浏览器按旧值夹取（卡死链的一环）。
        remapFlowScroll(false);
        if (removedTop) {
          const keep = Math.max(0, vp.scrollTop - removedTop);
          if (scrollArea) scrollArea.scrollTop = keep;
          scroller?.jumpTo(keep, true);
        }
        syncMangaAnchor();
      }
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

  /** 滚动接近两端时续章 + 裁剪远端保持窗口有界。
   *  scroll 事件只负责「记录最新 scrollTop + 请求一帧」：裁剪 / 续章判据 / 更新章号统一在 rAF
   *  里做（原先在 scroll 回调里同步做 2N 次强制布局读并同步改 $state，是掉帧主因）。 */
  function checkMangaWindowExtend() {
    const vp = mangaFlowViewport;
    if (!vp || !mangaWindow.length) return;
    pendingFlowTop = vp.scrollTop;
    if (flowScrollRaf) return;
    flowScrollRaf = requestAnimationFrame(runFlowScrollFrame);
  }

  /** 首屏定位/重建窗口后主动补一次续章判定（无需等 scroll 事件）。 */
  function scheduleExtendCheck() {
    if (flowScrollRaf) return;
    flowScrollRaf = requestAnimationFrame(runFlowScrollFrame);
  }

  let flowScrollRaf = 0;
  let pendingFlowTop = 0;

  async function runFlowScrollFrame() {
    flowScrollRaf = 0;
    const vp = mangaFlowViewport;
    if (!vp || !mangaWindow.length) return;
    // 裁剪导致的 scrollTop 写回也放在这里，不在 scroll 事件回调里。
    await mangaWindowTrim({ top: pendingFlowTop, height: vp.clientHeight });
    updateChapterFromWindow();
    // 章号更新后再同步锚点基线：裁剪的补偿已写进 scrollTop，不同步会被下一次重排再算一遍。
    syncMangaAnchor();
    // 续章判据用「窗口首/尾块真实几何」，不再用裁剪前的 scrollHeight（此刻 DOM 已变，读数失真）。
    const needTop = flowNeedTop();
    const needBottom = flowNeedBottom();
    if (needBottom) {
      // 两端同时告急时先补下方：向下饿死才是卡死链的下游，且头部随即会被裁剪回收。
      void extendMangaWindow(1);
    } else if (needTop) {
      void extendMangaWindow(-1);
    }
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
    // 横向量优先（既有横向平移语义不变）；没有横向量时回落纵向，用与正文同一套
    // 换算（旧实现只换算 line，deltaMode=2 的映射设备每下只挪 1px，表现为滚轮没反应）。
    const delta = mangaWheelDelta(e.deltaX, e.deltaMode, e.deltaY);
    if (Math.abs(delta) < 1) return;
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
    scheduleMangaPan();
  }

  function onMangaPointerUp(e: PointerEvent) {
    if (e.pointerId !== mangaPid) return;
    mangaPid = -1;
    if (mangaViewport) mangaViewport.style.cursor = "";
    if (mangaPanning) {
      mangaPanning = false;
      flushMangaPan(); // 拖拽的最后一帧不能还挂在 rAF 上，松手时画面必须已经到位
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
    hoverTarget = e.target as Element | null;
    if (hoverRaf) return;
    hoverRaf = requestAnimationFrame(() => {
      hoverRaf = 0;
      // 沉浸阅读唤回：只有光标进入顶栏/底栏带状区域才显示。
      // 不能给 .topbar 挂 mouseenter —— 隐藏态它带 pointer-events:none，收不到事件；
      // .reader 的 mousemove 一直有效，故在这里对两条 bar 的 rect 做命中。
      const inBar = (el?: HTMLElement) => {
        if (!el) return false;
        const r = el.getBoundingClientRect();
        return hoverY >= r.top && hoverY <= r.bottom;
      };
      // 顶栏/底栏弹出的浮层（.tb-panel）也算「在 chrome 上」：否则光标一移进菜单，2.6s 后顶栏就淡掉了。
      syncChromeHover(inBar(topbarEl) || inBar(footerEl) || !!hoverTarget?.closest?.(".tb-panel"));
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
      remapFlowScroll(true);
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
  // 滚轮翻页状态（累积量/上次事件时间/锁到期）。判定本身是纯函数
  // reduceWheelTurn()（lib/readerInput.ts），这里只负责把上一拍的状态传下去。
  let wheelState: WheelTurnState = WHEEL_TURN_IDLE;

  /** 滚轮归属：只有「目标（或其祖先）真的可滚」或命中表单控件时才归本地、交给它自己滚。
   *  旧实现按固定选择器一刀切——光标停在展开但不可滚的抽屉/弹出菜单上时，既不翻正文、
   *  容器自身又滚不了，表现就是「滚了完全没动静（看起来随机）」。 */
  function wheelIsLocal(e: WheelEvent): boolean {
    const dy = Math.abs(e.deltaY);
    const dx = Math.abs(e.deltaX);
    const axis: WheelAxis = dy >= dx ? "y" : "x";
    return isWheelLocal(e.target, axis);
  }

  function onWheel(e: WheelEvent) {
    if (!contentEl) return;
    // 判定必须在 mangaHorizontal() 之前：漫画模式同样受用（onMangaWheel 只被 onWheel 调用）。
    if (wheelIsLocal(e)) return; // 面板/浮层/表单控件自己能滚时滚它们，其余归正文
    if (mangaHorizontal()) {
      onMangaWheel(e);
      return;
    }
    if (!scrollLayout()) {
      if (!config.reader.wheelTurnPage) return;
      e.preventDefault();
      const r = reduceWheelTurn(wheelState, e.deltaMode, e.deltaY, Date.now());
      wheelState = r.state;
      if (r.turn > 0) nextPage();
      else if (r.turn < 0) prevPage();
      return;
    }
    e.preventDefault();
    const d = normalizeWheelDelta(e.deltaMode, e.deltaY);
    if (maxScroll <= 1 || !scroller) {
      if (d > 0) handleScrollEdge(1, 0);
      else if (d < 0) handleScrollEdge(-1, 0);
      return;
    }
    const res = scroller.wheelBy(d * 1.4);
    if (res.edge !== 0) handleScrollEdge(res.edge, res.carry);
  }

  function scrollStep(dy: number) {
    // 防御：非滚动布局没有滚动行程（maxScroll 恒为 0），落到底部那个分支会既被 320ms 节流
    // 吃掉、又会一按跳整章 —— 这里直接回翻页语义，与 next/prev 同义、不跨章。
    if (!scrollLayout()) {
      if (dy > 0) nextPage();
      else if (dy < 0) prevPage();
      return;
    }
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
      case "scrollDown": {
        // 翻页布局下 ↑/↓（多数映射设备的默认输出）必须等价于 next/prev：
        // 旧实现只有漫画横向与滚动两个分支，落到 scrollStep 后因 maxScroll 恒为 0 走
        // handleScrollEdge，既被 320ms 节流连按丢弃、又会一按跳整章。
        const route = scrollKeyRoute({ mangaHorizontal: mangaHorizontal(), scrollLayout: scrollLayout() });
        if (route === "pan") { mangaPosX = Math.min(mangaMaxX, mangaPosX + vh * 0.4); clampManga(); applyMangaPan(); }
        else if (route === "page") nextPage();
        else scrollStep(vh * 0.4);
        break;
      }
      case "scrollUp": {
        const route = scrollKeyRoute({ mangaHorizontal: mangaHorizontal(), scrollLayout: scrollLayout() });
        if (route === "pan") { mangaPosX = Math.max(0, mangaPosX - vh * 0.4); clampManga(); applyMangaPan(); }
        else if (route === "page") prevPage();
        else scrollStep(-vh * 0.4);
        break;
      }
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
        if (tbManagerOpen || imgMenuOpen || themeMenuOpen || blockedListOpen || mangaGuideOpen) {
          tbManagerOpen = false;
          imgMenuOpen = false;
          themeMenuOpen = false;
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
        runAction(def.id, e);
        return;
      }
    }
  }

  let ptr: { x: number; y: number; id: number } | null = null;

  function onContentPointerDown(e: PointerEvent) {
    if (mangaHorizontal()) return; // 漫画双页：手势由横向视口单独处理
    if (e.button !== 0) {
      // 非主键不参与选区/点击判定；侧键（3/4）由 mousedown/mouseup 单独处理。
      ptr = null;
      return;
    }
    ptr = { x: e.clientX, y: e.clientY, id: e.pointerId };
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
    ptr = null;
    if (!contentEl) return;
    // 意图顺序由 classifyPointerUp 决定，且**选区最优先、与位移无关**。
    // 旧实现把「位移超过阈值」排在选区判定之前：拖选 1–2 个字常常只有 4–8px，
    // 会先落到点击分支 —— removeAllRanges() 清掉刚拖出的选区再按热区翻页（划词回归）。
    // 时长仍不参与判定：映射设备常模拟带延时的长按，旧的 450ms 硬门槛会让点击完全没反应。
    const sel = window.getSelection();
    const hasSel = !!sel && !sel.isCollapsed && sel.rangeCount > 0;
    const intent = classifyPointerUp({ hasSelection: hasSel, moved });
    if (intent === "select" && sel) {
      // sel 非空由 hasSel 保证（intent==="select" ⇒ hasSel）；再 && 一次只为让 TS 收窄，避免类型断言。
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
      return; // 有选区但不属于正文或为空：交给浏览器，绝不清选区、不翻页
    }
    if (intent !== "click") return; // 拖过但没选中东西 = 划词/拖拽手势，不当作点击翻页
    // 纯点击：清掉可能残留的选区（含浏览器在空白处“整块选中”），避免误选与菜单复现。
    window.getSelection()?.removeAllRanges();
    const tgt = e.target as HTMLElement | null;
    // 点击已有划线 → 打开划线设置（改颜色 / 改样式 / 删除）
    const markEl = tgt?.closest?.("mark.fr-hl") as HTMLElement | null;
    if (markEl && contentEl.contains(markEl)) {
      const id = Number(markEl.dataset.hlid);
      const h = highlights.find((x) => x.id === id);
      if (h) {
        hlMenu = { h, anchor: rectAnchor(markEl.getBoundingClientRect()) };
        return;
      }
    }
    // 纯点击正文图片 → 屏蔽选择模式下屏蔽该图；否则打开全书图片查看器（不触发翻页/区域动作）。
    const imgEl = tgt?.closest?.("img") as HTMLImageElement | null;
    if (imgEl && contentEl.contains(imgEl)) {
      e.preventDefault();
      e.stopPropagation();
      if (blockPickMode) {
        const src = imgEl.getAttribute("src") || "";
        if (src) {
          const list = blockedImages();
          if (!list.includes(src)) setOv({ blockedImages: [...list, src] });
          toast(t("reader.toast.imageBlocked"));
        }
        blockPickMode = false;
        return;
      }
      openViewerForImage(imgEl);
      return;
    }
    // 可交互元素分流：链接交给 onBodyClick（跳章/复制），button/input 等交给各自处理器；
    // 只有正文空白处才按 config.reader.zones 热区翻页，否则点控件会「既点它又翻页」。
    if (tgt?.closest?.("a, button, input, textarea, select")) return;
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

  /** 鼠标侧键（button 3=XButton1「后退」、4=XButton2「前进」）翻页：
   *  映射设备常把翻页映射到侧键，而旧实现 button!==0 直接丢弃 → 完全没反应；
   *  WebView2 还可能顺手把侧键当历史前进/后退，两处 preventDefault 都是为了阻断它
   *  （mousedown/mouseup 各挡一次：不同 Chromium 版本在不同阶段取消默认行为）。
   *  走 mouse 系事件而非 pointer 系：Chromium 对「非主键按下」不保证派发 pointerdown/up，
   *  但 mouse 系一定有；主键点击不受影响（这里只认 button 3/4）。 */
  function onContentMouseDown(e: MouseEvent) {
    if (e.button === 3 || e.button === 4) e.preventDefault();
  }

  function onContentMouseUp(e: MouseEvent) {
    if (e.button !== 3 && e.button !== 4) return;
    e.preventDefault();
    runAction(e.button === 3 ? "prev" : "next");
  }

  function onBodyClick(e: MouseEvent) {
    const a = (e.target as HTMLElement | null)?.closest?.("a") as HTMLAnchorElement | null;
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    const href = a.getAttribute("href") || "";
    if (!href || href.startsWith("#")) return;
    if (/^https?:/i.test(href)) {
      copyText(href).then(() => toast(t("reader.toast.linkCopied")));
      return;
    }
    const clean = decodeURIComponent(href.split("#")[0]);
    const base = (clean.split(/[\\/]/).pop() || "").toLowerCase();
    if (!base || !base.includes(".")) return;
    const idx = book?.docs?.findIndex((d) => d.toLowerCase() === base) ?? -1;
    if (idx >= 0) {
      loadChapter(idx);
    } else {
      toast(t("reader.toast.jumpFailed", { target: base }));
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

  /** DEV 输入日志条目（__frDebug.inputLog）：取证用，release 构建随 DEV 块一起被摇掉。
   *  记 mouseup 而非 pointerup：侧键（button 3/4）不一定有 pointer 系事件，而 mouse 系一定有，
   *  且我们的侧键翻页本就挂在 mouseup 上。各事件类型不共有的字段用 -1/false 占位，避免类型断言。 */
  type InputLogRec = {
    type: "wheel" | "keydown" | "mouseup";
    key: string;
    code: string;
    deltaMode: number;
    deltaX: number;
    deltaY: number;
    button: number;
    ctrlKey: boolean;
    shiftKey: boolean;
    altKey: boolean;
    repeat: boolean;
    target: string;
    t: number;
  };
  /** DEV 输入日志监听的注销函数；release 构建下恒为 undefined。 */
  let devInputOff: (() => void) | undefined;

  onMount(() => {
    // 开发期调试钩子：便于自动化验证搜索框/笔记交互（仅 DEV 构建，release 不含）。
    if (import.meta.env.DEV) {
      // 输入取证：映射设备到底发 wheel 还是 keydown、deltaMode/deltaX 各是多少，
      // 用户自己也说不清，这里环形留最近 50 条原始事件供 __frDebug.inputLog() 读取。
      // 用捕获阶段挂 window：即使后面有 stopPropagation，日志也一定先记到。
      const inputLog: InputLogRec[] = [];
      const pushInput = (rec: Omit<InputLogRec, "t">) => {
        inputLog.push({ ...rec, t: Date.now() });
        if (inputLog.length > 50) inputLog.splice(0, inputLog.length - 50);
      };
      const classNameOf = (t: EventTarget | null): string => {
        const el = t as Element | null;
        return el && typeof el.className === "string" ? el.className : "";
      };
      const onInWheel = (e: WheelEvent) =>
        pushInput({
          type: "wheel", key: "", code: "",
          deltaMode: e.deltaMode, deltaX: e.deltaX, deltaY: e.deltaY, button: e.button,
          ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, repeat: false,
          target: classNameOf(e.target),
        });
      const onInKey = (e: KeyboardEvent) =>
        pushInput({
          type: "keydown", key: e.key, code: e.code,
          deltaMode: -1, deltaX: 0, deltaY: 0, button: -1,
          ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, repeat: e.repeat,
          target: classNameOf(e.target),
        });
      const onInMouse = (e: MouseEvent) =>
        pushInput({
          type: "mouseup", key: "", code: "",
          deltaMode: -1, deltaX: 0, deltaY: 0, button: e.button,
          ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, repeat: false,
          target: classNameOf(e.target),
        });
      window.addEventListener("wheel", onInWheel, true);
      window.addEventListener("keydown", onInKey, true);
      window.addEventListener("mouseup", onInMouse, true);
      devInputOff = () => {
        window.removeEventListener("wheel", onInWheel, true);
        window.removeEventListener("keydown", onInKey, true);
        window.removeEventListener("mouseup", onInMouse, true);
      };

      (window as any).__frDebug = {
        // i18n-allow:用户内容：全文搜索输入框的默认关键词，是用户输入的初值而非界面文案。
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
        /** 最近 50 条输入事件（wheel/keydown/mouseup）的副本，按到达顺序。 */
        inputLog: () => inputLog.slice(),
      };
    }
    ttsSetCallbacks({
      onSegment: (seg) => onTtsSegment(seg),
      onEnd: () => { void onTtsChapterEnd(); },
    });
    scroller = new SmoothScroller(applyScroll);
    scroller.onSettled((ratio) => {
      // settle 时精确同步一次响应式位置（滚动中仅节流更新）
      const p = Math.round(scroller!.pos);
      lastUiSync = performance.now();
      scrollRender = p;
      if (scrollTop !== p) scrollTop = p;
      saveProgressDebounced();
      // 漫画纵向：窗口边界上被 consumeEdge 丢弃的越界行程在这里补滚一次（此时窗口已补齐，
      // 是续滚而不是回弹）。carry 本身来自人造边界，丢弃它会让连续滚轮一格一格顿挫。
      if (isMangaMode() && scrollLayout()) {
        const c = pendingMangaCarry;
        pendingMangaCarry = 0;
        if (c > 0 && maxScroll - scroller!.targetPos > 2) {
          const res = scroller!.wheelBy(c);
          if (res.edge !== 0) scroller!.consumeEdge();
        }
        void ratio;
      }
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
      devInputOff?.(); // 仅 DEV 注册过输入取证监听；release 下是 undefined
      scroller?.destroy();
      if (statsTimer) clearInterval(statsTimer);
      if (mangaPanRaf) cancelAnimationFrame(mangaPanRaf);
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
    const ro = new ResizeObserver(() => scheduleRelayout());
    if (c) ro.observe(c);
    if (w) ro.observe(w);
    if (sw) ro.observe(sw);
    const onWin = () => scheduleRelayout();
    window.addEventListener("resize", onWin);
    // 按需/懒加载图片在渲染后才解码，加载完成会改变排版：捕获阶段监听 img load 重新测量。
    // 漫画瀑布流走独立的 rAF 合并口：img load 的连续触发会把 160ms 防抖 scheduleRelayout 饿死，
    // 导致滚动范围长期与真实 scrollHeight 不一致（maxScroll 偏大 → 卡死链）。
    const onImgLoad = (ev: Event) => {
      const t = ev.target as HTMLElement | null;
      if (!t || t.tagName !== "IMG") return;
      if (mangaFlow()) scheduleFlowRemap(true);
      else scheduleRelayout();
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

  /** 当前书的有效阅读底色 / 字色（本书方案 > 全局 config.reader > 主题预设；漫画模式可用独立底色）。 */
  function readerColors(): { bg: string; fg: string } {
    const t = THEMES[effTheme()];
    // 漫画模式可独立设置背景色（空则跟随普通主题/自定义背景）。
    const mangaBg = isMangaMode() && config.reader.manga.bgColor ? config.reader.manga.bgColor : "";
    return { bg: mangaBg || effBgColor() || t.bg, fg: effFgColor() || t.fg };
  }

  /**
   * 阅读区「表面令牌」子集：--rdr-bg/-fg 及其派生量（surface / line / radius / shadow）。
   * 单独抽出来是因为书内搜索浮层要 portal 到 body —— 一旦脱离 .reader 子树就拿不到
   * readerVars() 写在根节点上的内联值，只能由它在自身根元素上补一份。
   * 这里复用同一份 readerColors()，故浮层与它所浮着的书页永远同源（含「仅本书方案」覆盖）。
   */
  function readerSurfaceVars(): string {
    const { bg, fg } = readerColors();
    return [
      `--rdr-bg:${bg}`,
      `--rdr-fg:${fg}`,
      // 浮层配方：底色由阅读底/字做一次固定的轻微提亮/压暗（亮色主题变深、暗色主题变亮，
      // 方向自动正确），描边由阅读字色派生低 alpha 发丝边。见 app.css 的 .rdr-pop。
      `--rdr-surface:color-mix(in srgb, var(--rdr-bg) 94%, var(--rdr-fg))`,
      `--rdr-line:color-mix(in srgb, var(--rdr-fg) 22%, transparent)`,
      `--rdr-radius:var(--radius-md)`,
      `--rdr-shadow-pop:var(--shadow-md)`,
      `--rdr-shadow-panel:var(--shadow-drawer)`,
    ].join(";");
  }

  function readerVars(): string {
    // 有效阅读配色：优先本书单独套用的方案，其次全局。主题预设取有效主题。
    const t = THEMES[effTheme()];
    const { bg, fg } = readerColors();
    return [
      ...readerSurfaceVars().split(";"),
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
    const hideTextRule = isMangaMode() && r.manga.hideText // i18n-allow:算法输入：下面模板里带中文的是注入样式表的 CSS 注释，随 CSS 文本进样式块，不是界面文案。
      ? `
/* 漫画模式「仅图片」：隐藏不含图片的文本块 */
.reader.manga #chapter-content :is(p,div,span,h1,h2,h3,h4,h5,h6,li,blockquote,figcaption):not(:has(img,svg,video)) { display: none !important; }
`
      : "";
    // i18n-allow:算法输入：注入样式表里的 CSS 注释，随 CSS 文本进样式块，不是界面文案。
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
    // 边距改成三作用域记录：逐作用域各读一次，任一作用域改了都要重算分页/滚动范围，
    // 否则会出现「改了边距当前章节不重排，翻章才对」。
    void config.reader.margins.scroll.h;
    void config.reader.margins.scroll.v;
    void config.reader.margins.page.h;
    void config.reader.margins.page.v;
    void config.reader.margins.duo.h;
    void config.reader.margins.duo.v;
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
    if (!a || a === "none") return t("common.none");
    return labelOfId("bind", a);
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
    toast(t("reader.toast.bookmarkAdded"));
    if (bookmarksOpen) bookmarks = await api.listBookmarks(book.book.id);
    // 广播给独立“书签”窗口（按 bookId 过滤后重拉列表）
    void emit("tidepage-bookmarks-changed", { bookId: book.book.id });
  }

  async function deleteBookmark(id: number) {
    try {
      await api.deleteBookmark(id);
      bookmarks = bookmarks.filter((b) => b.id !== id);
      toast(t("reader.toast.bookmarkDeleted"));
      // 广播给独立“书签”窗口（按 bookId 过滤后重拉列表）
      if (book) void emit("tidepage-bookmarks-changed", { bookId: book.book.id });
    } catch (e) {
      toast(errText(e), "error");
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
      toast(t("reader.toast.hlCreated"));
    } catch (e) {
      toast(errText(e), "error");
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
      toast(errText(e), "error");
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
      toast(errText(e), "error");
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
      toast(note.trim() ? t("reader.toast.noteSaved") : t("reader.toast.noteCancelled"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  /** H3：把 AI 输出追加到某条划线的 note（不覆盖已有感想）。 */
  async function hlAppendNote(h: HighlightRow, addition: string) {
    try {
      const next = await api.appendHighlightNote(h.id, addition);
      h.note = next;
      hlVersion++;
      toast(t("reader.toast.addedToBookNote"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  async function hlDelete(h: HighlightRow) {
    const ok = await appConfirm(t("reader.confirm.hlDelete", { text: h.text.slice(0, 60) }));
    if (!ok) return;
    try {
      await api.deleteHighlight(h.id);
      const i = highlights.findIndex((x) => x.id === h.id);
      if (i >= 0) highlights.splice(i, 1);
      hlMenu = null;
      hlVersion++;
      await tick();
      paintNow();
      toast(t("reader.toast.hlDeleted"));
    } catch (e) {
      toast(errText(e), "error");
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
      toast(t("reader.toast.noNoteToAdd"), "error");
      return false;
    }
    // 来源①：已有划线
    if (q.highlightId != null) {
      const h = highlights.find((x) => x.id === q.highlightId);
      if (h && noteHasSegment(h.note, add)) {
        toast(t("reader.toast.noteAlreadySame"));
        return true;
      }
      try {
        const next = await api.appendHighlightNote(q.highlightId, add);
        if (h) h.note = next;
        hlVersion++;
        await tick();
        paintNow();
        toast(t("reader.toast.addedToNote"));
        return true;
      } catch (e) {
        toast(errText(e), "error");
        return false;
      }
    }
    // 来源②：新划选 → 先建划线（note 空）拿 id，再写入 note；写 note 失败则回滚删除，杜绝无 note 残留划线
    const s = q.sel;
    if (!s || !book || !chapter) {
      toast(t("reader.toast.noSelectionToNote"), "error");
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
      toast(t("reader.toast.hlCreateFailed", { err: String(e) }), "error");
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
        toast(t("reader.toast.noteRollbackFailed", { err: String(e) }), "error");
      } else {
        // 服务端未删成功 → 保留本地行（note 空，与服务端一致），如实提示可能残留
        hlVersion++;
        await tick();
        paintNow();
        toast(t("reader.toast.noteResidual"), "error");
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
    toast(t("reader.toast.hlCreatedWithNote"));
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
      toast(t("reader.toast.jumpedToChapter"));
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
    else toast(t("reader.toast.hlNotFound"));
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
      toast(t("reader.toast.noSystemTts"), "error");
      return;
    }
    const root = contentRoot();
    if (!root || !chapter) return;
    const text = root.textContent ?? "";
    const segs = splitSegments(text);
    if (!segs.length) {
      toast(t("reader.toast.noReadableText"));
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
      toast(t("reader.toast.readLastChapter"));
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

  /** 全书总字数。estTotalPages 只依赖它，原先每次调用都对 book.toc 做一遍 reduce。
   *  底栏状态项在滚动中会被反复求值（bookProgress 那档还依赖 scrollTop），
   *  于是「显示全书页数」开着时，长篇（数千章）的每次刷新都是 O(章节数) 次代理读。
   *  这里在开书时算一次存成标量；用普通变量而非 $state —— 它只随 book 变化，
   *  而 book 本身已是响应式的，estTotalPages 顶部的 book 解引用足以建立依赖。 */
  let tocCharsTotal = 0;

  /** 全书总页数（按当前章“每页多少字”估算；仅翻页模式有效）。 */
  function estTotalPages(): number {
    if (!book || scrollLayout() || mangaHorizontal()) return 0;
    const chars = chapter?.chars || 0;
    if (pageCount <= 0 || chars <= 0) return 0;
    const cpp = chars / pageCount;
    const total = tocCharsTotal;
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
        return t("common.chapterN", { n: chapterNum + 1 });
      case "chapterCount":
        return t("reader.status.chaptersTotal", { n: book.toc.length });
      case "chapterPages":
        return !scrollLayout() && !mangaHorizontal()
          ? t("reader.status.pageOf", { p: page + 1, n: pageCount })
          : `${Math.round(currentRatio() * 100)}%`;
      case "bookPages": {
        const tp = estTotalPages();
        return tp ? t("reader.status.pagesEst", { n: tp }) : "";
      }
      case "pagePercent":
        return `${Math.round(currentRatio() * 100)}%`;
      case "bookProgress":
        return `${totalPct().toFixed(1)}%`;
      case "chars": {
        const n = book.toc[chapterNum]?.chars || 0;
        return n ? t("common.unit.charsN", { n }) : "";
      }
      case "clock": {
        const d = new Date(nowTs);
        return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      }
      case "speed":
        return readSpeed > 0 ? t("reader.speed", { n: readSpeed }) : "";
      case "mode":
        if (mangaHorizontal()) return t("reader.mode.horizontal");
        if (mangaFlow()) return t("reader.mode.flow");
        return config.reader.mode === "page" ? (config.reader.duo ? t("reader.mode.duo") : t("reader.mode.page")) : t("reader.mode.scroll");
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
    const next = reorderByDrop(
      config.reader.statusItems.map((x) => ({ ...x })),
      from,
      target,
      place,
      (x) => x.id,
    );
    if (!next) return;
    config.reader.statusItems = next;
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
  class:vfade={!isMangaMode() && config.reader.mode === "scroll" && effMarginV() > 0}
  style={readerVars()}
  onwheel={onWheel}
  onmousemove={onReaderMove}
  onmouseleave={() => { fontCtlShow = false; syncChromeHover(false); }}
>
  {#if bgImageUrl}
    <div class="bg-layer" style="background-image:url('{bgImageUrl}'); opacity:{bookScheme()?.schemeBgOpacity ?? config.reader.bgOpacity};"></div>
  {/if}
  {#if showFsHint}
    <div class="fs-hint" transition:fade={{ duration: 220 }}>{t("reader.fullscreenExit")}</div>
  {/if}
  <div class="topbar" class:dim={chromeHidden} style="background:var(--rdr-top);" data-tauri-drag-region bind:clientHeight={tbH} bind:this={topbarEl}>
    <div class="tb-zone tb-l" data-tauri-drag-region>
      <button
        class="icon tb-btn tip-right"
        data-testid="reader-back"
        aria-label={standalone ? t("reader.close") : t("reader.back")}
        data-tip={standalone ? t("reader.closeEsc") : t("reader.backEsc")}
        onclick={goBack}
      >
        <IconArrowLeft size={20} stroke={1.5} />
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
          class:has-update={b.id === "settings" && !!updateState.latest}
          data-drag-id={b.id}
          data-testid={"tb-" + b.id}
          aria-label={label}
          data-tip={label}
          onpointerdown={(e) => tbPointerDown(b, e)}
          onpointerup={tbPointerUp}
          onpointerleave={tbPointerUp}
          onpointercancel={tbPointerUp}
          onclick={() => tbClick(b)}
        >
          <Icon size={20} stroke={1.5} />
          {#if b.id === "settings" && updateState.latest}
            <span class="update-dot abs" aria-hidden="true"></span>
          {/if}
        </button>
      {/each}
    </div>
  </div>

  {#if tbManagerOpen}
    <div class="tb-panel tb-manager rdr-pop" role="dialog" aria-label={t("reader.toolbar.manager")}>
      <div class="tb-panel-head"><span>{t("reader.manager.title")}</span><button class="icon" data-testid="tb-manager-close" title={t("common.action.close")} onclick={() => (tbManagerOpen = false)}><IconX /></button></div>
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
                <span class="tb-chip-label">{tbLabel(b)}</span>
              </label>
            {/if}
          {/each}
        </div>
        <p class="tb-panel-hint">{t("reader.manager.hint")}</p>
      </div>
    </div>
  {/if}

  {#if imgMenuOpen}
    <div class="tb-panel rdr-pop" role="menu" aria-label={t("reader.toolbar.images")}>
      <div class="tb-panel-head"><span>{t("reader.menu.imagesTitle")}</span><button class="icon" data-testid="tb-img-close" title={t("common.action.close")} onclick={() => (imgMenuOpen = false)}><IconX /></button></div>
      <div class="tb-panel-body">
        <button class="tb-menu-item" data-testid="tb-img-all" onclick={toggleAllImages}>{effShowImages() ? t("reader.menu.allImagesOff") : t("reader.menu.allImagesOn")}</button>
        <button class="tb-menu-item" data-testid="tb-img-block" onclick={startBlockPick}>{t("reader.menu.blockSelected")}</button>
        <button class="tb-menu-item" data-testid="tb-img-blocked" onclick={() => { imgMenuOpen = false; blockedListOpen = true; }}>{t("reader.menu.blockedCount", { n: blockedImages().length })}</button>
      </div>
    </div>
  {/if}

  {#if themeMenuOpen}
    <div class="tb-panel rdr-pop" bind:this={themeMenuEl} role="menu" aria-label={t("reader.toolbar.theme")} data-testid="tb-theme-panel">
      <div class="tb-panel-head"><span>{t("reader.toolbar.theme")}</span><button class="icon" data-testid="tb-theme-close" title={t("common.action.close")} aria-label={t("common.action.close")} onclick={() => (themeMenuOpen = false)}><IconX /></button></div>
      <div class="tb-panel-body">
        {#each THEME_ORDER as k (k)}
          <button class="tb-menu-item tb-theme-item" class:on={effTheme() === k} data-testid={"tb-theme-" + k}
            onclick={() => { applyTheme(k); themeMenuOpen = false; }}>
            <span class="tb-theme-swatch" style="background:{THEMES[k].bg}; color:{THEMES[k].fg};"></span>
            <span>{labelOfId("theme", k)}</span>
            {#if effTheme() === k}<IconCheck size={16} stroke={2} />{/if}
          </button>
        {/each}
      </div>
    </div>
  {/if}

  {#if wdMenuOpen}
    <!-- 关闭依赖窗口捕获阶段的外点监听（见下方 $effect），不再使用全屏遮罩元素。 -->
    <div class="tb-panel wd-menu rdr-pop" bind:this={wdMenuEl} role="menu" aria-label={t("reader.toolbar.webdav")} style="top:{wdMenuPos.top}px; right:{wdMenuPos.right}px">
      <div class="tb-panel-head"><span>{t("reader.menu.davTitle")}</span><button class="icon" data-testid="tb-dav-close" title={t("common.action.close")} aria-label={t("common.action.close")} onclick={() => (wdMenuOpen = false)}><IconX /></button></div>
      <div class="tb-panel-body">
        <button class="tb-menu-item" data-testid="tb-dav-upload" onclick={uploadBook} disabled={!book}>{t("reader.menu.davUploadBook")}</button>
      </div>
    </div>
  {/if}

  {#if blockedListOpen}
    <div class="tb-panel rdr-pop" role="dialog" aria-label={t("reader.menu.blockedTitle")}>
      <div class="tb-panel-head"><span>{t("reader.menu.blockedTitle")}</span><button class="icon" data-testid="tb-blocked-close" title={t("common.action.close")} onclick={() => (blockedListOpen = false)}><IconX /></button></div>
      <div class="tb-panel-body">
        {#if blockedImages().length === 0}
          <div class="tb-panel-empty">{t("reader.menu.blockedEmpty")}</div>
        {:else}
          {#each blockedImages() as src (src)}
            <div class="tb-block-row">
              <img class="tb-block-thumb" src={src} alt="" />
              <button class="tb-menu-item" data-testid="tb-unblock" onclick={() => unblockImage(src)}>{t("reader.menu.unblock")}</button>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  {/if}

  {#if mangaGuideOpen}
    <div class="tb-panel manga-guide rdr-pop" role="dialog" aria-label={t("reader.manga.sizeLabel")}>
      <div class="tb-panel-head"><span>{t("reader.manga.guideTitle")}</span><button class="icon" data-testid="tb-manga-close" title={t("common.action.close")} onclick={closeMangaGuide}><IconX /></button></div>
      <div class="tb-panel-body">
        <p class="tb-panel-hint">{t("reader.manga.guideHint")}</p>
        <div class="manga-slider">
          <span>{t("reader.manga.small")}</span>
          <input
            type="range"
            min="0.3"
            max="3"
            step="0.1"
            value={mangaGuideScale}
            oninput={onMangaGuideInput}
          />
          <span>{t("reader.manga.large")}</span>
          <span class="manga-val">{mangaGuideScale.toFixed(1)}×</span>
        </div>
        <div class="manga-actions">
          <button class="tb-menu-item" data-testid="tb-manga-done" onclick={closeMangaGuide}>{t("common.action.done")}</button>
          <button class="tb-menu-item" data-testid="tb-manga-reset" onclick={resetMangaGuide}>{t("reader.manga.resetDefault")}</button>
        </div>
      </div>
    </div>
  {/if}

  <div class="body" bind:this={bodyEl}>
    {#if openError}
      <div class="empty-state error-box">
        <div>
          <p data-testid="reader-open-failed">{t("reader.open.failed")}</p>
          <p class="err-detail">{openError}</p>
          <button data-testid="reader-open-back" onclick={() => (ui.view = "library")}>{labelOfId("bind", "close")}</button>
        </div>
      </div>
    {:else if !book}
      <div class="empty-state" data-testid="reader-opening">{t("reader.open.opening")}</div>
    {:else}
      <div class="stage">
        <div
          class="content"
          bind:this={contentEl}
          onclick={onBodyClick}
          onpointerdown={onContentPointerDown}
          onpointerup={onContentPointerUp}
          onmousedown={onContentMouseDown}
          onmouseup={onContentMouseUp}
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
            <div class="loading" aria-label={t("reader.loadingAria")} aria-busy="true">
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
      <div class="footer" class:dim={chromeHidden} bind:this={footerEl}>
        <span
          class="finfo finfo-l"
          role="button"
          tabindex="0"
          aria-label={t("reader.prevChapter")}
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
          aria-label={t("reader.progress.aria")}
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
          aria-label={t("reader.nextChapter")}
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
      <div class="panel-slot" out:outSlideL>
        <TocPanel toc={book.toc} current={chapterNum} onpick={(i) => { tocOpen = false; void loadChapter(i).then(alignStatsBaseline); }} onclose={() => (tocOpen = false)} onpopout={() => popOutPanel("toc")} />
      </div>
    {/if}

    {#if bookmarksOpen}
      <div class="sidepanel" out:outSlideR>
        <div class="sp-head" data-testid="bm-head" use:dragOutHandle={() => popOutPanel("bookmarks")} title={t("common.panel.popoutTip")}><span>{t("reader.bookmarks.title")}</span><div style="display:flex; gap:6px; align-items:center;"><button class="icon" data-testid="bm-popout" title={t("common.panel.popout")} onclick={() => popOutPanel("bookmarks")}><IconArrowsSplit2 /></button><button class="icon" data-testid="bm-close" title={t("common.action.close")} onclick={() => (rightPanel = null)}><IconX /></button></div></div>
        <div class="sp-list">
          {#each bookmarks as bm (bm.id)}
            <div class="sp-item bm-item">
              <div class="bm-main" onclick={() => { void loadChapter(bm.chapter_index).then(alignStatsBaseline); rightPanel = null; }}>
                <div class="bm-title">{bm.title || t("common.untitled")}</div>
                {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
              </div>
              <button class="icon bm-del" data-testid="bm-del" title={t("reader.bookmarks.deleteTip")} onclick={() => deleteBookmark(bm.id)}><IconTrash /></button>
            </div>
          {:else}
            <div class="sp-empty" data-testid="bm-empty">{t("reader.bookmarks.empty")}</div>
          {/each}
        </div>
      </div>
    {/if}

    {#if highlightsOpen && book}
      <div class="panel-slot" out:outSlideR>
      <HighlightsPanel
        bookId={book.book.id}
        version={hlVersion}
        onjump={jumpToHighlight}
        onclose={() => (rightPanel = null)}
        onchanged={refreshHighlights}
        onpopout={() => popOutPanel("highlights")}
      />
      </div>
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
      <div class="panel-slot" out:outSlideR>
        <AiPanel chapterTitle={chapter?.title ?? ""} chapterHtml={chapter?.html ?? ""} onclose={() => (rightPanel = null)} />
      </div>
    {/if}

    {#if statsOpen && book}
      <div class="panel-slot" out:outSlideR>
        <BookStatsModal bookId={book.book.id} bookTitle={book.book.title} onclose={() => (rightPanel = null)} onpopout={() => popOutPanel("stats")} />
      </div>
    {/if}

    {#if book && !openError}
      <div class="font-ctl" class:show={fontCtlShow}>
        <div class="ctl-row">
          <button class="icon" data-testid="font-dec" title={t("reader.font.decrease")} use:repeatAction={() => bumpFont(-1)}><IconTextDecrease /></button>
          <button class="icon" data-testid="font-inc" title={t("reader.font.increase")} use:repeatAction={() => bumpFont(1)}><IconTextIncrease /></button>
        </div>
        <div class="ctl-row">
          <button class="icon" data-testid="img-dec" title={t("reader.img.decrease")} use:repeatAction={() => bumpImgScale(-0.1)}><IconZoomOut /></button>
          <button class="icon" data-testid="img-inc" title={t("reader.img.increase")} use:repeatAction={() => bumpImgScale(0.1)}><IconZoomIn /></button>
          <button class="icon" class:on={effFitWidth()} data-testid="img-fit-w" title={t("reader.img.fitWidth")} onclick={toggleFitWidth}>{t("reader.img.fitWidthShort")}</button>
          <button class="icon" class:on={effFitHeight()} data-testid="img-fit-h" title={t("reader.img.fitHeight")} onclick={toggleFitHeight}>{t("reader.img.fitHeightShort")}</button>
        </div>
      </div>
    {/if}

  </div>

  <!-- 图库放在 .body 之外：.body 有 z-index 形成层叠上下文，放里面会被顶栏(--z-raised)盖住，
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
    <div class="fr-note-tip rdr-pop" style="left:{Math.round(hoverNote.x)}px; top:{Math.round(hoverNote.y)}px;">
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
      surfaceVars={readerSurfaceVars()}
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
    background: var(--rdr-bg);
    color: var(--rdr-fg);
    position: relative;
  }

  .bg-layer {
    position: absolute;
    inset: 0;
    background-size: cover;
    background-position: center;
    background-repeat: no-repeat;
    pointer-events: none;
    z-index: var(--z-base);
  }

  /* 任务 8：进入全屏时的大字号、居中淡入浮层提示（transition:fade 负责淡入/淡出） */
  .fs-hint {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: var(--z-toast);
    padding: 22px 40px;
    font-size: 30px;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: #fff;
    background: rgba(0, 0, 0, 0.62);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
    white-space: nowrap;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 7px var(--sp-3);
    /* 右侧留出自绘窗口控制按钮（约 3×30 + 间距 + 右边距 ≈ 100px）的宽度并留自然间距 */
    padding-right: 112px;
    background: color-mix(in srgb, var(--rdr-bg) 90%, black 10%);
    z-index: var(--z-raised);
    position: relative;
    user-select: none;
    /* 顶栏下方 1px 用顶栏背景色补齐：快速滚动时滚动层与顶栏交界处不会露出背景形成亮线/接缝 */
    box-shadow: 0 1px 0 0 var(--rdr-top);
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
    background: var(--rdr-line);
    pointer-events: none;
  }

  .btitle {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
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
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    opacity: 0.85;
    white-space: nowrap;
    cursor: grab;
    border-radius: var(--radius-xs);
    padding: 0 2px;
  }

  .stat-item {
    transition: background var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease-premium);
  }
  .stat-item:hover {
    background: color-mix(in srgb, var(--rdr-fg) 8%, transparent);
  }

  /* 按下 = 正在抓取：顶栏状态项可拖拽排序，此前只有 grab 光标没有按下反馈，
     拖起来与没拖起来在指针上完全一致。 */
  .stat-item:active {
    cursor: grabbing;
    transform: scale(0.96);
  }

  .stat-item.dragging {
    opacity: 0.4;
  }

  .stat-sep {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
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

  /* 阅读区内图标按钮跟随【阅读主题】而非外壳皮肤：外壳浅色 + 阅读夜间时，app.css 的
     button.icon（外壳 --fg、opacity .82）在 --rdr-top 上只有 ~1.2:1。这里改用 --rdr-fg，
     静止 0.9、hover 提亮，沿用「静止略暗 → hover 提亮」的既有约定；底也换成同一配方的
     --rdr-fg 8%（与 .stat-item:hover 相同），不引入任何硬编码颜色。 */
  .reader :global(button.icon) {
    color: var(--rdr-fg);
    opacity: 0.9;
  }
  .reader :global(button.icon:hover:not(:disabled)) {
    color: var(--rdr-fg);
    background: color-mix(in srgb, var(--rdr-fg) 8%, transparent);
    opacity: 1;
  }
  /* AI 抽屉有自己的配色命名空间：--ai-fg 来自 config.ai.style，用户关掉「跟随阅读样式」后
     可与阅读主题无关。上面那条通用规则会越界到 .ai-panel，此时图标会用阅读前景色画在 AI 面板
     的 --ai-bg 上，可能又低对比。这里给 AI 面板更具体的回落（(0,3,1)/(0,6,1) 均高于通用规则），
     --ai-fg 未定义时依旧回落 --rdr-fg。独立 AI 窗不在 .reader 内，不受影响。 */
  .reader :global(.ai-panel button.icon) {
    color: var(--ai-fg, var(--rdr-fg));
  }
  .reader :global(.ai-panel button.icon:hover:not(:disabled)) {
    color: var(--ai-fg, var(--rdr-fg));
  }

  /* 顶栏按钮管理 / 本书图片 / 已屏蔽列表 / 漫画引导 弹出面板。
     底色 / 描边 / 圆角 / 投影走全局 .rdr-pop（跟随阅读主题）。 */
  .tb-panel {
    position: absolute;
    top: 52px;
    right: 12px;
    width: 290px;
    max-height: 70vh;
    overflow: auto;
    z-index: var(--z-popover);
    padding: var(--sp-2);
    transform-origin: top right;
    animation: tb-panel-in var(--dur-page) var(--ease-premium) both;
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
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    padding: var(--sp-1) var(--sp-1) var(--sp-2);
  }

  .tb-panel-body { display: flex; flex-direction: column; gap: 2px; }

  /* 顶栏按钮管理：横排芯片式（替代竖排清单），窄面板自动换行；比清单更紧凑直观 */
  .tb-manager { width: 340px; }
  .tb-chip-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
    padding: 2px 2px var(--sp-1);
  }
  /* 芯片是药丸角色，用 --radius-pill（该值禁止用在普通按钮与卡片上）。
     transform 此前不在 transition 列表里：按下时全局 button:active 的
     translateY(0.5px) scale(0.985) 是瞬变的，与相邻芯片的插值读起来不是一件事。 */
  .tb-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 5px 11px 5px 9px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--rdr-line);
    background: transparent;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    line-height: 1;
    cursor: pointer;
    user-select: none;
    transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease), color var(--dur) var(--ease), opacity var(--dur) var(--ease),
      transform var(--dur) var(--ease-premium);
  }
  .tb-chip:hover { background: color-mix(in srgb, var(--rdr-fg) 9%, transparent); }
  .tb-chip.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent) 42%, transparent);
    color: var(--accent);
  }
  .tb-chip.fixed { cursor: default; opacity: 0.75; }
  .tb-chip.na { opacity: 0.42; }
  /* 开关芯片：勾上/取消时给一次轻微过冲，读作「咔哒扣上」。fixed/na 不参与。 */
  .tb-chip:not(.fixed):not(.na):active {
    transform: scale(0.94);
  }
  .tb-chip input { width: 13px; height: 13px; margin: 0; flex-shrink: 0; }
  .tb-chip-label { white-space: nowrap; }

  .tb-menu-item {
    text-align: left;
    background: transparent;
    border: none;
    color: inherit;
    padding: 8px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    transition: background var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease-premium);
  }
  .tb-menu-item:hover { background: color-mix(in srgb, var(--rdr-fg) 9%, transparent); }
  /* 此前只有 hover 底色、没有按下反馈：菜单项一长列按下时完全看不出点到了哪一条。 */
  .tb-menu-item:active {
    background: color-mix(in srgb, var(--rdr-fg) 13%, transparent);
    transform: scale(0.985);
  }

  .tb-theme-item { display: flex; align-items: center; gap: 8px; }
  .tb-theme-swatch { width: 16px; height: 16px; flex: none; border-radius: var(--radius-xs); border: 1px solid var(--rdr-line); }
  .tb-theme-item.on { color: var(--accent); }

  .tb-panel-hint { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); opacity: 0.6; margin: 8px 6px 2px; line-height: 1.5; }
  .tb-panel-empty { font-size: var(--fs-sm); letter-spacing: var(--ls-sm); opacity: 0.5; padding: 10px 8px; }
  .tb-block-row { display: flex; align-items: center; gap: 8px; padding: 4px 6px; }
  .tb-block-thumb {
    width: 44px;
    height: 44px;
    object-fit: cover;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--rdr-fg) 12%, transparent);
  }

  .manga-slider { display: flex; align-items: center; gap: 8px; padding: 6px; font-size: var(--fs-xs); letter-spacing: var(--ls-xs); }
  .manga-slider input[type="range"] { flex: 1; }
  .manga-val { min-width: 38px; text-align: right; opacity: 0.75; }
  .manga-actions { display: flex; gap: 8px; padding: 6px; }

  .font-ctl .icon.on { color: var(--accent); font-weight: 700; }

  .body {
    flex: 1;
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* 正文层压过背景图层：.body 形成层叠上下文（--z-raised），
       内部 .content / 侧面板都靠它与背景图分层。 */
    z-index: var(--z-raised);
  }

  /* 侧面板退场的承载层（说明见模板里 panel-slot 的注释）。
     铺满 .body 提供定位上下文 + 承接退场位移；pointer-events:none 让正文照常
     接收点击/框选，> * 显式 auto 把交互还给面板本身。
     刻意不写 z-index：面板的 --z-drawer 需要继续与正文层在同一层叠上下文中比较。 */
  .panel-slot {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  /* :global 是必须的：子元素是 TocPanel/AiPanel 等子组件的根元素，
     scoped 编译后会要求它带 Reader 的 scope class，而它带的是子组件的 class，
     普通后代选择器永远匹配不上（与 app.css 里那些 .tabler-icon 全局规则同理）。 */
  .panel-slot > :global(*) {
    pointer-events: auto;
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
    /* 滚动模式：上下边距从「内容 padding」改为「视口内缩」——
       原来只作用于正文首尾两端，滚动中文字会一路顶到顶栏/底栏；
       内缩后边距在整段滚动过程中始终生效。
       防塌陷：内缩最多到 (可视高-240px)/2，窗口很矮 + 边距拉满时不至于把阅读区压没。 */
    --rdr-inset-v: min(var(--rdr-mv, 80px), max(0px, calc((100% - 240px) / 2)));
    --rdr-edge: min(48px, var(--rdr-mv, 80px));
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

  /* 阅读区选区底色跟随「阅读主题」的 --rdr-sel（5 套主题 + 自由配色各自一份），
     与外壳区域的 ::selection（app.css，走 --accent）刻意不同。 */
  .content :global(::selection) {
    background: var(--rdr-sel);
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
    gap: var(--sp-1);
    z-index: var(--z-sticky);
    opacity: 0;
    pointer-events: none;
    transform: translateY(4px);
    transition: opacity var(--dur) var(--ease), transform var(--dur) var(--ease);
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
    /* fallback 同上：取真实默认值 320px，避免首帧先按 56px 排一版 */
    left: var(--rdr-mh, 320px);
    right: var(--rdr-mh, 320px);
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

  /* 滚动模式：正文滚动视口的上下内缩（值见 .content 的 --rdr-inset-v）。
     排除漫画瀑布流：它自带 padding: 0 var(--rdr-mh) 与原生滚动条，
     且 flow 布局本就声明不支持上下边距。 */
  .scroll-area:not(.manga-flow-viewport) {
    inset: var(--rdr-inset-v) 0;
  }

  /* 边界渐隐：用 mask-image 让接近边界处的文字 alpha 自然衰减；
     背景保持完全干净（不是盖一层半透明色，不会有脏灰塑料感），mask 交给合成器处理、不触发重排。
     只在 .reader.vfade 下挂 mask：--rdr-edge 为 0 时渐变会退化成硬切，
     故无上下边距 / 漫画 / 翻页模式一律彻底不挂。 */
  .reader.vfade .scroll-area:not(.manga-flow-viewport) {
    -webkit-mask-image: linear-gradient(
      to bottom,
      transparent 0px,
      rgba(0, 0, 0, 0.4) calc(var(--rdr-edge) * 0.4),
      black var(--rdr-edge),
      black calc(100% - var(--rdr-edge)),
      rgba(0, 0, 0, 0.4) calc(100% - (var(--rdr-edge) * 0.4)),
      transparent 100%
    );
    mask-image: linear-gradient(
      to bottom,
      transparent 0px,
      rgba(0, 0, 0, 0.4) calc(var(--rdr-edge) * 0.4),
      black var(--rdr-edge),
      black calc(100% - var(--rdr-edge)),
      rgba(0, 0, 0, 0.4) calc(100% - (var(--rdr-edge) * 0.4)),
      transparent 100%
    );
  }

  .col-scroll {
    /* 正常文档流：高度=正文高度，作为 .scroll-area 的可滚动内容，由浏览器按需分块绘制。
       不再 position:absolute + will-change:transform 把整章提升为超长合成层（撕裂/空白的根因）。 */
    /* 上下留白用 --rdr-edge（=min(48px, --rdr-mv)，见 .content）：章节首/末行永远落在边界渐隐带
       之外，否则滚到章末时最后一行会永久停在半透明区里；左右仍是 --rdr-mh。
       fallback 取与 readerVars() 一致的默认值，避免首帧先按别的值排一版再跳。 */
    padding: var(--rdr-edge, 48px) var(--rdr-mh, 320px);
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
    background: var(--rdr-bg);
    /* 漫画模式的边距默认 8px（config.reader.manga.marginV/H） */
    padding: var(--rdr-mv, 8px) 0;
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

  /* 一个目录章可能跨多个 spine 文档（后端 chapter_html_multi_ci 把每个文档包成
     <div class="epub-doc">…<img>…</div> 再拼接到同一 .manga-block）。横向模式必须让
     块内所有图片按阅读顺序「横向平铺」（每图一列），否则多图章会竖向堆叠在块内、
     块宽≈单页宽 → mangaMaxX≈0（滚轮无反应、第 2..N 图落在屏外够不到、左右键整章跳）。
     做法：块本身成为横排 flex 行，并把块内非媒体包裹层 display:contents 摊平，
     使任意嵌套深度的 img/svg/video 都直接成为该行的 flex item。 */
  .reader.manga-h .manga-h-canvas :global(.manga-block) {
    flex: 0 0 auto !important;
    margin: 0 var(--rdr-img-gap, 0) 0 0 !important;
    padding: 0 !important;
    width: auto !important;
    display: flex !important;
    flex-direction: row !important;
    align-items: center !important;
    gap: var(--rdr-img-gap, 0) !important;
  }

  .reader.manga-h .manga-h-canvas.rtl :global(.manga-block) {
    margin: 0 0 0 var(--rdr-img-gap, 0) !important;
    flex-direction: row-reverse !important;
  }

  /* 摊平块内所有非媒体包裹层（原书的 div/p/section 等各级 wrapper），
     让嵌套的图片直接参与 .manga-block 的横排 flex，而非被 wrapper 的块级流竖排。
     img/svg/video 本身保留盒子（下方规则置 margin:0 并给定高度）。
     关键：:not(svg *) 把内联 SVG 的内部元素（g/path/text…）排除在摊平之外——
     display:contents 会打乱 SVG 自身的渲染，SVG 子树须整体作为一个媒体盒子参与横排。 */
  .reader.manga-h .manga-h-canvas :global(.manga-block *:not(img):not(svg):not(video):not(svg *)) {
    display: contents !important;
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
    padding: 0 var(--rdr-mh, 8px);
    box-sizing: border-box;
    overflow-y: auto !important;
    overflow-x: hidden !important;
    overflow-anchor: none;
  }

  .manga-flow-canvas {
    width: 100%;
    /* 不再常驻 will-change: transform：这里承载的是可能两万像素高的滚动内容，
       强制提升为合成层等于要一块超纹理上限的图层（闪白/撕裂/掉帧），与 .scroll-area 同理。
       横向布局的 .manga-h-canvas 是固定视口内的内容层，保留。 */
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

  /* 激活态：外壳 --accent 在夜读底上只有 ~1.3:1，故取一个与 --rdr-fg 同族的强调色。
     accent 占比经 _test/reader_icon_contrast.mjs 对 5 主题 × 2 外壳扫描：25% 是仍能保住
     ≥4.5:1 的最大占比（30% 时 green/dark 掉到 4.43）。 */
  .reader .tb-btn.on {
    color: color-mix(in srgb, var(--accent) 25%, var(--rdr-fg));
    font-weight: 700;
    opacity: 1;
  }

  /* 激活态 + hover：上面这条 (0,3,0) 压不过 hover 规则 —— button.icon:hover:not(:disabled)
     的 :not(:disabled) 把 :disabled 的特异性算了进来，实际是 (0,4,1)，所以不给 .on 单独补一条
     hover 版的话，一悬停强调色就退回普通 --rdr-fg。这里 (0,5,1) 稳定胜出（不用 !important）。 */
  .reader :global(button.icon.on:hover:not(:disabled)) {
    color: color-mix(in srgb, var(--accent) 25%, var(--rdr-fg));
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
    border: 1px solid var(--rdr-line);
    padding: 4px 8px;
  }

  #chapter-content :global(mark.fr-hl) {
    background: transparent;
    color: inherit;
    padding: 0;
    cursor: pointer;
  }

  #chapter-content :global(mark.fr-tts) {
    background: color-mix(in srgb, var(--accent) 32%, transparent);
    color: inherit;
    border-radius: var(--radius-xs);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent);
  }

  #chapter-content :global(mark.fr-hl-underline) {
    text-decoration-line: underline;
    text-decoration-color: var(--hl-color);
    text-decoration-thickness: 2px;
    text-underline-offset: 2px;
  }

  #chapter-content :global(mark.fr-hl-wavy) {
    text-decoration-line: underline;
    text-decoration-style: wavy;
    text-decoration-color: var(--hl-color);
    text-decoration-thickness: 2px;
    text-underline-offset: 3px;
  }

  #chapter-content :global(mark.fr-hl-dotted) {
    text-decoration-line: underline;
    text-decoration-style: dotted;
    text-decoration-color: var(--hl-color);
    text-decoration-thickness: 2px;
    text-underline-offset: 2px;
  }

  #chapter-content :global(mark.fr-hl-double) {
    text-decoration-line: underline;
    text-decoration-style: double;
    text-decoration-color: var(--hl-color);
    text-decoration-thickness: 2px;
    text-underline-offset: 2px;
  }

  #chapter-content :global(mark.fr-hl-strike) {
    text-decoration-line: line-through;
    text-decoration-color: var(--hl-color);
    text-decoration-thickness: 2px;
  }

  #chapter-content :global(mark.fr-hl-background) {
    background: color-mix(in srgb, var(--hl-color) 38%, transparent);
    border-radius: var(--radius-xs);
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
    background: var(--hl-color);
    vertical-align: super;
  }

  /* H2 感想浮层：底色 / 描边 / 圆角 / 投影走全局 .rdr-pop（跟随阅读主题） */
  .fr-note-tip {
    position: fixed;
    z-index: var(--z-popover);
    transform: translate(-50%, calc(-100% - 8px));
    max-width: 320px;
    padding: var(--sp-2) 10px;
    pointer-events: none;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    animation: fr-rise-in var(--dur-fast) var(--ease-premium) both;
  }
  .fr-note-tip-quote {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.6;
    margin-bottom: var(--sp-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fr-note-tip-body { line-height: 1.6; white-space: pre-wrap; }

  #chapter-content :global(mark.fr-hl-box) {
    box-shadow: 0 0 0 1.5px var(--hl-color);
    border-radius: var(--radius-xs);
  }

  /* 拖拽选区提示框（选区高亮的可视边框） */
  .zone-hints {
    position: absolute;
    inset: 0;
    z-index: var(--z-raised);
    pointer-events: none;
  }

  .zh {
    position: absolute;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: color-mix(in srgb, var(--rdr-fg) 65%, transparent);
    background: color-mix(in srgb, var(--rdr-fg) 10%, transparent);
    border: 1px dashed color-mix(in srgb, var(--rdr-fg) 40%, transparent);
    border-radius: var(--radius-sm);
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

  /* 可视宽约 5px，但用 13px 透明命中区（左右各 4px padding），thumb 居中便于拖动。
     颜色复用全局滚动条 token（--scrollbar-thumb / --scrollbar-thumb-hover），
     使它与其余滚动面观感一致；命中区更宽是拖拽功能所需，刻意与全局不同。 */
  .scrollbar {
    position: absolute;
    right: 1px;
    /* 与内缩后的文字区对齐：轨道高度 == scrollArea.clientHeight - 28，
       正好等于 applyScroll() 的 thumb 行程（clientHeight - 28），两侧不会错位。 */
    top: calc(14px + var(--rdr-inset-v, 0px));
    bottom: calc(14px + var(--rdr-inset-v, 0px));
    width: 13px;
    padding: 0 4px;
    box-sizing: border-box;
    z-index: var(--z-sticky);
    cursor: pointer;
    touch-action: none;
  }

  /* 命中区固定 5px，hover 加粗靠 scale 的 X 轴 —— width 是触发重排的属性，在滚动条上尤其危险
     （每帧都在算滚动几何）。
     用独立的 scale 属性而不是 transform：JS 在 pointermove 里每帧写 thumb.style.transform，
     同属性会互相覆盖；scale 与 transform 是两个独立属性、按规范叠加，两者可以共存。
     只给 X 轴值（1.6 1）：.thumb 高度是模板内联的 height:8%，两轴缩放会让它从中心向上下
     额外扩出 0.3 倍高度，探出轨道 top，且缩放发生在滚动位置计算之后，hover 期间 thumb 的
     视觉中心会与它代表的滚动位置错位，拖拽时手感对不上。 */
  .thumb {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    width: 5px;
    margin: 0 auto;
    background: var(--scrollbar-thumb);
    border-radius: var(--radius-pill);
    transform-origin: center center;
    will-change: transform;
    transition: background var(--dur-fast) var(--ease), scale var(--dur-fast) var(--ease);
  }

  .scrollbar:hover .thumb,
  .scrollbar.dragging .thumb {
    scale: 1.6 1;
    background: var(--scrollbar-thumb-hover);
  }

  /* 加载骨架：用与正文同形的灰条替代「加载中…」药丸，减少版面跳动。
     这是保留 linear-gradient 的合理用途之一（骨架屏微光扫过）。 */
  .loading {
    position: absolute;
    top: 12%;
    left: 50%;
    transform: translateX(-50%);
    width: min(680px, 78%);
    z-index: var(--z-sticky);
    display: flex;
    flex-direction: column;
    gap: 14px;
    pointer-events: none;
  }

  .sk {
    display: block;
    height: 1em;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--rdr-fg) 12%, transparent);
    background-image: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--rdr-fg) 20%, transparent),
      transparent
    );
    background-size: 200% 100%;
    /* keyframes 定义在全局 app.css（Svelte 会重命名组件内 @keyframes，
       导致 app.css 的 reduce-motion 覆盖无法引用）；这里只引用其名字。
       曲线用 --ease 而非 ease-in-out —— 微光是匀速横扫最自然，
       ease-in-out 会让它「起步顿一下再冲出去」，读起来像卡了一下。 */
    animation: sk-shimmer 1.2s var(--ease) infinite;
  }

  .sk-title { height: 1.4em; width: 46%; margin: 0 auto var(--sp-2); }
  .sk-short { width: 68%; }

  .footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: var(--sp-1) 18px;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.8;
    position: relative;
    z-index: var(--z-base);
    user-select: none;
    transition: opacity var(--dur) var(--ease);
  }

  /* 可拖拽的进度刷：16px 是命中区高度（好点按），真正的轨道厚度取全局 --bar-h */
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
    height: var(--bar-h);
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--rdr-fg) 14%, transparent);
    pointer-events: none;
  }

  /* 填充条用 scaleX 走合成（不再动 width，拖动时不触发布局）。 */
  .progress-fill {
    position: absolute;
    left: 0;
    top: 50%;
    width: 100%;
    height: var(--bar-h);
    margin-top: calc(var(--bar-h) / -2);
    background: var(--accent);
    border-radius: var(--radius-pill);
    transform-origin: left center;
    z-index: var(--z-base);
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
    background: var(--accent);
    box-shadow: var(--shadow-sm);
    transform: translate(-50%, -50%);
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--dur-fast) var(--ease), left var(--dur) var(--ease);
    z-index: var(--z-raised);
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
    transition: opacity var(--dur-fast) var(--ease);
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
  /* 页码 / 进度百分比 / 字数都是数字：等宽对齐，翻页时不会左右抖动 */
  .fitem, .fsep, .finfo { font-variant-numeric: tabular-nums; }
  .fitem { opacity: 0.85; }
  .fsep { opacity: 0.45; margin: 0 4px; }

  /* 空状态走全局 .empty-state；这里只补「撑满剩余高度并垂直居中」这一条布局 */
  .empty-state {
    flex: 1;
    display: grid;
    place-items: center;
  }

  .error-box {
    opacity: 1;
  }

  .error-box p {
    margin: 6px 0;
    font-weight: 600;
  }

  .err-detail {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    font-weight: 400;
    opacity: 0.7;
    max-width: 480px;
    word-break: break-all;
    margin-bottom: 16px;
  }

  /* .sidepanel / .sp-head / .sp-list / .sp-item / .sp-empty 走 app.css 的全局基类
     （宽度由 --sidepanel-w 决定，底色/描边跟随阅读主题）。此处只留本面板特有部分。 */
  .bm-item { display: flex; align-items: flex-start; gap: var(--sp-2); }
  .bm-main { flex: 1; min-width: 0; cursor: pointer; }
  .bm-del { opacity: 0; flex-shrink: 0; transition: opacity var(--dur-fast) var(--ease); }
  .bm-item:hover .bm-del { opacity: 1; }

  .bm-title { font-size: var(--fs-sm); letter-spacing: var(--ls-sm); font-weight: 500; }
  .bm-excerpt {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.6;
    margin-top: 3px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
