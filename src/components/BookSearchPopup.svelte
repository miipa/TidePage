<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { toast } from "../lib/stores.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import { outFade } from "../lib/motion";
  import type { BookSearchHit } from "../lib/types";
  import { IconArrowLeft, IconSearch, IconX } from "@tabler/icons-svelte";

  let {
    bookId,
    initialQuery = "",
    anchor,
    canBack = false,
    surfaceVars = "",
    onpick,
    onback,
    onclose,
  }: {
    bookId: number;
    initialQuery?: string;
    anchor: PopupAnchor;
    canBack?: boolean;
    /** 阅读区表面令牌（--rdr-bg/-fg 及其派生量），由 Reader 的 readerSurfaceVars() 提供。
     *  本浮层 portal 到 body，脱离 .reader 子树，拿不到 Reader 写在根节点上的内联值，
     *  故必须自带一份——否则书内搜索会渲染成外壳皮肤的颜色（深色外壳 + 纸色书页时
     *  会出现深色小窗压在浅色书页上）。由调用方提供而非自行推导，是为了带上
     *  「仅本书方案」覆盖，且与它所浮着的书页严格同源。 */
    surfaceVars?: string;
    onpick: (chapterIndex: number, query: string) => void;
    onback: () => void;
    onclose: () => void;
  } = $props();

  let query = $state(initialQuery);
  let hits: BookSearchHit[] = $state([]);
  let searching = $state(false);
  let searched = $state(false);
  let indexing = $state(false);
  let popEl: HTMLDivElement | undefined = $state();

  async function run() {
    const q = query.trim();
    if (q.length < 2) return;
    searching = true;
    try {
      hits = await api.searchInBook(bookId, q);
      searched = true;
    } catch (e) {
      toast(String(e), "error");
    } finally {
      searching = false;
    }
  }

  async function buildIndex() {
    indexing = true;
    try {
      await api.ensureBookText(bookId);
      await run();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      indexing = false;
    }
  }

  let activeIdx = $state(-1);
  let autoPos = $state({ left: 0, top: 0 });
  let userPos = $state<{ left: number; top: number } | null>(null);

  // 用户拖动后固定位置；未拖动时按锚点自适应摆放
  const posStyle = $derived(
    userPos
      ? `left:${userPos.left}px; top:${userPos.top}px;`
      : `left:${autoPos.left}px; top:${autoPos.top}px;`,
  );

  // 内容高度随结果变化，需重新测量摆放（顶部/底部自动翻转并夹取到视口内）
  $effect(() => {
    void anchor.top;
    void anchor.bottom;
    void anchor.cx;
    void hits;
    void searching;
    void searched;
    if (!popEl || userPos) return;
    requestAnimationFrame(() => {
      if (!popEl || userPos) return;
      const p = placePopup(anchor, popEl.offsetWidth, popEl.offsetHeight);
      autoPos = p;
    });
  });

  onMount(() => {
    run();
  });

  function pick(i: number, h: BookSearchHit) {
    activeIdx = i;
    onpick(h.chapter_index, query.trim());
  }

  function startDrag(e: PointerEvent) {
    if ((e.target as HTMLElement)?.closest("button")) return;
    if (!popEl) return;
    const rect = popEl.getBoundingClientRect();
    const sx = e.clientX;
    const sy = e.clientY;
    const ox = rect.left;
    const oy = rect.top;
    const w = rect.width;
    const h = rect.height;
    const move = (ev: PointerEvent) => {
      // clamp 到视口（0..innerWidth/innerHeight），不再限制在 reader 内。
      const left = Math.max(0, Math.min(ox + ev.clientX - sx, window.innerWidth - w));
      const top = Math.max(0, Math.min(oy + ev.clientY - sy, window.innerHeight - h));
      userPos = { left, top };
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    e.preventDefault();
  }

  /** Portal：把弹框节点挂到 document.body，彻底摆脱任何祖先的
   *  transform/filter/contain/overflow 造成的 position:fixed 退化与裁剪。 */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      },
    };
  }
</script>

