<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { toast } from "../lib/stores.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import type { BookSearchHit } from "../lib/types";

  let {
    bookId,
    initialQuery = "",
    anchor,
    canBack = false,
    onpick,
    onback,
    onclose,
  }: {
    bookId: number;
    initialQuery?: string;
    anchor: PopupAnchor;
    canBack?: boolean;
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

<div class="sb-pop" use:portal bind:this={popEl} style={posStyle}>
  <div class="sb-head" onpointerdown={startDrag} title="按住拖动可移动此框">
    <span class="sb-drag">⠿ 全书搜索</span>
    <div class="sb-head-acts">
      {#if canBack}
        <button class="mini" onclick={onback} title="返回跳转前的位置">↩ 返回</button>
      {/if}
      <button class="x" onclick={onclose} title="关闭">×</button>
    </div>
  </div>
  <div class="sb-row">
    <input
      bind:value={query}
      placeholder="在本书中搜索…"
      onkeydown={(e) => e.key === "Enter" && run()}
    />
    <button class="primary" onclick={run} disabled={searching}>{searching ? "…" : "搜索"}</button>
  </div>
  <div class="sb-list">
    {#if searched && !hits.length}
      <div class="sb-empty">
        本书未找到匹配内容
        <div style="margin-top:8px">
          <button onclick={buildIndex} disabled={indexing}>{indexing ? "载入中…" : "重新载入全文后重试"}</button>
        </div>
      </div>
    {/if}
    {#each hits as h, i (i)}
      <div class="sb-hit" class:active={activeIdx === i} onclick={() => pick(i, h)}>
        <div class="sb-ch">{h.chapter_title || `第 ${h.chapter_index + 1} 节`}</div>
        <div class="sb-sn">{@html h.snippet.replaceAll("<mark>", '<mark class="mk">')}</div>
      </div>
    {/each}
  </div>
</div>

<style>
  .sb-pop {
    position: fixed;
    z-index: 980;
    width: min(440px, 88vw);
    max-height: min(60vh, 520px);
    display: flex;
    flex-direction: column;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 94%, #202020 6%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 12px;
    box-shadow: 0 12px 34px rgba(0, 0, 0, 0.3);
    overflow: hidden;
  }

  .sb-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    font-size: 13px;
    font-weight: 600;
    border-bottom: 1px solid rgba(128, 128, 128, 0.16);
    cursor: move;
    user-select: none;
    touch-action: none;
  }

  .sb-drag { cursor: move; }

  .sb-head-acts {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .sb-head .x {
    padding: 1px 8px;
    border-radius: 6px;
  }

  .sb-head .mini {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 6px;
  }

  .sb-row {
    display: flex;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.12);
  }

  .sb-row input { flex: 1; min-width: 0; }

  .sb-list {
    overflow-y: auto;
    flex: 1;
    padding: 4px;
  }

  .sb-hit {
    padding: 7px 9px;
    border-radius: 8px;
    cursor: pointer;
  }

  .sb-hit:hover { background: rgba(128, 128, 128, 0.14); }
  .sb-hit.active { background: color-mix(in srgb, var(--accent, #4a6b8a) 22%, transparent); }

  .sb-ch { font-size: 13px; font-weight: 600; }

  .sb-sn {
    font-size: 12px;
    opacity: 0.8;
    line-height: 1.55;
    margin-top: 2px;
    word-break: break-word;
  }

  .sb-sn :global(mark.mk) {
    background: #ffe28a;
    color: inherit;
    padding: 0 2px;
    border-radius: 2px;
  }

  .sb-empty {
    padding: 22px;
    text-align: center;
    font-size: 13px;
    opacity: 0.55;
  }
</style>
