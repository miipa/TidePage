<script lang="ts">
  /** D2：阅读统计面板（真实数据；紧凑无滚动；周/月/年/总计切换）。 */
  import { onMount } from "svelte";
  import { api, coverUrl, type ReadingStats } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { config, ui } from "../lib/stores.svelte";
  import type { BookRow } from "../lib/types";
  import {
    bucketMonthly,
    bucketYearly,
    chartTicks,
    computeStreak,
    compositionPct,
    fillDaily,
    fillMonthly,
    formatDuration,
    heatmapGrid,
    hourBuckets,
    percentChange,
    periodRange,
    type StatsPeriod,
  } from "../lib/readingStats";
  import { IconX } from "@tabler/icons-svelte";

  let period = $state<StatsPeriod>("week");
  let data = $state<ReadingStats | null>(null);
  let loading = $state(false);
  let error = $state("");
  const locale = $derived(config.language === "en" ? "en" : "zh");

  async function load(p: StatsPeriod = period) {
    loading = true;
    error = "";
    try {
      const r = periodRange(p, new Date());
      data = await api.getReadingStats(r.start, r.end, r.prevStart, r.prevEnd, r.heatStart, r.end);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => void load());

  function setPeriod(p: StatsPeriod) {
    if (p === period) return;
    period = p;
    void load(p);
  }

  const duration = (s: number) => formatDuration(s, locale);
  const range = $derived(periodRange(period, new Date()));

  /** 图表点：周/月逐日（补零）、年按月、总按年。 */
  const chart = $derived.by(() => {
    const d = data?.daily ?? [];
    if (period === "week") return fillDaily(range.start, range.end, d).map((x) => ({ label: x.day.slice(5), seconds: x.seconds, day: x.day }));
    if (period === "month") return fillDaily(range.start, range.end, d).map((x) => ({ label: x.day.slice(8), seconds: x.seconds, day: x.day }));
    if (period === "year") return fillMonthly(range.start, range.end, d).map((x) => ({ label: x.label.slice(5), seconds: x.seconds, day: x.label }));
    return bucketYearly(d).map((x) => ({ label: x.label, seconds: x.seconds, day: x.label }));
  });
  const chartMax = $derived(Math.max(1, ...chart.map((c) => c.seconds)));
  const heat = $derived(heatmapGrid(data?.heatmap ?? [], new Date()));
  const heatMax = $derived(Math.max(1, ...heat.map((h) => h.seconds)));
  const hours = $derived(hourBuckets(data?.hourly ?? []));
  const hourMax = $derived(Math.max(1, ...hours.hours));
  const streak = $derived(computeStreak(data?.activeDates ?? [], new Date()));
  const change = $derived(percentChange(data?.totalSeconds ?? 0, data?.prevSeconds ?? 0));
  const goalPct = $derived(Math.min(100, Math.round(((data?.todaySeconds ?? 0) / 1800) * 100)));
  const comp = $derived(data?.composition ?? []);
  const compPct = $derived(compositionPct(comp));
  const empty = $derived(!!data && data.totalSeconds === 0 && data.sessionCount === 0);
  const peakHour = $derived(hours.hours.indexOf(Math.max(...hours.hours)));

  function fmtDay(day: string): string {
    if (!day) return "";
    return day.length >= 10 ? day.slice(5) : day;
  }

  function completedCover(b: ReadingStats["completedBooks"][number]): string {
    const row = {
      id: b.id, cover_path: b.coverPath, cover_ver: b.coverVer, title: b.title, author: b.author,
      path: "", format: "", size: 0, mtime: 0, added_at: 0, meta_done: 1, last_read: b.lastRead,
      total_chars: 0, total_text_chars: 0, read_pct: 100, read_state: 1, last_opened: 0,
    } as unknown as BookRow;
    return coverUrl(row);
  }
</script>

<div class="stats-panel" data-testid="stats-panel">
  <header class="sp-head">
    <div class="sp-title">
      <h2>{t("stats.trail.title")}</h2>
      <p>{t("stats.trail.subtitle")}</p>
    </div>
    <div class="sp-tabs">
      {#each (["week", "month", "year", "all"] as StatsPeriod[]) as p (p)}
        <button class:active={period === p} onclick={() => setPeriod(p)}>{t(("stats.period." + p) as any)}</button>
      {/each}
    </div>
    <button class="icon sp-close" title={t("common.action.close")} onclick={() => (ui.statsOpen = false)}><IconX size={16} stroke={1.5} /></button>
  </header>

  {#if error}
    <div class="sp-empty">{t("stats.loadFailed")}</div>
  {:else if !data}
    <div class="sp-empty">{t("common.loading")}</div>
  {:else if empty}
    <div class="sp-empty">
      <b>{t("stats.emptyTitle")}</b>
      <span>{t("stats.emptyHint")}</span>
    </div>
  {:else}
    <div class="sp-strip">
      <span>{t("stats.strip.active", { n: data.activeDays })}</span>
      <span>{t("stats.strip.peak", { h: peakHour >= 0 ? String(peakHour).padStart(2, "0") : "—" })}</span>
      {#if change !== null}<span>{t("stats.strip.change", { pct: change > 0 ? "+" + change : String(change) })}</span>{/if}
      <span class="muted">{period === "all" ? t("stats.range.all") : range.label}</span>
    </div>

    <div class="sp-cards">
      <div class="card">
        <span class="k">{t("stats.card.total")}</span>
        <b>{duration(data.totalSeconds)}</b>
        <span class="s">{t("stats.card.dailyAvg", { v: duration(data.activeDays ? Math.round(data.totalSeconds / Math.max(1, data.activeDays)) : 0) })}</span>
      </div>
      <div class="card">
        <span class="k">{t("stats.card.chars")}</span>
        <b>{data.totalChars.toLocaleString()}</b>
        <span class="s">{data.readSpeedCpm > 0 ? t("stats.card.speed", { n: data.readSpeedCpm }) : t("stats.card.speedNone")}</span>
      </div>
      <div class="card">
        <span class="k">{t("stats.card.books")}</span>
        <b>{data.finishedBooks} / {data.finishedBooks + data.readingBooks}</b>
        <span class="s" title={data.currentBook?.title ?? ""}>{data.currentBook ? t("stats.card.current", { title: data.currentBook.title }) : t("stats.card.noCurrent")}</span>
      </div>
      <div class="card">
        <span class="k">{t("stats.card.streak")}</span>
        <b>{streak.current} / {streak.longest}</b>
        <span class="s">{t("stats.card.todayGoal", { v: duration(data.todaySeconds) })}</span>
        <div class="goal"><i style="width:{goalPct}%"></i><em>{goalPct}%</em></div>
      </div>
    </div>

    <div class="sp-mid">
      <section class="box daily">
        <div class="box-head"><span>{t("stats.daily.title")}</span>{#if period === "week" || period === "month"}<span class="muted">{t("stats.daily.goal", { v: duration(1800) })}</span>{:else}<span class="muted">{t("stats.daily.avg")}</span>{/if}</div>
        <div class="bar-wrap">
          <div class="yticks">
            {#each chartTicks(chartMax) as tk (tk.value)}
              <span style="bottom:{tk.pct}%">{tk.value >= 60 ? Math.round(tk.value / 60) + "m" : tk.value + "s"}</span>
            {/each}
          </div>
          <div class="bars">
            {#each chart as c, i (c.day + i)}
            <div class="bar-col" title="{fmtDay(c.day)} · {duration(c.seconds)}">
              <div class="bar" class:goal-hit={c.seconds >= 1800} style="height:{Math.max(2, (c.seconds / chartMax) * 100)}%"></div>
              {#if chart.length <= 12 || i % Math.ceil(chart.length / 12) === 0}<span class="lbl">{c.label}</span>{/if}
            </div>
          {/each}
          </div>
          {#if period === "week" || period === "month"}
            <div class="goal-line" style="bottom:{Math.min(96, (1800 / chartMax) * 100)}%"><span>30m</span></div>
          {/if}
        </div>
      </section>
      <section class="box heatbox">
        <div class="box-head"><span>{t("stats.heat.title")}</span><span class="muted">{t("stats.heat.hint")}</span></div>
        <div class="heat">
          {#each heat as h (h.day)}
            <span class="cell" class:on={h.seconds > 0} title="{fmtDay(h.day)} · {duration(h.seconds)}" style="opacity:{0.15 + (h.seconds / heatMax) * 0.85}"></span>
          {/each}
        </div>
        <div class="heat-foot"><span>{t("stats.heat.less")}</span><i></i><span>{t("stats.heat.more")}</span></div>
      </section>
    </div>

    <div class="sp-low">
      <section class="box clock">
        <div class="box-head"><span>{t("stats.clock.title")}</span><span class="muted">{t("stats.clock.longest", { v: duration(data.longestSessionSeconds) })}</span></div>
        <div class="clockbars">
          {#each hours.hours as v, h (h)}
            <span class="cbar" style="height:{Math.max(4, (v / hourMax) * 100)}%" title="{h}:00 · {duration(v)}"></span>
          {/each}
        </div>
        <div class="clocklabels"><span>0</span><span>6</span><span>12</span><span>18</span><span>23</span></div>
        <div class="clockgroups">
          {#each hours.groups as g (g.key)}
            <span>{t(("stats.clock." + g.key) as any)} <b>{g.pct}%</b></span>
          {/each}
        </div>
      </section>
      <section class="box comp">
        <div class="box-head"><span>{t("stats.comp.title")}</span><span class="muted">{t("stats.comp.hint")}</span></div>
        {#if comp.length}
          <div class="stack">
            {#each comp as c, i (c.label)}
              <span style="width:{compPct[i]}%" class:alt={i % 2 === 1} title="{c.label} · {duration(c.seconds)}"></span>
            {/each}
          </div>
          <div class="legend">
            {#each comp as c, i (c.label)}
              <span><i class="dot" class:alt={i % 2 === 1}></i>{c.label.toUpperCase()} · {compPct[i]}%</span>
            {/each}
          </div>
        {:else}<div class="muted">{t("stats.comp.none")}</div>{/if}
      </section>
    </div>

    <footer class="sp-foot">
      <section class="quote">
        <div class="box-head"><span>{t("stats.quote.title")}</span><span class="muted">{t("stats.quote.counts", { h: data.highlightsTotal, n: data.notesTotal })}</span></div>
        {#if data.quote}
          <blockquote>“{data.quote.text}”</blockquote>
          <span class="muted">— {data.quote.bookTitle}{data.quote.bookAuthor ? " · " + data.quote.bookAuthor : ""}</span>
          {#if data.quote.note}<div class="note">{data.quote.note}</div>{/if}
        {:else}<div class="muted">{t("stats.quote.none")}</div>{/if}
      </section>
      <section class="finished">
        <div class="box-head"><span>{t("stats.finished.title")}</span><span class="muted">{data.finishedBooks}</span></div>
        {#if data.completedBooks.length}
          <div class="fb">
            {#each data.completedBooks as b (b.id)}
              <div class="fbi" title="{b.title} · {duration(b.seconds)}">
                <img src={completedCover(b)} alt="" loading="lazy" />
                <span>{b.title}</span>
                <em>{duration(b.seconds)} · {t("stats.finished.hl", { n: b.highlightCount })}</em>
              </div>
            {/each}
          </div>
        {:else}<div class="muted">{t("stats.finished.none")}</div>{/if}
      </section>
    </footer>
  {/if}
  {#if loading}<div class="sp-loading">{t("common.loading")}</div>{/if}
</div>

<style>
  .stats-panel { position: relative; width: min(1160px, 94vw); height: min(700px, 88vh); display: grid; grid-template-rows: auto auto auto 1fr 1fr auto; gap: 10px; padding: 18px 20px; overflow: hidden; background: var(--bg-elev); color: var(--fg); font-variant-numeric: tabular-nums; }
  .sp-head { display: grid; grid-template-columns: 1fr auto auto; align-items: center; gap: 12px; }
  .sp-title h2 { margin: 0; font-size: var(--fs-lg); letter-spacing: var(--ls-md); }
  .sp-title p { margin: 2px 0 0; font-size: var(--fs-xs); color: var(--fg2); }
  .sp-tabs { display: flex; gap: 2px; background: var(--panel2); border: 1px solid var(--line); border-radius: 999px; padding: 2px; }
  .sp-tabs button { border: none; background: transparent; color: var(--fg2); font-size: var(--fs-xs); padding: 4px 10px; border-radius: 999px; cursor: pointer; }
  .sp-tabs button.active { background: var(--accent); color: var(--accent-fg); }
  .sp-close { width: 28px; height: 28px; display: grid; place-items: center; border: none; background: transparent; color: var(--fg2); border-radius: var(--radius-sm); cursor: pointer; }
  .sp-close:hover { color: var(--fg); background: var(--hover-bg); }
  .sp-strip { display: flex; gap: 14px; flex-wrap: wrap; font-size: var(--fs-xs); color: var(--fg2); }
  .muted { color: var(--fg2); font-size: var(--fs-xs); }
  .sp-cards { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; }
  .card { min-width: 0; border: 1px solid var(--line); border-radius: var(--radius-md); background: var(--panel); padding: 8px 10px; display: flex; flex-direction: column; gap: 2px; }
  .card .k { font-size: var(--fs-2xs); color: var(--fg2); }
  .card b { font-size: var(--fs-lg); letter-spacing: var(--ls-sm); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .card .s { font-size: var(--fs-2xs); color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .goal { height: 4px; background: var(--panel2); border-radius: 2px; position: relative; margin-top: 3px; }
  .goal i { display: block; height: 100%; background: var(--accent); border-radius: 2px; }
  .goal em { position: absolute; right: 0; top: -13px; font-size: 9px; color: var(--fg2); }
  .sp-mid, .sp-low { display: grid; grid-template-columns: minmax(0, 1.55fr) minmax(0, 1fr); gap: 10px; min-height: 0; }
  .sp-low { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
  .box { position: relative; min-width: 0; min-height: 0; border: 1px solid var(--line); border-radius: var(--radius-md); background: var(--panel); padding: 8px 10px; display: flex; flex-direction: column; }
  .bar-wrap { flex: 1; min-height: 0; display: flex; gap: 4px; }
  .yticks { position: relative; width: 30px; flex: none; font-size: 9px; color: var(--fg2); }
  .yticks span { position: absolute; right: 2px; transform: translateY(50%); white-space: nowrap; }
  .yticks span:nth-child(1) { transform: none; bottom: 0 !important; }
  .yticks span:last-child { transform: none; top: 0; bottom: auto !important; }
  .box-head { display: flex; justify-content: space-between; align-items: baseline; font-size: var(--fs-xs); margin-bottom: 4px; }
  .bars { flex: 1; min-height: 0; display: flex; align-items: flex-end; gap: 2px; position: relative; }
  .bar-col { flex: 1; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 2px; min-width: 0; }
  .bar { width: 100%; max-width: 18px; background: color-mix(in srgb, var(--fg2) 35%, transparent); border-radius: 2px 2px 0 0; }
  .bar.goal-hit { background: var(--accent); }
  .lbl { font-size: 9px; color: var(--fg2); white-space: nowrap; overflow: hidden; }
  .goal-line { position: absolute; left: 34px; right: 0; border-top: 1px dashed color-mix(in srgb, var(--accent) 55%, transparent); pointer-events: none; }
  .goal-line span { position: absolute; right: 0; top: -12px; font-size: 9px; color: var(--accent); }
  .heat { flex: 1; min-height: 0; display: grid; grid-template-rows: repeat(7, 1fr); grid-auto-flow: column; grid-auto-columns: 1fr; gap: 2px; }
  .cell { background: var(--accent); border-radius: 1px; min-height: 0; }
  .cell:not(.on) { background: var(--panel2); opacity: 0.6 !important; }
  .heat-foot { display: flex; align-items: center; gap: 4px; font-size: 9px; color: var(--fg2); margin-top: 3px; }
  .heat-foot i { width: 14px; height: 6px; background: var(--accent); opacity: 0.6; border-radius: 1px; }
  .clockbars { flex: 1; min-height: 0; display: flex; align-items: flex-end; gap: 2px; }
  .cbar { flex: 1 1 0; min-width: 3px; background: color-mix(in srgb, var(--accent) 65%, transparent); border-radius: 2px 2px 0 0; min-height: 2px; }
  .clocklabels { display: flex; justify-content: space-between; font-size: 9px; color: var(--fg2); }
  .clockgroups { display: flex; gap: 10px; flex-wrap: wrap; font-size: var(--fs-2xs); color: var(--fg2); margin-top: 3px; }
  .clockgroups b { color: var(--fg); }
  .stack { display: flex; gap: 2px; height: 16px; border-radius: 4px; overflow: hidden; background: var(--panel2); padding: 1px; }
  .stack span { display: block; height: 100%; background: var(--accent); border-radius: 2px; min-width: 2px; }
  .stack span.alt { background: color-mix(in srgb, var(--accent) 55%, var(--fg2)); }
  .legend { display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: var(--fs-2xs); color: var(--fg2); margin-top: 5px; }
  .dot { display: inline-block; width: 6px; height: 6px; border-radius: 50%; background: var(--accent); margin-right: 3px; }
  .dot.alt { background: color-mix(in srgb, var(--accent) 55%, var(--fg2)); }
  .sp-foot { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr); gap: 10px; min-height: 0; }
  .quote, .finished { min-width: 0; border: 1px solid var(--line); border-radius: var(--radius-md); background: var(--panel); padding: 8px 10px; overflow: hidden; }
  blockquote { margin: 4px 0 3px; font-family: var(--font-serif, serif); font-size: var(--fs-sm); line-height: 1.5; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .note { font-size: var(--fs-2xs); color: var(--fg2); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .fb { display: flex; gap: 8px; overflow: hidden; margin-top: 4px; }
  .fbi { flex: 0 0 64px; display: flex; flex-direction: column; gap: 2px; min-width: 0; font-size: 10px; }
  .fbi img { width: 64px; height: 48px; object-fit: cover; border-radius: 3px; background: var(--panel2); }
  .fbi span { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; line-height: 1.2; }
  .fbi em { color: var(--fg2); font-style: normal; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sp-empty { flex: 1; display: grid; place-items: center; gap: 6px; color: var(--fg2); font-size: var(--fs-sm); text-align: center; padding: 30px; }
  .sp-empty b { color: var(--fg); font-size: var(--fs-lg); }
  .sp-loading { position: absolute; right: 18px; top: 54px; font-size: var(--fs-2xs); color: var(--fg2); background: var(--panel); border: 1px solid var(--line); border-radius: 999px; padding: 2px 8px; }
  @media (max-width: 1100px), (max-height: 720px) {
    .stats-panel {
      width: min(940px, 96vw);
      height: min(540px, 94vh);
      padding: 8px 12px;
      gap: 4px;
      grid-template-rows: auto auto auto minmax(0, 1fr);
    }
    .sp-title h2 { font-size: var(--fs-md); }
    .sp-title p { display: none; }
    .card { padding: 6px 8px; }
    .card b { font-size: var(--fs-md); }
    .sp-low, .sp-foot { display: none; }
    .sp-cards { gap: 6px; }
    .sp-mid { grid-template-columns: minmax(0, 1.5fr) minmax(0, 1fr); min-height: 0; }
    .sp-strip { gap: 8px; }
    .box-head .muted { display: none; }
  }
  @media (max-width: 900px) { .stats-panel { height: min(760px, 94vh); } .sp-cards { grid-template-columns: 1fr 1fr; } }
</style>
