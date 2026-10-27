import { describe, it, expect } from "vitest";
import { RequestSeq } from "./requestSeq";

describe("RequestSeq（F9）", () => {
  it("next 自增，只有最新 token 有效", () => {
    const s = new RequestSeq();
    const a = s.next();
    const b = s.next();
    expect(a).toBe(1);
    expect(b).toBe(2);
    expect(s.isCurrent(b)).toBe(true);
    expect(s.isCurrent(a)).toBe(false);
    expect(s.isCurrent(0)).toBe(false);
    expect(s.isCurrent(99)).toBe(false);
  });
});
