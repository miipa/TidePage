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
<div class="ctx-menu" use:portal bind:this={el} style="left:{pos.left}px; top:{pos.top}px;">
  {#each items as it, i (i)}
    {#if it.separator}
      <div class="ctx-sep"></div>
    {:else}
      <button class="ctx-item" class:danger={it.danger} disabled={it.disabled} onclick={() => run(it)}>
        {#if it.icon}
          {@const Icon = it.icon}
          <span class="ci"><Icon size={16} stroke={1.5} /></span>
        {/if}
        {it.label}
      </button>
    {/if}
  {/each}
</div>

<style>
  /* 菜单本体、菜单项、分隔线、图标框全部走 app.css 的全局 .ctx-menu/.ctx-item/.ctx-sep。
     本组件特有的是「portal 到 body」这一件事：
     菜单必须高于 --z-modal，否则右键设置弹窗内的输入框时菜单会被模态盖住。 */
  .ctx-menu {
    position: fixed;
    z-index: var(--z-portal);
  }

  .ctx-item:active {
    transform: none;
  }
</style>
