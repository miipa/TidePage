import { describe, it, expect } from "vitest";
import { versionNewer } from "$lib/version";

describe("versionNewer", () => {
  it("相等不提示", () => {
    expect(versionNewer("0.1.25", "v0.1.25")).toBe(false);
    expect(versionNewer("0.1.25", "0.1.25")).toBe(false);
  });

  it("bug 回归锁定：预发布/构建后缀不应判为更新", () => {
    expect(versionNewer("0.1.25-beta.1", "v0.1.25")).toBe(false);
    expect(versionNewer("0.1.25+build.7", "v0.1.25")).toBe(false);
    expect(versionNewer("0.1.25-rc", "0.1.25")).toBe(false);
  });

  it("真·更新", () => {
    expect(versionNewer("0.1.26", "v0.1.25")).toBe(true);
    expect(versionNewer("0.2.0", "v0.1.25")).toBe(true);
    expect(versionNewer("1.0.0", "v0.9.9")).toBe(true);
  });

  it("更旧", () => {
    expect(versionNewer("0.1.24", "v0.1.25")).toBe(false);
  });

  it("v 前缀两侧任意组合不误判", () => {
    expect(versionNewer("v0.1.25", "0.1.25")).toBe(false);
    expect(versionNewer("V0.1.25", "v0.1.25")).toBe(false);
    expect(versionNewer("v0.1.26", "v0.1.25")).toBe(true);
    expect(versionNewer("0.1.25", "V0.1.26")).toBe(false);
  });

  it("段数不等不误判", () => {
    expect(versionNewer("0.1.25.0", "v0.1.25")).toBe(false);
    expect(versionNewer("v0.1.25", "0.1.25.0")).toBe(false);
    expect(versionNewer("0.1.25.1", "v0.1.25")).toBe(true);
  });
});
