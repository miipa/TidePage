import type { TocEntry } from "./types";

// PDF 渲染的纯函数层：页面显示尺寸、切片划分、位图分辨率夹取、页码目录。
// 与 pdf.js 运行时（pdf.ts）分开，既好单测，也避免测试环境去加载 pdf.js。

/** PDF 用户单位（pt）→ CSS px 的换算：PDF 以 72dpi 计量，CSS 以 96dpi 计量 → 4/3。
 *  这是「页面按真实尺寸呈现」的基准：A4（595×842pt）→ 793×1123 CSS px。 */
export const PDF_PX_PER_PT = 4 / 3;

/** 位图长边上限（物理像素）：只挡「极端长条」，真正的内存硬闸是下面的总像素上限。
 *  取 8192 是因为超宽页在 200% 下单切片宽可达 ~4500px（4096 会把 200% 降成 ~0.91× 的上采样）。 */
export const PDF_BITMAP_MAX_LONG = 8192;
/** 位图总像素上限（约 1200 万）：≈48MB/张 RGBA，配合并发 2 与缓存上限控制峰值内存。 */
export const PDF_BITMAP_MAX_PIXELS = 12_000_000;

/** 一页显示高度超过「多少个屏幕」就开始纵向切片。1.5 屏以内容忍轻微超出，避免普通页被切碎。 */
export const PDF_SLICE_MAX_SCREENS = 1.5;

export interface PageSize {
  /** PDF 页尺寸（pt，= pdf.js viewport(scale:1) 的宽高）。 */
  ptW: number;
  ptH: number;
}

export interface DisplayBox {
  /** 该页在屏幕上的 CSS 尺寸（我们已经把它「烧」进位图与 <img> 的显式宽高里）。 */
  w: number;
  h: number;
  /** 实际生效的缩放系数（CSS px per pt）。 */
  k: number;
}

/** 算一页在屏幕上的显示尺寸（CSS px）。
 *  为什么按「页尺寸 × 4/3 × 有效缩放」解析计算，而不是让浏览器自己算：位图分辨率必须等于
 *  「显示尺寸 × DPR」，否则要么上采样发虚（显示比位图大）、要么白耗内存（位图比显示大）。
 *  依赖浏览器布局算显示尺寸会形成「位图尺寸 → 布局尺寸 → 位图尺寸」的反馈环，必须解析求解。
 *  - flow（纵向滚动）：以倍率为准，但不超过容器宽（否则横向溢出被裁）；
 *  - horizontal（横向）：漫画横向布局是按高度定尺寸的，故先把「容器高 × 倍率」作为目标高；
 *  - fitWidth / fitHeight：直接按容器宽/高反推（与既有漫画语义一致，优先于倍率）。 */
export function pdfDisplayBox(opts: {
  page: PageSize;
  containerW: number;
  containerH: number;
  imgScale: number;
  fitWidth: boolean;
  fitHeight: boolean;
  horizontal: boolean;
}): DisplayBox {
  const { page, containerW, containerH, imgScale, fitWidth, fitHeight, horizontal } = opts;
  const baseW = Math.max(1, page.ptW * PDF_PX_PER_PT);
  const baseH = Math.max(1, page.ptH * PDF_PX_PER_PT);
  const scale = Number.isFinite(imgScale) && imgScale > 0 ? imgScale : 1;
  const cw = Number.isFinite(containerW) && containerW > 40 ? containerW : 800;
  const ch = Number.isFinite(containerH) && containerH > 40 ? containerH : 800;
  let k: number;
  if (fitWidth) {
    k = cw / baseW;
  } else if (fitHeight) {
    k = ch / baseH;
  } else if (horizontal) {
    // 横向布局是「一屏高」定尺寸：倍率作用在容器高上
    k = (ch / baseH) * scale;
  } else {
    // 纵向滚动：直接用倍率，**不夹到容器宽**。页面比视口宽时由 PDF 的横向滚动查看
    // （.reader.pdf .manga-flow-viewport 放开了 overflow-x），夹取会让「放大」变成空操作。
    k = scale;
  }
  if (!Number.isFinite(k) || k <= 0) k = 1;
  return { w: Math.max(1, Math.round(baseW * k)), h: Math.max(1, Math.round(baseH * k)), k };
}

/** 位图分辨率：显示尺寸 × DPR，再按「长边 ≤ maxLong、总像素 ≤ maxPixels」夹取（内存上限）。
 *  夹取只降位图分辨率、不改显示尺寸 —— 超大页面必然降分辨率，这就是要切片的原因。 */
