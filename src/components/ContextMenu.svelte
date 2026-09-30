<script module lang="ts">
  import type { Component } from "svelte";

  export type CtxItem = {
    label?: string;
    icon?: Component;
    danger?: boolean;
    disabled?: boolean;
    separator?: boolean;
    action?: () => void;
  };
</script>

<script lang="ts">
  import { portal } from "../lib/portal";

  let {
    x,
    y,
    items,
    onclose,
  }: { x: number; y: number; items: CtxItem[]; onclose: () => void } = $props();

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ left: x, top: y });

  // 渲染后按菜单实际尺寸夹取到视口内（贴近右下角时不被裁切）。
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    const maxX = window.innerWidth - r.width - 8;
    const maxY = window.innerHeight - r.height - 8;
    pos = {
      left: Math.max(8, Math.min(x, maxX)),
      top: Math.max(8, Math.min(y, maxY)),
    };
  });

  function run(it: CtxItem) {
    if (it.disabled || it.separator || !it.action) return;
    it.action();
    onclose();
  }
</script>

<div
  class="ctx-backdrop"
  onclick={onclose}
  oncontextmenu={(e) => {
    e.preventDefault();
    onclose();
  }}
></div>
<div class="ctx" use:portal bind:this={el} style="left:{pos.left}px; top:{pos.top}px;">
  {#each items as it, i (i)}
    {#if it.separator}
      <div class="ctx-sep"></div>
    {:else}
      <button class:danger={it.danger} disabled={it.disabled} onclick={() => run(it)}>
        {#if it.icon}
          {@const Icon = it.icon}
          <span class="ci"><Icon size={17} stroke={1.75} /></span>
        {/if}
        {it.label}
      </button>
    {/if}
  {/each}
</div>

<style>
  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 2999;
  }

  .ctx {
    position: fixed;
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius, 12px);
    box-shadow: var(--shadow-lg), var(--edge-hi);
    padding: 6px;
    min-width: 168px;
    z-index: 3000;
    transform-origin: top left;
    animation: ctx-in 0.2s var(--ease-premium);
  }

  @keyframes ctx-in {
    from {
      opacity: 0;
      transform: translateY(-6px) scale(0.96);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }

  .ctx button {
    border: none;
    background: transparent;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px 7px 10px;
    border-radius: var(--radius-sm, 8px);
    font-size: var(--fs-sm, 13px);
    color: var(--fg);
    transition: background var(--dur, 0.2s) var(--ease), color var(--dur, 0.2s) var(--ease);
  }

  .ctx .ci {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    opacity: 0.85;
  }

  .ctx-sep {
    height: 1px;
    margin: 5px 8px;
    background: var(--line);
  }

  .ctx button:hover:not(:disabled) {
    background: var(--accent-soft, var(--hover-bg));
  }

  .ctx button:active {
    transform: none;
  }

  .ctx button:disabled {
    opacity: 0.42;
    cursor: default;
  }

  .ctx button.danger {
    color: var(--danger);
  }

  .ctx button.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    color: var(--danger);
  }
</style>
