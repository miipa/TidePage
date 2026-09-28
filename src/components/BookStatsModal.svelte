<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { dragOutHandle } from "../lib/windowSnap";
  import type { BookSession } from "../lib/types";

  let { bookId, bookTitle, onclose, onpopout }: {
    bookId: number;
    bookTitle: string;
    onclose: () => void;
    onpopout?: () => void;
  } = $props();

  let sessions: BookSession[] = $state([]);
  let loaded = $state(false);
  let wc = $state<{ total: number; pure: number } | null>(null);

  onMount(async () => {
    try {
      sessions = await api.getBookSessions(bookId);
    } catch { /* ignore */ }
    try {
      wc = await api.bookWordCount(bookId);
    } catch { /* ignore */ }
    loaded = true;
  });

  interface DayGroup {
    day: string;
    seconds: number;
    chars: number;
    items: BookSession[];
  }

  const groups = $derived.by(() => {
    const map = new Map<string, DayGroup>();
    for (const s of sessions) {
      let g = map.get(s.day);
      if (!g) {
        g = { day: s.day, seconds: 0, chars: 0, items: [] };
        map.set(s.day, g);
      }
      g.seconds += s.seconds;
      g.chars += s.chars;
      g.items.push(s);
    }
    return [...map.values()];
  });

  const totalSeconds = $derived(groups.reduce((a, g) => a + g.seconds, 0));
  const totalChars = $derived(groups.reduce((a, g) => a + g.chars, 0));
  const speed = $derived(totalSeconds >= 60 ? Math.round(totalChars / (totalSeconds / 60)) : 0);

  function fmt(sec: number): string {
    const h = Math.floor(sec / 3600);
    const m = Math.round((sec % 3600) / 60);
    const s = sec % 60;
    if (h) return `${h}小时${m}分`;
    if (m && s) return `${m}分${s}秒`;
    if (m) return `${m}分`;
    return `${s}秒`;
  }

  function fmtTime(ts: number): string {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }

  function fmtChars(n: number): string {
    if (n >= 10000) return (n / 10000).toFixed(1) + "万";
    return String(n);
  }

  function sessionSpeed(s: BookSession): string {
    if (s.seconds < 30 || s.chars <= 0) return "—";
    return Math.round(s.chars / (s.seconds / 60)) + " 字/分";
  }
</script>

<div class="sidepanel">
  <div class="sp-head" use:dragOutHandle={onpopout} title="按住标题栏拖出面板外可弹为独立窗口">
    <span>本书统计</span>
    <div style="display:flex; gap:6px; align-items:center;">
      {#if onpopout}<button class="icon" title="弹出为独立窗口" onclick={onpopout}>⧉</button>{/if}
      <button class="icon" title="关闭" onclick={onclose}>×</button>
    </div>
  </div>
  <div class="stats-body">
    {#if !loaded}
      <div class="empty">加载中…</div>
    {:else}
      <div class="book-title">{bookTitle}</div>
      {#if !wc}
        <div class="empty">字数统计生成中…稍后再看。</div>
      {:else}
      <div class="summary">
        <div class="sum-item">
          <div class="v">{fmtChars(wc.total)}</div>
          <div class="k">全书字数</div>
        </div>
        <div class="sum-item">
          <div class="v">{fmtChars(wc.pure)}</div>
          <div class="k">纯文字</div>
        </div>
        <div class="sum-item">
          <div class="v">{fmtChars(totalChars)}</div>
          <div class="k">已读字数</div>
        </div>
        <div class="sum-item">
          <div class="v">{wc.total > 0 ? Math.min(100, Math.round((totalChars / wc.total) * 100)) + "%" : "—"}</div>
          <div class="k">按字数进度</div>
        </div>
        <div class="sum-item">
          <div class="v">{fmt(totalSeconds)}</div>
          <div class="k">总时长</div>
        </div>
        <div class="sum-item">
          <div class="v">{speed || "—"}</div>
          <div class="k">平均 字/分</div>
        </div>
      </div>

      {#if !sessions.length}
        <div class="empty">暂无阅读记录，阅读约 30 秒后开始统计。</div>
      {:else}
      {#each groups as g (g.day)}
        <div class="day-block">
          <div class="day-head">
            <span class="day">{g.day}</span>
            <span class="day-sum">{fmt(g.seconds)} · {fmtChars(g.chars)}字 · {g.items.length} 次</span>
          </div>
          {#each g.items as s (s.started_at)}
            <div class="s-row">
              <span class="s-time">{fmtTime(s.started_at)}</span>
              <span class="s-len">{fmt(s.seconds)}</span>
              <span class="s-chars">{fmtChars(s.chars)}字</span>
              <span class="s-speed">{sessionSpeed(s)}</span>
            </div>
          {/each}
        </div>
      {/each}
      {/if}
      {/if}
    {/if}
  </div>
</div>

<style>
  .sidepanel {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 340px;
    background: color-mix(in srgb, var(--rdr-bg) 94%, black 6%);
    border-left: 1px solid rgba(128, 128, 128, 0.2);
    z-index: 40;
    display: flex;
    flex-direction: column;
    box-shadow: -6px 0 18px rgba(0, 0, 0, 0.08), -18px 0 48px rgba(0, 0, 0, 0.14);
    animation: slide-in 0.34s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes slide-in {
    from { transform: translateX(24px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .sp-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    font-weight: 600;
    border-bottom: 1px solid rgba(128, 128, 128, 0.15);
  }

  .stats-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
  }

  .book-title {
    font-size: 13px;
    font-weight: 600;
    opacity: 0.8;
    margin-bottom: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .summary {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }

  .sum-item {
    flex: 1;
    background: rgba(128, 128, 128, 0.1);
    border-radius: 10px;
    padding: 10px 6px;
    text-align: center;
  }

  .sum-item .v {
    font-size: 16px;
    font-weight: 700;
    color: var(--accent);
  }

  .sum-item .k {
    font-size: 12px;
    opacity: 0.65;
    margin-top: 3px;
  }

  .day-block {
    margin-bottom: 14px;
  }

  .day-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 4px 0;
    border-bottom: 1px dashed rgba(128, 128, 128, 0.25);
  }

  .day {
    font-size: 13px;
    font-weight: 600;
  }

  .day-sum {
    font-size: 12px;
    opacity: 0.6;
  }

  .s-row {
    display: flex;
    gap: 10px;
    padding: 5px 2px;
    font-size: 13px;
  }

  .s-time { width: 44px; opacity: 0.7; }
  .s-len { width: 56px; }
  .s-chars { flex: 1; }
  .s-speed { opacity: 0.7; }

  .empty {
    padding: 30px 10px;
    text-align: center;
    opacity: 0.55;
    line-height: 1.8;
  }
</style>
