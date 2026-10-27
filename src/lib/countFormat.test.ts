import { describe, it, expect } from "vitest";
import { formatCountShort, fmtDurationParts } from "./countFormat";

describe("formatCountShort（H22）", () => {
  it("中文：万单位与边界", () => {
    expect(formatCountShort(9999, "万", "k", "zh")).toBe("9999");
    expect(formatCountShort(10000, "万", "k", "zh")).toBe("1.0万");
    expect(formatCountShort(12300, "万", "k", "zh")).toBe("1.2万");
    expect(formatCountShort(12345, "万", "k", "zh")).toBe("1.2万");
  });
  it("英文：k 单位与边界", () => {
    expect(formatCountShort(999, "万", "k", "en")).toBe("999");
    expect(formatCountShort(1000, "万", "k", "en")).toBe("1.0k");
    expect(formatCountShort(12345, "万", "k", "en")).toBe("12.3k");
  });
  it("非法/负数归零", () => {
    expect(formatCountShort(-5, "万", "k", "zh")).toBe("0");
    expect(formatCountShort(Number.NaN, "万", "k", "en")).toBe("0");
  });
});

describe("fmtDurationParts（E14）", () => {
  it("按整秒拆 h/m/s，不出现 60m/60s", () => {
    expect(fmtDurationParts(59)).toEqual({ h: 0, m: 0, s: 59 });
    expect(fmtDurationParts(60)).toEqual({ h: 0, m: 1, s: 0 });
    expect(fmtDurationParts(3599)).toEqual({ h: 0, m: 59, s: 59 });
    expect(fmtDurationParts(3600)).toEqual({ h: 1, m: 0, s: 0 });
    expect(fmtDurationParts(3659)).toEqual({ h: 1, m: 0, s: 59 });
    expect(fmtDurationParts(3599.6)).toEqual({ h: 1, m: 0, s: 0 });
  });
  it("负数归零", () => {
    expect(fmtDurationParts(-10)).toEqual({ h: 0, m: 0, s: 0 });
  });
});
