import { describe, it, expect } from "vitest";
import { canWriteSecrets, shouldSuppressConfigWrite } from "./configSyncGuard";

const base = { labelResolved: true, isMain: false, hasLocalConfig: true, configSynced: true };

describe("shouldSuppressConfigWrite（F9）", () => {
  it("主窗恒不抑制（即使标签尚未解析）", () => {
    expect(shouldSuppressConfigWrite({ ...base, isMain: true, labelResolved: false })).toBe(false);
    expect(shouldSuppressConfigWrite({ ...base, isMain: true, hasLocalConfig: false, configSynced: false })).toBe(false);
  });

  it("标签未解析（含 2s 超时）时保守抑制", () => {
    expect(shouldSuppressConfigWrite({ ...base, labelResolved: false, isMain: false })).toBe(true);
    expect(shouldSuppressConfigWrite({ ...base, labelResolved: false, isMain: false, hasLocalConfig: false })).toBe(true);
  });

  it("子窗无本地配置且未同步：抑制", () => {
    expect(shouldSuppressConfigWrite({ ...base, hasLocalConfig: false, configSynced: false })).toBe(true);
  });

  it("子窗有本地配置：不抑制", () => {
    expect(shouldSuppressConfigWrite({ ...base, hasLocalConfig: true, configSynced: false })).toBe(false);
  });

  it("子窗已同步：不抑制", () => {
    expect(shouldSuppressConfigWrite({ ...base, hasLocalConfig: false, configSynced: true })).toBe(false);
  });
});

describe("canWriteSecrets（F9）", () => {
  it("密钥回填完成前禁止写 secret", () => {
    expect(canWriteSecrets(true)).toBe(true);
    expect(canWriteSecrets(false)).toBe(false);
  });
});
