<script lang="ts">
  import { HIGHLIGHT_STYLES, config, saveConfig } from "../lib/stores.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import type { HighlightRow } from "../lib/types";
  import ColorPalette from "./ColorPalette.svelte";

  let {
    highlight,
    anchor,
    oncolor,
    onstyle,
    onnote,
    onai,
    ondelete,
    onclose,
  }: {
    highlight: HighlightRow;
    anchor: PopupAnchor;
    oncolor: (color: string) => void;
    onstyle: (style: string) => void;
    onnote: (note: string) => void;
    /** H3：对该划线发起 AI 释义（结果可添加到笔记）。 */
    onai: () => void;
    ondelete: () => void;
    onclose: () => void;
  } = $props();

  let menuEl: HTMLDivElement | undefined = $state();
  let posStyle = $state("visibility:hidden;");
  /** H1：添加/编辑感想。 */
  let noteOpen = $state(false);
  let noteText = $state(highlight.note ?? "");

  $effect(() => {
    const h = (e: PointerEvent) => {
      if (menuEl && !menuEl.contains(e.target as Node)) onclose();
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  $effect(() => {
    void anchor.top;
    void anchor.bottom;
    void anchor.cx;
    if (!menuEl) return;
    requestAnimationFrame(() => {
      if (!menuEl) return;
      const p = placePopup(anchor, menuEl.offsetWidth, menuEl.offsetHeight);
      posStyle = `left:${p.left}px; top:${p.top}px;`;
    });
  });
</script>

<div class="hl-menu" bind:this={menuEl} style={posStyle}>
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
    <button onclick={onai} title="AI 翻译/释义（结果可添加到笔记）">AI</button>
    <button onclick={() => (noteOpen = !noteOpen)} title="添加/编辑感想（会进入笔记列表）">
      {highlight.note ? "编辑感想" : "添加感想"}
    </button>
    <button class="danger" onclick={ondelete} title="删除此划线">删除</button>
  </div>
  {#if noteOpen}
    <div class="hm-note">
      <textarea
        rows="3"
        placeholder="写下你的感想…（留空则取消笔记）"
        bind:value={noteText}
        onkeydown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) onnote(noteText); }}
      ></textarea>
      <div class="hm-note-acts">
        <button class="primary" onclick={() => onnote(noteText)}>保存感想</button>
        <button onclick={() => (noteOpen = false)}>取消</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .hl-menu {
    position: fixed;
    z-index: 980;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 90%, #202020 10%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 10px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16), 0 20px 44px rgba(0, 0, 0, 0.24);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 7px;
    max-width: 340px;
    animation: hm-in 0.18s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes hm-in {
    from { opacity: 0; transform: translateY(4px) scale(0.96); }
    to { opacity: 1; transform: none; }
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

  .hm-note { display: flex; flex-direction: column; gap: 6px; }
  .hm-note textarea {
    width: 100%;
    box-sizing: border-box;
    font-size: 13px;
    resize: vertical;
    min-height: 60px;
  }
  .hm-note-acts { display: flex; gap: 6px; justify-content: flex-end; }
</style>
