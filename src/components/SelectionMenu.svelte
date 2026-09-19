<script lang="ts">
  import { HIGHLIGHT_COLORS, HIGHLIGHT_STYLES, config } from "../lib/stores.svelte";
  import { normalizeColor } from "../lib/color";
  import ColorPicker from "./ColorPicker.svelte";

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
    onhighlight: (style: string, color: string) => void;
    onsearch: () => void;
    onbrowser: () => void;
    onai: () => void;
    onclose: () => void;
  } = $props();

  let menuEl: HTMLDivElement | undefined = $state();
  let activeColor = $state(config.reader.highlightColor || HIGHLIGHT_COLORS[0]);

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
  <div class="hl-row" title="点击样式即按当前颜色划线">
    {#each HIGHLIGHT_STYLES as s (s.id)}
      <button class="sw sw-{s.id}" title={s.name} onclick={() => onhighlight(s.id, activeColor)}>
        <span class="sw-line" style="color:{activeColor};">文</span>
      </button>
    {/each}
  </div>
  <div class="color-row">
    {#each HIGHLIGHT_COLORS as c (c)}
      <button
        class="csw"
        class:active={normalizeColor(activeColor) === normalizeColor(c)}
        style="background:{c};"
        title={c}
        onclick={() => (activeColor = c)}
      ></button>
    {/each}
    <ColorPicker
      bind:value={activeColor}
      title="自定义划线颜色"
      onchange={() => { /* 仅选定，点样式或「划线」应用 */ }}
    />
  </div>
  <div class="act-row">
    <button class="hl-main" onclick={() => onhighlight(config.reader.highlightStyle || "underline", activeColor)} title="按默认样式与当前颜色划线">划线</button>
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

  .color-row {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .csw {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 1px solid rgba(128, 128, 128, 0.4);
    padding: 0;
    cursor: pointer;
    flex-shrink: 0;
  }

  .csw.active {
    outline: 2px solid var(--accent, #4a6b8a);
    outline-offset: 1px;
  }

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
