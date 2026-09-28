<script lang="ts">
  import { normalizeColor, parseColor, toHex, toRgbString } from "../lib/color";
  import { portal } from "../lib/portal";

  let {
    value = $bindable(""),
    presets = [
      "#c0392b",
      "#e67e22",
      "#f1c40f",
      "#27ae60",
      "#2980b9",
      "#8e44ad",
      "#d3549e",
      "#7f8c8d",
    ],
    allowEmpty = false,
    emptyLabel = "跟随默认",
    title = "颜色",
    onchange,
  }: {
    value?: string;
    presets?: string[];
    allowEmpty?: boolean;
    emptyLabel?: string;
    title?: string;
    onchange?: () => void;
  } = $props();

  let open = $state(false);
  let btnEl: HTMLButtonElement | undefined = $state();
  let popEl: HTMLDivElement | undefined = $state();
  let hexText = $state("");
  let rgbText = $state("");
  let pos = $state({ x: 0, y: 0 });

  const swatch = $derived(value || "");

  function syncTexts() {
    const c = parseColor(value);
    hexText = c ? toHex(c) : "";
    rgbText = c ? toRgbString(c) : "";
  }

  function apply(v: string) {
    value = v;
    syncTexts();
    onchange?.();
  }

  function onHexInput() {
    const n = normalizeColor(hexText);
    if (n) {
      value = n;
      const c = parseColor(n)!;
      rgbText = toRgbString(c);
      onchange?.();
    }
  }

  function onRgbInput() {
    const n = normalizeColor(rgbText);
    if (n) {
      value = n;
      const c = parseColor(n)!;
      hexText = toHex(c);
      onchange?.();
    }
  }

  function toggle() {
    if (open) {
      open = false;
      return;
    }
    syncTexts();
    if (btnEl) {
      const r = btnEl.getBoundingClientRect();
      const popW = 250;
      const popH = 240;
      let x = Math.max(8, Math.min(r.left, window.innerWidth - popW - 8));
      let y = r.bottom + 6;
      if (y + popH > window.innerHeight - 8) y = Math.max(8, r.top - popH - 6);
      pos = { x, y };
    }
    open = true;
  }

  $effect(() => {
    if (!open) return;
    const h = (e: PointerEvent) => {
      if (btnEl?.contains(e.target as Node) || popEl?.contains(e.target as Node)) return;
      open = false;
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });
</script>

<button
  type="button"
  class="cp-trigger"
  bind:this={btnEl}
  onclick={toggle}
  title={title + (value ? `：${value}` : "")}
>
  {#if swatch}
    <span class="cp-dot" style="background:{swatch};"></span>
  {:else}
    <span class="cp-dot cp-none"></span>
  {/if}
  <span class="cp-val">{value || emptyLabel}</span>
</button>

{#if open}
  <div class="cp-pop" data-cp-pop use:portal bind:this={popEl} style="left:{pos.x}px; top:{pos.y}px;">
    <div class="cp-title">{title}</div>
    <div class="cp-grid">
      {#each presets as p (p)}
        <button
          type="button"
          class="cp-swatch"
          class:active={normalizeColor(value) === normalizeColor(p)}
          style="background:{p};"
          title={p}
          onclick={() => apply(normalizeColor(p) ?? p)}
        ></button>
      {/each}
      {#if allowEmpty}
        <button type="button" class="cp-swatch cp-clear" class:active={!value} title={emptyLabel} onclick={() => apply("")}>∅</button>
      {/if}
    </div>

    <div class="cp-row">
      <label>HEX</label>
      <input bind:value={hexText} oninput={onHexInput} placeholder="#rrggbb" spellcheck="false" />
    </div>
    <div class="cp-row">
      <label>RGB</label>
      <input bind:value={rgbText} oninput={onRgbInput} placeholder="rgb(255, 0, 0)" spellcheck="false" />
    </div>
    <div class="cp-row">
      <label>取色</label>
      <input
        type="color"
        class="cp-native"
        value={parseColor(value) ? toHex(parseColor(value)!) : "#000000"}
        oninput={(e) => apply(normalizeColor(e.currentTarget.value) ?? e.currentTarget.value)}
      />
    </div>
    <div class="cp-tip">支持 #hex 与 rgb()/rgba() 两种写法</div>
  </div>
{/if}

<style>
  .cp-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--panel);
    font-size: 12px;
    max-width: 100%;
  }

  .cp-dot {
    width: 14px;
    height: 14px;
    border-radius: 4px;
    border: 1px solid rgba(128, 128, 128, 0.4);
    flex-shrink: 0;
  }

  .cp-none {
    background:
      linear-gradient(45deg, transparent 45%, var(--danger, #c0392b) 45%, var(--danger, #c0392b) 55%, transparent 55%),
      var(--panel2);
  }

  .cp-val {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 150px;
    opacity: 0.8;
  }

  .cp-pop {
    position: fixed;
    z-index: 1100;
    width: 250px;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.14), 0 20px 44px rgba(0, 0, 0, 0.22);
    padding: 10px;
    animation: cp-in 0.18s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes cp-in {
    from { opacity: 0; transform: translateY(5px) scale(0.97); }
    to { opacity: 1; transform: none; }
  }

  .cp-title {
    font-size: 12px;
    font-weight: 600;
    opacity: 0.75;
    margin-bottom: 8px;
  }

  .cp-grid {
    display: grid;
    grid-template-columns: repeat(9, 1fr);
    gap: 5px;
    margin-bottom: 10px;
  }

  .cp-swatch {
    width: 20px;
    height: 20px;
    border-radius: 5px;
    border: 1px solid rgba(128, 128, 128, 0.35);
    cursor: pointer;
    padding: 0;
  }

  .cp-swatch.active {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .cp-clear {
    background: var(--panel2);
    font-size: 12px;
    line-height: 1;
  }

  .cp-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }

  .cp-row label {
    width: 34px;
    font-size: 12px;
    opacity: 0.6;
    flex-shrink: 0;
  }

  .cp-row input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
  }

  .cp-native {
    padding: 0;
    height: 26px;
    cursor: pointer;
  }

  .cp-tip {
    font-size: 12px;
    opacity: 0.5;
    margin-top: 4px;
  }
</style>
