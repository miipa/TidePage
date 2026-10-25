/** D2：阅读统计纯函数（周期/聚合/连续/格式化/占比），不依赖 DOM。 */
export type StatsPeriod = "week" | "month" | "year" | "all";
export interface DailyPoint { day: string; seconds: number; chars?: number }
export interface StatsRange { start: string; end: string; prevStart: string; prevEnd: string; heatStart: string; label: string }

const DAY = 86_400_000;
function localDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}
function atMidnight(d: Date): Date { return new Date(d.getFullYear(), d.getMonth(), d.getDate()); }
function addDays(d: Date, n: number): Date { return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n); }

/** 周期区间（本地日历）：周=本周一至今天；月=本月；年=本年；all=极早至今天。prev 为上一周期同段。 */
export function periodRange(period: StatsPeriod, now = new Date()): StatsRange {
  const today = atMidnight(now);
  let start: Date;
  let prevStart: Date;
  let prevEnd: Date;
  if (period === "week") {
    const dow = (today.getDay() + 6) % 7;
    start = addDays(today, -dow);
    prevStart = addDays(start, -7);
    prevEnd = addDays(start, -1);
  } else if (period === "month") {
    start = new Date(today.getFullYear(), today.getMonth(), 1);
    prevStart = new Date(today.getFullYear(), today.getMonth() - 1, 1);
    prevEnd = addDays(start, -1);
  } else if (period === "year") {
    start = new Date(today.getFullYear(), 0, 1);
    prevStart = new Date(today.getFullYear() - 1, 0, 1);
    prevEnd = addDays(start, -1);
  } else {
    start = new Date(1970, 0, 1);
    prevStart = start;
    prevEnd = start;
  }
  return {
    start: localDate(start),
    end: localDate(today),
    prevStart: localDate(prevStart),
    prevEnd: localDate(prevEnd),
    heatStart: localDate(addDays(today, -83)),
    label: `${localDate(start)} – ${localDate(today)}`,
  };
}

/** 区间内逐日补零（最多 371 天，all 周期由 bucketDaily 聚合，不再逐日）。 */
export function fillDaily(start: string, end: string, daily: DailyPoint[], maxDays = 371): DailyPoint[] {
  const map = new Map(daily.map((d) => [d.day, d]));
  const s = new Date(start + "T00:00:00");
  const e = new Date(end + "T00:00:00");
  const out: DailyPoint[] = [];
  for (let d = s, n = 0; d <= e && n < maxDays; d = addDays(d, 1), n++) {
    const key = localDate(d);
    out.push(map.get(key) ?? { day: key, seconds: 0, chars: 0 });
  }
  return out;
}

/** 年周期用：按 start/end 补满整年月份（无数据月为 0），label=YYYY-MM。 */
export function fillMonthly(
  start: string,
  end: string,
  daily: DailyPoint[],
): { label: string; seconds: number }[] {
  const map = new Map<string, number>();
  for (const d of daily) {
    const k = d.day.slice(0, 7);
    map.set(k, (map.get(k) ?? 0) + d.seconds);
  }
  const out: { label: string; seconds: number }[] = [];
  const s = new Date(start.slice(0, 4) + "-" + start.slice(5, 7) + "-01T00:00:00");
  const e = new Date(end.slice(0, 4) + "-" + end.slice(5, 7) + "-01T00:00:00");
  for (let y = s.getFullYear(), m = s.getMonth(); y < e.getFullYear() || (y === e.getFullYear() && m <= e.getMonth()); m++) {
    if (m > 11) { m = 0; y++; }
    const label = `${y}-${String(m + 1).padStart(2, "0")}`;
    out.push({ label, seconds: map.get(label) ?? 0 });
  }
  return out;
}

/** 年周期按月份聚合；all 周期按年份聚合（最近 12 年 + 更早合并）。 */
export function bucketMonthly(daily: DailyPoint[]): { label: string; seconds: number }[] {
  const out: { label: string; seconds: number }[] = [];
  for (const d of daily) {
    const label = d.day.slice(0, 7);
    const last = out[out.length - 1];
    if (last && last.label === label) last.seconds += d.seconds;
    else out.push({ label, seconds: d.seconds });
  }
  return out;
}
export function bucketYearly(daily: DailyPoint[]): { label: string; seconds: number }[] {
  const map = new Map<string, number>();
  for (const d of daily) {
    const y = d.day.slice(0, 4);
    map.set(y, (map.get(y) ?? 0) + d.seconds);
  }
  const years = [...map.keys()].sort();
  const recent = years.slice(-12);
  const earlier = years.slice(0, Math.max(0, years.length - 12));
  const out = recent.map((y) => ({ label: y, seconds: map.get(y) ?? 0 }));
  if (earlier.length) out.unshift({ label: "更早", seconds: earlier.reduce((a, y) => a + (map.get(y) ?? 0), 0) });
  return out;
}

