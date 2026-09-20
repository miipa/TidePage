<script lang="ts">
  import { onMount, tick } from "svelte";
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
    BIND_DEFS,
    comboOf,
    bindKeyMatches,
    toggleAi,
    ZONE_DEFS,
    copyText,
    stripNonTextHtml,
    requestOpenBook,
    SCROLL_BOUNCE_PRESETS,
    type BindAction,
    type ZoneKey,
  } from "../lib/stores.svelte";
  import { SmoothScroller } from "../lib/smoothScroll";
  import { preloadReaderFonts } from "../lib/fontfaces";
  import { charRangeToRange, findTextRange, paintHighlights, selectionOffsets } from "../lib/highlights";
  import type { PopupAnchor } from "../lib/placePopup";
  import AiPanel from "./AiPanel.svelte";
  import TocPanel from "./TocPanel.svelte";
  import BookStatsModal from "./BookStatsModal.svelte";
  import SelectionMenu from "./SelectionMenu.svelte";
  import BookSearchPopup from "./BookSearchPopup.svelte";
  import QuickAiPopup from "./QuickAiPopup.svelte";
  import HighlightsPanel from "./HighlightsPanel.svelte";
  import HighlightMenu from "./HighlightMenu.svelte";
  import type { ChapterData, HighlightRow, OpenResult, Progress } from "../lib/types";

  let { standalone = false }: { standalone?: boolean } = $props();

  let book: OpenResult | null = $state(null);
  let chapter: ChapterData | null = $state(null);
  let loadingChapter = $state(false);
  let chapterNum = $state(0);

  let scrollWrap: HTMLDivElement | undefined = $state();
  let pageWrap: HTMLDivElement | undefined = $state();
  let contentEl: HTMLDivElement | undefined = $state();
  let bodyEl: HTMLDivElement | undefined = $state();
  let thumbEl: HTMLDivElement | undefined = $state();

  // A−/A＋ 悬浮控件：默认完全隐藏（pointer-events:none），仅指针进入右下角热区才淡入，
  // 避免透明热区常驻拦截正文框选/右下角翻页点击。
  let fontCtlShow = $state(false);
  let hoverRaf = 0;
  let hoverX = 0;
  let hoverY = 0;

  let scrollTop = $state(0);
  let maxScroll = $state(0);

  let page = $state(0);
  let pageCount = $state(1);
  let colW = 0;
  let gap = 48;
  let stepPx = 0;
  let contentW = $state(0);

  let scroller: SmoothScroller | null = null;

  let tocOpen = $state(false);
  let bookmarksOpen = $state(false);
  let statsOpen = $state(false);
  let bookmarks: { id: number; chapter_index: number; title: string; excerpt: string }[] = $state([]);

  let highlights: HighlightRow[] = $state([]);
  let highlightsOpen = $state(false);
  let hlVersion = $state(0);
  let pendingHighlightJump: HighlightRow | null = null;

  type SelMenuState = { text: string; start: number; end: number; anchor: PopupAnchor };
  let selMenu: SelMenuState | null = $state(null);
  let hlMenu: { h: HighlightRow; anchor: PopupAnchor } | null = $state(null);
  let searchPopup: { query: string; anchor: PopupAnchor } | null = $state(null);
  let quickAiPopup: { text: string; anchor: PopupAnchor } | null = $state(null);

  // 搜索结果跳转历史：可逐步/一键返回跳转前的位置
  type NavPos = { chapter_index: number; ratio: number };
  let searchNavStack: NavPos[] = $state([]);
  let searchOrigin: NavPos | null = null;

  let statsTimer: ReturnType<typeof setInterval> | undefined;
  let progressSaveTimer: ReturnType<typeof setTimeout> | undefined;
  let measureRaf = 0;
  let openError = $state("");
  let handledNonce = 0;

  let sessionId = 0;
  let lastPos = 0;
  let lastFlushTs = 0;

  const chapterCache = new Map<number, ChapterData>();

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
      book = await api.openBook(bookId);
      if (book.encoding_warning) console.log(book.encoding_warning);
      let restore: Progress | null = null;
      try {
        restore = await api.loadProgress(bookId);
      } catch { /* ignore */ }
      chapterNum = restore && restore.chapter_index < book.toc.length ? restore.chapter_index : 0;
      await loadChapter(chapterNum, restore ?? undefined);
      refreshHighlights();
      {
        const openedBookId = book.book.id;
        api
          .indexBookFts(openedBookId)
          .catch(() => {})
          .finally(() => { api.ensureBookText(openedBookId).catch(() => {}); });
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
      } catch { sessionId = 0; }
      statsTimer = setInterval(() => {
        if (document.visibilityState === "visible") flushSession();
      }, 30_000);
      if (config.webdav.enabled && config.webdav.syncOnOpen) {
        webdavSync(restore).catch(() => {});
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
    webdavPush();
    await doSaveProgress();
    await flushSession(true);
    sessionId = 0;
    if (statsTimer) clearInterval(statsTimer);
    scroller?.destroy();
    book = null;
    chapter = null;
  }

  // 退出书籍时把本地进度推送远端（同一软件多端同步，新的覆盖旧的）
  async function webdavPush() {
    if (!book || !chapter || !config.webdav.enabled || !config.webdav.syncOnOpen) return;
    const w = config.webdav;
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
    } catch { /* ignore */ }
  }

  async function webdavSync(localP: Progress | null) {
    if (!book) return;
    const w = config.webdav;
    const remote = await api.webdavSyncProgress(
      w.base, w.user, w.pass, book.book.id, book.book.path, localP,
    );
    if (remote.chapter_index !== chapterNum && remote.chapter_index < book.toc.length) {
      toast("已从 WebDAV 恢复阅读进度");
      await loadChapter(remote.chapter_index, remote);
    }
    if (config.webdav.uploadBooks) {
      api.webdavUploadBook(w.base, w.user, w.pass, book.book.path).catch(() => {});
    }
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
    loadingChapter = true;
    try {
      // 章节首次绘制前先确保设定字体已就绪，避免回退字体闪烁
      await preloadReaderFonts();
      let ch = chapterCache.get(idx);
      if (!ch) {
        ch = await api.getChapter(book.book.id, idx);
        chapterCache.set(idx, ch);
        if (chapterCache.size > 8) {
          const first = chapterCache.keys().next().value;
          if (first !== undefined && first !== idx) chapterCache.delete(first);
        }
        for (const n of [idx + 1, idx - 1]) {
          if (!chapterCache.has(n) && n >= 0 && n < book.toc.length) {
            api.getChapter(book.book.id, n).then((c) => chapterCache.set(n, c)).catch(() => {});
          }
        }
      }
      chapter = ch;
      chapterNum = idx;
      await tick();
      await fontsReady();
      scroller?.reset();
      resetScrollCommit();
      if (config.reader.mode === "scroll") {
        measureScroll();
        let pos = 0;
        if (entry) pos = entry.at === "tail" ? maxScroll - (entry.extra || 0) : entry.extra || 0;
        else if (restore) pos = restore.chapter_ratio * maxScroll;
        pos = Math.max(0, Math.min(pos, maxScroll));
        scroller?.jumpTo(pos, true);
        scheduleCommit();
        if (restore && restore.chapter_ratio > 0) {
          setTimeout(() => {
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
        gotoPage(Math.max(0, Math.min(pg, Math.max(0, pageCount - 1))), true);
        setTimeout(recheckPages, 350);
        if (entry?.at === "tail") {
          requestAnimationFrame(() => pinToEnd());
        }
      }
      try {
        localStorage.setItem(
          "fr.ai.ctx",
          JSON.stringify({
            title: `${book.book.title} · ${ch.title}`,
            html: aiCtxHtml(ch.html),
          })
        );
      } catch { /* ignore */ }
      maybeAutoAi();
      saveProgressDebounced();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      loadingChapter = false;
    }
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
    if (!pageWrap || config.reader.mode === "scroll") return;
    const before = pageCount;
    computePageCount();
    if (pageCount !== before) applyPage();
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
        new CustomEvent("flowread-ai-chapter", {
          detail: { title: ch.title, html: aiCtxHtml(ch.html) },
        }),
      );
      // 跨窗口广播：AI 独立窗口也能收到
      import("@tauri-apps/api/event")
        .then((m) =>
          m.emit("flowread-ai-chapter", {
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

  function saveProgressDebounced() {
    clearTimeout(progressSaveTimer);
    progressSaveTimer = setTimeout(doSaveProgress, 1200);
  }

  function currentRatio(): number {
    if (config.reader.mode === "scroll") return maxScroll > 0 ? Math.min(1, scrollTop / maxScroll) : 0;
    return pageCount > 1 ? Math.min(1, page / (pageCount - 1)) : 0;
  }

  function totalPct(): number {
    if (!book) return 0;
    const n = book.toc.length || 1;
    return Math.min(100, ((chapterNum + currentRatio()) / n) * 100);
  }

  async function doSaveProgress() {
    if (!book) return;
    const p: Progress = {
      chapter_index: chapterNum,
      chapter_ratio: currentRatio(),
      pct: totalPct(),
      updated_at: Date.now(),
      device: "pc",
    };
    try {
      await api.saveProgress(book.book.id, p);
    } catch { /* ignore */ }
  }

  function bookPos(): number {
    if (!book) return 0;
    let before = 0;
    for (let i = 0; i < chapterNum && i < book.toc.length; i++) before += book.toc[i].chars || 0;
    const cur = book.toc[chapterNum]?.chars || 0;
    return before + cur * currentRatio();
  }

  async function flushSession(force = false) {
    if (!book || !sessionId) return;
    const now = Date.now();
    const elapsed = (now - lastFlushTs) / 1000;
    if (!force && elapsed < 15) return;
    const pos = bookPos();
    const dChars = Math.max(0, Math.round(pos - lastPos));
    lastPos = pos;
    lastFlushTs = now;
    const secs = Math.min(Math.round(elapsed), 600);
    if (secs < 3 && dChars === 0) return;
    try {
      await api.updateSession(sessionId, secs, dChars);
    } catch { /* ignore */ }
  }

  /* ================= scroll mode ================= */
  let lastEdgeAt = 0;

  function measureScroll() {
    if (!contentEl || !scrollWrap) return;
    maxScroll = Math.max(0, scrollWrap.scrollHeight - contentEl.clientHeight);
    scroller?.setMax(maxScroll);
  }

  /// 递归 rAF 钉到章末：直到（图片等延迟加载后）滚动位置真正落底/页数稳定才停止。
  /// scroll：scrollTop + clientHeight >= scrollHeight - 5；page：钉住最后一页且 pageCount 不再变。
  function pinToEnd(r = 0) {
    if (r > 180) return;
    let done = false;
    if (config.reader.mode === "scroll") {
      measureScroll();
      scroller?.jumpTo(maxScroll, true);
      done =
        !contentEl ||
        !scrollWrap ||
        maxScroll - scrollRender <= 5 ||
        scrollRender + contentEl.clientHeight >= scrollWrap.scrollHeight - 5;
    } else {
      const before = pageCount;
      recheckPages();
      gotoPage(Number.MAX_SAFE_INTEGER, true);
      done = before === pageCount && (pageCount <= 1 || page >= pageCount - 1);
    }
    if (done) return;
    requestAnimationFrame(() => pinToEnd(r + 1));
  }

  let scrollBase = 0;
  /** 每帧更新的纯 DOM 渲染位置（非响应式，供 pinToEnd/保存进度即时读取）。 */
  let scrollRender = 0;
  /** 响应式 UI（进度百分比等）上次同步时间，节流到 ~150ms。 */
  let lastUiSync = 0;

  function applyScroll(renderPos: number) {
    scrollRender = renderPos;
    const p = Math.round(renderPos);
    // 每帧只做纯 DOM 写入：滚动层 transform + 滚动条 thumb transform（不改 top，避免布局）。
    if (scrollWrap) {
      const d = scrollBase - p;
      scrollWrap.style.transform = d === 0 ? "" : `translateY(${d}px)`;
    }
    if (thumbEl) {
      const trackH = (contentEl?.clientHeight || 0) - 28;
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

  function commitScroll() {
    if (!scrollWrap || !scroller) return;
    const p = Math.round(scroller.pos);
    scrollBase = p;
    scrollWrap.style.top = `${-p}px`;
    scrollWrap.style.transform = "";
  }

  let commitTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleCommit() {
    clearTimeout(commitTimer);
    commitTimer = setTimeout(commitScroll, 240);
  }

  function resetScrollCommit() {
    clearTimeout(commitTimer);
    scrollBase = 0;
    if (scrollWrap) {
      scrollWrap.style.top = "0px";
      scrollWrap.style.transform = "";
    }
    if (thumbEl) thumbEl.style.transform = "";
  }

  function handleScrollEdge(dir: number, carry: number) {
    const now = Date.now();
    if (now - lastEdgeAt < 320 || loadingChapter || !book) return;
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

  function computePageCount() {
    if (!pageWrap) return;
    const totalCols = Math.max(1, Math.round((pageWrap.scrollWidth + gap) / (colW + gap)));
    pageCount = Math.max(1, Math.ceil(totalCols / screenCols));
    page = Math.min(page, pageCount - 1);
  }

  /** 有效左右边距：maxLineChars>0 时忽略 marginH，按「每行 N 字」自动居中。 */
  function effMarginH(): number {
    const mlc = config.reader.maxLineChars;
    const w = contentW || contentEl?.clientWidth || 0;
    if (mlc > 0 && w > 0) return Math.max(0, (w - mlc * config.reader.fontSize) / 2);
    return config.reader.marginH;
  }

  function measurePages() {
    if (!contentEl || !pageWrap || !chapter) return;
    const mh = effMarginH();
    const mv = config.reader.marginV;
    const W = contentEl.clientWidth;
    const H = contentEl.clientHeight;
    const avail = Math.max(260, W - mh * 2);
    const vh = Math.max(240, H - mv * 2);
    gap = config.reader.colGap;
    screenCols = config.reader.duo ? 2 : 1;
    colW = Math.floor((avail - (screenCols - 1) * gap) / screenCols);
    const effAvail = screenCols * colW + (screenCols - 1) * gap;
    stepPx = screenCols * (colW + gap);
    const st = pageWrap.style;
    st.left = "0px";
    st.top = `${mv}px`;
    st.width = `${effAvail}px`;
    st.height = `${vh}px`;
    st.columnWidth = `${colW}px`;
    st.columnGap = `${gap}px`;
    cancelAnimationFrame(measureRaf);
    measureRaf = requestAnimationFrame(() => {
      computePageCount();
      applyPage();
    });
  }

  function reducedMotion(): boolean {
    return typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
  }

  /** 页面偏移统一由 transform 表达（GPU 合成）；animate=true 时仅用户主动翻页才启用过渡。 */
  function applyPage(animate = false) {
    if (!pageWrap) return;
    const x = -Math.round(page * stepPx);
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

  /** 阅读页内快捷字号调整（免进设置）。 */
  function bumpFont(delta: number) {
    const n = Math.max(10, Math.min(64, config.reader.fontSize + delta));
    if (n !== config.reader.fontSize) {
      config.reader.fontSize = n;
      saveConfig();
    }
  }

  /** 指针是否位于 .body 右下角热区（约 150×130），用 rAF 节流后短算。 */
  function onReaderMove(e: MouseEvent) {
    hoverX = e.clientX;
    hoverY = e.clientY;
    if (hoverRaf) return;
    hoverRaf = requestAnimationFrame(() => {
      hoverRaf = 0;
      const el = bodyEl;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const inCorner = hoverX >= r.right - 150 && hoverY >= r.bottom - 130;
      if (inCorner !== fontCtlShow) fontCtlShow = inCorner;
    });
  }

  function relayout() {
    if (!chapter) return;
    // 滚动模式下先同步节流的响应式位置，保证重排后按真实位置恢复
    if (config.reader.mode === "scroll") scrollTop = Math.round(scrollRender);
    const ratio = currentRatio();
    if (config.reader.mode === "scroll") {
      measureScroll();
      scroller?.jumpTo(ratio * maxScroll, true);
    } else {
      measurePages();
      gotoPage(Math.round(ratio * (pageCount - 1)), true);
    }
  }

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
    const d = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    if (config.reader.mode === "page") {
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
        config.reader.mode === "page" ? nextPage() : scrollStep(vh);
        break;
      case "prev":
        config.reader.mode === "page" ? prevPage() : scrollStep(-vh);
        break;
      case "scrollDown":
        scrollStep(vh * 0.4);
        break;
      case "scrollUp":
        scrollStep(-vh * 0.4);
        break;
      case "chapterNext":
        if (config.reader.mode === "scroll") handleScrollEdge(1, 0);
        else nextChapter(true);
        break;
      case "chapterPrev":
        prevChapter(true);
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
        toggleAi();
        break;
      case "bookStats":
        statsOpen = !statsOpen;
        break;
      case "settings":
        ui.settingsOpen = true;
        break;
      case "close":
        if (tocOpen || bookmarksOpen || statsOpen || highlightsOpen || selMenu || searchPopup || quickAiPopup || hlMenu) {
          tocOpen = false;
          bookmarksOpen = false;
          statsOpen = false;
          highlightsOpen = false;
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
    const tgt = e.target as HTMLElement | null;
    if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA" || tgt.isContentEditable)) return;

    // 任何弹窗/抽屉/确认框打开时都不响应阅读快捷键（Esc 关闭弹窗由 +page 全局处理）
    if (ui.settingsOpen || ui.searchOpen || ui.statsOpen || ui.webdavOpen || convertState.open || confirmBox.active) return;

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
    if (e.button !== 0) {
      ptr = null;
      return;
    }
    ptr = { x: e.clientX, y: e.clientY, t: Date.now(), id: e.pointerId };
  }

  function onContentPointerUp(e: PointerEvent) {
    if (!ptr || ptr.id !== e.pointerId) {
      ptr = null;
      return;
    }
    const moved = Math.hypot(e.clientX - ptr.x, e.clientY - ptr.y);
    const quick = Date.now() - ptr.t < 450;
    ptr = null;
    // 有选区时优先弹划词菜单（划线 / 全文 / 浏览器 / AI），不触发点击区域动作
    if (contentEl) {
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
    }
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
    speechSynthesis.cancel();
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
    scroller = new SmoothScroller(applyScroll);
    scroller.onSettled(() => {
      // settle 时精确同步一次响应式位置（滚动中仅节流更新）
      const p = Math.round(scroller!.pos);
      lastUiSync = performance.now();
      scrollRender = p;
      if (scrollTop !== p) scrollTop = p;
      saveProgressDebounced();
      if (config.reader.mode === "scroll") commitScroll();
    });
    const onJump = (e: Event) => {
      const idx = (e as CustomEvent<number>).detail;
      if (book && idx >= 0 && idx < book.toc.length) loadChapter(idx);
    };
    const onUnload = () => {
      doSaveProgress();
      flushSession(true);
    };
    window.addEventListener("flowread-jump", onJump);
    window.addEventListener("pagehide", onUnload);
    return () => {
      scroller?.destroy();
      if (statsTimer) clearInterval(statsTimer);
      window.removeEventListener("flowread-jump", onJump);
      window.removeEventListener("pagehide", onUnload);
      doSaveProgress();
      const el = document.getElementById("flowread-dynamic-css");
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
    const sw = scrollWrap;
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
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onWin);
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
    const t = THEMES[config.reader.theme];
    const bg = config.reader.bgColor || t.bg;
    const fg = config.reader.fgColor || t.fg;
    return [
      `--rdr-bg:${bg}`,
      `--rdr-fg:${fg}`,
      `--rdr-font:${fontStack(config.reader.fontFamily, config.reader.latinFont)}`,
      `--rdr-fs:${config.reader.fontSize}px`,
      `--rdr-lh:${config.reader.lineHeight}`,
      `--rdr-ps:${config.reader.paraSpacing}em`,
      `--rdr-mh:${effMarginH()}px`,
      `--rdr-mv:${config.reader.marginV}px`,
      `--rdr-indent:${config.reader.indent}`,
      `--rdr-align:${config.reader.textAlign}`,
      `--rdr-fw:${config.reader.fontWeight}`,
      `--rdr-sel:${t.sel}`,
      `--hl-color:${config.reader.highlightColor || "#c0392b"}`,
      `--rdr-top:${config.reader.topbarColor || "color-mix(in srgb, " + bg + " 90%, black 10%)"}`,
      `--rdr-stat:${config.reader.statusColor || t.fg}`,
      `--rdr-stat-align:${config.reader.statusPos === "none" ? "center" : config.reader.statusPos}`,
    ].join(";");
  }

  function buildDynamicCss(): string {
    const r = config.reader;
    const overrideRule = r.overrideBookCss
      ? `
#chapter-content { color: var(--rdr-fg) !important; font-family: var(--rdr-font) !important; text-align: var(--rdr-align) !important; }
#chapter-content p { font-size: var(--rdr-fs) !important; text-indent: var(--rdr-indent) !important; margin: var(--rdr-ps) 0 !important; line-height: var(--rdr-lh) !important; font-weight: var(--rdr-fw) !important; }
#chapter-content :is(h1,h2,h3,h4,h5,h6) { font-size: 1.2em !important; font-weight: 600 !important; text-align: center !important; margin: 0.4em 0 0.8em !important; }
#chapter-content :is(img,svg,video) { height: auto !important; max-width: 100% !important; }
#chapter-content sup { font-size: 0.7em !important; }
`
      : "";
    return `
${overrideRule}
${r.cssExtra || ""}
/* 硬编码根治发糊：正文统一禁止合成加粗/斜体（与 .ai-mark 一致），避免第三方字体假粗假斜发糊 */
:root #chapter-content { font-synthesis: none !important; }
`;
  }

  $effect(() => {
    const r = config.reader;
    void r.overrideBookCss;
    void r.cssExtra;
    const css = buildDynamicCss();
    if (!styleTag) {
      styleTag = document.createElement("style");
      styleTag.id = "flowread-dynamic-css";
      document.head.appendChild(styleTag);
    }
    styleTag.textContent = css;
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

  const bgImageUrl = $derived(
    config.reader.bgImage ? `http://flowread.localhost/bg/${config.reader.bgImage}` : ""
  );

  function zoneName(k: ZoneKey): string {
    const a = config.reader.zones[k];
    if (!a || a === "none") return "无";
    return BIND_DEFS.find((d) => d.id === a)?.name ?? a;
  }

  async function loadBookmarks() {
    if (!book) return;
    bookmarks = await api.listBookmarks(book.book.id);
    bookmarksOpen = !bookmarksOpen;
  }

  async function addBookmark() {
    if (!book || !chapter) return;
    const sel = window.getSelection()?.toString().slice(0, 80) ?? "";
    await api.addBookmark(book.book.id, chapterNum, chapter.title, sel);
    toast("书签已添加");
    if (bookmarksOpen) bookmarks = await api.listBookmarks(book.book.id);
  }

  async function deleteBookmark(id: number) {
    try {
      await api.deleteBookmark(id);
      bookmarks = bookmarks.filter((b) => b.id !== id);
      toast("书签已删除");
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

  async function hlDelete(h: HighlightRow) {
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
    quickAiPopup = { text: s.text, anchor: s.anchor };
  }

  function scrollRangeIntoView(range: Range) {
    const sel = window.getSelection();
    if (sel) {
      sel.removeAllRanges();
      sel.addRange(range);
    }
    if (config.reader.mode === "scroll") {
      if (!contentEl || !scroller) return;
      const contentRect = contentEl.getBoundingClientRect();
      const r = range.getBoundingClientRect();
      const target = scrollRender + (r.top - contentRect.top) - contentRect.height * 0.3;
      scroller.jumpTo(Math.max(0, Math.min(target, maxScroll)), true);
      scheduleCommit();
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
    if (!root) return;
    const range = findTextRange(root, needle);
    if (!range) {
      toast("已跳转到章节（未精确定位到文字）");
      return;
    }
    scrollRangeIntoView(range);
  }

  async function gotoNavPos(p: NavPos) {
    if (p.chapter_index !== chapterNum || !chapter) {
      await loadChapter(p.chapter_index);
      await tick();
    }
    applyRatio(p.ratio);
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
    highlightsOpen = false;
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
  }

  function speak() {
    if (!chapter) return;
    const text = chapter.html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").slice(0, 6000);
    const u = new SpeechSynthesisUtterance(text);
    u.lang = "zh-CN";
    speechSynthesis.cancel();
    speechSynthesis.speak(u);
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
    if (config.reader.mode === "scroll") scroller?.jumpTo(r * maxScroll, true);
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

  function statusText(): string {
    if (!book) return "正在打开…";
    const parts: string[] = [];
    parts.push(`第${chapterNum + 1}/${book.toc.length}章`);
    if (chapter?.title) parts.push(chapter.title);
    if (config.reader.mode === "page") parts.push(`${page + 1}/${pageCount}页`);
    else parts.push(`${Math.round(currentRatio() * 100)}%`);
    parts.push(`${totalPct().toFixed(1)}%`);
    return parts.join("  ·  ");
  }

  function progUp(e: PointerEvent) {
    if (!progDrag) return;
    progDrag = false;
    progSeek(progFromEvent(e)).then(() => saveProgressDebounced());
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<div class="reader" class:ovr={config.reader.overrideBookCss} style={readerVars()} onwheel={onWheel} onmousemove={onReaderMove} onmouseleave={() => (fontCtlShow = false)}>
  {#if bgImageUrl}
    <div class="bg-layer" style="background-image:url('{bgImageUrl}'); opacity:{config.reader.bgOpacity};"></div>
  {/if}
  <div class="topbar" style="background:var(--rdr-top);">
    <div class="tb-zone tb-l">
      <button class="icon" title={standalone ? "关闭 (Esc)" : "返回 (Esc)"} onclick={goBack}>←</button>
      <div class="btitle">{book?.book.title ?? ""}</div>
    </div>
    {#if config.reader.statusPos !== "none"}
      <div
        class="tb-zone tb-c"
        style="justify-content:{config.reader.statusPos === 'left' ? 'flex-start' : config.reader.statusPos === 'right' ? 'flex-end' : 'center'}"
      >
        <span class="stat-txt" style="color:var(--rdr-stat)">{statusText()}</span>
      </div>
    {/if}
    <div class="tb-zone tb-r">
      {#if config.webdav.enabled}
        <button class="icon" title="手动同步进度到 WebDAV" onclick={() => webdavSync(null).then(() => toast("进度已同步")).catch((e) => toast(String(e), "error"))}>⇅</button>
      {/if}
      {#if book?.book.format === "txt"}
        <button class="icon" title="转换格式" onclick={convertThis}>⇄</button>
      {/if}
      <button class="icon" title="添加书签" onclick={addBookmark}>🔖</button>
      <button class="icon" title="书签列表" onclick={loadBookmarks}>≡</button>
      <button class="icon" title="朗读本章" onclick={speak}>▶</button>
      <button class="icon" title="停止朗读" onclick={() => speechSynthesis.cancel()}>■</button>
      <button class="icon" title="划线 / 批注" onclick={() => (highlightsOpen = !highlightsOpen)}>✎</button>
      <button class="icon" title="AI 助手" onclick={toggleAi}>AI</button>
      <button class="icon" title="本书统计" onclick={() => (statsOpen = !statsOpen)}>📊</button>
      <button class="icon" title="目录 (F3)" onclick={() => (tocOpen = !tocOpen)}>☰</button>
      <button class="icon" title="设置" onclick={() => (ui.settingsOpen = true)}>⚙</button>
    </div>
  </div>

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
        >
          {#if config.reader.mode === "scroll"}
            <div class="scroll-area">
              <div class="col-scroll" bind:this={scrollWrap}>
                <div id="chapter-content">
                  {#if chapter}{@html chapter.html}
                  {:else}<p>加载中…</p>{/if}
                </div>
              </div>
            </div>
            <div class="scrollbar">
              <div class="thumb" bind:this={thumbEl} style="height:8%"></div>
            </div>
          {:else}
            <div class="clip">
              <div class="col-page" bind:this={pageWrap}>
                <div id="chapter-content">{@html chapter?.html ?? ""}</div>
              </div>
            </div>
          {/if}
          {#if loadingChapter}
            <div class="loading">加载中…</div>
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
      <div class="footer">
        <span class="chapinfo">{chapterNum + 1}/{book.toc.length} 章</span>
        <div
          class="progress-track"
          class:dragging={progDrag}
          role="slider"
          tabindex="-1"
          onpointerdown={progDown}
          onpointermove={progMove}
          onpointerup={progUp}
          onpointercancel={progUp}
        >
          <div class="progress-fill" style="width:{totalPct()}%"></div>
        </div>
        <span class="pctinfo">
          {#if config.reader.mode === "page"}{page + 1}/{pageCount} 页 · {:else}{Math.round(currentRatio() * 100)}% · {/if}{totalPct().toFixed(1)}%
        </span>
      </div>
    {/if}

    {#if tocOpen && book}
      <TocPanel toc={book.toc} current={chapterNum} onpick={(i) => { tocOpen = false; loadChapter(i); }} onclose={() => (tocOpen = false)} />
    {/if}

    {#if bookmarksOpen}
      <div class="sidepanel">
        <div class="sp-head"><span>书签</span><button class="icon" onclick={() => (bookmarksOpen = false)}>×</button></div>
        <div class="sp-list">
          {#each bookmarks as bm (bm.id)}
            <div class="sp-item bm-item">
              <div class="bm-main" onclick={() => { loadChapter(bm.chapter_index); bookmarksOpen = false; }}>
                <div class="bm-title">{bm.title || "未命名"}</div>
                {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
              </div>
              <button class="icon bm-del" title="删除书签" onclick={() => deleteBookmark(bm.id)}>×</button>
            </div>
          {:else}
            <div class="sp-empty">暂无书签，点 🔖 添加</div>
          {/each}
        </div>
      </div>
    {/if}

    {#if highlightsOpen && book}
      <HighlightsPanel
        bookId={book.book.id}
        version={hlVersion}
        onjump={jumpToHighlight}
        onclose={() => (highlightsOpen = false)}
        onchanged={refreshHighlights}
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

    {#if quickAiPopup}
      <QuickAiPopup text={quickAiPopup.text} anchor={quickAiPopup.anchor} onclose={() => (quickAiPopup = null)} />
    {/if}

    {#if hlMenu}
      <HighlightMenu
        highlight={hlMenu.h}
        anchor={hlMenu.anchor}
        oncolor={(c) => hlSetColor(hlMenu!.h, c)}
        onstyle={(s) => hlSetStyle(hlMenu!.h, s)}
        ondelete={() => hlDelete(hlMenu!.h)}
        onclose={closeHlMenu}
      />
    {/if}

    {#if ui.aiOpen}
      <AiPanel chapterTitle={chapter?.title ?? ""} chapterHtml={chapter?.html ?? ""} />
    {/if}

    {#if statsOpen && book}
      <BookStatsModal bookId={book.book.id} bookTitle={book.book.title} onclose={() => (statsOpen = false)} />
    {/if}

    {#if book && !openError}
      <div class="font-ctl" class:show={fontCtlShow}>
        <button class="icon" title="减小字号" onclick={() => bumpFont(-1)}>A−</button>
        <button class="icon" title="增大字号" onclick={() => bumpFont(1)}>A＋</button>
      </div>
    {/if}
  </div>
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

  .topbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 12px;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 90%, black 10%);
    border-bottom: 1px solid rgba(128, 128, 128, 0.15);
    z-index: 30;
    position: relative;
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
  }

  .tb-r {
    gap: 6px;
    flex-shrink: 0;
    margin-left: auto;
  }

  .stat-txt {
    font-size: 12.5px;
    opacity: 0.85;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .spacer { flex: 1; }

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
  }

  .content :global(::selection) {
    background: var(--rdr-sel, #e8e0cd);
  }

  .font-ctl {
    position: absolute;
    right: 14px;
    bottom: 46px;
    display: flex;
    gap: 4px;
    z-index: 25;
    opacity: 0;
    pointer-events: none;
    transform: translateY(4px);
    transition: opacity 0.18s, transform 0.18s;
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
    transition: transform 200ms ease;
  }

  .scroll-area {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }

  .col-scroll {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    width: 100%;
    padding: var(--rdr-mv, 48px) var(--rdr-mh, 56px);
    box-sizing: border-box;
    /* 仅滚动模式存在此元素；提升为合成层，滚动时不做布局 */
    will-change: transform;
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
  }

  #chapter-content :global(img) {
    max-width: 100%;
    height: auto;
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
    font-size: 11px;
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

  .scrollbar {
    position: absolute;
    right: 5px;
    top: 14px;
    bottom: 14px;
    width: 5px;
    z-index: 20;
  }

  .thumb {
    position: absolute;
    top: 0;
    width: 100%;
    background: rgba(128, 128, 128, 0.35);
    border-radius: 3px;
    will-change: transform;
  }

  .loading {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 12px;
    opacity: 0.6;
    z-index: 20;
    background: rgba(128, 128, 128, 0.12);
    padding: 3px 12px;
    border-radius: 20px;
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

  .progress-fill {
    height: 4px;
    background: var(--accent, #4a6b8a);
    border-radius: 2px;
    position: relative;
    z-index: 1;
    pointer-events: none;
    transition: width 0.2s ease;
  }

  .progress-track.dragging {
    cursor: grab;
  }

  .progress-track.dragging .progress-fill {
    transition: none;
    height: 6px;
  }

  .progress-fill::after {
    content: "";
    position: absolute;
    right: -6px;
    top: 50%;
    transform: translateY(-50%);
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent, #4a6b8a);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
    opacity: 0;
    transition: opacity 0.15s;
  }

  .progress-track:hover .progress-fill::after,
  .progress-track.dragging .progress-fill::after {
    opacity: 1;
  }

  .chapinfo, .pctinfo { white-space: nowrap; }

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
    box-shadow: -8px 0 24px rgba(0, 0, 0, 0.1);
    animation: slide-in 0.18s ease;
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
