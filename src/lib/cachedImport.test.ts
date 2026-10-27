import { describe, it, expect, vi } from "vitest";
import { cachedImport } from "./cachedImport";

describe("cachedImport", () => {
  it("成功后缓存：再次调用不重复加载", async () => {
    const loader = vi.fn(async () => ({ ok: 1 }));
    const load = cachedImport(loader);
    expect(await load()).toEqual({ ok: 1 });
    expect(await load()).toEqual({ ok: 1 });
    expect(loader).toHaveBeenCalledTimes(1);
  });

  it("失败后清空缓存：下一次调用重新加载并成功", async () => {
    let n = 0;
    const loader = vi.fn(async () => {
      n++;
      if (n === 1) throw new Error("first fail");
      return { ok: n };
    });
    const load = cachedImport(loader);
    await expect(load()).rejects.toThrow("first fail");
    await expect(load()).resolves.toEqual({ ok: 2 });
    expect(loader).toHaveBeenCalledTimes(2);
  });

  it("并发调用只加载一次；失败时所有等待者都收到同一错误", async () => {
    let reject!: (e: Error) => void;
    const loader = vi.fn(
      () =>
        new Promise<{ ok: boolean }>((_resolve, rej) => {
          reject = rej;
        }),
    );
    const load = cachedImport(loader);
    const a = load();
    const b = load();
    expect(a).toBe(b);
    reject(new Error("boom"));
    await expect(a).rejects.toThrow("boom");
    await expect(b).rejects.toThrow("boom");
    expect(loader).toHaveBeenCalledTimes(1);
    // 失败后可以重试
    const c = load();
    expect(c).not.toBe(a);
    expect(loader).toHaveBeenCalledTimes(2);
    reject(new Error("stop"));
    await expect(c).rejects.toThrow("stop");
  });

  it("loader 同步抛异常：直接抛出且不缓存，下次可重试", async () => {
    let n = 0;
    const loader = vi.fn(() => {
      n++;
      if (n === 1) throw new Error("sync");
      return Promise.resolve("ok");
    });
    const load = cachedImport(loader);
    expect(() => load()).toThrow("sync");
    await expect(load()).resolves.toBe("ok");
    expect(loader).toHaveBeenCalledTimes(2);
  });
});
