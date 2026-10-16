import { getDocument, GlobalWorkerOptions, TextLayer, type PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import workerSrc from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { api } from "./api";
import { pdfTextUsable, type PageSize, type PdfOutlineItem } from "./pdfPage";

// worker 指向 Vite 打出来的同源 URL：只设一次，不能每本书重设。
GlobalWorkerOptions.workerSrc = workerSrc;

// 所有权：**PDF 文档与位图（blob URL）的回收权只在这里**。阅读器只通过 pinSlices 告诉它
// 「哪些切片还贴着视口」，其余按 LRU 回收。避免两处各自 revoke 同一个 URL 把在屏内容弄白。

/** 同一时刻只保留一本 PDF 的文档（换书先释放上一本）。 */
interface PdfSession {
  bookId: number;
  doc: PDFDocumentProxy;
}

/** 未被钉住的位图上限（张）：一张普通页位图约 4~16MB，6 张 ≈ 上限百 MB 级。 */
const PAGE_CACHE_MAX = 6;
/** 渲染并发上限：pdf.js 单条切片很贵（数百毫秒 + 数 MB 位图），并发会瞬间吃满内存与主线程。 */
const RENDER_CONCURRENCY = 2;

let session: PdfSession | null = null;
let sessionBookId = -1;
let sessionPromise: Promise<PdfSession> | null = null;
/** 换代计数：换书/关书时 +1，用来丢弃上一代在途的渲染结果（否则旧书位图会塞进新书）。 */
let generation = 0;

interface BitmapEntry {
  url: string;
  page: number;
}
/** 位图缓存：token → 位图。Map 的插入顺序即 LRU 顺序。 */
const bitmaps = new Map<string, BitmapEntry>();
/** 仍贴着视口的切片 token：绝不回收。 */
const pinnedTokens = new Set<string>();
/** 页尺寸（pt）缓存：算显示尺寸要用，换书清空。 */
const geometry = new Map<number, PageSize>();

let active = 0;
const queue: (() => void)[] = [];

/** 渲染排队（全局并发上限）。排队中的任务在换书后仍会执行完，但结果会被换代计数丢弃。 */
function withSlot<T>(fn: () => Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const run = () => {
      active++;
      let p: Promise<T>;
      try {
        p = fn();
      } catch (e) {
        // 同步抛错也必须归还并发槽，否则队列会永久卡死（当前调用点都是 async 函数，属防御性）。
        active--;
        const next = queue.shift();
        if (next) next();
        reject(e);
        return;
      }
      p.then(resolve, reject).finally(() => {
        active--;
        const next = queue.shift();
        if (next) next();
      });
    };
    if (active < RENDER_CONCURRENCY) run();
    else queue.push(run);
  });
}

function revoke(url: string) {
  try {
    URL.revokeObjectURL(url);
  } catch {
    /* 已失效的 URL 忽略 */
  }
}

/** 缓存 token：页号 + 条号 + 位图高 + 起始分数。**必须含渲染尺寸**，否则缩放后会命中旧分辨率的位图。 */
export function sliceToken(page: number, slice: number, bitH: number, f0: number): string {
  return `${page}#${slice}@${bitH}@${Math.round(f0 * 10000)}`;
}

/** 回收未被钉住的位图，直到数量落回上限（最旧的先回收，绝不会回收刚放进来的那张）。 */
function sweep() {
  let unpinned = 0;
  for (const t of bitmaps.keys()) if (!pinnedTokens.has(t)) unpinned++;
  if (unpinned <= PAGE_CACHE_MAX) return;
  for (const [t, e] of [...bitmaps]) {
    if (unpinned <= PAGE_CACHE_MAX) break;
    if (pinnedTokens.has(t)) continue;
    bitmaps.delete(t);
    revoke(e.url);
    unpinned--;
  }
}

/** 登记「哪些切片贴着视口」：被登记切片的位图只归阅读器的视口判定决定生死。 */
export function pinSlices(bookId: number, tokens: string[]): void {
  if (!session || session.bookId !== bookId) return;
  pinnedTokens.clear();
  for (const t of tokens) if (bitmaps.has(t)) pinnedTokens.add(t);
  sweep();
}

