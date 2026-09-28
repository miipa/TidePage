<script lang="ts">
  import { placePopup, type PopupAnchor } from "../lib/placePopup";

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

<div class="ts-menu" bind:this={menuEl} style={posStyle}>
  <div class="ts-title">从哪里开始朗读？</div>
  <button class="ts-btn" onclick={onpage}>从本页开始</button>
  <button class="ts-btn" onclick={onchapter}>从章节开头开始</button>
</div>

<style>
  .ts-menu {
    position: fixed;
    z-index: 985;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 92%, #202020 8%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 10px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16), 0 20px 44px rgba(0, 0, 0, 0.24);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 150px;
    animation: ts-in 0.18s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes ts-in {
    from { opacity: 0; transform: translateY(5px) scale(0.97); }
    to { opacity: 1; transform: none; }
  }

  .ts-title {
    font-size: 12px;
    opacity: 0.65;
    padding: 2px 4px 4px;
  }

  .ts-btn {
    text-align: left;
    font-size: 13px;
    padding: 8px 12px;
    border-radius: 7px;
  }

  .ts-btn:hover {
    background: rgba(128, 128, 128, 0.16);
  }
</style>
