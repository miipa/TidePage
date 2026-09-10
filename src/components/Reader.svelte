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
    type BindAction,
    type ZoneKey,
  } from "../lib/stores.svelte";
  import { SmoothScroller } from "../lib/smoothScroll";
  import AiPanel from "./AiPanel.svelte";
  import TocPanel from "./TocPanel.svelte";
  import BookStatsModal from "./BookStatsModal.svelte";
  import type { ChapterData, OpenResult, Progress } from "../lib/types";

  let { standalone = false }: { standalone?: boolean } = $props();

  let book: OpenResult | null = $state(null);
  let chapter: ChapterData | null = $state(null);
  let loadingChapter = $state(false);
  let chapterNum = $state(0);

  let scrollWrap: HTMLDivElement | undefined = $state();
  let pageWrap: HTMLDivElement | undefined = $state();
  let contentEl: HTMLDivElement | undefined = $state();

  let scrollTop = $state(0);
  let maxScroll = $state(0);

  let page = $state(0);
  let pageCount = $state(1);
  let colW = 0;
  let gap = 48;
  let stepPx = 0;

  let scroller: SmoothScroller | null = null;

  let tocOpen = $state(false);
  let bookmarksOpen = $state(false);
  let statsOpen = $state(false);
  let bookmarks: { id: number; chapter_index: number; title: string; excerpt: string }[] = $state([]);

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
      let restore: Progress | null = null;
      try {
        restore = await api.loadProgress(bookId);
      } catch { /* ignore */ }
      chapterNum = restore && restore.chapter_index < book.toc.length ? restore.chapter_index : 0;
      await loadChapter(chapterNum, restore ?? undefined);
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
        if ((restore && restore.chapter_ratio > 0) || entry?.at === "tail") {
          setTimeout(() => {
            measureScroll();
            let p2 = 0;
            if (entry) p2 = entry.at === "tail" ? maxScroll - (entry.extra || 0) : entry.extra || 0;
            else if (restore) p2 = restore.chapter_ratio * maxScroll;
            scroller?.jumpTo(Math.max(0, Math.min(p2, maxScroll)), true);
          }, 300);
        }
      } else {
        measurePages();
        let pg = restore ? Math.round(restore.chapter_ratio * Math.max(1, pageCount - 1)) : 0;
        if (entry) pg = entry.at === "tail" ? Math.max(0, pageCount - 1) : 0;
        gotoPage(Math.max(0, Math.min(pg, Math.max(0, pageCount - 1))), true);
        setTimeout(recheckPages, 350);
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

  function prevChapter() {
    if (book && chapterNum > 0) loadChapter(chapterNum - 1);
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

  let scrollBase = 0;

  function applyScroll(renderPos: number) {
    scrollTop = renderPos;
    if (!scrollWrap) return;
    const p = Math.round(renderPos);
    const d = scrollBase - p;
    if (d === 0) {
      scrollWrap.style.transform = "";
    } else {
      scrollWrap.style.transform = `translateY(${d}px)`;
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

  function measurePages() {
    if (!contentEl || !pageWrap || !chapter) return;
    const mh = config.reader.marginH;
    const mv = config.reader.marginV;
    const W = contentEl.clientWidth;
    const H = contentEl.clientHeight;
    const avail = Math.max(260, W - mh * 2);
    const vh = Math.max(240, H - mv * 2);
    gap = 48;
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
    st.transform = "";
    cancelAnimationFrame(measureRaf);
    measureRaf = requestAnimationFrame(() => {
      computePageCount();
      applyPage();
    });
  }

  function applyPage() {
    if (!pageWrap) return;
    pageWrap.style.left = `${-Math.round(page * stepPx)}px`;
    pageWrap.style.transform = "";
  }

  function gotoPage(n: number, silent = false) {
    page = Math.max(0, Math.min(n, pageCount - 1));
    applyPage();
    if (!silent) {
      if (config.reader.pageTurnKeys) saveProgressDebounced();
      else saveProgressDebounced();
    }
  }

  function nextPage() {
    if (page < pageCount - 1) gotoPage(page + 1);
    else nextChapter(true);
  }

  function prevPage() {
    if (page > 0) gotoPage(page - 1);
    else if (chapterNum > 0) prevChapter();
  }

  function relayout() {
    if (!chapter) return;
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
        if (config.reader.mode === "scroll") handleScrollEdge(-1, 0);
        else prevChapter();
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
        if (tocOpen || bookmarksOpen || statsOpen) {
          tocOpen = false;
          bookmarksOpen = false;
          statsOpen = false;
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
      scroller.bounce = config.reader.scrollBounce;
      scroller.strength = config.reader.scrollBounceStrength;
      scroller.threshold = config.reader.scrollBounceThreshold;
    }
  });

  $effect(() => {
    const w = pageWrap;
    const sw = scrollWrap;
    const c = contentEl;
    if (!w && !sw && !c) return;
    const ro = new ResizeObserver(() => scheduleRelayout());
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

  function fontStack(raw: string): string {
    const parts = (raw || "")
      .split(",")
      .map((s) => s.trim().replace(/^[\"']|[\"']$/g, ""))
      .filter(Boolean);
    if (!parts.length) return '"Microsoft YaHei", "Segoe UI", sans-serif';
    return parts.map((s) => (/^[A-Za-z][A-Za-z0-9-]*$/.test(s) ? s : `"${s}"`)).join(", ");
  }

  function readerVars(): string {
    const t = THEMES[config.reader.theme];
    const bg = config.reader.bgColor || t.bg;
    const fg = config.reader.fgColor || t.fg;
    return [
      `--rdr-bg:${bg}`,
      `--rdr-fg:${fg}`,
      `--rdr-font:${fontStack(config.reader.fontFamily)}`,
      `--rdr-fs:${config.reader.fontSize}px`,
      `--rdr-lh:${config.reader.lineHeight}`,
      `--rdr-ps:${config.reader.paraSpacing}em`,
      `--rdr-mh:${config.reader.marginH}px`,
      `--rdr-mv:${config.reader.marginV}px`,
      `--rdr-indent:${config.reader.indent}`,
      `--rdr-ls:${config.reader.letterSpacing}em`,
      `--rdr-fsw:${config.reader.fauxBold ? "auto" : "none"}`,
      `--rdr-fsi:${config.reader.fauxItalic ? "auto" : "none"}`,
      `--rdr-sel:${t.sel}`,
      `--rdr-top:${config.reader.topbarColor || "color-mix(in srgb, " + bg + " 90%, black 10%)"}`,
      `--rdr-stat:${config.reader.statusColor || t.fg}`,
      `--rdr-stat-align:${config.reader.statusPos === "none" ? "center" : config.reader.statusPos}`,
    ].join(";");
  }

  function buildDynamicCss(): string {
    const r = config.reader;
    const imp = r.overrideBookCss ? " !important" : "";
    const fontFaces = (r.customFonts || [])
      .map(
        (f) =>
          `@font-face { font-family: "${f.family}"; src: url('http://flowread.localhost/fonts/${f.file}'); }`
      )
      .join("\n");
    const overrideRule = r.overrideBookCss
      ? `
#chapter-content { color: var(--rdr-fg) !important; font-family: var(--rdr-font) !important; }
#chapter-content p { font-size: var(--rdr-fs) !important; text-indent: var(--rdr-indent) !important; margin: var(--rdr-ps) 0 !important; line-height: var(--rdr-lh) !important; }
#chapter-content :is(h1,h2,h3,h4,h5,h6) { font-size: 1.2em !important; font-weight: 600 !important; text-align: center !important; margin: 0.4em 0 0.8em !important; }
#chapter-content :is(img,svg,video) { height: auto !important; max-width: 100% !important; }
#chapter-content sup { font-size: 0.7em !important; }
`
      : "";
    return `
${fontFaces}
${overrideRule}
${r.cssExtra || ""}
`;
  }

  $effect(() => {
    const r = config.reader;
    void r.overrideBookCss;
    void r.cssExtra;
    void r.customFonts;
    void r.fontFamily;
    void r.fontSize;
    void r.lineHeight;
    void r.indent;
    void r.paraSpacing;
    void r.marginH;
    void r.marginV;
    void r.theme;
    void r.bgColor;
    void r.fgColor;
    void r.fauxBold;
    void r.fauxItalic;
    void r.letterSpacing;
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
    r.fontSize;
    r.lineHeight;
    r.paraSpacing;
    r.marginH;
    r.marginV;
    r.indent;
    r.fontFamily;
    r.cssExtra;
    r.overrideBookCss;
    r.customFonts;
    r.fauxBold;
    r.fauxItalic;
    r.letterSpacing;
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

<div class="reader" style={readerVars()} onwheel={onWheel}>
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
      <button class="icon" title="AI 助手" onclick={toggleAi}>AI</button>
      <button class="icon" title="本书统计" onclick={() => (statsOpen = !statsOpen)}>📊</button>
      <button class="icon" title="目录 (F3)" onclick={() => (tocOpen = !tocOpen)}>☰</button>
      <button class="icon" title="设置" onclick={() => (ui.settingsOpen = true)}>⚙</button>
    </div>
  </div>

  <div class="body">
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
              <div class="thumb" style="height:8%;top:{maxScroll > 0 ? (scrollTop / maxScroll) * 92 : 0}%"></div>
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
            <div class="sp-item" onclick={() => { loadChapter(bm.chapter_index); bookmarksOpen = false; }}>
              <div class="bm-title">{bm.title || "未命名"}</div>
              {#if bm.excerpt}<div class="bm-excerpt">{bm.excerpt}</div>{/if}
            </div>
          {:else}
            <div class="sp-empty">暂无书签，点 🔖 添加</div>
          {/each}
        </div>
      </div>
    {/if}

    {#if ui.aiOpen}
      <AiPanel chapterTitle={chapter?.title ?? ""} chapterHtml={chapter?.html ?? ""} />
    {/if}

    {#if statsOpen && book}
      <BookStatsModal bookId={book.book.id} bookTitle={book.book.title} onclose={() => (statsOpen = false)} />
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
  }

  #chapter-content {
    font-family: var(--rdr-font);
    font-size: var(--rdr-fs);
    line-height: var(--rdr-lh);
    color: var(--rdr-fg);
    letter-spacing: var(--rdr-ls, 0em);
    font-synthesis-weight: var(--rdr-fsw, auto);
    font-synthesis-style: var(--rdr-fsi, auto);
    overflow-wrap: break-word;
    text-align: justify;
    text-justify: inter-ideograph;
  }

  #chapter-content :global(p) {
    text-indent: var(--rdr-indent, 2em);
    margin: var(--rdr-ps, 0.7em) 0;
  }

  #chapter-content :global(h1),
  #chapter-content :global(h2),
  #chapter-content :global(h3),
  #chapter-content :global(h4) {
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
    width: 100%;
    background: rgba(128, 128, 128, 0.35);
    border-radius: 3px;
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

  .bm-title { font-size: 13px; font-weight: 500; }
  .bm-excerpt { font-size: 12px; opacity: 0.6; margin-top: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-empty { padding: 24px; text-align: center; opacity: 0.5; }
</style>