<div class="sb-pop rdr-pop" out:outFade={{ from: 0.96 }} use:portal bind:this={popEl} style="{surfaceVars};{posStyle}">
  <div class="sb-head" data-testid="bsp-head" onpointerdown={startDrag} title={t("search.popup.moveTip")}>
    <span class="sb-drag"><IconSearch />{t("search.popup.title")}</span>
    <div class="sb-head-acts">
      {#if canBack}
        <button class="mini ico" data-testid="bsp-back" onclick={onback} title={t("search.popup.backTip")}><IconArrowLeft />{t("search.popup.back")}</button>
      {/if}
      <button class="x" data-testid="bsp-close" onclick={onclose} title={t("common.action.close")}><IconX /></button>
    </div>
  </div>
  <div class="sb-row">
    <input
      data-testid="bsp-input"
      bind:value={query}
      placeholder={t("search.popup.placeholder")}
      onkeydown={(e) => e.key === "Enter" && run()}
    />
    <button class="primary" data-testid="bsp-go" onclick={run} disabled={searching}>{searching ? "…" : t("common.action.search")}</button>
  </div>
  <div class="sb-list">
    {#if searched && !hits.length}
      <div class="sb-empty" data-testid="bsp-empty">
        {t("search.popup.empty")}
        <div style="margin-top:8px">
          <button data-testid="bsp-reload" onclick={buildIndex} disabled={indexing}>{indexing ? t("search.popup.loading") : t("search.popup.reloadHint")}</button>
        </div>
      </div>
    {/if}
    {#each hits as h, i (i)}
      <div class="sb-hit" class:active={activeIdx === i} onclick={() => pick(i, h)}>
        <div class="sb-ch">{h.chapter_title || t("common.sectionN", { n: h.chapter_index + 1 })}</div>
        <div class="sb-sn">{@html h.snippet.replaceAll("<mark>", '<mark class="mk">')}</div>
      </div>
    {/each}
  </div>
</div>

<style>
  /* 底色 / 描边 / 圆角 / 投影走全局 .rdr-pop。
     本浮层 portal 到 body、拿不到 .reader 上的 --rdr-*，由 app.css :root 的
     阅读区变量兜底退回外壳面板色（与取色框同一策略）。 */
  .sb-pop {
    position: fixed;
    z-index: var(--z-popover);
    width: min(440px, 88vw);
    max-height: min(60vh, 520px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: tp-pop-in var(--dur) var(--ease-premium) both;
  }

  .sb-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    font-weight: 600;
    border-bottom: 1px solid var(--line);
    cursor: move;
    user-select: none;
    touch-action: none;
  }

  .sb-drag {
    cursor: move;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
  }


  .sb-head-acts {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
  }

  .sb-head .x {
    padding: 1px var(--sp-2);
    border-radius: var(--radius-sm);
  }

  .sb-head .mini {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    padding: 2px var(--sp-2);
    border-radius: var(--radius-sm);
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
  }

  .sb-row {
    display: flex;
    gap: var(--sp-1);
    padding: var(--sp-2) var(--sp-3);
    border-bottom: 1px solid var(--line-soft);
  }

  .sb-row input { flex: 1; min-width: 0; }

  .sb-list {
    overflow-y: auto;
    flex: 1;
    padding: var(--sp-1);
  }

  .sb-hit {
    padding: 7px 9px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .sb-hit {
    transition: background var(--dur-fast) var(--ease);
  }
  .sb-hit:hover { background: var(--hover-row); }
  .sb-hit.active { background: var(--accent-soft); }

  .sb-ch { font-size: var(--fs-sm); letter-spacing: var(--ls-sm); font-weight: 600; }

  .sb-sn {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.8;
    line-height: 1.55;
    margin-top: 2px;
    word-break: break-word;
  }

  /* 命中片段底色：以 --warning 为色相做浅底，跟随皮肤而非固定黄 */
  .sb-sn :global(mark.mk) {
    background: color-mix(in srgb, var(--warning) 32%, var(--bg-elev));
    color: inherit;
    padding: 0 2px;
    border-radius: var(--radius-xs);
  }

  .sb-empty {
    padding: 48px var(--sp-4);
    text-align: center;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    color: var(--fg2);
    line-height: 1.8;
  }
</style>
