<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { toast } from "../lib/stores.svelte";
  import ContextMenu, { type CtxItem } from "../components/ContextMenu.svelte";
  // i18n 的 t() 与本文件里的局部变量 t 撞名，故导入别名 tr（本文件只有这两处用中文文案）。
  import { t as tr } from "../lib/i18n/index.svelte";

  let { children } = $props();

  let ctx = $state<{ x: number; y: number; target: HTMLElement } | null>(null);

  const TEXT_INPUT_TYPES = new Set(["text", "search", "url", "tel", "email", "password", "number"]);
  const EDITABLE_SEL =
    "input, textarea, [contenteditable=''], [contenteditable='true'], [contenteditable='plaintext-only']";

  /** 命中可编辑元素才返回（input 仅限文本类，排除 checkbox/button 等）。 */
  function editableOf(t: EventTarget | null): HTMLElement | null {
    const el = t as HTMLElement | null;
    if (!el || typeof el.closest !== "function") return null;
    const found = el.closest(EDITABLE_SEL) as HTMLElement | null;
    if (!found) return null;
    if (found instanceof HTMLInputElement && !TEXT_INPUT_TYPES.has(found.type)) return null;
    return found;
  }

  function isTextField(el: HTMLElement): boolean {
    return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
  }

  function hasSelection(el: HTMLElement): boolean {
    if (isTextField(el)) {
      const i = el as HTMLInputElement | HTMLTextAreaElement;
      return i.selectionStart != null && i.selectionEnd != null && i.selectionEnd > i.selectionStart;
    }
    const sel = window.getSelection();
    return !!sel && sel.toString().length > 0;
  }

  function focusEl(el: HTMLElement) {
    try {
      el.focus({ preventScroll: true });
    } catch {
      try {
        el.focus();
      } catch {
        /* ignore */
      }
    }
  }

  function copySel(el: HTMLElement) {
    focusEl(el);
    try {
      document.execCommand("copy");
    } catch {
      /* ignore */
    }
  }

  function cutSel(el: HTMLElement) {
    focusEl(el);
    try {
      document.execCommand("cut");
    } catch {
      /* ignore */
    }
  }

  async function pasteSel(el: HTMLElement) {
    focusEl(el);
    try {
      if (document.execCommand("paste")) return;
    } catch {
      /* 多数浏览器禁用 execCommand("paste")，走 Clipboard API */
    }
    try {
      const text = await navigator.clipboard.readText();
      if (isTextField(el)) {
        const i = el as HTMLInputElement | HTMLTextAreaElement;
        const s = i.selectionStart ?? i.value.length;
        const e = i.selectionEnd ?? i.value.length;
        i.setRangeText(text, s, e, "end");
        i.dispatchEvent(new Event("input", { bubbles: true }));
      } else {
        document.execCommand("insertText", false, text);
      }
    } catch {
      /* 无剪贴板权限时忽略 */
    }
  }

  function selectAllEl(el: HTMLElement) {
    focusEl(el);
    if (isTextField(el)) {
      (el as HTMLInputElement | HTMLTextAreaElement).select();
    } else {
      try {
        document.execCommand("selectAll");
      } catch {
        /* ignore */
      }
    }
  }

  function buildItems(el: HTMLElement): CtxItem[] {
    const t = el as HTMLInputElement;
    const ro = isTextField(el) && (t.readOnly || false);
    const dis = isTextField(el) && (t.disabled || false);
    const canSel = hasSelection(el);
    return [
      { label: tr("common.action.cut"), disabled: !canSel || ro || dis, action: () => cutSel(el) },
      { label: tr("common.action.copy"), disabled: !canSel, action: () => copySel(el) },
      { label: tr("common.action.paste"), disabled: ro || dis, action: () => void pasteSel(el) },
      { separator: true },
      { label: tr("common.action.selectAll"), action: () => selectAllEl(el) },
    ];
  }

  onMount(() => {
    const onErr = (e: ErrorEvent) => {
      toast(tr("common.error.script", { msg: e.message }), "error");
      console.error(e.error);
    };
    const onRej = (e: PromiseRejectionEvent) => {
      toast(tr("common.error.unhandled", { reason: String(e.reason) }), "error");
      console.error(e.reason);
    };
    // 捕获阶段统一处理右键：可编辑元素弹自绘菜单；其它区域仅阻止默认菜单，
    // 不 stopPropagation，保留 Library/Webdav/Reader 各自的右键处理。
    const onCtx = (e: MouseEvent) => {
      const el = editableOf(e.target);
      if (el) {
        e.preventDefault();
        e.stopPropagation();
        focusEl(el);
        ctx = { x: e.clientX, y: e.clientY, target: el };
        return;
      }
      e.preventDefault();
    };
    window.addEventListener("error", onErr);
    window.addEventListener("unhandledrejection", onRej);
    document.addEventListener("contextmenu", onCtx, true);
    return () => {
      window.removeEventListener("error", onErr);
      window.removeEventListener("unhandledrejection", onRej);
      document.removeEventListener("contextmenu", onCtx, true);
    };
  });

  const ctxItems = $derived(ctx ? buildItems(ctx.target) : []);
</script>

<svelte:head>
  <title>TidePage</title>
</svelte:head>

{@render children()}

{#if ctx}
  <ContextMenu x={ctx.x} y={ctx.y} items={ctxItems} onclose={() => (ctx = null)} />
{/if}
