<script lang="ts">
  /** D6-2：PDF 页面缩略图（固定网格虚拟滚动，只建可见行 DOM；图片进入视口才渲染）。 */
  import { t } from "../lib/i18n/index.svelte";
  import { IconArrowsSplit2, IconX } from "@tabler/icons-svelte";
  import { dragOutHandle } from "../lib/windowSnap";
  import { renderPdfThumb } from "../lib/pdf";
  import { rowOfPage, visibleThumbRows } from "../lib/pdfThumb";

  let {
    bookId,
    current,
    total,
    onjump,
    onclose,
    onpopout,
  }: {
    bookId: number;
    current: number;
    total: number;
    onjump: (page: number) => void;
    onclose?: () => void;
    onpopout?: () => void;
  } = $props();

  const COLS = 2;
  const ROW_H = 150;
  const GAP = 8;

  let listEl = $state<HTMLDivElement | undefined>();
  let scrollTop = $state(0);
  let viewH = $state(600);

  const range = $derived(visibleThumbRows(scrollTop, viewH, ROW_H, COLS, total));
  const rows = $derived(
    Array.from({ length: Math.max(0, range.endRow - range.startRow) }, (_, i) => range.startRow + i),
  );

  function onScroll(e: Event) {
    scrollTop = (e.currentTarget as HTMLDivElement).scrollTop;
  }

  $effect(() => {
    if (!listEl) return;
    viewH = listEl.clientHeight || 600;
  });

  $effect(() => {
    if (!listEl) return;
    const row = rowOfPage(current, COLS);
    const target = Math.max(0, row * ROW_H - viewH / 2 + ROW_H / 2);
    if (Math.abs(listEl.scrollTop - target) > ROW_H) listEl.scrollTop = target;
    scrollTop = listEl.scrollTop;
  });

  /** 可见才渲染（虚拟化后仍保留懒加载，避免滚动时并发解码）。 */
  function lazyThumb(node: HTMLElement, page: number) {
    let img: HTMLImageElement | null = null;
    let dead = false;
    const io = new IntersectionObserver((entries) => {
      if (dead || !entries.some((e) => e.isIntersecting)) return;
      io.disconnect();
      void renderPdfThumb(bookId, page)
        .then((url) => {
          if (dead) return;
          img = document.createElement("img");
          img.src = url;
          img.alt = "";
          img.loading = "lazy";
          img.onerror = () => node.classList.add("th-failed");
          node.replaceChildren(img);
        })
        .catch(() => node.classList.add("th-failed"));
    }, { rootMargin: "120px" });
    io.observe(node);
    return {
      destroy() {
        dead = true;
        io.disconnect();
        if (img) img.removeAttribute("src");
      },
    };
  }
</script>

<div class="sidepanel">
  <div class="sp-head" use:dragOutHandle={onpopout} title={t("common.panel.popoutTip")}>
    <span>{t("reader.pdf.thumbsTitle")}</span>
    <div style="display:flex; gap:6px; align-items:center;">
      {#if onpopout}<button class="icon" title={t("common.panel.popout")} onclick={onpopout}><IconArrowsSplit2 /></button>{/if}
      {#if onclose}<button class="icon" title={t("common.action.close")} onclick={onclose}><IconX /></button>{/if}
    </div>
  </div>
  <div class="sp-list th-scroll" bind:this={listEl} onscroll={onScroll}>
    <div class="th-canvas" style="height:{range.rows * ROW_H}px">
      {#each rows as row (row)}
        {#each Array(COLS) as _, col (col)}
          {@const page = row * COLS + col}
          {#if page < total}
            <button
              class="th-cell"
              class:active={page === current}
              use:lazyThumb={page}
              style="top:{row * ROW_H}px; left:calc({col} * (50% + {GAP / 2}px)); width:calc(50% - {GAP / 2}px); height:{ROW_H - GAP}px;"
              title={t("reader.pdf.page", { n: page + 1 })}
              onclick={() => onjump(page)}
            >
              <span class="th-num">{page + 1}</span>
            </button>
          {/if}
        {/each}
      {/each}
    </div>
  </div>
</div>

<style>
  .th-scroll { position: relative; overflow-y: auto; padding: 8px; }
  .th-canvas { position: relative; width: 100%; }
  .th-cell {
    position: absolute;
    border: 1px solid var(--border, #8884);
    border-radius: 4px;
    background: var(--panel, transparent);
    overflow: hidden;
    cursor: pointer;
    padding: 0;
  }
  .th-cell.active { outline: 2px solid var(--accent); }
  .th-cell img { width: 100%; height: 100%; object-fit: contain; background: #fff; }
  .th-num { position: absolute; left: 4px; bottom: 2px; font-size: 11px; opacity: 0.85; background: #0008; color: #fff; padding: 0 3px; border-radius: 3px; }
  .th-cell.th-failed::after { content: "×"; position: absolute; inset: 0; display: grid; place-items: center; opacity: 0.6; }
</style>
