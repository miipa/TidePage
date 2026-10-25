import { describe, it, expect } from "vitest";
import { mergeBookOverride } from "./bookOverrideMerge";

type Ov = { fontSize?: number; ttsVoice?: string; ttsRate?: number };

describe("mergeBookOverride", () => {
  it("写入/覆盖 TTS 字段并保留其它字段", () => {
    const cur: Ov = { fontSize: 18 };
    const a = mergeBookOverride(cur, { ttsVoice: "v1", ttsRate: 1.25 });
    expect(a).toEqual({ fontSize: 18, ttsVoice: "v1", ttsRate: 1.25 });
    const b = mergeBookOverride(a, { ttsRate: 1.5 });
    expect(b.ttsVoice).toBe("v1");
    expect(b.ttsRate).toBe(1.5);
  });

  it("undefined 清除 TTS 覆盖（回退全局），不删其它字段", () => {
    const cur: Ov = { fontSize: 18, ttsVoice: "v1", ttsRate: 1.25 };
    const next = mergeBookOverride(cur, { ttsVoice: undefined, ttsRate: undefined });
    expect(next).toEqual({ fontSize: 18 });
  });
});
