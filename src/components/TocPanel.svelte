<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
import { IconArrowsSplit2, IconX } from "@tabler/icons-svelte";
  import type { TocEntry } from "../lib/types";
  import { dragOutHandle } from "../lib/windowSnap";
  import { displayChapterTitle } from "../lib/chapterTitle";

  let { toc, current, note, onpick, onclose, onpopout }: {
    toc: TocEntry[];
    current: number;
    /** 可选提示（如 PDF 无内置目录时说明按页导航） */
    note?: string;
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
  // 自动切章（无锚点块级切分）的章标题是空字符串，统一用「第 N 节」兜底，绝不显示空白项。
  function displayTitle(pos: number, title: string): string {
    return displayChapterTitle(title, pos, (n) => t("common.sectionN", { n }));
  }
  const filtered = $derived(
    query
      ? indexed.filter(({ t, pos }) => displayTitle(pos, t.title).toLowerCase().includes(query.toLowerCase()))
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
  <div class="sp-search">
    <input data-testid="toc-search" placeholder={t("reader.toc.searchPlaceholder")} bind:value={query} />
  </div>
  {#if note}<div class="sp-empty" style="padding:6px 10px;font-size:12px" data-testid="toc-note">{note}</div>{/if}
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
            style="top:{i * ROW_H}px; height:{ROW_H}px; padding-left:{10 + (item.t.depth ?? 0) * 14}px;"
            onclick={() => onpick(item.pos)}
          >
            {displayTitle(item.pos, item.t.title)}
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

  .sp-search input {
    width: 100%;
  }

  /* 目录行是虚拟滚动的绝对定位列表，容器不能有横向 padding，故这里覆盖全局基类 */
  .sp-list {
    padding: 0 6px 10px;
  }

  .sp-inner {
    position: relative;
    width: 100%;
  }

  /* 目录行此前完全没有 transition：hover 底色与 .current 高亮都是瞬变的，
     切章时高亮会「啪」地跳到新行上，看不出是从哪一行移过去的。 */
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
    transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease),
      transform var(--dur-fast) var(--ease-premium);
  }

  .toc-item:hover {
    background: color-mix(in srgb, var(--rdr-fg) 8%, transparent);
  }

  /* 按下反馈：目录是高频点击区（连续跳章），没有按下反馈时点错了行也感知不到。 */
  .toc-item:active {
    background: color-mix(in srgb, var(--rdr-fg) 13%, transparent);
    transform: scale(0.99);
  }

  .toc-item.current {
    color: var(--accent);
    font-weight: 600;
    background: color-mix(in srgb, var(--rdr-fg) 7%, transparent);
    /* 当前章整体向右让 2px，配合左侧 accent 竖条：
       「你在哪」比「哪一行是当前章」更需要一眼可辨的位置感。
       用 transform 而非 margin —— 前者是合成属性，虚拟滚动重排时不触发布局。 */
    transform: translateX(2px);
  }

  /* 当前章的左侧竖条：scaleY 从 0 长出来（只用 transform/opacity），
     切章时新行的竖条会「长」出来，上一行的收回 —— 高亮转移因此有了方向。 */
  .toc-item::before {
    content: "";
    position: absolute;
    left: 3px;
    top: 8px;
    bottom: 8px;
    width: 2px;
    border-radius: var(--radius-pill);
    background: var(--accent);
    transform: scaleY(0);
    transition: transform var(--dur) var(--ease-spring);
  }

  .toc-item.current::before {
    transform: scaleY(1);
  }
</style>
