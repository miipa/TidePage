<script lang="ts">
  import { HIGHLIGHT_STYLES, config, saveConfig } from "../lib/stores.svelte";
  import type { HighlightRow } from "../lib/types";
  import ColorPalette from "./ColorPalette.svelte";

  let {
    highlight,
    x = 0,
    y = 0,
    below = false,
    oncolor,
    onstyle,
    ondelete,
    onclose,
  }: {
    highlight: HighlightRow;
    x?: number;
    y?: number;
    below?: boolean;
    oncolor: (color: string) => void;
    onstyle: (style: string) => void;
    ondelete: () => void;
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

<div class="hl-menu" bind:this={menuEl} style={pos}>
  <div class="hm-text" title={highlight.text}>{highlight.text}</div>
  <div class="hm-colors">
    <ColorPalette
      bind:colors={config.reader.highlightColors}
      value={highlight.color}
      applyLabel="应用"
      onapply={oncolor}
      onpalette={saveConfig}
    />
  </div>
  <div class="hm-row">
    <label>样式</label>
    <select
      value={highlight.style || "underline"}
      onchange={(e) => onstyle((e.currentTarget as HTMLSelectElement).value)}
    >
      {#each HIGHLIGHT_STYLES as s (s.id)}
        <option value={s.id}>{s.name}</option>
      {/each}
    </select>
    <button class="danger" onclick={ondelete} title="删除此划线">删除</button>
  </div>
</div>

<style>
  .hl-menu {
    position: fixed;
    z-index: 980;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 90%, #202020 10%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 10px;
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.28);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 7px;
    max-width: 340px;
  }

  .hm-text {
    font-size: 12px;
    opacity: 0.65;
    max-width: 320px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hm-colors {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .csw {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 1px solid rgba(128, 128, 128, 0.4);
    padding: 0;
    cursor: pointer;
  }

  .csw.active {
    outline: 2px solid var(--accent, #4a6b8a);
    outline-offset: 1px;
  }

  .hm-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .hm-row label {
    font-size: 12px;
    opacity: 0.6;
  }

  .hm-row select {
    flex: 1;
    min-width: 0;
    font-size: 12px;
  }

  .hm-row .danger {
    color: var(--danger, #c0392b);
    font-size: 12px;
  }
</style>