/** 取当前这本书的 PDF 文档（换书时先释放上一本的文档与全部位图）。 */
export function ensurePdf(bookId: number): Promise<PdfSession> {
  if (session && session.bookId === bookId) return Promise.resolve(session);
  if (sessionPromise && sessionBookId === bookId) return sessionPromise;
  releaseAll();
  sessionBookId = bookId;
  const gen = generation;
  const p = (async (): Promise<PdfSession> => {
    const buf = await api.readBookBytes(bookId);
    const task = getDocument({
      data: new Uint8Array(buf),
      cMapUrl: "/pdfjs/cmaps/",
      cMapPacked: true,
      standardFontDataUrl: "/pdfjs/standard_fonts/",
      // pdf.js 默认用 eval 加速，CSP 不允许；这里也没有收益。
      isEvalSupported: false,
    });
    const doc = await task.promise;
    // 等待期间换了书：这份文档直接销毁，别留下无人引用的 worker。
    if (gen !== generation) {
      await doc.destroy().catch(() => {});
      throw new Error("已切换书籍");
    }
    session = { bookId, doc };
    return session;
  })();
  sessionPromise = p;
  p.catch(() => {
    if (sessionPromise === p) sessionPromise = null;
  });
  return p;
}

/** 页尺寸（pt）：显示尺寸与切片划分都由它算出来。 */
export async function pageGeometry(bookId: number, page: number): Promise<PageSize> {
  const hit = geometry.get(page);
  if (hit) return hit;
  const { doc } = await ensurePdf(bookId);
  const p = await doc.getPage(page + 1);
  try {
    const vp = p.getViewport({ scale: 1 });
    const size: PageSize = { ptW: vp.width, ptH: vp.height };
    geometry.set(page, size);
    return size;
  } finally {
    p.cleanup();
  }
}

export interface SliceRequest {
  /** 0-based 页号。 */
  page: number;
  /** 该页内的条序号（0..n-1），只作缓存键的一部分。 */
  slice: number;
  /** 该条在页内的纵向起止分数（0..1，自上而下）。 */
  f0: number;
  f1: number;
  /** 该条位图高度（物理像素）。宽度由页宽比推出。 */
  bitH: number;
}

export interface SliceBitmap {
  url: string;
  token: string;
}

/** 渲染「一页的一条」为位图：位图分辨率由调用方按「显示尺寸 × DPR」算好（见 pdfPage.clampBitmapSize）。
 *  超长页只渲染当前贴着视口的那几条，整页不会被一次性画出来（否则必被像素上限压掉分辨率）。 */
export async function renderPageSlice(bookId: number, req: SliceRequest): Promise<SliceBitmap> {
  const { doc } = await ensurePdf(bookId);
  const gen = generation;
  const token = sliceToken(req.page, req.slice, req.bitH, req.f0);
  const hit = bitmaps.get(token);
  if (hit) {
    bitmaps.delete(token);
    bitmaps.set(token, hit);
    // 不变式：只要有人正在要这一条（视口要展示它），就不许被判成「未钉住」而回收。
    pinnedTokens.add(token);
    return { url: hit.url, token };
  }
  return withSlot(async () => {
    const again = bitmaps.get(token);
    if (again) {
      bitmaps.delete(token);
      bitmaps.set(token, again);
      pinnedTokens.add(token);
      return { url: again.url, token };
    }
    const page = await doc.getPage(req.page + 1);
    try {
      const full = page.getViewport({ scale: 1 });
      const bandPt = Math.max(0.5, full.height * (req.f1 - req.f0));
      const scale = Math.max(0.01, req.bitH / bandPt);
      const viewport = page.getViewport({ scale });
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(viewport.width));
      canvas.height = Math.max(1, Math.round(req.bitH));
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("canvas 上下文不可用");
      // transform 把整页内容上移 f0 段（坐标系是缩放后的 viewport 坐标），这样只画本条。
      const ty = -req.f0 * viewport.height;
      await page.render({ canvasContext: ctx, viewport, transform: ty !== 0 ? [1, 0, 0, 1, 0, ty] : undefined }).promise;
      const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
      if (!blob) throw new Error("页面位图生成失败");
      const url = URL.createObjectURL(blob);
      if (gen !== generation) {
        revoke(url);
        throw new Error("已切换书籍");
      }
      bitmaps.set(token, { url, page: req.page });
      // 渲染刚完成、马上要进 DOM：先钉住再回收判定，杜绝「进 DOM 前被并发的 sweep 收掉」。
      pinnedTokens.add(token);
      sweep();
      return { url, token };
    } finally {
      page.cleanup();
    }
  });
}

/** 一页里的可点链接：坐标已归一化到 [0,1]（PDF 原点在左下，这里换算成「从上往下」的分数），
 *  阅读器再乘显示尺寸即可得到热点位置，缩放/切片重建时天然重算。 */