export function clampBitmapSize(
  displayW: number,
  displayH: number,
  dpr: number,
  maxLong: number = PDF_BITMAP_MAX_LONG,
  maxPixels: number = PDF_BITMAP_MAX_PIXELS,
): { bitW: number; bitH: number; scale: number } {
  const w = Math.max(1, displayW);
  const h = Math.max(1, displayH);
  const r = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
  let s = r;
  const long = Math.max(w, h) * s;
  if (long > maxLong) s = maxLong / Math.max(w, h);
  const px = w * s * h * s;
  if (px > maxPixels) s = Math.sqrt(maxPixels / (w * h));
  return { bitW: Math.max(1, Math.round(w * s)), bitH: Math.max(1, Math.round(h * s)), scale: s };
}

/** 一页要切成几条：显示高超过 maxScreens×屏高才切，返回 [f0,f1] 分数区间（自上而下）。
 *  切片的意义：超长页整页渲染必被 maxPixels 压掉分辨率（11 屏高的页只剩 1/3 清晰度），
 *  切成若干「一屏多」的条后，每条都能按自己的显示尺寸拿到足量像素。 */
export function pdfSliceRanges(displayH: number, screenH: number, maxScreens: number = PDF_SLICE_MAX_SCREENS): [number, number][] {
  const h = Math.max(1, displayH);
  const sh = Number.isFinite(screenH) && screenH > 40 ? screenH : 800;
  const limit = Math.max(200, sh * Math.max(0.5, maxScreens));
  const n = Math.max(1, Math.ceil(h / limit));
  const out: [number, number][] = [];
  for (let i = 0; i < n; i++) out.push([i / n, (i + 1) / n]);
  return out;
}

/** PDF 自带目录（outline）的一项：page 为解析出的 0-based 页号，-1 表示解析失败。 */
export interface PdfOutlineItem {
  title: string;
  page: number;
  depth: number;
}

/** 把 PDF 自带目录的标题合并进「一页一项」的目录：命中页用 outline 标题与层级，
 *  其余页保持「第 N 页」。数组下标恒等于页号（阅读器与弹出窗都按数组下标导航），
 *  所以合并只改标题/缩进，不动 index 语义。 */
export function mergeOutlineTitles(
  toc: TocEntry[],
  outline: PdfOutlineItem[],
  label: (pageNo: number) => string,
): TocEntry[] {
  const byPage = new Map<number, { title: string; depth: number }>();
  for (const o of outline) {
    if (o.page >= 0 && o.page < toc.length && o.title) byPage.set(o.page, { title: o.title, depth: o.depth });
  }
  return toc.map((e, i) => {
    const hit = byPage.get(i);
    return hit
      ? { title: hit.title, index: e.index, chars: e.chars, depth: hit.depth }
      : { title: label(i + 1), index: e.index, chars: e.chars };
  });
}

/** 判断 pdf.js 抽出的文本能否用于划词/搜索/朗读（不可用就如实降级，绝不产出乱码数据）。
 *  判据来自实测：
 *   - 缺 /ToUnicode 的文件（15、Nginx.pdf）抽出文本里 **25% 是控制字符**，而真实文本为 0%；
 *   - 缺字形的文件会给出 U+FFFD 或私用区（U+E000–U+F8FF）码位；
 *   - 扫描版根本没有文字项（空）；
 *   - 最后一道是「主导文种」判据：把码位归入 11 个文种桶，要求最大桶占比 ≥ 50%
 *     或前两桶合计 ≥ 80%（乱码文本的码位往往散布在大量互不相干的文种里）。
 *     **不得**改用 CJK 占比阈值 —— 那会把俄语/希腊语/阿拉伯语等整类文档误杀。
 *  宁可误判为不可用（少一个功能）也不误判为可用（划出来的线、搜出来的命中全是垃圾）。 */
export function pdfTextUsable(text: string, ctrlMax = 0.02, badMax = 0.02): boolean {
  const t = (text || "").replace(/\s+/g, "");
  if (t.length < 8) return false;
  let ctrl = 0;
  let bad = 0;
  for (const ch of t) {
    const c = ch.codePointAt(0) ?? 0;
    if (c < 0x20 || (c >= 0x7f && c <= 0x9f)) ctrl++;
    if (c === 0xfffd || (c >= 0xe000 && c <= 0xf8ff)) bad++;
  }
  const n = t.length;
  if (ctrl / n > ctrlMax) return false;
  if (bad / n > badMax) return false;
  // 主导文种判据：**不得**再用「CJK 占比」阈值 —— 那会把俄语/希腊语/阿拉伯语等所有非中文
  // 文档误判为不可用。这里只要求「有一个（或两个字种合计）明显主导」；纯乱码通常是
  // 码位散布在大量互不相干的文种里，主导度很低。
  const buckets = new Map<number, number>();
  let scripted = 0;
  for (const ch of t) {
    const c = ch.codePointAt(0) ?? 0;
    const idx = scriptBucket(c);
    if (idx < 0) continue;
    scripted++;
    buckets.set(idx, (buckets.get(idx) ?? 0) + 1);
  }
  const top = [...buckets.values()].sort((a, b) => b - a);
  const top1 = (top[0] ?? 0) / Math.max(1, scripted);
  const top2 = (top1 + (top[1] ?? 0) / Math.max(1, scripted));
  if (top1 < 0.5 && top2 < 0.8) return false;
  return true;
}

