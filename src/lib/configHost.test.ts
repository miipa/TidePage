import { describe, it, expect } from "vitest";
import { changedEndpointSecrets, hostChanged, urlOrigin } from "./configHost";

describe("urlOrigin", () => {
  it("解析协议、主机与端口并小写归一", () => {
    expect(urlOrigin("https://API.Example.com/v1", "https")).toBe("https://api.example.com");
    expect(urlOrigin("HTTPS://API.EXAMPLE.COM:8443/path", "https")).toBe("https://api.example.com:8443");
    expect(urlOrigin("http://127.0.0.1:8080", "http")).toBe("http://127.0.0.1:8080");
    // 默认端口会被 URL 归一掉（等价地址不应误报）
    expect(urlOrigin("https://a.com:443", "https")).toBe("https://a.com");
    expect(urlOrigin("http://a.com:80", "http")).toBe("http://a.com");
  });

  it("无 scheme 时按 fallbackScheme 补齐", () => {
    expect(urlOrigin("api.example.com", "https")).toBe("https://api.example.com");
    expect(urlOrigin("api.example.com", "http")).toBe("http://api.example.com");
  });

  it("空串、空白、非法值返回 null", () => {
    expect(urlOrigin("", "https")).toBeNull();
    expect(urlOrigin("   ", "https")).toBeNull();
    expect(urlOrigin("http://", "http")).toBeNull();
    expect(urlOrigin("::::", "https")).toBeNull();
    expect(urlOrigin(null as unknown as string, "https")).toBeNull();
  });
});

describe("hostChanged", () => {
  it("同 host 不同路径/大小写不算变化", () => {
    expect(hostChanged("https://a.com/v1", "https://A.COM/v2", "https")).toBe(false);
    expect(hostChanged("a.com", "https://a.com/api", "https")).toBe(false);
  });

  it("协议或端口变化算变化", () => {
    expect(hostChanged("https://a.com", "http://a.com", "https")).toBe(true);
    expect(hostChanged("https://a.com:443", "https://a.com:8443", "https")).toBe(true);
    expect(hostChanged("api.a.com", "api.b.com", "https")).toBe(true);
  });

  it("任一侧为空视为未配置", () => {
    expect(hostChanged("", "https://a.com", "https")).toBe(false);
    expect(hostChanged("https://a.com", "", "https")).toBe(false);
  });
});

describe("changedEndpointSecrets", () => {
  it("按 profile.id 对齐并排除新增端点", () => {
    const oldCfg = {
      webdav: { base: "https://dav.old-host.com" },
      ai: { profiles: [{ id: 1, baseUrl: "https://api.old.com" }, { id: 2, baseUrl: "https://same.com" }] },
    };
    const newCfg = {
      webdav: { base: "https://dav.new-host.com" },
      ai: {
        profiles: [
          { id: 1, baseUrl: "https://api.new.com" },
          { id: 2, baseUrl: "https://same.com/other-path" },
          { id: 3, baseUrl: "https://brand-new.com" },
        ],
      },
    };
    const out = changedEndpointSecrets(oldCfg, newCfg);
    expect(out.webdav).toBe(true);
    expect(out.profileIds).toEqual(["1"]);
  });

  it("地址置空或端点被删除时不误报", () => {
    const oldCfg = {
      webdav: { base: "https://dav.com" },
      ai: { profiles: [{ id: 1, baseUrl: "https://a.com" }, { id: 2, baseUrl: "https://b.com" }] },
    };
    const newCfg = {
      webdav: { base: "" },
      ai: { profiles: [{ id: 1, baseUrl: "" }] },
    };
    const out = changedEndpointSecrets(oldCfg, newCfg);
    expect(out.webdav).toBe(false);
    expect(out.profileIds).toEqual([]);
  });

  it("缺少 profiles/webdav 的旧配置不 panic", () => {
    expect(changedEndpointSecrets({}, {})).toEqual({ webdav: false, profileIds: [] });
    expect(changedEndpointSecrets(null, null)).toEqual({ webdav: false, profileIds: [] });
  });
});