export interface PdfLink {
  url: string | null;
  /** dest 解析出的目标页（0-based），-1 表示无/解析失败。 */
  page: number;
  nx: number;
  ny: number;
  nw: number;
  nh: number;
}

/** 取一页的链接注解（subtype=Link）：url 走外链、dest 走跳页。单条解析失败只丢该条。 */
export async function pageLinks(bookId: number, page: number): Promise<PdfLink[]> {
  const { doc } = await ensurePdf(bookId);
  const p = await doc.getPage(page + 1);
  try {
    const vp = p.getViewport({ scale: 1 });
    const ptW = Math.max(1, vp.width);
    const ptH = Math.max(1, vp.height);
    let anns: unknown[] = [];
    try {
      anns = (await p.getAnnotations({ intent: "display" })) as unknown[];
    } catch {
      return [];
    }
    const out: PdfLink[] = [];
    for (const a of anns) {
      const an = a as { subtype?: string; url?: string; dest?: unknown; rect?: number[] };
      if (an.subtype !== "Link") continue;
      if (!Array.isArray(an.rect) || an.rect.length < 4) continue;
      let destPage = -1;
      if (an.dest != null) {
        try {
          // 用新常量接收：直接给 d 赋值会被 typeof 收窄成 string，导致 any[]|null 赋不进去
          const rawDest = an.dest;
          const d: unknown = typeof rawDest === "string" ? await doc.getDestination(rawDest) : rawDest;
          if (Array.isArray(d) && d.length) {
            const idx = await doc.getPageIndex(d[0] as Parameters<typeof doc.getPageIndex>[0]);
            if (typeof idx === "number") destPage = idx;
          }
        } catch {
          destPage = -1;
        }
      }
      const [x1, y1, x2, y2] = an.rect;
      const left = Math.min(x1, x2);
      const right = Math.max(x1, x2);
      const bottom = Math.min(y1, y2);
      const top = Math.max(y1, y2);
      // pdf.js 把非 http(s)/可疑链接放在 unsafeUrl（不放进 url）：只在确认是 http/https 时才采用，
      // 免得 javascript:/data: 之类的链接被我们当成外链打开（实测 Nginx.pdf 有 4 条走 unsafeUrl）。
      const rawUrl = typeof an.url === "string" && an.url ? an.url : typeof (an as { unsafeUrl?: string }).unsafeUrl === "string" ? (an as { unsafeUrl?: string }).unsafeUrl! : "";
      const safeUrl = /^https?:\/\//i.test(rawUrl) ? rawUrl : null;
      out.push({
        url: safeUrl,
        page: destPage,
        nx: left / ptW,
        ny: (ptH - top) / ptH,
        nw: (right - left) / ptW,
        nh: (top - bottom) / ptH,
      });
    }
    return out;
  } finally {
    p.cleanup();
  }
}

/** 读 PDF 自带目录（outline），并把 dest（可能是数组或字符串名）解析成 0-based 页号。
 *  解析失败的单条保留标题但 page=-1（调用方跳过），绝不因此整份目录失败。 */
export async function docOutline(bookId: number): Promise<PdfOutlineItem[]> {
  const { doc } = await ensurePdf(bookId);
  let raw: unknown = null;
  try {
    raw = await doc.getOutline();
  } catch {
    return [];
  }
  if (!Array.isArray(raw) || !raw.length) return [];
  const out: PdfOutlineItem[] = [];
  const walk = async (items: unknown[], depth: number): Promise<void> => {
    for (const it of items) {
      const node = it as { title?: unknown; dest?: unknown; items?: unknown };
      let page = -1;
      try {
        let dest = node.dest;
        if (typeof dest === "string") dest = await doc.getDestination(dest);
        if (Array.isArray(dest) && dest.length) {
          const idx = await doc.getPageIndex(dest[0] as Parameters<typeof doc.getPageIndex>[0]);
          if (typeof idx === "number") page = idx;
        }
      } catch {
        page = -1;
      }
      out.push({ title: String(node.title ?? ""), page, depth });
      if (Array.isArray(node.items) && node.items.length) await walk(node.items, depth + 1);
    }
  };
  await walk(raw, 0);
  return out;
}

export interface TextLayerResult {
  /** 该页文本是否可用于划词/搜索/朗读（乱码/扫描版为 false）。 */
  usable: boolean;
  /** 该页纯文本（不可用时也返回原始抽取结果，供诊断）。 */
  text: string;
}

