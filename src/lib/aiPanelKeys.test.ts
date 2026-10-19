import { describe, expect, it } from "vitest";
import { AI_MODE_KEY, AI_WIN_W_KEY, migrateAiPanelKeys, type KeyStore } from "./aiPanelKeys";

/** 历史键前缀：拼接构造（与守门脚本、实现同款写法，避免本文件出现字面量）。 */
const LEGACY_PREFIX = "f" + "r" + ".ai.";

function memoryStore(seed: Record<string, string> = {}): KeyStore {
  const m = new Map(Object.entries(seed));
  return {
    getItem: (k) => (m.has(k) ? m.get(k)! : null),
    setItem: (k, v) => void m.set(k, v),
    removeItem: (k) => void m.delete(k),
  };
}

describe("migrateAiPanelKeys", () => {
  it("把历史键搬到新键，并删除历史键", () => {
    const s = memoryStore({ [LEGACY_PREFIX + "mode"]: "window", [LEGACY_PREFIX + "winw"]: "512" });
    migrateAiPanelKeys(s);
    expect(s.getItem(AI_MODE_KEY)).toBe("window");
    expect(s.getItem(AI_WIN_W_KEY)).toBe("512");
    expect(s.getItem(LEGACY_PREFIX + "mode")).toBeNull();
    expect(s.getItem(LEGACY_PREFIX + "winw")).toBeNull();
  });

  it("新键已有值时以新键为准，历史键照样清掉（历史值不复活）", () => {
    const s = memoryStore({ [AI_MODE_KEY]: "drawer", [LEGACY_PREFIX + "mode"]: "window" });
    migrateAiPanelKeys(s);
    expect(s.getItem(AI_MODE_KEY)).toBe("drawer");
    expect(s.getItem(LEGACY_PREFIX + "mode")).toBeNull();
  });

  it("没有历史键时不动任何键", () => {
    const s = memoryStore({ [AI_MODE_KEY]: "window" });
    migrateAiPanelKeys(s);
    expect(s.getItem(AI_MODE_KEY)).toBe("window");
  });

  it("重复调用幂等", () => {
    const s = memoryStore({ [LEGACY_PREFIX + "winw"]: "512" });
    migrateAiPanelKeys(s);
    migrateAiPanelKeys(s);
    expect(s.getItem(AI_WIN_W_KEY)).toBe("512");
    expect(s.getItem(LEGACY_PREFIX + "winw")).toBeNull();
  });

  it("没有可用存储时静默返回", () => {
    expect(() => migrateAiPanelKeys(null)).not.toThrow();
  });

  it("单个键读写抛错不影响其余键", () => {
    const s = memoryStore({ [LEGACY_PREFIX + "mode"]: "window", [LEGACY_PREFIX + "winw"]: "512" });
    s.removeItem = () => {
      throw new Error("quota");
    };
    migrateAiPanelKeys(s);
    // 删除失败，但两个键都已补上新值
    expect(s.getItem(AI_MODE_KEY)).toBe("window");
    expect(s.getItem(AI_WIN_W_KEY)).toBe("512");
  });
});
