<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";
  import { outFade } from "../lib/motion";

  let {
    anchor,
    onpage,
    onchapter,
    onclose,
  }: {
    anchor: PopupAnchor;
    onpage: () => void;
    onchapter: () => void;
    onclose: () => void;
  } = $props();

  let menuEl: HTMLDivElement | undefined = $state();
  let posStyle = $state("visibility:hidden;");

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

<div class="ts-menu rdr-pop" out:outFade={{ from: 0.96 }} bind:this={menuEl} style={posStyle}>
  <div class="ts-title">{t("tts.start.ask")}</div>
  <button class="ts-btn" data-testid="ts-page" onclick={onpage}>{t("tts.start.page")}</button>
  <button class="ts-btn" data-testid="ts-chapter" onclick={onchapter}>{t("tts.start.chapter")}</button>
</div>

<style>
  /* 底色 / 描边 / 圆角 / 投影走全局 .rdr-pop（跟随阅读主题） */
  .ts-menu {
    position: fixed;
    z-index: var(--z-popover);
    padding: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-width: 150px;
    animation: tp-pop-in var(--dur-fast) var(--ease-premium) both;
  }

  .ts-title {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.65;
    padding: 2px var(--sp-1) var(--sp-1);
  }

  .ts-btn {
    text-align: left;
    font-size: var(--fs-sm);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-sm);
  }

  .ts-btn:hover {
    background: color-mix(in srgb, var(--rdr-fg) 10%, transparent);
  }
</style>
