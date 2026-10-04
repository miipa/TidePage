<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
import { IconArrowsSplit2, IconX } from "@tabler/icons-svelte";
  import type { TocEntry } from "../lib/types";
  import { dragOutHandle } from "../lib/windowSnap";

  let { toc, current, onpick, onclose, onpopout }: {
    toc: TocEntry[];
    current: number;
    onpick: (index: number) => void;
    onclose?: () => void;
    onpopout?: () => void;
  } = $props();

  let query = $state("");
  let listEl: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(600);

  // 固定行高 + 虚拟滚动：超大目录（如 7000+ 章）只渲染可视区，避免一次性建数万个 DOM 节点卡死。
  const ROW_H = 34;
  const OVERSCAN = 6;

  // 保留每一项在 toc 数组中的原始下标（pos）。章节导航（loadChapter / get_chapter）一律以
  // 「toc 数组下标」为准；TocEntry.index 是 spine 下标（EPUB 中常与数组下标不一致），
  // 不能拿它当章节号，否则会跳到错章、甚至越界渲染出空白章。
  const indexed = $derived(toc.map((t, pos) => ({ t, pos })));
  const filtered = $derived(
    query
      ? indexed.filter(({ t }) => t.title.toLowerCase().includes(query.toLowerCase()))
      : indexed
  );

  const total = $derived(filtered.length);
  const firstIdx = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const lastIdx = $derived(Math.min(total, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN));
  const visible = $derived(filtered.slice(firstIdx, lastIdx));

  let scrollRaf = 0;
  function onScroll(e: Event) {
    const el = e.currentTarget as HTMLDivElement;
    const top = el.scrollTop;
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      scrollTop = top;
    });
  }

  $effect(() => {
    if (!listEl) return;
    viewH = listEl.clientHeight || 600;
  });

  // 打开目录时滚动定位到当前章（用虚拟滚动计算目标位置并夹取）。
  $effect(() => {
    if (!listEl) return;
    const i = filtered.findIndex((x) => x.pos === current);
    if (i >= 0) {
      listEl.scrollTop = Math.max(0, i * ROW_H - viewH / 2 + ROW_H / 2);
      scrollTop = listEl.scrollTop;
    }
    // 仅在切换当前章/首次挂载时定位；输入搜索时也重新定位
  });
</script>

<div class="sidepanel sidepanel--left" style="--sidepanel-w: 320px;">
  <div class="sp-head" data-testid="toc-head" use:dragOutHandle={onpopout} title={t("common.panel.popoutTip")}>
    <span>{t("toc.title")}</span>
    <div class="sp-head-acts">
      {#if onpopout}<button class="icon" data-testid="toc-popout" title={t("common.panel.popout")} onclick={onpopout}><IconArrowsSplit2 /></button>{/if}
      {#if onclose}<button class="icon" data-testid="toc-close" title={t("common.action.close")} onclick={onclose}><IconX /></button>{/if}
    </div>
  </div>
  <!-- 搜索框 + 清除键：包裹层只提供定位参照与右侧留白（padding-right 契约与书库
       .search-wrap 同一个，见 app.css 的 .fr-clear 小节），× 的规格全走公共类 .fr-clear。
       data-testid 必须留在 input 自身。onclick 只把绑定值置空 —— 虚拟滚动的定位/过滤照常经它生效。 -->
  <div class="sp-search">
    <span class="toc-search-wrap">
      <input data-testid="toc-search" placeholder={t("reader.toc.searchPlaceholder")} bind:value={query} />
      {#if query}
        <button
          class="fr-clear"
          data-testid="toc-search-clear"
          title={t("common.action.clear")}
          aria-label={t("library.toolbar.clearSearchAria")}
          onclick={() => (query = "")}><IconX size={14} stroke={1.5} /></button>
      {/if}
    </span>
  </div>
  <div class="sp-list" bind:this={listEl} onscroll={onScroll}>
    {#if total === 0}
      <div class="sp-empty" data-testid="toc-empty">{t("toc.empty")}</div>
    {:else}
      <div class="sp-inner" style="height:{total * ROW_H}px;">
        {#each visible as item, vi (item.pos)}
          {@const i = firstIdx + vi}
          <button
            class="toc-item"
            class:current={item.pos === current}
            style="top:{i * ROW_H}px; height:{ROW_H}px;"
            onclick={() => onpick(item.pos)}
          >
            {item.t.title}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  /* .sidepanel / .sp-head / .sp-list / .sp-empty 走 app.css 的全局基类；
     目录面板是唯一的左侧抽屉，故模板上加 .sidepanel--left 并把宽度收窄到 320px。 */
  .sp-head-acts { display: flex; align-items: center; gap: var(--sp-1); }
  .sp-head-acts .icon { font-weight: 400; }

  .sp-search {
    padding: 0 var(--sp-3) 10px;
  }

  /* 清除键的包裹层：position 给绝对定位的 × 提供参照（.fr-clear 容器契约的第一条）。
     display:block + width:100% —— 原来 input 直接挂在 .sp-search 下，现在多了一层包裹，
     宽度必须由包裹层接过来，否则里面 input 的 width:100% 失去参照。 */
  .toc-search-wrap {
    display: block;
    width: 100%;
    position: relative;
  }

  .sp-search input {
    width: 100%;
    /* = 键宽 20 + 键的 inset 4 + 余量 2，与书库 .search-wrap .search 是同一个契约 */
    padding-right: 26px;
  }

  /* 目录行是虚拟滚动的绝对定位列表，容器不能有横向 padding，故这里覆盖全局基类 */
  .sp-list {
    padding: 0 6px 10px;
  }

  .sp-inner {
    position: relative;
    width: 100%;
  }

  .toc-item {
    position: absolute;
    left: 0;
    right: 0;
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    padding: 0 var(--sp-3);
    cursor: pointer;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    line-height: 34px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    box-sizing: border-box;
  }

  .toc-item:hover {
    background: color-mix(in srgb, var(--rdr-fg) 8%, transparent);
  }

  /* 当前章节：中性色表达（深字 + 浅底 + 600），不用 accent。
     accent 是稀缺资源，而「当前章节」与下方那一大片章节列表里的其它项是同级关系，
     用主色会让整行读作一枚可点的标签 chip。字色取 --rdr-fg（阅读主题字色）而不是
     外壳 --fg：本面板整体跟随阅读主题，换 UI 皮肤不该改动它的观感。 */
  .toc-item.current {
    color: var(--rdr-fg);
    font-weight: 600;
    background: color-mix(in srgb, var(--rdr-fg) 7%, transparent);
  }
</style>
