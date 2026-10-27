import { describe, it, expect } from "vitest";
import { sanitizeEndpointUrl } from "./urlSecrets";
import { hostChanged } from "./configHost";

describe("sanitizeEndpointUrl：URL 内嵌凭据脱敏（F9）", () => {
  it("去掉 scheme:// 后的 userinfo，其余部分原样保留", () => {
    expect(sanitizeEndpointUrl("https://user:pass@dav.example.com/a/b")).toBe("https://dav.example.com/a/b");
    expect(sanitizeEndpointUrl("http://user@host:8080/x?y=1")).toBe("http://host:8080/x?y=1");
  });

  it("query 里的凭据类键值替换为 ***，其它参数与顺序/编码保持原样", () => {
    const url = "https://api.example.com/v1?access_token=abc123&model=x&api_key=k&api-version=2";
    expect(sanitizeEndpointUrl(url)).toBe(
      "https://api.example.com/v1?access_token=***&model=x&api_key=***&api-version=2",
    );
    // 大小写不敏感；无 = 的开关参数不动；fragment 原样保留。
    expect(sanitizeEndpointUrl("https://h/p?TOKEN=1&flag&Password=2#frag")).toBe(
      "https://h/p?TOKEN=***&flag&Password=***#frag",
    );
  });

  it("非 URL / 空串 / 无凭据 URL 原样返回（幂等）", () => {
    expect(sanitizeEndpointUrl("")).toBe("");
    expect(sanitizeEndpointUrl("localhost:8080/v1")).toBe("localhost:8080/v1");
    expect(sanitizeEndpointUrl("not a url")).toBe("not a url");
    const clean = "https://dav.example.com/dav";
    expect(sanitizeEndpointUrl(clean)).toBe(clean);
    const once = sanitizeEndpointUrl("https://u:p@h/p?access_token=x&api-version=2");
    expect(sanitizeEndpointUrl(once)).toBe(once);
  });

  it("脱敏不改变 host：hostChanged(orig, sanitize(orig)) 为 false", () => {
    const urls = [
      "https://user:pass@dav.example.com:8443/a/b?access_token=x",
      "http://user@host/x",
      "https://h/p?api_key=1&api-version=2",
    ];
    for (const u of urls) {
      expect(hostChanged(u, sanitizeEndpointUrl(u))).toBe(false);
    }
    // 真实主机变化仍能识别（H13 的 host 判定不受脱敏影响）。
    expect(hostChanged("https://a.example.com/x", "https://b.example.com/x")).toBe(true);
  });
});
