<script lang="ts">
  import { HIGHLIGHT_STYLES, config } from "../lib/stores.svelte";

  let {
    x = 0,
    y = 0,
    below = false,
    onhighlight,
    onsearch,
    onbrowser,
    onai,
    onclose,
  }: {
    x?: number;
    y?: number;
    below?: boolean;
    onhighlight: (style: string) => void;
    onsearch: () => void;
    onbrowser: () => void;
    onai: () => void;
    onclose: () => void;
  } = $props();

  let menuEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    const h = (e: PointerEvent) => {
      if (menuEl && !menuEl.contains(e.target as Node)) onclose();
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  const pos = $derived(
    `left:${x}px; top:${y}px; transform:translate(-50%, ${below ? "0" : "-100%"});`,
  );
</script>

<div class="sel-menu" bind:this={menuEl} style={pos} oncontextmenu={(e) => e.preventDefault()}>
  <div class="hl-row">
    {#each HIGHLIGHT_STYLES as s (s.id)}
      <button class="sw sw-{s.id}" title={s.name} onclick={() => onhighlight(s.id)}>
        <span class="sw-line">文</span>
      </button>
    {/each}
  </div>
  <div class="act-row">
    <button class="hl-main" onclick={() => onhighlight(config.reader.highlightStyle || "underline")} title="按默认样式划线">划线</button>
    <button onclick={onsearch} title="在本书中搜索选中文字">全文</button>
    <button onclick={onbrowser} title="用浏览器搜索选中文字">浏览器</button>
    <button onclick={onai} title="AI 翻译 / 释义">AI 释义</button>
  </div>
</div>

<style>
  .sel-menu {
    position: fixed;
    z-index: 980;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 88%, #202020 12%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.28);
    border-radius: 10px;
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.28);
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    user-select: none;
    animation: sel-in 0.12s ease;
  }

  @keyframes sel-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .hl-row {
    display: flex;
    gap: 5px;
    align-items: center;
  }

  .sw {
    width: 26px;
    height: 22px;
    padding: 0;
    display: grid;
    place-items: center;
    border-radius: 6px;
    background: transparent;
    border: 1px solid rgba(128, 128, 128, 0.25);
    cursor: pointer;
  }

  .sw:hover {
    background: rgba(128, 128, 128, 0.16);
  }

  .sw-line {
    display: block;
    font-size: 15px;
    line-height: 1.15;
    color: var(--hl-color, var(--accent, #c0392b));
  }

  .sw-underline .sw-line { text-decoration: underline 2px currentColor; text-underline-offset: 2px; }
  .sw-wavy .sw-line { text-decoration: underline wavy 2px currentColor; text-underline-offset: 3px; }
  .sw-dotted .sw-line { text-decoration: underline dotted 2px currentColor; text-underline-offset: 2px; }
  .sw-double .sw-line { text-decoration: underline double 2px currentColor; text-underline-offset: 2px; }
  .sw-strike .sw-line { text-decoration: line-through 2px currentColor; }
  .sw-box .sw-line { box-shadow: 0 0 0 1.5px currentColor; border-radius: 3px; }
  .sw-background .sw-line { background: color-mix(in srgb, currentColor 42%, transparent); border-radius: 3px; }

  .act-row {
    display: flex;
    gap: 4px;
  }

  .act-row button {
    font-size: 12px;
    padding: 3px 9px;
    border-radius: 7px;
  }
</style>
