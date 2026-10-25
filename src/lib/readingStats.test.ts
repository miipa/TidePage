import { describe, it, expect } from "vitest";
import {
  bucketMonthly,
  chartTicks,
  bucketYearly,
  computeStreak,
  compositionPct,
  fillDaily,
  fillMonthly,
  formatDuration,
  heatmapGrid,
  hourBuckets,
  percentChange,
  periodRange,
} from "./readingStats";

describe("readingStats", () => {
  it("周期区间：周/月/年/全部与上期", () => {
    const now = new Date(2026, 0, 7); // 周三
    const w = periodRange("week", now);
    expect(w.start).toBe("2026-01-05");
    expect(w.end).toBe("2026-01-07");
    expect(w.prevStart).toBe("2025-12-29");
    expect(w.prevEnd).toBe("2026-01-04");
    const m = periodRange("month", now);
    expect([m.start, m.prevStart, m.prevEnd]).toEqual(["2026-01-01", "2025-12-01", "2025-12-31"]);
    const y = periodRange("year", now);
    expect([y.start, y.prevStart]).toEqual(["2026-01-01", "2025-01-01"]);
    expect(periodRange("all", now).start).toBe("1970-01-01");
  });

  it("逐日补零与月/年聚合", () => {
    const filled = fillDaily("2026-01-01", "2026-01-03", [{ day: "2026-01-02", seconds: 60 }]);
    expect(filled.map((d) => d.seconds)).toEqual([0, 60, 0]);
    expect(bucketMonthly([{ day: "2026-01-01", seconds: 10 }, { day: "2026-01-02", seconds: 5 }, { day: "2026-02-01", seconds: 1 }]))
      .toEqual([{ label: "2026-01", seconds: 15 }, { label: "2026-02", seconds: 1 }]);
    const years = Array.from({ length: 14 }, (_, i) => ({ day: `${2000 + i}-01-01`, seconds: 1 }));
    const b = bucketYearly(years);
    expect(b[0]).toEqual({ label: "更早", seconds: 2 });
    expect(b[b.length - 1].label).toBe("2013");
  });

  it("年月份补零（跨年/仅一个月）", () => {
    expect(fillMonthly("2026-01-01", "2026-12-31", [{ day: "2026-03-05", seconds: 30 }]))
      .toEqual(Array.from({ length: 12 }, (_, i) => ({ label: `2026-${String(i + 1).padStart(2, "0")}`, seconds: i === 2 ? 30 : 0 })));
    const cross = fillMonthly("2025-11-01", "2026-02-28", [{ day: "2026-01-02", seconds: 5 }]);
    expect(cross.map((m) => m.label)).toEqual(["2025-11", "2025-12", "2026-01", "2026-02"]);
    expect(cross.find((m) => m.label === "2026-01")?.seconds).toBe(5);
    expect(fillMonthly("2026-05-01", "2026-05-31", [])).toEqual([{ label: "2026-05", seconds: 0 }]);
  });

  it("热力网格 84 格、小时桶与占比", () => {
    const grid = heatmapGrid([{ day: "2026-01-07", seconds: 60 }], new Date(2026, 0, 7));
    expect(grid).toHaveLength(84);
    expect(grid.find((g) => g.day === "2026-01-07")?.seconds).toBe(60);
    const hb = hourBuckets([{ hour: 2, seconds: 30 }, { hour: 9, seconds: 50 }, { hour: 20, seconds: 20 }]);
    expect(hb.hours[2]).toBe(30);
    expect(hb.groups.map((g) => g.pct).reduce((a, b) => a + b, 0)).toBe(100);
  });

  it("柱图刻度 0/中/峰值去重", () => {
    expect(chartTicks(1800)).toEqual([{ value: 0, pct: 0 }, { value: 900, pct: 50 }, { value: 1800, pct: 100 }]);
    expect(chartTicks(1).map((t) => t.value)).toEqual([0, 1]);
    expect(chartTicks(0).map((t) => t.value)).toEqual([0, 1]);
  });

  it("连续天数、时长与百分比", () => {
    expect(computeStreak(["2026-01-05", "2026-01-06", "2026-01-07", "2026-01-01"], new Date(2026, 0, 7)))
      .toEqual({ current: 3, longest: 3 });
    expect(computeStreak([], new Date(2026, 0, 7))).toEqual({ current: 0, longest: 0 });
    expect(formatDuration(3600 + 120, "zh")).toBe("1 小时 2 分");
    expect(formatDuration(120, "en")).toBe("2 min");
    expect(percentChange(120, 100)).toBe(20);
    expect(percentChange(10, 0)).toBeNull();
  });

  it("构成占比合计 100", () => {
    expect(compositionPct([{ seconds: 1 }, { seconds: 1 }, { seconds: 1 }])).toEqual([34, 33, 33]);
    expect(compositionPct([{ seconds: 0 }])).toEqual([0]);
  });
});
