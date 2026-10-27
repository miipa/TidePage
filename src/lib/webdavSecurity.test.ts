import { describe, it, expect } from "vitest";
import { isInsecurePublicWebdav } from "./webdavSecurity";

describe("isInsecurePublicWebdav（S7）", () => {
  it("https 公网不警告", () => {
    expect(isInsecurePublicWebdav("https://dav.example.com/dav")).toBe(false);
    expect(isInsecurePublicWebdav("HTTPS://dav.example.com")).toBe(false);
  });

  it("http 公网警告（含端口/路径/userinfo）", () => {
    expect(isInsecurePublicWebdav("http://dav.example.com/dav")).toBe(true);
    expect(isInsecurePublicWebdav("http://dav.example.com:8080/x")).toBe(true);
    expect(isInsecurePublicWebdav("http://user:pass@dav.example.com/x")).toBe(true);
    expect(isInsecurePublicWebdav("HTTP://dav.example.com")).toBe(true);
  });

  it("http 本机/私网/NAS 不警告", () => {
    for (const u of [
      "http://127.0.0.1:5005",
      "http://127.8.9.10/x",
      "http://localhost:8080",
      "http://10.1.2.3/dav",
      "http://172.16.0.10",
      "http://172.20.5.5:8080/x",
      "http://172.31.255.254",
      "http://192.168.1.5/dav",
      "http://169.254.10.20",
      "http://[::1]:8080",
      "http://[fd12::1]/dav",
      "http://[fe80::1]/dav",
      "http://nas/dav",
      "http://disk.local/dav",
    ]) {
      expect(isInsecurePublicWebdav(u), u).toBe(false);
    }
  });

  it("无 scheme/空/非法地址不警告", () => {
    expect(isInsecurePublicWebdav("")).toBe(false);
    expect(isInsecurePublicWebdav("dav.example.com/dav")).toBe(false);
    expect(isInsecurePublicWebdav("ftp://dav.example.com")).toBe(false);
    expect(isInsecurePublicWebdav("http:///dav")).toBe(false);
    expect(isInsecurePublicWebdav("http://300.300.300.300")).toBe(false);
  });
});