/** 码位 → 文种桶（同一文种内不区分大小写/字母数字）；-1 = 不属于任何已列文种（含标点/符号）。
 *  只用于「主导度」判断，不做任何具体语种的等价判断。 */
export function scriptBucket(c: number): number {
  // 导出仅为单测（纯函数、无副作用）。
  if (c >= 0x41 && c <= 0x5a) return 0; // 拉丁大写 A-Z
  if (c >= 0x61 && c <= 0x7a) return 0; // 拉丁小写 a-z
  // 0x5B-0x60（[ \ ] ^ _ `）与 0x7B-0x7F 不是字母，落到末尾的 -1；原先 0x41..=0x7a 把这段也
  // 算进拉丁桶，纯标点/下划线的文本会被误判成「有拉丁内容」。
  if (c >= 0x30 && c <= 0x39) return 0; // 阿拉伯数字并入拉丁桶（纯数字代码/表格常见）
  if (c >= 0xc0 && c <= 0x24f) return 0; // 拉丁扩展
  if (c >= 0x370 && c <= 0x3ff) return 1; // 希腊
  if (c >= 0x400 && c <= 0x4ff) return 2; // 西里尔
  if (c >= 0x530 && c <= 0x58f) return 3; // 亚美尼亚
  if (c >= 0x590 && c <= 0x5ff) return 4; // 希伯来
  if (c >= 0x600 && c <= 0x6ff) return 5; // 阿拉伯
  if (c >= 0x900 && c <= 0x97f) return 6; // 天城文
  if (c >= 0xe00 && c <= 0xe7f) return 7; // 泰文
  if (c >= 0x3040 && c <= 0x30ff) return 8; // 假名
  if ((c >= 0x3400 && c <= 0x4dbf) || (c >= 0x4e00 && c <= 0x9fff)) return 9; // 汉字
  if (c >= 0xac00 && c <= 0xd7af) return 10; // 谚文
  return -1;
}

/** 用前端的页码文案重写 PDF 目录标题（保持 index/chars 不变）。
 *  后端 `pdf_page_toc` 给的「第 N 页」只是无前端时的兜底（Rust 侧不知道界面语言），
 *  凡是要把 toc 上屏的地方都必须过这一道：阅读器内目录与「弹出目录」独立窗口。 */
export function retitlePdfPageToc(toc: TocEntry[], label: (pageNo: number) => string): TocEntry[] {
  return toc.map((e) => ({ ...e, title: label(e.index + 1) }));
}

/** 用真实页数生成页码目录：一个 PDF 页即一个章节（index 从 0 连续、chars 恒 0）。
 *  label 由调用方传入（页码文案要跟随界面语言，不能写死在纯函数里）。 */
export function buildPageToc(pages: number, label: (pageNo: number) => string): TocEntry[] {
  const n = Number.isFinite(pages) ? Math.max(0, Math.floor(pages)) : 0;
  return Array.from({ length: n }, (_, i) => ({ title: label(i + 1), index: i, chars: 0 }));
}

/** 单页渲染失败时的占位 HTML：给一个固定高度的空块，既不阻断滚动，也不让窗口按 0 高无限外扩。 */
export function pdfPageErrorHtml(): string {
  return `<div class="pdf-page-error" style="height:60vh"></div>`;
}

/** 把一个 mark 的 client rect 列表换算成高亮叠层里的绝对色块（原点 = .pdf-page 的视口矩形）。
 *  为什么抽成纯函数：跨行 mark 会给出多个矩形，负坐标（滚出视口）与零/负尺寸都很常见，
 *  这些边界在 DOM 里不好构造，单独测更省事。负坐标保留（相对于页面原点本来就是负的），
 *  尺寸非正的矩形丢弃（画出来是空块，还会白占一个 DOM 节点）。 */
