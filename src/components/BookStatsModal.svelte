<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
import { IconArrowsSplit2, IconX } from "@tabler/icons-svelte";
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { dragOutHandle } from "../lib/windowSnap";
  import type { BookSession } from "../lib/types";

  let { bookId, bookTitle, onclose, onpopout, version = 0 }: {
    bookId: number;
    bookTitle: string;
    onclose?: () => void;
    onpopout?: () => void;
    version?: number;
  } = $props();

  let sessions: BookSession[] = $state([]);
  let loaded = $state(false);
  let wc = $state<{ total: number; pure: number } | null>(null);

  async function load() {
    try {
      sessions = await api.getBookSessions(bookId);
    } catch { /* ignore */ }
    try {
      wc = await api.bookWordCount(bookId);
    } catch { /* ignore */ }
    loaded = true;
  }

  onMount(load);

  // 独立“统计”窗口：主窗口 flushSession 后 version++ 触发重载（内嵌调用不传 version，仅首挂载加载）。
  $effect(() => {
    if (version > 0) void load();
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
    if (h) return t("common.dur.hmShort", { h, m });
    if (m && s) return t("common.dur.ms", { m, s });
    if (m) return t("common.dur.mShort", { m });
    return t("common.dur.sShort", { s });
  }

  function fmtTime(ts: number): string {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }

  function fmtChars(n: number): string {
    if (n >= 10000) return (n / 10000).toFixed(1) + t("common.unit.tenThousand");
    return String(n);
  }

  function sessionSpeed(s: BookSession): string {
    if (s.seconds < 30 || s.chars <= 0) return "—";
    return Math.round(s.chars / (s.seconds / 60)) + " " + t("common.unit.perMin");
  }
</script>

<div class="sidepanel">
  <div class="sp-head" data-testid="bsm-head" use:dragOutHandle={onpopout} title={t("common.panel.popoutTip")}>
    <span>{t("bookStats.title")}</span>
    <div style="display:flex; gap:6px; align-items:center;">
      {#if onpopout}<button class="icon" data-testid="bsm-popout" title={t("common.panel.popout")} onclick={onpopout}><IconArrowsSplit2 /></button>{/if}
      {#if onclose}<button class="icon" data-testid="bsm-close" title={t("common.action.close")} onclick={onclose}><IconX /></button>{/if}
    </div>
  </div>
  <div class="stats-body">
    {#if !loaded}
      <div class="empty-state">{t("common.loading")}</div>
    {:else}
      <div class="book-title">{bookTitle}</div>
      {#if !wc}
        <div class="empty-state">{t("bookStats.calculating")}</div>
      {:else}
      <div class="summary">
        <div class="sum-item">
          <div class="v">{fmtChars(wc.total)}</div>
          <div class="k">{t("bookStats.totalChars")}</div>
        </div>
        <div class="sum-item">
          <div class="v">{fmtChars(wc.pure)}</div>
          <div class="k">{t("bookStats.pureChars")}</div>
        </div>
        <div class="sum-item">
          <div class="v">{fmtChars(totalChars)}</div>
          <div class="k">{t("bookStats.readChars")}</div>
        </div>
        <div class="sum-item">
          <div class="v">{wc.total > 0 ? Math.min(100, Math.round((totalChars / wc.total) * 100)) + "%" : "—"}</div>
          <div class="k">{t("bookStats.byChars")}</div>
        </div>
        <div class="sum-item">
          <div class="v">{fmt(totalSeconds)}</div>
          <div class="k">{t("bookStats.totalTime")}</div>
        </div>
        <div class="sum-item">
          <div class="v">{speed || "—"}</div>
          <div class="k">{t("bookStats.avgSpeed", { v: speed })}</div>
        </div>
      </div>

      {#if !sessions.length}
        <div class="empty-state">{t("bookStats.empty")}</div>
      {:else}
      {#each groups as g (g.day)}
        <div class="day-block">
          <div class="day-head">
            <span class="day">{g.day}</span>
            <span class="day-sum">{fmt(g.seconds)} · {fmtChars(g.chars)}{t("common.unit.chars")}{t("bookStats.unitLine", { n: g.items.length })}</span>
          </div>
          {#each g.items as s (s.started_at)}
            <div class="s-row">
              <span class="s-time">{fmtTime(s.started_at)}</span>
              <span class="s-len">{fmt(s.seconds)}</span>
              <span class="s-chars">{fmtChars(s.chars)}{t("common.unit.chars")}</span>
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
  /* .sidepanel / .sp-head 走 app.css 的全局基类。 */
  .stats-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--sp-3) var(--sp-4);
  }

  .book-title {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    font-weight: 600;
    opacity: 0.8;
    margin-bottom: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .summary {
    display: flex;
    gap: var(--sp-2);
    margin-bottom: 14px;
  }

  .sum-item {
    flex: 1;
    background: color-mix(in srgb, var(--rdr-fg) 7%, transparent);
    border-radius: var(--radius-md);
    padding: 10px var(--sp-1);
    text-align: center;
  }

  /* 统计数值：等宽对齐，避免刷新时数字左右跳动 */
  .sum-item .v {
    font-size: var(--fs-lg);
    font-weight: 700;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .sum-item .k {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.65;
    margin-top: 3px;
  }

  .day-block {
    margin-bottom: 14px;
  }

  /* 虚线只用于「按日期分组的组标题」这一种用途 */
  .day-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: var(--sp-1) 0;
    border-bottom: 1px dashed var(--rdr-line);
  }

  .day {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .day-sum {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.6;
    font-variant-numeric: tabular-nums;
  }

  .s-row {
    display: flex;
    gap: 10px;
    padding: 5px 2px;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    font-variant-numeric: tabular-nums;
  }

  .s-time { width: 44px; opacity: 0.7; }
  .s-len { width: 56px; }
  .s-chars { flex: 1; }
  .s-speed { opacity: 0.7; }
</style>
