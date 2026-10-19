<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { HIGHLIGHT_STYLES, config, saveConfig } from "../lib/stores.svelte";
  import { labelOfId } from "../lib/i18n/index.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import { outFade } from "../lib/motion";
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

<div class="sel-menu rdr-pop" out:outFade={{ from: 0.96 }} bind:this={menuEl} style={posStyle} oncontextmenu={(e) => e.preventDefault()}>
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
  /* 底色 / 描边 / 圆角 / 投影走全局 .rdr-pop（跟随阅读主题） */
  .sel-menu {
    position: fixed;
    z-index: var(--z-popover);
    padding: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 208px;
    user-select: none;
    animation: tp-pop-in var(--dur-fast) var(--ease-premium) both;
  }

  /* 组间细分隔线，层次更清晰 */
  .sel-sep {
    height: 1px;
    margin: 1px 2px;
    background: color-mix(in srgb, var(--rdr-fg) 16%, transparent);
  }

  .hl-row {
    display: flex;
    gap: var(--sp-1);
    align-items: center;
  }

  .sw {
    width: 28px;
    height: 24px;
    padding: 0;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    background: transparent;
    border: 1px solid var(--rdr-line);
    cursor: pointer;
    transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease), transform var(--dur) var(--ease-premium);
  }

  .sw:hover {
    background: color-mix(in srgb, var(--rdr-fg) 10%, transparent);
    border-color: color-mix(in srgb, var(--rdr-fg) 34%, transparent);
  }

  .sw:active { transform: scale(0.92); }

  .sw-line {
    display: block;
    font-size: var(--fs-lg);
    line-height: 1.15;
    color: var(--hl-color);
  }

  .sw-underline .sw-line { text-decoration: underline 2px currentColor; text-underline-offset: 2px; }
  .sw-wavy .sw-line { text-decoration: underline wavy 2px currentColor; text-underline-offset: 3px; }
  .sw-dotted .sw-line { text-decoration: underline dotted 2px currentColor; text-underline-offset: 2px; }
  .sw-double .sw-line { text-decoration: underline double 2px currentColor; text-underline-offset: 2px; }
  .sw-strike .sw-line { text-decoration: line-through 2px currentColor; }
  .sw-box .sw-line { box-shadow: 0 0 0 1.5px currentColor; border-radius: var(--radius-xs); }
  .sw-background .sw-line { background: color-mix(in srgb, currentColor 42%, transparent); border-radius: var(--radius-xs); }

  .color-row {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .act-row {
    display: flex;
    gap: var(--sp-1);
  }

  .act-row button {
    flex: 1;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    padding: 5px var(--sp-2);
    border-radius: var(--radius-sm);
    border: 1px solid var(--rdr-line);
    background: transparent;
    color: inherit;
    white-space: nowrap;
    transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease), color var(--dur) var(--ease), transform var(--dur) var(--ease-premium);
  }

  .act-row button:hover {
    background: color-mix(in srgb, var(--rdr-fg) 10%, transparent);
    border-color: color-mix(in srgb, var(--rdr-fg) 34%, transparent);
  }

  .act-row button:active { transform: scale(0.96); }

  /* 主操作「划线」用主色实底，AI 用主色描边，强调层次 */
  .act-row .hl-main {
    background: var(--accent);
    border-color: transparent;
    color: var(--accent-fg);
    font-weight: 600;
  }

  .act-row .hl-main:hover {
    background: var(--accent-hover);
    border-color: transparent;
  }

  .act-row .ai {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 42%, transparent);
  }

  .act-row .ai:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
</style>