export function hlBlocksFromRects(
  pageRect: { left: number; top: number },
  rects: readonly { left: number; top: number; width: number; height: number }[],
  mergeGapPx = 1.5,
): { left: number; top: number; width: number; height: number }[] {
  const boxes: { left: number; top: number; width: number; height: number }[] = [];
  for (const r of rects) {
    if (!(r.width > 0) || !(r.height > 0)) continue;
    boxes.push({ left: r.left - pageRect.left, top: r.top - pageRect.top, width: r.width, height: r.height });
  }
  // 同一行的相邻/重叠矩形合并成一块：跨 span 的划线会切出几十个紧挨的小矩形，
  // 每个都带圆角 —— 交界处会露出「发丝缝」（荧光笔样式下最明显）。合并后一行一块，
  // 既无缝也少 DOM。判据：竖直重叠超过较矮者的 50%（同一行）且水平相接（gap ≤ 容差）。
  // 单趟：先按行分组，再在行内自左向右贪心合并。
  // 为什么改：原实现是 while(merged) + 双循环 + break 重扫，矩形上千时接近立方级。
  boxes.sort((a, b) => a.top - b.top || a.left - b.left);
  /** 「同一行」三条判据全过才算：高度比 ≥ 0.5、竖直重叠 ≥ 较矮者 50%、行顶差 ≤ 较矮者高 × 0.3。
   *  为什么必须带高度比：只按 minH 判重叠时，一个 100px 的跨行块会把同 top 的 12px 普通行并进去
   *  （12 ≥ 6 且 0 ≤ 3.6 都成立），结果是整块高 100px 的错误色块。
   *  另一条（行顶差）挡住双栏排版：左右两栏同一基线的碎片不该并成横跨整页的一块。
   *  取舍：大块与小行交界处不再合并，重绘时交界留约 1px 缝 —— 与「并成错误高块」相比，
   *  缝是观感问题、错误高块是内容遮挡，故选带高度比这一版。 */
  const sameLine = (rTop: number, rBottom: number, b: { top: number; height: number }): boolean => {
    const rH = rBottom - rTop;
    const minH = Math.min(rH, b.height);
    const maxH = Math.max(rH, b.height);
    const overlap = Math.min(rBottom, b.top + b.height) - Math.max(rTop, b.top);
    return (
      minH / Math.max(1, maxH) >= 0.5 && overlap >= minH * 0.5 && Math.abs(rTop - b.top) <= minH * 0.3
    );
  };
  const rows: { top: number; bottom: number; segs: typeof boxes }[] = [];
  for (const b of boxes) {
    let row: (typeof rows)[number] | null = null;
    for (let i = rows.length - 1; i >= 0; i--) {
      if (rows[i].top > b.top + b.height) continue; // 该行整体在新块下方，排序保证早于它出现
      if (sameLine(rows[i].top, rows[i].bottom, b)) {
        row = rows[i];
        break;
      }
    }
    if (!row) {
      rows.push({ top: b.top, bottom: b.top + b.height, segs: [b] });
      continue;
    }
    // 行内找可并入的段：排序是按 (top,left)，同一行里先到的未必在最左（例如行顶更小的
    // 上标先到），所以不能只跟「最后一段」比 —— 用对称的间距公式挑 gap 最小的那段。
    let best = -1;
    let bestGap = Number.POSITIVE_INFINITY;
    for (let i = 0; i < row.segs.length; i++) {
      const seg = row.segs[i];
      const gapX = Math.max(b.left, seg.left) - Math.min(b.left + b.width, seg.left + seg.width);
      if (gapX <= mergeGapPx && gapX < bestGap) {
        best = i;
        bestGap = gapX;
      }
    }
    if (best >= 0) {
      // 与行内某段相接/重叠：并入它（跨 span 的划线会切出几十个紧挨的小矩形）
      const seg = row.segs[best];
      const left = Math.min(seg.left, b.left);
      const top = Math.min(seg.top, b.top);
      const right = Math.max(seg.left + seg.width, b.left + b.width);
      const bottom = Math.max(seg.top + seg.height, b.top + b.height);
      row.segs[best] = { left, top, width: right - left, height: bottom - top };
    } else {
      row.segs.push(b);
    }
    if (b.top < row.top) row.top = b.top;
    if (b.top + b.height > row.bottom) row.bottom = b.top + b.height;
  }
  const out = rows.flatMap((r) => r.segs);
  out.sort((a, b) => a.top - b.top || a.left - b.left);
  return out;
}

/** 整页图（图库）缓存的小 LRU 步进：给定现有顺序（旧→新）、本次要用的 token 与容量，
 *  返回新顺序与本次被淘汰的 token（旧→新，可能一次淘汰多个）。
 *  为什么抽成纯函数：淘汰是「revoke 掉 blob」的前置判定，最容易写错（重复插入、命中后变旧、
 *  容量为 0），单独测比在 pdf.ts 里边渲染边观察便宜得多。
 *  命中会刷新为最新，其余元素相对顺序不变 —— 这才是 LRU；图库来回翻页时不会把刚看过的页淘汰掉。 */
export function fullPageLruStep(
  order: readonly string[],
  token: string,
  capacity: number,
): { order: string[]; evicted: string[] } {
  const cap = Math.max(0, Math.floor(capacity));
  const next = order.filter((t) => t !== token);
  next.push(token);
  const evicted: string[] = [];
  while (next.length > cap) evicted.push(next.shift() as string);
  return { order: next, evicted };
}
