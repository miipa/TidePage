import { describe, it, expect } from "vitest";
import { readFixOptionsKey, type ReadFixOptions } from "./readFix";

const base: ReadFixOptions = {
  fontFamily: "serif",
  monoFont: "monospace",
  fontSize: 32,
  lineHeight: 1.5,
  color: "#1a1a1a",
  background: "transparent",
};

describe("readFixOptionsKey", () => {
  it("同值同 key，字段变化 key 变化", () => {
    expect(readFixOptionsKey({ ...base })).toBe(readFixOptionsKey({ ...base }));
    expect(readFixOptionsKey({ ...base, fontFamily: "Arial" })).not.toBe(readFixOptionsKey(base));
    expect(readFixOptionsKey({ ...base, fontSize: 40 })).not.toBe(readFixOptionsKey(base));
    expect(readFixOptionsKey({ ...base, lineHeight: 1.8 })).not.toBe(readFixOptionsKey(base));
    expect(readFixOptionsKey({ ...base, color: "#000" })).not.toBe(readFixOptionsKey(base));
    expect(readFixOptionsKey({ ...base, background: "transparent" })).toBe(readFixOptionsKey(base));
  });

  it("非法值回退后与合法等价 key 相同", () => {
    const invalid: ReadFixOptions = {
      ...base,
      fontSize: 0,
      lineHeight: 0,
      color: "not-a-color",
      fontFamily: "bad;font",
      monoFont: "url(evil)",
    };
    const safeDefault: ReadFixOptions = {
      ...base,
      fontFamily: "serif",
      monoFont: "Consolas, \"Courier New\", monospace",
      fontSize: 32,
      lineHeight: 1.5,
      color: "#1a1a1a",
    };
    expect(readFixOptionsKey(invalid)).toBe(readFixOptionsKey(safeDefault));
  });
});
