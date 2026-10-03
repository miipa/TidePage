<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { tick } from "svelte";
  import { normalizeColor, parseColor, toHex, toRgbString } from "../lib/color";
  import { portal } from "../lib/portal";
  import { IconPlus } from "@tabler/icons-svelte";

  let {
    colors = $bindable([] as string[]),
    value = "",
    applyLabel = undefined,
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
  let popEl: HTMLDivElement | undefined = $state();
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
      const t = e.target as Node;
      // 取色框已 portal 到 body，需同时排除 wrapEl 与 popEl，否则点框内会误关。
      if (wrapEl?.contains(t) || popEl?.contains(t)) return;
      closeEditor();
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
      data-testid="cpal-swatch"
      title={t("palette.tip")}
      onclick={() => onapply(c)}
      oncontextmenu={(e) => { e.preventDefault(); openEditor(i); }}
    ></button>
  {/each}
  <button
    type="button"
    class="cp-add"
    data-testid="cpal-add"
    title={t("palette.add")}
    onclick={addColor}
    oncontextmenu={(e) => e.preventDefault()}
    aria-label={t("palette.add")}
  ><IconPlus /></button>

  {#if editIdx >= 0}
    <div class="cp-pop pop" data-cp-pop use:portal bind:this={popEl} style="left:{popPos.x}px; top:{popPos.y}px;">
      <div class="cp-row">
        <label>HEX</label>
        <input bind:value={hexText} oninput={onHex} placeholder="#rrggbb" spellcheck="false" />
      </div>
      <div class="cp-row">
        <label>RGB</label>
        <input bind:value={rgbText} oninput={onRgb} placeholder="rgb(255, 0, 0)" spellcheck="false" />
      </div>
      <div class="cp-row">
        <label>{t("common.color.pick")}</label>
        <input
          type="color"
          class="cp-native"
          value={parseColor(colors[editIdx] || "") ? toHex(parseColor(colors[editIdx] || "")!) : "#000000"}
          oninput={onNative}
        />
      </div>
      <div class="cp-acts">
        <button type="button" class="danger" data-testid="cpal-del" onclick={removeColor} title={t("palette.deleteTip")}>{t("common.action.delete")}</button>
        <span class="flex1"></span>
        <button type="button" class="primary" data-testid="cpal-apply" onclick={apply}>{applyLabel ?? t("common.action.apply")}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .palette {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .cp-swatch {
    width: 30px;
    height: 18px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--line-strong);
    padding: 0;
    cursor: pointer;
    flex-shrink: 0;
    transition: transform var(--dur) var(--ease-premium), box-shadow var(--dur) var(--ease);
  }

  .cp-swatch:hover { transform: translateY(-1px); }

  /* 同 Library 选中态：outline 不可过渡，改用可插值的 box-shadow */
  .cp-swatch {
    transition: transform var(--dur) var(--ease-premium), box-shadow var(--dur) var(--ease);
  }

  .cp-swatch.active {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .cp-add {
    width: 30px;
    height: 18px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    border: 1px dashed var(--line-strong);
    background: transparent;
    color: inherit;
    padding: 0;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    line-height: 1;
  }

  /* 底色 / 描边 / 圆角 / 投影走全局 .pop（外壳浮层，跟随外壳皮肤）。
     本组件被设置弹窗与阅读区划线/划线面板共用，两种语境下一律跟外壳：
     它是配置工具而非页面内容，且能保证底色与文字色必然成对同源、不会对比度不足。 */
  .cp-pop {
    position: fixed;
    z-index: var(--z-popover);
    width: 250px;
    padding: 10px;
    user-select: none;
    animation: fr-pop-in var(--dur-fast) var(--ease-premium) both;
  }

  .cp-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-bottom: var(--sp-1);
  }

  .cp-row label {
    width: 34px;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.6;
    flex-shrink: 0;
  }

  .cp-row input {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
  }

  .cp-native {
    padding: 0;
    height: var(--ctl-h-sm);
    cursor: pointer;
  }

  .cp-acts {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    margin-top: var(--sp-1);
  }

  .cp-acts .danger { color: var(--danger); font-size: var(--fs-xs); }
  .cp-acts .primary { font-size: var(--fs-xs); padding: var(--sp-1) var(--sp-3); }
  .flex1 { flex: 1; }
</style>
