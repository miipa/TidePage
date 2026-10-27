import { describe, it, expect } from "vitest";
import { resolveLoadWrite } from "./webdavLoad";

describe("resolveLoadWrite", () => {
  const prev = ["a", "b"];
  it("最新请求成功：应用新列表与新路径", () => {
    const r = resolveLoadWrite(3, 3, true, ["c"], prev, "old/path", "new/path");
    expect(r).toEqual({ apply: true, entries: ["c"], path: "new/path" });
  });
  it("乱序回包（旧序号）：不写，保留旧列表与旧路径", () => {
    const r = resolveLoadWrite(3, 2, true, ["stale"], prev, "old/path", "stale/path");
    expect(r.apply).toBe(false);
    expect(r.entries).toEqual(prev);
    expect(r.path).toBe("old/path");
  });
  it("最新请求失败：保留旧列表，不提交目标路径", () => {
    const r = resolveLoadWrite(3, 3, false, [], prev, "old/path", "new/path");
    expect(r).toEqual({ apply: false, entries: prev, path: "old/path" });
  });
  it("旧请求失败：同样不写", () => {
    const r = resolveLoadWrite(3, 1, false, [], prev, "old/path", "old/path");
    expect(r.apply).toBe(false);
    expect(r.entries).toEqual(prev);
  });
});

import { shouldReloadAfterConfig, downloadGate } from "./webdavLoad";

describe("shouldReloadAfterConfig（F5）", () => {
  it("配置由未就绪变就绪：未尝试过或上次失败时重连", () => {
    expect(shouldReloadAfterConfig(false, true, false, "", false)).toBe(true);
    expect(shouldReloadAfterConfig(false, true, true, "boom", false)).toBe(true);
  });
  it("已经就绪/成功过/加载中不重复拉取", () => {
    expect(shouldReloadAfterConfig(true, true, false, "", false)).toBe(false);
    expect(shouldReloadAfterConfig(false, true, true, "", false)).toBe(false);
    expect(shouldReloadAfterConfig(false, true, false, "", true)).toBe(false);
  });
  it("仍是未就绪不触发", () => {
    expect(shouldReloadAfterConfig(false, false, false, "", false)).toBe(false);
  });
});

describe("downloadGate（F5）", () => {
  it("并发上限与同路径在途", () => {
    const empty = new Set<string>();
    expect(downloadGate(0, empty, "a")).toBe(true);
    expect(downloadGate(2, empty, "a")).toBe(true);
    expect(downloadGate(3, empty, "a")).toBe(false);
    expect(downloadGate(0, new Set(["a"]), "a")).toBe(false);
    expect(downloadGate(0, new Set(["a"]), "b")).toBe(true);
    expect(downloadGate(0, new Set(["a"]), "b", 1)).toBe(true);
    expect(downloadGate(1, empty, "b", 1)).toBe(false);
  });
});