/** 给一页装透明文本层：文本层与位图同一坐标系（宽度 = 显示宽），文字透明、只用于选取/划线。
 *  scale 取「显示宽 / 页宽(pt)」——与位图的显示尺寸同源，所以选中框与画面严格对齐。
 *  返回纯文本与可用性：不可用（扫描版/乱码）时**不往 DOM 放文本层**，避免乱码进入选区与搜索。 */
export async function buildTextLayer(
  bookId: number,
  page: number,
  host: HTMLElement,
  opts: { displayW: number; ptW: number },
): Promise<TextLayerResult> {
  const { doc } = await ensurePdf(bookId);
  const gen = generation;
  const p = await doc.getPage(page + 1);
  try {
    const scale = opts.ptW > 0 ? Math.max(0.01, opts.displayW / opts.ptW) : 1;
    const viewport = p.getViewport({ scale });
    const tc = await p.getTextContent();
    const text = tc.items.map((it) => ("str" in it ? String(it.str) : "")).join("");
    const usable = pdfTextUsable(text);
    // 等待期间换了书：别把上一页的文本层塞进新书
    if (gen !== generation) throw new Error("已切换书籍");
    host.textContent = "";
    host.style.setProperty("--scale-factor", String(scale));
    if (usable) {
      const layer = new TextLayer({ textContentSource: tc, container: host, viewport });
      await layer.render();
      if (gen !== generation) {
        host.textContent = "";
        throw new Error("已切换书籍");
      }
    }
    return { usable, text };
  } finally {
    p.cleanup();
  }
}

/** 整页位图（图库用）：按长边 ≤ maxLong 渲染，独立于切片缓存（图库看的是整页，不是切片）。
 *  仍登记进位图表（合成 token）→ 关书/换书时一并 revoke，不留悬挂 blob。 */
export async function renderPageImageFull(bookId: number, page: number, maxLong = 2000): Promise<string> {
  const { doc } = await ensurePdf(bookId);
  const gen = generation;
  const token = `full#${page}@${maxLong}`;
  const hit = bitmaps.get(token);
  if (hit) return hit.url;
  return withSlot(async () => {
    const p = await doc.getPage(page + 1);
    try {
      const base = p.getViewport({ scale: 1 });
      const scale = Math.max(0.05, maxLong / Math.max(base.width, base.height));
      const viewport = p.getViewport({ scale });
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(viewport.width));
      canvas.height = Math.max(1, Math.round(viewport.height));
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("canvas 上下文不可用");
      await p.render({ canvasContext: ctx, viewport }).promise;
      const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
      if (!blob) throw new Error("整页位图生成失败");
      const url = URL.createObjectURL(blob);
      if (gen !== generation) {
        revoke(url);
        throw new Error("已切换书籍");
      }
      bitmaps.set(token, { url, page });
      pinnedTokens.add(token); // 图库正在看它：不参与 LRU 淘汰
      sweep();
      return url;
    } finally {
      p.cleanup();
    }
  });
}

/** 渲染第 1 页为低分辨率 PNG（data URL），供「首次打开生成封面」用。
 *  独立于切片缓存（分辨率不同、只渲染一次），失败时抛错由调用方静默处理。 */
export async function renderCoverPng(bookId: number, maxWidth = 800): Promise<string> {
  const { doc } = await ensurePdf(bookId);
  const gen = generation;
  return withSlot(async () => {
    const page = await doc.getPage(1);
    try {
      const base = page.getViewport({ scale: 1 });
      const viewport = page.getViewport({ scale: Math.max(0.1, maxWidth / Math.max(1, base.width)) });
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(viewport.width));
      canvas.height = Math.max(1, Math.round(viewport.height));
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("canvas 上下文不可用");
      await page.render({ canvasContext: ctx, viewport }).promise;
      const url = canvas.toDataURL("image/png");
      if (gen !== generation) throw new Error("已切换书籍");
      return url;
    } finally {
      page.cleanup();
    }
  });
}

/** 释放当前 PDF：回收全部位图、销毁文档（关书/换书/组件销毁时调用；非 PDF 时是空操作）。 */
export function releaseAll(): void {
  generation++;
  for (const e of bitmaps.values()) revoke(e.url);
  bitmaps.clear();
  pinnedTokens.clear();
  geometry.clear();
  const s = session;
  session = null;
  sessionPromise = null;
  sessionBookId = -1;
  if (s) void s.doc.destroy().catch(() => {});
}