/** 近 12 周热力网格：返回 84 个槽（按周一起始，最后一天为今天）。 */
export function heatmapGrid(heat: DailyPoint[], now = new Date()): { day: string; seconds: number }[] {
  const map = new Map(heat.map((d) => [d.day, d.seconds]));
  const today = atMidnight(now);
  const dow = (today.getDay() + 6) % 7;
  const firstMonday = addDays(today, -dow - 77);
  const out: { day: string; seconds: number }[] = [];
  for (let j = 0; j < 84; j++) {
    const d = addDays(firstMonday, j);
    const key = localDate(d);
    out.push({ day: key, seconds: d > today ? 0 : (map.get(key) ?? 0) });
  }
  return out;
}

/** 24 小时桶（缺省补零）+ 深夜/清晨/午后/傍晚占比。 */
export function hourBuckets(hourly: { hour: number; seconds: number }[]): {
  hours: number[];
  groups: { key: "lateNight" | "morning" | "afternoon" | "evening"; pct: number }[];
} {
  const hours = Array.from({ length: 24 }, () => 0);
  for (const h of hourly) if (h.hour >= 0 && h.hour < 24) hours[h.hour] += h.seconds;
  const sum = hours.reduce((a, b) => a + b, 0);
  const part = (from: number, to: number) => hours.slice(from, to).reduce((a, b) => a + b, 0);
  const pct = (v: number) => (sum > 0 ? Math.round((v / sum) * 100) : 0);
  return {
    hours,
    groups: [
      { key: "lateNight", pct: pct(part(0, 6) + part(22, 24)) },
      { key: "morning", pct: pct(part(6, 12)) },
      { key: "afternoon", pct: pct(part(12, 18)) },
      { key: "evening", pct: pct(part(18, 22)) },
    ],
  };
}

/** 当前/最长连续阅读天数（activeDates 为升序 YYYY-MM-DD）。 */
export function computeStreak(activeDates: string[], today = new Date()): { current: number; longest: number } {
  const set = new Set(activeDates);
  let longest = 0;
  let run = 0;
  let prev: Date | null = null;
  for (const s of [...set].sort()) {
    const d = new Date(s + "T00:00:00");
    run = prev && (d.getTime() - prev.getTime()) === DAY ? run + 1 : 1;
    longest = Math.max(longest, run);
    prev = d;
  }
  let current = 0;
  for (let d = atMidnight(today), i = 0; i < 4000; i++, d = addDays(d, -1)) {
    if (!set.has(localDate(d))) break;
    current++;
  }
  return { current, longest };
}

export function formatDuration(sec: number, locale: "zh" | "en" = "zh"): string {
  const s = Math.max(0, Math.round(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return locale === "en" ? `${h}h ${m}m` : `${h} 小时 ${m} 分`;
  return locale === "en" ? `${m} min` : `${m} 分钟`;
}

/** 每日柱图 Y 轴刻度：0 / 中间 / 峰值（峰值与中间重合时去重）。 */
export function chartTicks(maxSeconds: number): { value: number; pct: number }[] {
  const max = Math.max(1, maxSeconds);
  const values = [0, Math.round(max / 2), max].filter((v, i, a) => a.indexOf(v) === i);
  return values.map((value) => ({ value, pct: (value / max) * 100 }));
}

/** 较上期百分比：上期为 0 时返回 null（UI 不显示百分比）。 */
export function percentChange(cur: number, prev: number): number | null {
  if (prev <= 0) return null;
  return Math.round(((cur - prev) / prev) * 100);
}

/** 构成占比（合计 100 向下取整余数给最大项）。 */
export function compositionPct(items: { seconds: number }[]): number[] {
  const total = items.reduce((a, b) => a + b.seconds, 0);
  if (total <= 0) return items.map(() => 0);
  const raw = items.map((i) => (i.seconds / total) * 100);
  const floor = raw.map(Math.floor);
  const rest = 100 - floor.reduce((a, b) => a + b, 0);
  if (rest > 0) {
    let max = 0;
    for (let i = 1; i < raw.length; i++) if (raw[i] > raw[max]) max = i;
    floor[max] += rest;
  }
  return floor;
}
