/**
 * H22：数量单位格式化（纯函数）。
 * 中文按「万」，英文按键值（当前为 k）换算；单位文本由调用方传入 i18n 值，
 * 避免纯模块反向依赖 i18n。9999 及以下保持原数字。
 */
export function formatCountShort(
  n: number,
  unit10k: string,
  unitK: string,
  locale: string,
): string {
  const v = Math.max(0, Math.trunc(Number.isFinite(n) ? n : 0));
  if (locale.toLowerCase().startsWith("zh")) {
    if (v < 10000) return String(v);
    return (v / 10000).toFixed(1) + unit10k;
  }
  if (v < 1000) return String(v);
  return (v / 1000).toFixed(1) + unitK;
}

/** E14：总秒数拆 h/m/s（四舍五入到整秒后再拆，避免 3599.6 出现 60m）。 */
export function fmtDurationParts(totalSec: number): { h: number; m: number; s: number } {
  const total = Math.max(0, Math.round(Number.isFinite(totalSec) ? totalSec : 0));
  return {
    h: Math.floor(total / 3600),
    m: Math.floor((total % 3600) / 60),
    s: total % 60,
  };
}
