import { describe, it, expect } from "vitest";
import {
  SYNTH_MAX_ATTEMPTS,
  PLAYBACK_MAX_ATTEMPTS,
  synthAttemptsAllowed,
  playbackAttemptsAllowed,
  sessionMatches,
} from "./ttsRetry";

describe("ttsRetry", () => {
  it("合成最多 2 次", () => {
    expect(SYNTH_MAX_ATTEMPTS).toBe(2);
    expect(synthAttemptsAllowed(0)).toBe(true);
    expect(synthAttemptsAllowed(1)).toBe(true);
    expect(synthAttemptsAllowed(2)).toBe(false);
    expect(synthAttemptsAllowed(-1)).toBe(false);
    expect(synthAttemptsAllowed(NaN)).toBe(false);
  });

  it("播放重试独立且有限", () => {
    expect(PLAYBACK_MAX_ATTEMPTS).toBe(2);
    expect(playbackAttemptsAllowed(0)).toBe(true);
    expect(playbackAttemptsAllowed(1)).toBe(true);
    expect(playbackAttemptsAllowed(2)).toBe(false);
    expect(playbackAttemptsAllowed(1, 1)).toBe(false);
  });

  it("会话代际匹配", () => {
    expect(sessionMatches(3, 3)).toBe(true);
    expect(sessionMatches(3, 4)).toBe(false);
    expect(sessionMatches(NaN, NaN)).toBe(false);
    expect(sessionMatches(1.5, 1.5)).toBe(false);
  });
});
