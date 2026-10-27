/**
 * H13：配置恢复时的端点身份比较（纯 TS、无副作用）。
 *
 * 场景：恢复/导入的 config 会把 webdav.base、ai.profiles[].baseUrl 换成备份里的主机，
 * 而本机已存密文（webdav.pass / ai:{id}:apiKey）会被「为空则回填本机值」的逻辑保留下来，
 * 于是下一次同步/对话会把本机凭据发往备份里的主机。这里只判断「host 有没有变」，
 * 由调用方决定不保留对应密文并提示重新填写。
 */

export interface EndpointSecretChanges {
  /** webdav.base 的 host 是否变化（true ⇒ 不保留 webdav.pass）。 */
  webdav: boolean;
  /** host 变化的 AI 端点 id（字符串，用于 ai:{id}:apiKey 键）。 */
  profileIds: string[];
}

/**
 * 解析端点地址的 origin（协议 + 主机 + 端口，小写）。
 * - 无 scheme 时补 fallbackScheme（http 类地址用 "http"，AI 端点用 "https"）；
 * - 空串/非法/无主机返回 null。
 */
export function urlOrigin(raw: string, fallbackScheme: string): string | null {
  const s = typeof raw === "string" ? raw.trim() : "";
  if (!s) return null;
  const hasScheme = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(s);
  const candidate = hasScheme ? s : fallbackScheme + "://" + s;
  try {
    const u = new URL(candidate);
    if (!u.hostname) return null;
    return (u.protocol.toLowerCase() + "//" + u.host.toLowerCase()) as string;
  } catch {
    return null;
  }
}

/** 两侧都能解析出 origin 且不同 ⇒ host 变化；任一侧为空/不可解析视为未配置，返回 false。 */
export function hostChanged(oldUrl: string, newUrl: string, fallbackScheme = "https"): boolean {
  const a = urlOrigin(oldUrl, fallbackScheme);
  const b = urlOrigin(newUrl, fallbackScheme);
  if (!a || !b) return false;
  return a !== b;
}

interface MinimalConfig {
  webdav?: { base?: unknown };
  ai?: { profiles?: Array<{ id?: unknown; baseUrl?: unknown }> };
}

const baseOf = (cfg: MinimalConfig | null | undefined): string =>
  typeof cfg?.webdav?.base === "string" ? (cfg.webdav.base as string) : "";

const profilesOf = (
  cfg: MinimalConfig | null | undefined,
): Array<{ id?: unknown; baseUrl?: unknown }> =>
  Array.isArray(cfg?.ai?.profiles) ? (cfg.ai!.profiles as Array<{ id?: unknown; baseUrl?: unknown }>) : [];

/**
 * 比较新旧配置里各端点的 host 变化：
 * - webdav.base 用 http 兜底 scheme；
 * - AI 端点按 profile.id 对齐，baseUrl 用 https 兜底 scheme；新增端点无旧密文可保留，不计入；
 * - baseUrl 置空/删除视为未配置，不误报。
 */
export function changedEndpointSecrets(
  oldCfg: MinimalConfig | null | undefined,
  newCfg: MinimalConfig | null | undefined,
): EndpointSecretChanges {
  const webdav = hostChanged(baseOf(oldCfg), baseOf(newCfg), "http");
  const oldProfiles = new Map<string, string>();
  for (const p of profilesOf(oldCfg)) {
    if (p && p.id !== undefined && p.id !== null) {
      oldProfiles.set(String(p.id), typeof p.baseUrl === "string" ? p.baseUrl : "");
    }
  }
  const profileIds: string[] = [];
  for (const p of profilesOf(newCfg)) {
    if (!p || p.id === undefined || p.id === null) continue;
    const id = String(p.id);
    const oldBase = oldProfiles.get(id);
    if (oldBase === undefined) continue; // 新增端点：没有旧密文可泄漏
    const newBase = typeof p.baseUrl === "string" ? p.baseUrl : "";
    if (hostChanged(oldBase, newBase, "https")) profileIds.push(id);
  }
  return { webdav, profileIds };
}
