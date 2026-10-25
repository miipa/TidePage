/** R10-3：正文内部链接解析（纯函数）。
 *
 * 处理 text00001.html#filepos123、ch1.xhtml#sec2、#top、带 query 的 href、
 * percent 编码（文件名牌与 fragment 都解码，失败时保留原文）与空 href。
 * 只解析、不决定跳转；findAnchor 的 DOM 匹配在 Reader 内完成。
 */

export interface InternalHref {
  /** 目标文件名（basename、小写；空串=同文档锚点）。 */
  fileBase: string;
  /** 已 percent 解码的 fragment（不含 #；空串=无锚点）。 */
  fragment: string;
  /** 是否同文档锚点（href 只有 #fragment）。 */
  sameDoc: boolean;
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** 解析正文内部 href；空 href / 只有 # 无 fragment / 无文件也无 fragment 返回 null。 */
export function parseInternalHref(href: string): InternalHref | null {
  const raw = (href ?? "").trim();
  if (!raw) return null;
  const hashAt = raw.indexOf("#");
  const beforeHash = hashAt >= 0 ? raw.slice(0, hashAt) : raw;
  const fragment = hashAt >= 0 ? safeDecode(raw.slice(hashAt + 1)) : "";
  const qAt = beforeHash.indexOf("?");
  const pathPart = qAt >= 0 ? beforeHash.slice(0, qAt) : beforeHash;
  const decodedPath = safeDecode(pathPart);
  const base = (decodedPath.split(/[\\/]/).pop() || "").toLowerCase();
  if (!base && !fragment) return null;
  return { fileBase: base, fragment, sameDoc: base === "" };
}

/** R13-A1：同文件 #锚点定位到后端算出的虚拟章；无效/当前章时沿用当前章。 */
export function pickSameDocAnchorTarget(
  currentChapter: number,
  located: number | null | undefined,
): number {
  if (located == null || !Number.isInteger(located) || located < 0) return currentChapter;
  return located;
}
