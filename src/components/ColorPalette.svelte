<script lang="ts">
  import { tick } from "svelte";
  import { normalizeColor, parseColor, toHex, toRgbString } from "../lib/color";

  let {
    colors = $bindable([] as string[]),
    value = "",
    applyLabel = "应用",
    onapply,
    onpalette,
  }: {
    colors?: string[];
    value?: string;
    applyLabel?: string;
    onapply: (color: string) => void;
    onpalette?: () => void;
  } = $props();

  let wrapEl: HTMLDivElement | undefined = $state();
  let editIdx = $state(-1);
  let hexText = $state("");
  let rgbText = $state("");
  let popPos = $state({ x: 0, y: 0 });

  function syncTexts(c: string) {
    const p = parseColor(c);
    hexText = p ? toHex(p) : "";
    rgbText = p ? toRgbString(p) : "";
  }

  function openEditor(i: number) {
    editIdx = i;
    syncTexts(colors[i] || "");
    const el = wrapEl?.querySelector<HTMLElement>(`[data-ci="${i}"]`);
    if (el) {
      const r = el.getBoundingClientRect();
      const w = 250;
      const h = 222;
      let x = Math.max(8, Math.min(r.left, window.innerWidth - w - 8));
      let y = r.bottom + 6;
      if (y + h > window.innerHeight - 8) y = Math.max(8, r.top - h - 6);
      popPos = { x, y };
    }
  }

  function closeEditor() {
    editIdx = -1;
  }

  function onHex() {
    const n = normalizeColor(hexText);
    if (!n || editIdx < 0) return;
    colors[editIdx] = n;
    const p = parseColor(n)!;
    rgbText = toRgbString(p);
    onpalette?.();
  }

  function onRgb() {
    const n = normalizeColor(rgbText);
    if (!n || editIdx < 0) return;
    colors[editIdx] = n;
    const p = parseColor(n)!;
    hexText = toHex(p);
    onpalette?.();
  }

  function onNative(e: Event) {
    const n = normalizeColor((e.currentTarget as HTMLInputElement).value);
    if (!n || editIdx < 0) return;
    colors[editIdx] = n;
    syncTexts(n);
    onpalette?.();
  }

  function apply() {
    const c = editIdx >= 0 ? colors[editIdx] : normalizeColor(hexText);
    if (c) onapply(c);
    closeEditor();
  }

  function removeColor() {
    if (editIdx < 0) return;
    colors.splice(editIdx, 1);
    onpalette?.();
    closeEditor();
  }

  async function addColor() {
    colors.push("#888888");
    onpalette?.();
    await tick();
    openEditor(colors.length - 1);
  }

  $effect(() => {
    if (editIdx < 0) return;
    const h = (e: PointerEvent) => {
      if (wrapEl && !wrapEl.contains(e.target as Node)) closeEditor();
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  const activeHex = $derived(normalizeColor(value || "") || "");
</script>

<div class="palette" bind:this={wrapEl}>
  {#each colors as c, i (i)}
    <button
      type="button"
      class="cp-swatch"
      class:active={activeHex && activeHex === normalizeColor(c)}
      data-ci={i}
      style="background:{c};"
      title="点击设置该颜色"
      onclick={() => openEditor(i)}
    ></button>
  {/each}
  <button type="button" class="cp-add" title="添加颜色" onclick={addColor}>＋</button>

  {#if editIdx >= 0}
    <div class="cp-pop" style="left:{popPos.x}px; top:{popPos.y}px;">
      <div class="cp-row">
        <label>HEX</label>
        <input bind:value={hexText} oninput={onHex} placeholder="#rrggbb" spellcheck="false" />
      </div>
      <div class="cp-row">
        <label>RGB</label>
        <input bind:value={rgbText} oninput={onRgb} placeholder="rgb(255, 0, 0)" spellcheck="false" />
      </div>
      <div class="cp-row">
        <label>取色</label>
        <input
          type="color"
          class="cp-native"
          value={parseColor(colors[editIdx] || "") ? toHex(parseColor(colors[editIdx] || "")!) : "#000000"}
          oninput={onNative}
        />
      </div>
      <div class="cp-acts">
        <button type="button" class="danger" onclick={removeColor} title="从调色板删除此颜色">删除</button>
        <span class="flex1"></span>
        <button type="button" class="primary" onclick={apply}>{applyLabel}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .palette {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .cp-swatch {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 1px solid rgba(128, 128, 128, 0.4);
    padding: 0;
    cursor: pointer;
    flex-shrink: 0;
  }

  .cp-swatch.active {
    outline: 2px solid var(--accent, #4a6b8a);
    outline-offset: 1px;
  }

  .cp-add {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 1px dashed rgba(128, 128, 128, 0.6);
    background: transparent;
    color: inherit;
    padding: 0;
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
    flex-shrink: 0;
  }

  .cp-pop {
    position: fixed;
    z-index: 1100;
    width: 250px;
    background: var(--rdr-bg, var(--panel, #fff));
    color: var(--rdr-fg, var(--fg, #222));
    border: 1px solid var(--line, rgba(128, 128, 128, 0.3));
    border-radius: 10px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.14), 0 20px 44px rgba(0, 0, 0, 0.22);
    padding: 10px;
    user-select: none;
    animation: cp-in 0.18s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes cp-in {
    from { opacity: 0; transform: translateY(5px) scale(0.97); }
    to { opacity: 1; transform: none; }
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

  .cp-acts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }

  .cp-acts .danger { color: var(--danger, #c0392b); font-size: 12px; }
  .cp-acts .primary { font-size: 12px; padding: 4px 12px; }
  .flex1 { flex: 1; }
</style>
