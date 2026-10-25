/** R13-A3：旧数据（书签/划线）跳转的章节校正（纯函数，不写 DB）。
 *
 * 旧布局下存的 chapter_index/title 可能已因切分/重排变化：
 * - 索引在范围内且标题匹配（或旧标题为空/纯数字）→ 原索引，corrected=true；
 * - 否则用归一化标题在 toc 中找唯一同标题章 → 校正索引，corrected=true；
 * - 找不到/越界 → 原索引（越界夹到 0），corrected=false，由调用方决定提示。
 */

export interface LegacyTocLike {
  title: string;
  chars?: number;
}

export interface LegacyChapterResolution {
  index: number;
  corrected: boolean;
}

const NUMERIC_TITLE = /^[\s\d.．、,，]+$/;

export function resolveLegacyChapterIndex(
  storedIndex: number,
  storedTitle: string,
  toc: LegacyTocLike[],
  normalize: (s: string) => string,
): LegacyChapterResolution {
  if (!toc.length) return { index: Math.max(0, storedIndex), corrected: false };
  const last = toc.length - 1;
  const raw = (storedTitle ?? "").trim();
  const inRange = Number.isInteger(storedIndex) && storedIndex >= 0 && storedIndex <= last;
  const key = normalize(raw);
  const titleMatches = (i: number) => normalize((toc[i]?.title ?? "").trim()) === key;

  if (inRange && (raw === "" || NUMERIC_TITLE.test(raw) || key === "" || titleMatches(storedIndex))) {
    return { index: storedIndex, corrected: true };
  }
  if (key !== "") {
    let found = -1;
    let hits = 0;
    for (let i = 0; i <= last; i++) {
      if (normalize((toc[i]?.title ?? "").trim()) === key) {
        hits++;
        found = i;
        if (hits > 1) break;
      }
    }
    if (hits === 1) {
      return { index: found, corrected: true };
    }
  }
  return { index: inRange ? storedIndex : 0, corrected: false };
}

/** 按全书进度比例（0-100）估算章节：用于 chapter_index 越界时的兜底；失败返回 null。 */
export function resolveProgressByPct(
  progress: number,
  toc: LegacyTocLike[],
): number | null {
  if (!toc.length || !Number.isFinite(progress)) return null;
  const total = toc.reduce((s, e) => s + Math.max(0, e.chars ?? 0), 0);
  if (total <= 0) return null;
  const target = Math.max(0, Math.min(100, progress)) / 100 * total;
  let acc = 0;
  for (let i = 0; i < toc.length; i++) {
    acc += Math.max(0, toc[i].chars ?? 0);
    if (target <= acc) return i;
  }
  return toc.length - 1;
}
