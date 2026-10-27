/**
 * S7：WebDAV 明文 http 风险判定（纯函数）。
 *
 * 后端 dav.rs 对 http:// 的 base URL 会以 Basic 认证明文发送账号密码；
 * 局域网 NAS/本机是有意支持的常见场景，不禁止，但公网主机必须在设置页显式警告。
 * 只对「显式 http://」判定；https/无 scheme/非法地址一律不警告。
 */

/** 把主机名从 authority 中取出（去 userinfo 与端口；IPv6 保留去方括号后的地址）。 */
function hostOf(authority: string): string {
  let hostport = authority;
  const at = hostport.lastIndexOf("@");
  if (at >= 0) hostport = hostport.slice(at + 1);
  if (hostport.startsWith("[")) {
    const end = hostport.indexOf("]");
    if (end < 0) return "";
    return hostport.slice(1, end).toLowerCase();
  }
  const colon = hostport.indexOf(":");
  if (colon >= 0) hostport = hostport.slice(0, colon);
  return hostport.toLowerCase();
}

function isPrivateIpv4(host: string): boolean | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (!m) return null;
  const parts = [Number(m[1]), Number(m[2]), Number(m[3]), Number(m[4])];
  if (parts.some((n) => n > 255)) return null; // 非法地址：不判定为风险
  const [a, b] = parts;
  // 127.0.0.0/8 本机回环；10/8、172.16/12、192.168/16 私网；169.254/16 link-local。
  if (a === 127 || a === 10) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 168) return true;
  if (a === 169 && b === 254) return true;
  return false;
}

function isPrivateIpv6(host: string): boolean | null {
  if (!host.includes(":")) return null;
  if (host === "::1") return true;
  const first = host.split(":")[0] || "0";
  const head = Number.parseInt(first, 16);
  if (Number.isNaN(head)) return null;
  // fc00::/7（ULA）与 fe80::/10（link-local）。
  if ((head & 0xfe00) === 0xfc00) return true;
  if ((head & 0xffc0) === 0xfe80) return true;
  return false;
}

/** 公网 http:// 返回 true（需要警告）；https、本机/私网、.local、单标签主机、非法/空一律 false。 */
export function isInsecurePublicWebdav(url: string): boolean {
  if (typeof url !== "string") return false;
  const m = /^http:\/\/([^/?#]+)/i.exec(url.trim());
  if (!m) return false;
  const host = hostOf(m[1]);
  if (!host) return false;
  if (host === "localhost" || host.endsWith(".local")) return false;
  const v4 = isPrivateIpv4(host);
  if (v4 !== null) return !v4;
  const v6 = isPrivateIpv6(host);
  if (v6 !== null) return !v6;
  // 形似 IPv4 但数值非法（如 300.300.300.300）：按非法地址处理，不警告。
  if (/^[\d.]+$/.test(host)) return false;
  // 单标签主机名（如 nas、myhost）按内网处理；其余带点的域名视为公网。
  if (!host.includes(".")) return false;
  return true;
}
