<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { ui } from "../lib/stores.svelte";

  let stats: [string, number][] = $state([]);

  onMount(async () => {
    try {
      stats = await api.getStats();
    } catch { /* ignore */ }
  });

  const maxVal = $derived(Math.max(1, ...stats.map((s) => s[1])));
  const total = $derived(stats.reduce((a, s) => a + s[1], 0));

  function fmt(sec: number): string {
    const h = Math.floor(sec / 3600);
    const m = Math.round((sec % 3600) / 60);
    return h ? `${h} 小时 ${m} 分` : `${m} 分钟`;
  }
</script>

<div class="modal-body">
  <h3>阅读统计（近 14 天）</h3>
  {#if !stats.length}
    <div class="empty">暂无数据，开始阅读后每 30 秒记录一次。</div>
  {:else}
    <div class="total">共阅读 {fmt(total)}</div>
    <div class="bars">
      {#each stats as [day, sec] (day)}
        <div class="bar-col" title="{day}: {fmt(sec)}">
          <div class="bar" style="height:{Math.max(3, (sec / maxVal) * 100)}%"></div>
          <div class="lbl">{day.slice(5)}</div>
        </div>
      {/each}
    </div>
  {/if}
  <div class="close-row">
    <button onclick={() => (ui.statsOpen = false)}>关闭</button>
  </div>
</div>

<style>
  .total {
    font-size: 14px;
    font-weight: 600;
    margin-bottom: 14px;
    color: var(--accent);
  }

  .bars {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    height: 160px;
    padding: 0 4px;
  }

  .bar-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    height: 100%;
    justify-content: flex-end;
  }

  .bar {
    width: 100%;
    max-width: 36px;
    background: linear-gradient(to top, var(--accent), color-mix(in srgb, var(--accent) 55%, white));
    border-radius: 4px 4px 0 0;
    min-height: 3px;
  }

  .lbl {
    font-size: 10px;
    color: var(--fg2);
  }

  .empty {
    padding: 40px;
    text-align: center;
    color: var(--fg2);
  }

  .close-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 14px;
  }
</style>
