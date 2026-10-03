<script lang="ts">
  import { normalizeColor, parseColor, toHex, toRgbString } from "../lib/color";
  import { portal } from "../lib/portal";
  import { t } from "../lib/i18n/index.svelte";

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
    // 默认文案在**使用点**用 t() 现取（不能在 $props() 默认值里求值：那样切语言不会更新）。
    emptyLabel = undefined,
    title = undefined,
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
  /** 标题/空值的默认文案：调用点没传就用本地化默认值（放在函数里求值，切语言即时生效）。 */
  const titleText = $derived(title ?? t("common.color.name"));
  const emptyText = $derived(emptyLabel ?? t("common.color.followDefault"));
</script>

<button
  type="button"
  class="cp-trigger"
  bind:this={btnEl}
  onclick={toggle}
  title={value ? t("common.color.titleWithValue", { name: titleText, value }) : titleText}
>
  {#if swatch}
    <span class="cp-dot" style="background:{swatch};"></span>
  {:else}
    <span class="cp-dot cp-none"></span>
  {/if}
  <span class="cp-val">{value || emptyText}</span>
</button>

{#if open}
    <div class="cp-pop pop" data-cp-pop use:portal bind:this={popEl} style="left:{pos.x}px; top:{pos.y}px;">
    <div class="cp-title">{titleText}</div>
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
        <!-- 「无颜色」不再叠加 ∅ 字形：45° 斜纹底（.cp-none）本身就是正确信号，
             再加一个字形只会把它变成第二个含义不同的标记。按钮保留 title/aria-label。 -->
        <button
          type="button"
          class="cp-swatch cp-clear"
          class:active={!value}
          title={emptyText}
          aria-label={emptyText}
          onclick={() => apply("")}
        ></button>
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
      <label>{t("common.color.pick")}</label>
      <input
        type="color"
        class="cp-native"
        value={parseColor(value) ? toHex(parseColor(value)!) : "#000000"}
        oninput={(e) => apply(normalizeColor(e.currentTarget.value) ?? e.currentTarget.value)}
      />
    </div>
    <div class="cp-tip">{t("common.color.hint")}</div>
  </div>
{/if}

<style>
  .cp-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 3px var(--sp-2);
    border: 1px solid var(--line);
    border-radius: var(--radius-sm);
    background: var(--panel);
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    max-width: 100%;
  }

  .cp-dot {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-xs);
    border: 1px solid var(--line-strong);
    flex-shrink: 0;
  }

  /* 「无颜色」斜纹：唯一保留的 linear-gradient 用途（语义标记，非装饰） */
  .cp-none {
    background:
      linear-gradient(45deg, transparent 45%, var(--danger) 45%, var(--danger) 55%, transparent 55%),
      var(--panel2);
  }

  .cp-val {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 150px;
    opacity: 0.8;
  }

  /* 底色 / 描边 / 圆角 / 投影走全局 .pop（外壳浮层，跟随外壳皮肤）。
     本组件被设置弹窗与阅读区划线菜单共用，两种语境下一律跟外壳：
     它是配置工具而非页面内容，且能保证底色与文字色必然成对同源、不会对比度不足。 */
  .cp-pop {
    position: fixed;
    z-index: var(--z-popover);
    width: 250px;
    padding: 10px;
    animation: fr-pop-in var(--dur-fast) var(--ease-premium) both;
  }

  .cp-title {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    font-weight: 600;
    opacity: 0.75;
    margin-bottom: var(--sp-2);
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
    border-radius: var(--radius-sm);
    border: 1px solid var(--line-strong);
    cursor: pointer;
    padding: 0;
  }

  /* 同 ColorPalette：outline 不可过渡，改用可插值的 box-shadow */
  .cp-swatch {
    transition: box-shadow var(--dur) var(--ease);
  }

  .cp-swatch.active {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .cp-clear {
    background: var(--panel2);
    font-size: var(--fs-xs);
    line-height: 1;
    display: grid;
    place-items: center;
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

  .cp-tip {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.5;
    margin-top: var(--sp-1);
  }
</style>
