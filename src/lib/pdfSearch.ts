/** D6-3：PDF 文本搜索纯逻辑（逐页取文本在 pdf.ts；导航/片段/出现序号/文本层定位在此）。 */
export interface PdfSearchHit {
  page: number;
  start: number;
  end: number;
  /** 页内第几次出现（0 基）；文本层高亮按它定位，避免跨文本层偏移不一致。 */
  occurrence: number;
  snippet: string;
  /** B11：跨页命中的各页片段（起始页高亮/翻到后续页时继续高亮）。 */
  segments?: { page: number; start: number; end: number; occurrence: number }[];
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** 查询归一：trim + 连续空白折叠（为空返回空串）。 */
export function normalizePdfQuery(query: string): string {
  return (query ?? "").trim().replace(/\s+/g, " ");
}

/** 去掉空白并记录每个字符回原文的映射（搜索与文本层都按去空白后的文本匹配出现次数）。 */
function stripWithMap(text: string): { s: string; map: number[] } {
  let s = "";
  const map: number[] = [];
  for (let i = 0; i < text.length; i++) {
    if (/\s/.test(text[i])) continue;
    s += text[i];
    map.push(i);
  }
  return { s, map };
}

/** 在单页文本中查找（大小写不敏感、忽略空白差异）；start/end 为原文偏移。 */
export function findPdfMatches(text: string, query: string, page: number, limit = 500): PdfSearchHit[] {
  const q = normalizePdfQuery(query).replace(/\s+/g, "");
  if (!q || !text) return [];
  const { s, map } = stripWithMap(text);
  let re: RegExp;
  try {
    re = new RegExp(escapeRegExp(q), "gi");
  } catch {
    return [];
  }
  const out: PdfSearchHit[] = [];
  for (let m = re.exec(s); m && out.length < limit; m = re.exec(s)) {
    if (m[0].length === 0) {
      re.lastIndex += 1;
      continue;
    }
    const startS = m.index;
    const endS = m.index + m[0].length - 1;
    const start = map[startS] ?? 0;
    const end = (map[endS] ?? start) + 1;
    out.push({ page, start, end, occurrence: out.length, snippet: snippetFor(text, start, end) });
  }
  return out;
}

/** 在文本层文本中按「查询 + 第 occurrence 次出现」定位（同样忽略空白差异）；无命中返回 null。 */
export function locateOccurrence(
  text: string,
  query: string,
  occurrence: number,
): { start: number; end: number } | null {
  const q = normalizePdfQuery(query).replace(/\s+/g, "");
  if (!q || !text) return null;
  const { s, map } = stripWithMap(text);
  let re: RegExp;
  try {
    re = new RegExp(escapeRegExp(q), "gi");
  } catch {
    return null;
  }
  let idx = 0;
  for (let m = re.exec(s); m; m = re.exec(s)) {
    if (m[0].length === 0) {
      re.lastIndex += 1;
      continue;
    }
    if (idx === occurrence) {
      const startS = m.index;
      const endS = m.index + m[0].length - 1;
      const start = map[startS] ?? 0;
      const end = (map[endS] ?? start) + 1;
      return { start, end };
    }
    idx++;
  }
  return null;
}

/** 命中片段：前后各留约 30 个字符，空白折叠，首尾省略号。 */
export function snippetFor(text: string, start: number, end: number, radius = 30): string {
  const s = Math.max(0, start - radius);
  const e = Math.min(text.length, end + radius);
  const head = text.slice(s, start).replace(/\s+/g, " ");
  const body = text.slice(start, end).replace(/\s+/g, " ");
  const tail = text.slice(end, e).replace(/\s+/g, " ");
  return `${s > 0 ? "…" : ""}${head}${body}${tail}${e < text.length ? "…" : ""}`;
}


/* ---------------- B11：跨页合并 / 文本层二分定位 ---------------- */
export interface PdfSearchSegment {
  page: number;
  start: number;
  end: number;
  occurrence: number;
}
export interface PdfSearchGroup {
  startPage: number;
  endPage: number;
  segments: PdfSearchSegment[];
  snippet: string;
}

/** 把按页文本拼成一条流，跨页/跨行查询只产生一条结果；同页多次按页内 occurrence 编号。 */
export function groupPdfMatches(
  pages: { page: number; text: string }[],
  query: string,
  limit = 500,
): PdfSearchGroup[] {
  const q = normalizePdfQuery(query).replace(/\s+/g, "");
  if (!q || !pages.length) return [];
  const combined = pages.map((p) => p.text).join("\n");
  const { s, map } = stripWithMap(combined);
  let re: RegExp;
  try {
    re = new RegExp(escapeRegExp(q), "gi");
  } catch {
    return [];
  }
  // 每页去掉空白后的起始下标与字符映射。
  const pageRanges: { page: number; start: number; end: number; stripStart: number; stripLen: number }[] = [];
  let stripPos = 0;
  let rawPos = 0;
  for (const p of pages) {
    const stripped = stripWithMap(p.text).s;
    pageRanges.push({ page: p.page, start: rawPos, end: rawPos + p.text.length, stripStart: stripPos, stripLen: stripped.length });
    stripPos += stripped.length;
    rawPos += p.text.length + 1; // joined "\n"
  }
  const groups: PdfSearchGroup[] = [];
  const pageOcc = new Map<number, number>();
  for (let m = re.exec(s); m && groups.length < limit; m = re.exec(s)) {
    if (m[0].length === 0) { re.lastIndex += 1; continue; }
    const gs = m.index;
    const ge = gs + m[0].length; // 去空白后的独占结束
    const segments: PdfSearchSegment[] = [];
    for (const pr of pageRanges) {
      const a = Math.max(gs, pr.stripStart);
      const b = Math.min(ge, pr.stripStart + pr.stripLen);
      if (b <= a) continue;
      const localA = a - pr.stripStart;
      const localB = b - pr.stripStart;
      // map 存的是整本拼接文本的 raw 偏移；segment 必须是页内偏移（pr.start 只减一次）。
      const globalStart = map[pr.stripStart + localA] ?? pr.start;
      const globalEnd = (map[pr.stripStart + localB - 1] ?? globalStart) + 1;
      const pageLen = pr.end - pr.start;
      const start = Math.max(0, globalStart - pr.start);
      const end = Math.min(pageLen, Math.max(start + 1, globalEnd - pr.start));
      const occ = pageOcc.get(pr.page) ?? 0;
      pageOcc.set(pr.page, occ + 1);
      segments.push({ page: pr.page, start, end, occurrence: occ });
    }
    if (!segments.length) continue;
    groups.push({
      startPage: segments[0].page,
      endPage: segments[segments.length - 1].page,
      segments,
      snippet: snippetFor(combined, map[gs] ?? segments[0].start, (map[Math.max(gs, ge - 1)] ?? segments[segments.length - 1].end - 1) + 1),
    });
  }
  return groups;
}

/** 文本节点累计偏移二分：返回最后一个 start<=offset 的节点下标（空数组 -1）。 */
export function findNodeIndexByOffset(starts: number[], offset: number): number {
  if (!starts.length) return -1;
  let lo = 0, hi = starts.length - 1, ans = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (starts[mid] <= offset) { ans = mid; lo = mid + 1; } else hi = mid - 1;
  }
  return ans;
}

/** 把文本层 [start,end) 映射到节点下标与节点内偏移；定位失败返回 null。 */
export function nodeRangeForOffsets(
  starts: number[],
  lengths: number[],
  start: number,
  end: number,
): { startIndex: number; startOffset: number; endIndex: number; endOffset: number } | null {
  if (!starts.length || starts.length !== lengths.length || end <= start) return null;
  const si = findNodeIndexByOffset(starts, start);
  if (si < 0 || start >= starts[si] + lengths[si]) return null;
  const ei = findNodeIndexByOffset(starts, Math.max(start, end - 1));
  if (ei < 0) return null;
  return {
    startIndex: si,
    startOffset: start - starts[si],
    endIndex: ei,
    endOffset: Math.min(lengths[ei], Math.max(1, end - starts[ei])),
  };
}
/** 命中导航：dir=1 下一条 / -1 上一条，循环；空集合返回 0。 */
export function stepHit(current: number, total: number, dir: 1 | -1): number {
  if (total <= 0) return 0;
  return (((current + dir) % total) + total) % total;
}

/** 进度数值（0–100；UI 负责格式化）。 */
export function searchProgress(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((done / total) * 100)));
}
