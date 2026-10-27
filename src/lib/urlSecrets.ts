/**
 * F9：URL 内嵌凭据的脱敏（纯函数，无 DOM/Tauri 依赖）。
 *
 * 为什么需要：WebDAV baseUrl 与 AI 端点 baseUrl 可能写成
 * `https://user:pass@host/dav` 或 `https://host/v1?access_token=xxx`。
 * 这些 URL 会随 localStorage、跨窗广播、导出与云端设置备份明文带出去；
 * 但持久化副本一旦被打码，重启/其它窗口会把 `***` 当真实地址用（功能回归），
 * 因此约定：持久化/广播只放 sanitizeEndpointUrl 后的地址，原始地址由
 * collectSecrets 收集进 secret.config（DPAPI 加密），applySecrets 在本机回填。
 */

/** query 里视为凭据的键名（小写比较）：值会被替换为 ***（保留键名与参数顺序）。 */
export const CREDENTIAL_QUERY_KEYS: ReadonlySet<string> = new Set([
  "access_token",
  "access-token",
  "refreshtoken",
  "refresh_token",
  "refresh-token",
  "token",
  "auth",
  "authorization",
  "api_key",
  "api-key",
  "apikey",
  "password",
  "passwd",
  "pass",
  "secret",
  "client_secret",
  "client-secret",
  "sig",
  "signature",
  "x-amz-signature",
  "x-amz-credential",
  "x-amz-security-token",
]);

/** scheme:// 前缀（只脱敏带 scheme 的绝对地址；其它字符串原样返回）。 */
const SCHEME_RE = /^[A-Za-z][A-Za-z0-9+.-]*:\/\//;
/** scheme:// 后到第一个路径/查询/片段/@ 之间的 userinfo（可含 user:pass）。 */
const USERINFO_RE = /^([A-Za-z][A-Za-z0-9+.-]*:\/\/)[^/?#@]*@/;

/** 单个 query 参数脱敏：键名命中凭据名单时值替换为 ***（按第一个 = 拆 key/value）。 */
function sanitizeQuery(query: string): string {
  return query
    .split("&")
    .map((param) => {
      const eq = param.indexOf("=");
      if (eq < 0) return param;
      const key = param.slice(0, eq);
      let decoded = key;
      try {
        decoded = decodeURIComponent(key);
      } catch {
        /* 非法编码按原样比较 */
      }
      if (!CREDENTIAL_QUERY_KEYS.has(decoded.trim().toLowerCase())) return param;
      return key + "=***";
    })
    .join("&");
}

/**
 * 去掉 URL 里可被当作凭据的部分：
 *  - `scheme://userinfo@` 的 userinfo（保留 scheme 与其余部分原样，不用 URL 重新序列化）；
 *  - query 中凭据类键名的值改为 `***`，其它参数与 fragment 原样保留。
 * 非 URL、空串、空 query 一律原样返回（幂等：sanitize(sanitize(x)) === sanitize(x)）。
 */
export function sanitizeEndpointUrl(raw: string): string {
  if (typeof raw !== "string" || !raw) return raw;
  if (!SCHEME_RE.test(raw)) return raw;
  let out = raw.replace(USERINFO_RE, "$1");
  const hashIdx = out.indexOf("#");
  const fragment = hashIdx >= 0 ? out.slice(hashIdx) : "";
  const beforeFragment = hashIdx >= 0 ? out.slice(0, hashIdx) : out;
  const qIdx = beforeFragment.indexOf("?");
  if (qIdx < 0) return out;
  const base = beforeFragment.slice(0, qIdx);
  const query = beforeFragment.slice(qIdx + 1);
  if (!query) return out;
  out = base + "?" + sanitizeQuery(query) + fragment;
  return out;
}
