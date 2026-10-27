// 阅读配色方案「只应用排版字段」的白名单与纯合并函数。
// 为什么需要：旧实现把方案 cfg（完整 DEFAULTS.reader 快照）整体替换 config.reader，
// 会连 multiWindow / browserSearchUrl / conv / aiPanelWidth 等非排版设置一起重置。
// 白名单与 ReaderConfig 实际字段一一对应：fontFamily/latinFont/fontSize/fontWeight/lineHeight/
// paraSpacing/margins/indent/textAlign/theme/bgColor/fgColor/bgImage/bgOpacity/cssExtra/overrideBookCss。
// （ReaderConfig 无 monoFont/letterSpacing；monoFont 是 readFix 的固定值，不参与配色方案。）

export const TYPESETTING_READER_KEYS = [
  "fontFamily",
  "latinFont",
  "fontSize",
  "fontWeight",
  "lineHeight",
  "paraSpacing",
  "margins",
  "indent",
  "textAlign",
  "theme",
  "bgColor",
  "fgColor",
  "bgImage",
  "bgOpacity",
  "cssExtra",
  "overrideBookCss",
] as const;

export type TypesettingReaderKey = (typeof TYPESETTING_READER_KEYS)[number];

function cloneValue<T>(v: T): T {
  if (v && typeof v === "object") {
    try {
      return structuredClone(v);
    } catch {
      return JSON.parse(JSON.stringify(v)) as T;
    }
  }
  return v;
}

/**
 * 只把方案里的排版字段合并到当前阅读配置：非白名单字段保持 current 原值。
 * - 方案有该字段 → 用方案值（深拷贝，不共享引用）；
 * - 方案缺该字段（旧方案结构）→ 用 defaults 的值补齐，避免 undefined 写进运行态；
 * - 不修改任何入参（返回新对象）。
 */
export function pickSchemeTypesetting<T extends object>(
  schemeCfg: Partial<T>,
  current: T,
  defaults: T,
): T {
  const next = cloneValue(current);
  for (const k of TYPESETTING_READER_KEYS) {
    const s = (schemeCfg as Record<string, unknown>)[k];
    if (s !== undefined) {
      (next as Record<string, unknown>)[k] = cloneValue(s);
    } else {
      const d = (defaults as Record<string, unknown>)[k];
      if (d !== undefined) (next as Record<string, unknown>)[k] = cloneValue(d);
    }
  }
  return next;
}
