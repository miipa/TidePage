export interface RGBA {
  r: number;
  g: number;
  b: number;
  a: number;
}

const clamp255 = (n: number) => Math.max(0, Math.min(255, Math.round(n)));
const clampA = (n: number) => Math.max(0, Math.min(1, n));

/** 解析 #rgb / #rgba / #rrggbb / #rrggbbaa 或 rgb()/rgba()，失败返回 null。 */
export function parseColor(input: string): RGBA | null {
  const s = (input || "").trim().toLowerCase();
  if (!s) return null;
  const hex = s.match(/^#([0-9a-f]{3,8})$/);
  if (hex) {
    let h = hex[1];
    if (h.length === 3 || h.length === 4) {
      h = h.split("").map((c) => c + c).join("");
    }
    if (h.length !== 6 && h.length !== 8) return null;
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    const a = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
    if ([r, g, b].some((v) => Number.isNaN(v))) return null;
    return { r, g, b, a: clampA(a) };
  }
  const rgb = s.match(/^rgba?\(([^)]+)\)$/);
  if (rgb) {
    const parts = rgb[1].split(/[,/\s]+/).map((x) => x.trim()).filter(Boolean);
    if (parts.length < 3) return null;
    const nums = parts.slice(0, 3).map((p) => (p.endsWith("%") ? parseFloat(p) * 2.55 : parseFloat(p)));
    if (nums.some((n) => Number.isNaN(n))) return null;
    const a = parts[3] != null ? (parts[3].endsWith("%") ? parseFloat(parts[3]) / 100 : parseFloat(parts[3])) : 1;
    return { r: clamp255(nums[0]), g: clamp255(nums[1]), b: clamp255(nums[2]), a: clampA(Number.isNaN(a) ? 1 : a) };
  }
  return null;
}

function h2(n: number): string {
  return clamp255(n).toString(16).padStart(2, "0");
}

/** #rrggbb（忽略透明度）。 */
export function toHex(c: RGBA): string {
  return `#${h2(c.r)}${h2(c.g)}${h2(c.b)}`;
}

/** rgb(r, g, b) 或 rgba(r, g, b, a)。 */
export function toRgbString(c: RGBA): string {
  const r = clamp255(c.r);
  const g = clamp255(c.g);
  const b = clamp255(c.b);
  if (c.a >= 1) return `rgb(${r}, ${g}, ${b})`;
  return `rgba(${r}, ${g}, ${b}, ${Math.round(c.a * 100) / 100})`;
}

/** 归一化为规范颜色字符串（供应用使用）；失败返回 null。 */
export function normalizeColor(input: string): string | null {
  const c = parseColor(input);
  if (!c) return null;
  if (c.a >= 1) return toHex(c);
  return toRgbString(c);
}

/** 便于样式使用的安全颜色：非法则回退 fallback。 */
export function safeColor(input: string, fallback: string): string {
  return normalizeColor(input) ?? fallback;
}
