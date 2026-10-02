<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { HIGHLIGHT_STYLES, config, saveConfig } from "../lib/stores.svelte";
  import { labelOfId } from "../lib/i18n/index.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import ColorPalette from "./ColorPalette.svelte";

  let {
    anchor,
    onhighlight,
    onsearch,
    onbrowser,
    onai,
    onclose,
  }: {
    anchor: PopupAnchor;
    onhighlight: (style: string, color: string) => void;
    onsearch: () => void;
    onbrowser: () => void;
    onai: () => void;
    onclose: () => void;
  } = $props();

  let menuEl: HTMLDivElement | undefined = $state();
  let activeColor = $state(config.reader.highlightColor || config.reader.highlightColors[0] || "#c0392b");
  let posStyle = $state("visibility:hidden;");

  $effect(() => {
    const h = (e: PointerEvent) => {
      const t = e.target as Element | null;
      // 取色框已 portal 到 body（在 menuEl 之外）：点其内部不应关闭本菜单。
      if (t?.closest?.("[data-cp-pop]")) return;
      if (menuEl && !menuEl.contains(t as Node)) onclose();
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

<div class="sel-menu" bind:this={menuEl} style={posStyle} oncontextmenu={(e) => e.preventDefault()}>
  <div class="hl-row" title={t("selection.styleTip")}>
    {#each HIGHLIGHT_STYLES as s (s.id)}
      <button class="sw sw-{s.id}" data-testid={"sel-style-" + s.id} title={labelOfId("hlStyle", s.id)} onclick={() => onhighlight(s.id, activeColor)}>
        <span class="sw-line" style="color:{activeColor};">{t("selection.text")}</span>
      </button>
    {/each}
  </div>
  <div class="color-row" title={t("selection.paletteTip")}>
    <ColorPalette
      bind:colors={config.reader.highlightColors}
      value={activeColor}
      applyLabel={t("selection.use")}
      onapply={(c) => (activeColor = c)}
      onpalette={saveConfig}
    />
  </div>
  <div class="sel-sep"></div>
  <div class="act-row">
    <button class="hl-main" data-testid="sel-hl" onclick={() => onhighlight(config.reader.highlightStyle || "underline", activeColor)} title={t("selection.hlTip")}>{t("highlight.hl")}</button>
    <button data-testid="sel-search" onclick={onsearch} title={t("selection.searchBookTip")}>{t("selection.searchBook")}</button>
    <button data-testid="sel-browser" onclick={onbrowser} title={t("selection.searchBrowserTip")}>{t("selection.searchBrowser")}</button>
    <button class="ai" data-testid="sel-ai" onclick={onai} title={t("selection.aiTip")}>{t("selection.aiLabel")}</button>
  </div>
</div>

<style>
  .sel-menu {
    position: fixed;
    z-index: 980;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 88%, #202020 12%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.28);
    border-radius: 12px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16), 0 20px 44px rgba(0, 0, 0, 0.24);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 208px;
    user-select: none;
    animation: sel-in 0.18s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes sel-in {
    from { opacity: 0; transform: translateY(4px) scale(0.96); }
    to { opacity: 1; transform: none; }
  }

  /* 组间细分隔线，层次更清晰 */
  .sel-sep {
    height: 1px;
    margin: 1px 2px;
    background: rgba(128, 128, 128, 0.2);
  }

  .hl-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .sw {
    width: 28px;
    height: 24px;
    padding: 0;
    display: grid;
    place-items: center;
    border-radius: 7px;
    background: transparent;
    border: 1px solid rgba(128, 128, 128, 0.25);
    cursor: pointer;
    transition: background var(--dur, 0.2s) var(--ease), border-color var(--dur, 0.2s) var(--ease), transform var(--dur, 0.2s) var(--ease-premium, ease);
  }

  .sw:hover {
    background: rgba(128, 128, 128, 0.16);
    border-color: rgba(128, 128, 128, 0.4);
  }

  .sw:active { transform: scale(0.92); }

  .sw-line {
    display: block;
    font-size: 16px;
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
    gap: 6px;
  }

  .act-row button {
    flex: 1;
    font-size: 12px;
    padding: 5px 8px;
    border-radius: 8px;
    border: 1px solid rgba(128, 128, 128, 0.28);
    background: transparent;
    color: inherit;
    white-space: nowrap;
    transition: background var(--dur, 0.2s) var(--ease), border-color var(--dur, 0.2s) var(--ease), color var(--dur, 0.2s) var(--ease), transform var(--dur, 0.2s) var(--ease-premium, ease);
  }

  .act-row button:hover {
    background: rgba(128, 128, 128, 0.14);
    border-color: rgba(128, 128, 128, 0.42);
  }

  .act-row button:active { transform: scale(0.96); }

  /* 主操作「划线」用主色实底，AI 用主色描边，强调层次 */
  .act-row .hl-main {
    background: var(--accent, #4a6b8a);
    border-color: transparent;
    color: #fff;
    font-weight: 600;
  }

  .act-row .hl-main:hover {
    background: color-mix(in srgb, var(--accent, #4a6b8a) 88%, #000);
    border-color: transparent;
  }

  .act-row .ai {
    color: var(--accent, #4a6b8a);
    border-color: color-mix(in srgb, var(--accent, #4a6b8a) 42%, transparent);
  }

  .act-row .ai:hover {
    background: color-mix(in srgb, var(--accent, #4a6b8a) 14%, transparent);
    border-color: color-mix(in srgb, var(--accent, #4a6b8a) 55%, transparent);
  }
</style>
