import type { TocEntry } from "./types";

// PDF 渲染的纯函数层：页面显示尺寸、切片划分、位图分辨率夹取、页码目录。
// 与 pdf.js 运行时（pdf.ts）分开，既好单测，也避免测试环境去加载 pdf.js。

/** PDF 用户单位（pt）→ CSS px 的换算：PDF 以 72dpi 计量，CSS 以 96dpi 计量 → 4/3。
 *  这是「页面按真实尺寸呈现」的基准：A4（595×842pt）→ 793×1123 CSS px。 */
export const PDF_PX_PER_PT = 4 / 3;

/** 位图长边上限（物理像素）：单张位图的内存上限（4096² × 4B ≈ 67MB 已是极限）。 */
export const PDF_BITMAP_MAX_LONG = 4096;
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
    k = scale;
    // 纵向滚动下不能比容器还宽：否则右边被裁掉（overflow-x: hidden）
    const maxK = cw / baseW;
    if (k > maxK) k = maxK;
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
 *   - 非 ASCII 占多数时（中日韩文档正常）还要求其中 CJK/假名/谚文/全角标点占比够高，
 *     否则视为「码位看似合法但并非可读文字」。
 *  宁可误判为不可用（少一个功能）也不误判为可用（划出来的线、搜出来的命中全是垃圾）。 */
export function pdfTextUsable(text: string, ctrlMax = 0.02, badMax = 0.02): boolean {
  const t = (text || "").replace(/\s+/g, "");
  if (t.length < 8) return false;
  let ctrl = 0;
  let bad = 0;
  let nonAscii = 0;
  let cjkish = 0;
  for (const ch of t) {
    const c = ch.codePointAt(0) ?? 0;
    if (c < 0x20 || (c >= 0x7f && c <= 0x9f)) ctrl++;
    if (c === 0xfffd || (c >= 0xe000 && c <= 0xf8ff)) bad++;
    if (c > 0x7f) {
      nonAscii++;
      const isCjk =
        (c >= 0x4e00 && c <= 0x9fff) ||
        (c >= 0x3400 && c <= 0x4dbf) ||
        (c >= 0x3040 && c <= 0x30ff) ||
        (c >= 0xac00 && c <= 0xd7af) ||
        (c >= 0x3000 && c <= 0x303f) ||
        (c >= 0xff00 && c <= 0xffef);
      if (isCjk) cjkish++;
    }
  }
  const n = t.length;
  if (ctrl / n > ctrlMax) return false;
  if (bad / n > badMax) return false;
  if (nonAscii / n > 0.6 && cjkish / Math.max(1, nonAscii) < 0.5) return false;
  return true;
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
