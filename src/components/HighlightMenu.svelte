<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { HIGHLIGHT_STYLES, config, saveConfig } from "../lib/stores.svelte";
  import { labelOfId } from "../lib/i18n/index.svelte";
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

<div class="hl-menu rdr-pop" bind:this={menuEl} style={posStyle}>
  <div class="hm-text" title={highlight.text}>{highlight.text}</div>
  <div class="hm-colors">
    <ColorPalette
      bind:colors={config.reader.highlightColors}
      value={highlight.color}
      applyLabel={t("common.action.apply")}
      onapply={oncolor}
      onpalette={saveConfig}
    />
  </div>
  <div class="hm-row">
    <label>{t("highlight.menu.style")}</label>
    <select
      value={highlight.style || "underline"}
      onchange={(e) => onstyle((e.currentTarget as HTMLSelectElement).value)}
    >
      {#each HIGHLIGHT_STYLES as s (s.id)}
        <option value={s.id}>{labelOfId("hlStyle", s.id)}</option>
      {/each}
    </select>
    <button data-testid="hm-ai" onclick={onai} title={t("highlight.menu.aiTip")}>AI</button>
    <button data-testid="hm-note" onclick={() => (noteOpen = !noteOpen)} title={t("highlight.menu.noteTip")}>
      {highlight.note ? t("highlight.menu.noteEdit") : t("highlight.menu.noteAdd")}
    </button>
    <button class="danger" data-testid="hm-delete" onclick={ondelete} title={t("highlight.deleteThis")}>{t("common.action.delete")}</button>
  </div>
  {#if noteOpen}
    <div class="hm-note">
      <textarea
        rows="3"
        data-testid="hm-note-text"
        placeholder={t("highlight.menu.notePlaceholder")}
        bind:value={noteText}
        onkeydown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) onnote(noteText); }}
      ></textarea>
      <div class="hm-note-acts">
        <button class="primary" data-testid="hm-note-save" onclick={() => onnote(noteText)}>{t("highlight.menu.noteSave")}</button>
        <button data-testid="hm-note-cancel" onclick={() => (noteOpen = false)}>{t("common.confirm.cancel")}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  /* 底色 / 描边 / 圆角 / 投影走全局 .rdr-pop（跟随阅读主题） */
  .hl-menu {
    position: fixed;
    z-index: var(--z-popover);
    padding: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: 7px;
    max-width: 340px;
    animation: fr-pop-in var(--dur-fast) var(--ease-premium) both;
  }

  .hm-text {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.65;
    max-width: 320px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hm-colors {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .hm-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .hm-row label {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.6;
  }

  .hm-row select {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
  }

  .hm-row .danger {
    color: var(--danger);
    font-size: var(--fs-xs);
  }

  .hm-note { display: flex; flex-direction: column; gap: var(--sp-1); }
  .hm-note textarea {
    width: 100%;
    box-sizing: border-box;
    font-size: var(--fs-sm);
    resize: vertical;
    min-height: 60px;
  }
  .hm-note-acts { display: flex; gap: var(--sp-1); justify-content: flex-end; }
</style>
