/** R13-A4：打开失败/封面生成失败的稳定分类（纯函数，UI 只展示 i18n 文案，绝不透传原始错误串）。 */

export type OpenErrorKind = "missing" | "permission" | "damaged" | "unsupported" | "unknown";

/** 与 i18n 键一一对应；键本身即返回给调用方的稳定标识。 */
export const OPEN_ERROR_KEYS = {
  missing: "reader.err.fileMissing",
  permission: "reader.err.filePermission",
  damaged: "reader.err.fileDamaged",
  unsupported: "reader.err.fileUnsupported",
  unknown: "reader.err.openUnknown",
} as const;

function textOf(e: unknown): string {
  try {
    return String(e ?? "").toLowerCase();
  } catch {
    return "";
  }
}

export function classifyOpenError(e: unknown): OpenErrorKind {
  const s = textOf(e);
  if (!s) return "unknown";
  if (/no such file|os error 2\b|书籍不存在|找不到(?:该)?文件|file not found|errno 2/.test(s)) {
    return "missing";
  }
  if (/access is denied|permission denied|os error 5\b|拒绝访问|eacces|eperm/.test(s)) {
    return "permission";
  }
  if (
    /invalidpdfexception|invalid pdf structure|parsetimeoutorcorrupted|corrupt|malformed|bad (?:pdf|file|zip)|not a zip|损坏|格式错误|解析失败/.test(s)
  ) {
    return "damaged";
  }
  if (/unsupported|not supported|不支持的?格式|palmdoc|pdb|unknown (?:format|file type)/.test(s)) {
    return "unsupported";
  }
  return "unknown";
}

export function openErrorKey(e: unknown): (typeof OPEN_ERROR_KEYS)[OpenErrorKind] {
  return OPEN_ERROR_KEYS[classifyOpenError(e)];
}

/** 封面生成失败专用提示（不区分具体原因，避免暴露异常；不影响阅读）。 */
export const COVER_FAIL_KEY = "toast.library.coverFailed";
