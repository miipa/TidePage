<script lang="ts">
  /** D6-3：整本 PDF 文本搜索（逐页 getTextContent，可取消，分批让出）。 */
  import { t } from "../lib/i18n/index.svelte";
  import { IconArrowsSplit2, IconChevronDown, IconChevronUp, IconSearch, IconX } from "@tabler/icons-svelte";
  import { dragOutHandle } from "../lib/windowSnap";
  import { onDestroy } from "svelte";
  import { searchPdfText, type PdfSearchResult } from "../lib/pdf";
  import { RequestSeq } from "../lib/requestSeq";
  import { searchProgress, stepHit, type PdfSearchHit } from "../lib/pdfSearch";

  let {
    bookId,
    onjump,
    onclose,
    onpopout,
  }: {
    bookId: number;
    onjump: (page: number, hit: PdfSearchHit, query: string) => void;
    onclose?: () => void;
    onpopout?: () => void;
  } = $props();

  let query = $state("");
  let busy = $state(false);
  let progress = $state(0);
  let result = $state<PdfSearchResult | null>(null);
  let err = $state("");
  let currentHit = $state(0);
  let showFailed = $state(false);
  let ctrl: AbortController | null = null;
  // F6：关闭/卸载后继续扫描没有意义；序号保证只有最新一次搜索能写结果。
  let disposed = false;
  const seq = new RequestSeq();

  async function run() {
    const q = query.trim();
    if (!q) return;
    const token = seq.next();
    ctrl?.abort();
    ctrl = new AbortController();
    busy = true;
    err = "";
    result = null;
    currentHit = 0;
    progress = 0;
    try {
      const r = await searchPdfText(bookId, q, {
        signal: ctrl.signal,
        onProgress: (done, total) => {
          if (!disposed && seq.isCurrent(token)) progress = searchProgress(done, total);
        },
      });
      if (disposed || !seq.isCurrent(token)) return;
      result = r;
      const g0 = r.groups[0];
      if (g0) {
        const seg = g0.segments[0];
        onjump(seg.page, { page: seg.page, start: seg.start, end: seg.end, occurrence: seg.occurrence, snippet: g0.snippet, segments: g0.segments }, q);
      }
    } catch (e) {
      if (disposed || !seq.isCurrent(token)) return;
      if (!(e instanceof DOMException && e.name === "AbortError")) err = String(e);
    } finally {
      if (!disposed && seq.isCurrent(token)) busy = false;
    }
  }

  onDestroy(() => {
    disposed = true;
    ctrl?.abort();
    seq.next(); // 让在途请求的 token 立即失效
  });

  function go(dir: 1 | -1) {
    const groups = result?.groups ?? [];
    if (!groups.length) return;
    currentHit = stepHit(currentHit, groups.length, dir);
    const g = groups[currentHit];
    const seg = g.segments[0];
    onjump(seg.page, { page: seg.page, start: seg.start, end: seg.end, occurrence: seg.occurrence, snippet: g.snippet, segments: g.segments }, query.trim());
  }

  function cancel() {
    ctrl?.abort();
    busy = false;
  }
</script>

<div class="sidepanel">
  <div class="sp-head" use:dragOutHandle={onpopout} title={t("common.panel.popoutTip")}>
    <span>{t("reader.pdf.searchTitle")}</span>
    <div style="display:flex; gap:6px; align-items:center;">
      {#if onpopout}<button class="icon" title={t("common.panel.popout")} onclick={onpopout}><IconArrowsSplit2 /></button>{/if}
      {#if onclose}<button class="icon" title={t("common.action.close")} onclick={onclose}><IconX /></button>{/if}
    </div>
  </div>
  <div class="sp-search" style="display:flex; gap:6px;">
    <input
      placeholder={t("reader.pdf.searchPlaceholder")}
      bind:value={query}
      onkeydown={(e) => { if (e.key === "Enter") void run(); }}
    />
    <button class="icon" title={t("common.action.search")} onclick={() => void run()} disabled={busy}><IconSearch /></button>
  </div>
  <div class="sp-list">
    {#if busy}
      <div class="sp-empty">{t("reader.pdf.searching", { n: progress })} <button class="icon" onclick={cancel}>{t("common.confirm.cancel")}</button></div>
    {:else if err}
      <div class="sp-empty">{t("reader.pdf.searchFailed")}</div>
    {:else if result}
      {#if !result.usable}
        <div class="sp-empty">{t("reader.pdf.searchNoText")}</div>
      {:else if !result.groups.length}
        <div class="sp-empty">{t("reader.pdf.searchEmpty")}</div>
      {:else}
        <div class="sp-search" style="display:flex; justify-content:space-between; align-items:center; padding:4px 8px;">
          <span class="tip">{t("reader.pdf.searchCount", { n: result.groups.length })}</span>
          <span style="display:flex; gap:4px;">
            <button class="icon" title={t("reader.pdf.searchPrev")} onclick={() => go(-1)}><IconChevronUp /></button>
            <button class="icon" title={t("reader.pdf.searchNext")} onclick={() => go(1)}><IconChevronDown /></button>
          </span>
        </div>
        {#each result.groups as g, i (g.startPage + ":" + g.segments[0].start + ":" + g.snippet)}
          <button class="sp-item" class:active={i === currentHit} onclick={() => { currentHit = i; const seg = g.segments[0]; onjump(seg.page, { page: seg.page, start: seg.start, end: seg.end, occurrence: seg.occurrence, snippet: g.snippet, segments: g.segments }, query.trim()); }}>
            <div class="bm-title">{g.startPage === g.endPage ? t("reader.pdf.page", { n: g.startPage + 1 }) : t("reader.pdf.pageRange", { a: g.startPage + 1, b: g.endPage + 1 })}</div>
            <div class="bm-excerpt">{g.snippet}</div>
          </button>
        {/each}
      {/if}
      {#if result.failedPages.length || result.emptyPages.length}
        <button class="sp-item" onclick={() => (showFailed = !showFailed)}>
          {t("reader.pdf.failedPages", { n: result.failedPages.length })}{#if result.emptyPages.length} · {t("reader.pdf.emptyPages", { n: result.emptyPages.length })}{/if}
        </button>
        {#if showFailed}
          {#if result.failedPages.length}<div class="tip" style="padding:4px 10px">{t("reader.pdf.failedPages", { n: result.failedPages.length })}</div>{/if}
          {#each result.failedPages as p (p)}<div class="tip" style="padding:1px 14px">{t("reader.pdf.page", { n: p + 1 })}</div>{/each}
          {#if result.emptyPages.length}<div class="tip" style="padding:4px 10px">{t("reader.pdf.emptyPages", { n: result.emptyPages.length })}</div>{/if}
          {#each result.emptyPages as p (p)}<div class="tip" style="padding:1px 14px">{t("reader.pdf.page", { n: p + 1 })}</div>{/each}
        {/if}
      {/if}
    {:else}
      <div class="sp-empty">{t("reader.pdf.searchHint")}</div>
    {/if}
  </div>
</div>
