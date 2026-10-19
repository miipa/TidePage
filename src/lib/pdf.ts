import { getDocument, GlobalWorkerOptions, PDFDataRangeTransport, TextLayer, type PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import workerSrc from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { api } from "./api";
import { fullPageLruStep, pdfTextUsable, type PageSize, type PdfOutlineItem } from "./pdfPage";

// worker 指向 Vite 打出来的同源 URL：只设一次，不能每本书重设。
GlobalWorkerOptions.workerSrc = workerSrc;

// 所有权：**PDF 文档与位图（blob URL）的回收权只在这里**。阅读器只通过 pinSlices 告诉它
// 「哪些切片还贴着视口」，其余按 LRU 回收。避免两处各自 revoke 同一个 URL 把在屏内容弄白。

/** 同一时刻只保留一本 PDF 的文档（换书先释放上一本）。 */
interface PdfSession {
  bookId: number;
  doc: PDFDocumentProxy;
  /** 走 range 分块时保留传输对象：换书/关书要 abort 掉在途请求。 */
  range?: PDFDataRangeTransport;
  /** true = 这本书走系统原生渲染（pdf.js 打不开这种体积，见 PDF_NATIVE_MIN_BYTES）。
   *  此时 doc 只是「页数占位」，不能拿它调 pdf.js 的任何方法。 */
  native: boolean;
}

/** 未被钉住的位图上限（张）：一张普通页位图约 4~16MB，6 张 ≈ 上限百 MB 级。 */
const PAGE_CACHE_MAX = 6;
/** 整页图（图库）缓存上限（张）：图库一次最多看「当前页 ±1」，3 张正好；
 *  整页图比切片大得多（单张可达 ~48MB），绝不能像切片那样按 6 张常驻。 */
const FULL_PAGE_CACHE_MAX = 3;
/** 渲染并发上限：pdf.js 单条切片很贵（数百毫秒 + 数 MB 位图），并发会瞬间吃满内存与主线程。 */
/** 渲染并发（同时最多 2 条切片在渲染）。导出给 Reader 做「每帧最多新发 2 条」的限流同一口径。 */
export const RENDER_CONCURRENCY = 2;

/** range 路径的打开超时：pdf.js 的断言被吞掉时表现为「不报错也不成功」，必须自己兜住。 */
const PDF_RANGE_OPEN_TIMEOUT =
  // DEV 专用：置 window.__pdfForceRangeTimeout1ms = true 可把超时压到 1ms，
  // 用来验证「range 失败 → 回退整读」这条防护真的生效。生产构建里 import.meta.env.DEV 为 false，
  // 整个表达式被摇掉（构建后 grep __pdfForceRangeTimeout1ms 应为 0 命中）。
  import.meta.env.DEV && (globalThis as { __pdfForceRangeTimeout1ms?: boolean }).__pdfForceRangeTimeout1ms
    ? 1
    : 20_000;
/** 给 Promise 加超时（range 路径最怕无限挂起，超时后要能退化整读）。 */
function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((res, rej) => {
    const t = setTimeout(() => rej(new Error(`打开超时（${ms}ms）`)), ms);
    p.then(
      (v) => { clearTimeout(t); res(v); },
      (e) => { clearTimeout(t); rej(e); },
    );
  });
}

/** 小于这个大小就整份读入（一次 IPC 换最少往返）；大于等于则走 range 分块。 */
const PDF_RANGE_MIN_BYTES = 8 * 1024 * 1024;
/** range 打开失败时允许「回退整份读入」的**文件大小上限**。
 *  为什么必须有：range 路径的兜底逻辑会把整个文件经 IPC 读进内存，而真实书库里有
 *  3.75GB 的扫描 PDF（书库总量 518GB，单册中位数几十 MB）。没有这道闸，一次回退就是
 *  数 GB 的分配 —— 实测启动期补封面时把宿主工作集推到 6.8GB 后崩溃（0xE0000008）。
 *  超限时如实报「打不开」，绝不拿内存换一次侥幸。 */
const PDF_FULL_READ_MAX = 96 * 1024 * 1024;
/** 带稳定错误码的报错：用户可见文案一律由 i18n 的错误码表（ERR_CODES）给出，本文件不写中文。
 *  为什么这样：pdf.ts 不是组件，拿不到 t()；错误码 + params 是既有机制（见 i18n/index）。
 *  params 由 errText 从错误对象上读取，用于填 {mb} 这类动态值。 */
function pdfError(code: string, params?: Record<string, string | number>): Error {
  const e = new Error(code) as Error & { code?: string; params?: Record<string, string | number> };
  e.code = code;
  e.params = params;
  return e;
}

/** 首个分块：pdf.js 需要先拿到头部才能开始解析（64KB 足够覆盖 xref 头）。 */
const PDF_RANGE_FIRST = 64 * 1024;
/** 本运行时（WebView2 / Chromium 渲染进程）单次 ArrayBuffer 分配的硬上限：2GiB-1 字节。
 *  为什么必须知道：pdf.js 走 range 路径时用的是 ChunkedStream，它在构造时就
 *  `new Uint8Array(length)`（pdf.worker.mjs 的 ChunkedStream 构造函数）——**按整份文件长度
 *  要一块连续缓冲**，跟 disableStream / disableAutoFetch 无关；超过上限就抛
 *  RangeError: Array buffer allocation failed，且在抛之前一个区间请求都不会发。
 *  实测（同一台机器、同一 WebView2）：1.75GiB 分配成功、2.00GiB 失败；
 *  8MB/40MB/253MB/1.34GiB 的 PDF 都能正常打开，3.31GiB/3.50GiB 必失败。
 *  所以超过这个长度的 PDF 在本运行时里无法打开，不是开关问题，提前如实说明。 */
// PDF_SINGLE_BUFFER_MAX = 2³¹−1（2GiB−1，本运行时单次 ArrayBuffer 上限）—— 只剩注释里的说明，
// 代码里没有对应检查：>1.9GiB 一律走原生通道，pdf.js 分支永远见不到超过它的文件。
// 不要据此再写回「len > 2GiB−1 就报错」这类不可达检查（历史上写过一次，已删）。
/** 超过这个体积改走系统原生渲染（Rust pdfnative.rs）：pdf.js 的 range 路径用 ChunkedStream，
 *  构造时就按整份长度 new Uint8Array(length)，而 WebView2 单次 ArrayBuffer 上限是 2³¹−1 字节
 *  （实测 1.75GiB 成功、2.00GiB 失败）。阈值取 1.9GiB：那块缓冲之外，pdf.js 还要给页面对象
 *  与画布留空间。原生通道按页渲染，不需要整份物化。 */
const PDF_NATIVE_MIN_BYTES = Math.round(1.9 * 1024 * 1024 * 1024);
/** 原生文档句柄（页数；页尺寸走 geometry 缓存）。同一时刻只留一本，换书即关。 */
let nativeSession: { bookId: number; pages: number } | null = null;

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
/** 整页图（full# 前缀）token 的 LRU 顺序（旧 → 新）。与 bitmaps 分开存：同一张表里还混着切片，
 *  Map 的插入顺序没法同时表达两套淘汰策略。 */
let fullOrder: string[] = [];
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
  // 只重建「切片」钉子，保留 full# 前缀的整页图钉子：否则这里一 clear 就把图库正在显示的那张
  // 整页图丢了钉子，sweep() 会把它 revoke 掉 —— 图库中途白屏。
  for (const t of [...pinnedTokens]) {
    if (!t.startsWith("full#")) pinnedTokens.delete(t);
  }
  for (const t of tokens) if (bitmaps.has(t)) pinnedTokens.add(t);
  sweep();
}

/** 回收**全部**整页图（图库关闭时调用）：revoke 并从缓存删除，同时清空 LRU 顺序。
 *  尺寸修正后单张可达 ~48MB，不能像切片那样等 LRU 慢慢淘汰，否则每关一次图库都常驻几张。 */
export function releaseFullPage(): void {
  for (const [token, b] of [...bitmaps]) {
    if (!token.startsWith("full#")) continue;
    revoke(b.url);
    bitmaps.delete(token);
    pinnedTokens.delete(token);
  }
  fullOrder = [];
}

/** 精确遗弃一张整页图（按 URL）：图库关闭后「迟到」的渲染结果用它清理。
 *  为什么不用 releaseFullPage：那会把用户可能已经重新打开的那几页一起 revoke 掉（白屏）。 */
export function releaseFullPageUrl(url: string): void {
  for (const [token, b] of [...bitmaps]) {
    if (b.url !== url) continue;
    revoke(b.url);
    bitmaps.delete(token);
    pinnedTokens.delete(token);
    fullOrder = fullOrder.filter((t) => t !== token);
  }
}

/** 该整页图 URL 是否仍在缓存里（没被小 LRU 淘汰、也没被关闭回收）。
 *  图库清单只能信它：只看「URL 非空」会把已被 revoke 的 URL 当成可用图，切回去就是破图。 */
export function isFullPageCached(url: string): boolean {
  for (const b of bitmaps.values()) if (b.url === url) return true;
  return false;
}

/** 整页图缓存现状（token，旧→新）。供图库诊断/自动验证读取「整页图存活数」——
 *  光看 createObjectURL/revokeObjectURL 计数分不出切片与整页图，而容量约束只管整页图。 */
export function fullPageCacheTokens(): string[] {
  return fullOrder.filter((t) => bitmaps.has(t));
}

/** 取当前这本书的 PDF 文档（换书时先释放上一本的文档与全部位图）。 */
export function ensurePdf(bookId: number): Promise<PdfSession> {
  if (session && session.bookId === bookId) return Promise.resolve(session);
  if (sessionPromise && sessionBookId === bookId) return sessionPromise;
  releaseAll();
  sessionBookId = bookId;
  const gen = generation;
  const p = (async (): Promise<PdfSession> => {
    // 小文件整读（往返少），大文件走 range 分块：pdf.js 只拉它真正需要的字节。
    // 注意：pdf.js 的 ChunkedStream 仍会按整份长度预留一块连续缓冲（pdf.worker.mjs:6406 的
    // new Uint8Array(length)）；看起来「内存很低」是操作系统把未触碰的页推迟提交 ——
    // range 省的是实际驻留与读取量，不是「不再按文件大小分配」。
    const len = await api.pdfFileSize(bookId);
    // 超大文件：pdf.js 在本运行时无法表示，改走系统原生渲染（按页出图，整份不物化）。
    if (len > PDF_NATIVE_MIN_BYTES) {
      let pages: number;
      try {
        pages = await api.pdfNativeOpen(bookId);
      } catch (e) {
        // 系统渲染不可用（缺组件/文件损坏）时给出准确说明，不把底层错误抛给用户。
        if (import.meta.env.DEV) console.warn("[pdf] 原生渲染打开失败：", e);
        // 32 位构建/非 Windows 平台：与「文件太大」区分开，文案见 ERR_CODES。
        if (String(e).includes("pdf_native_unsupported")) throw pdfError("pdf_native_unsupported");
        // 废弃线程到上限（连续超时）：与「平台不支持」分开说明，文案里给出下一步。
        if (String(e).includes("pdf_native_abandoned")) throw pdfError("pdf_native_abandoned");
        throw pdfError("pdf_native_failed");
      }
      if (gen !== generation) {
        void api.pdfNativeClose().catch(() => {});
        throw new Error("已切换书籍");
      }
      nativeSession = { bookId, pages };
      const ns: PdfSession = {
        bookId,
        // 调用方只读 numPages（见 Reader 的目录构建），给一个最小形状即可。
        doc: { numPages: pages } as unknown as PDFDocumentProxy,
        native: true,
      };
      session = ns;
      return ns;
    }
    const opts: Record<string, unknown> = {
      cMapUrl: "/pdfjs/cmaps/",
      cMapPacked: true,
      standardFontDataUrl: "/pdfjs/standard_fonts/",
      // pdf.js 默认用 eval 加速，CSP 不允许；这里也没有收益。
      isEvalSupported: false,
    };
    let range: PDFDataRangeTransport | null = null;
    // 到这里必有 len <= PDF_NATIVE_MIN_BYTES(1.9GiB)；而 1.9GiB < 2³¹−1（运行时单次 ArrayBuffer 上限），
    // 所以 pdf.js 分支永远见不到「超过单次缓冲上限」的文件 —— 原先这里那道不可达检查已删。
    if (len >= PDF_RANGE_MIN_BYTES) {
      // 首块必须**在构造之前**读出来交给 initialData。手工 rt.onDataRange(0, head) 会走进
      // pdf.js 的断言：_onReceiveData 要求存在 _begin === begin 的 range reader，而首块走的是
      // getFullReader 那条路（不是 range reader）→ 断言抛错 → 被 catch 吞掉 → 全读流永远收不到
      // 数据 → task.promise 永久挂起（表现就是「没报错、没日志、一直打不开」）。
      const head = await api.pdfReadRange(bookId, 0, Math.min(PDF_RANGE_FIRST, len));
      range = new PDFDataRangeTransport(len, new Uint8Array(head), false);
      const rt = range;
      // 失败重试一次：range 请求偶发失败会让 pdf.js 直接报损坏，重试一次能挡住抖动
      const fetchRange = async (begin: number, end: number, retry: boolean) => {
        try {
          const buf = await api.pdfReadRange(bookId, begin, end);
          if (gen !== generation) return; // 换书后到达的分块直接丢
          rt.onDataRange(begin, new Uint8Array(buf));
        } catch (e) {
          if (retry && gen === generation) {
            await fetchRange(begin, end, false);
          } else {
            throw e;
          }
        }
      };
      rt.requestDataRange = (begin: number, end: number) => {
        void fetchRange(begin, end, true).catch(() => {});
      };
      opts.range = rt;
      opts.length = len;
      // 数据源支持按需区间读取，所以 disableStream 用 false（= 支持流式），交给 pdf.js 正常选择
      // NetworkPdfManager；disableAutoFetch 保持 true：只关掉「后台预取剩余区块」，不影响按需取块。
      // 注意：>2GiB 的 PDF 打不开与这两个开关无关 —— 卡点是上面那块「整份长度」的连续缓冲
      // （2³¹−1 是运行时单次 ArrayBuffer 上限），而 >1.9GiB 的文件已被原生通道接走。
      opts.disableStream = false;
      opts.disableAutoFetch = true;
    } else {
      const buf = await api.readBookBytes(bookId);
      opts.data = new Uint8Array(buf);
    }
    // range 只负责「更快打开 + 更低内存」；它一旦失败/超时，绝不能让用户「打不开这本书」，
    // 所以这里兜一层：range 异常即 abort 掉传输并退回整读重开，只有两条路都失败才报错。
    let task = getDocument(opts as Parameters<typeof getDocument>[0]);
    let doc: PDFDocumentProxy;
    try {
      doc = range ? await withTimeout(task.promise, PDF_RANGE_OPEN_TIMEOUT) : await task.promise;
    } catch (e) {
      if (!range) throw e;
      range.abort();
      range = null;
      task.destroy().catch(() => {});
      if (import.meta.env.DEV) console.warn("[pdf] range 打开失败，回退整份读入：", e);
      // 超大文件不回退整读：宁可打不开，也不把几 GB 拉进内存（见 PDF_FULL_READ_MAX 注释）。
      if (len > PDF_FULL_READ_MAX) {
        throw pdfError("pdf_too_large", { mb: Math.round(len / 1048576) });
      }
      const buf = await api.readBookBytes(bookId);
      task = getDocument({
        cMapUrl: "/pdfjs/cmaps/",
        cMapPacked: true,
        standardFontDataUrl: "/pdfjs/standard_fonts/",
        isEvalSupported: false,
        data: new Uint8Array(buf),
      } as Parameters<typeof getDocument>[0]);
      doc = await task.promise;
    }
    // 等待期间换了书：这份文档直接销毁，别留下无人引用的 worker。
    if (gen !== generation) {
      await doc.destroy().catch(() => {});
      throw new Error("已切换书籍");
    }
      session = { bookId, doc, range: range ?? undefined, native: false };
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
  const s = await ensurePdf(bookId);
  if (s.native) {
    const [w, h] = await api.pdfNativePageGeometry(bookId, page);
    const size: PageSize = { ptW: Math.max(1, w), ptH: Math.max(1, h) };
    geometry.set(page, size);
    return size;
  }
  const doc = s.doc;
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
  const s = await ensurePdf(bookId);
  const doc = s.doc;
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
    if (s.native) {
      const pt = await pageGeometry(bookId, req.page);
      const url = await nativeSliceUrl(bookId, req, pt);
      if (gen !== generation) {
        revoke(url);
        throw new Error("已切换书籍");
      }
      bitmaps.set(token, { url, page: req.page });
      pinnedTokens.add(token);
      sweep();
      return { url, token };
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
  const s = await ensurePdf(bookId);
  // 原生通道不做注解解析：这类文件的链接入口在阅读器里一并置灰。
  if (s.native) return [];
  const doc = s.doc;
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
  const s = await ensurePdf(bookId);
  // 原生通道不解析自带目录：页码目录由页数生成（见 Reader 的 buildPageToc）。
  if (s.native) return [];
  const doc = s.doc;
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
  const s = await ensurePdf(bookId);
  // 原生通道没有文字层（系统渲染只出位图）：如实返回不可用，阅读器据此隐藏相关入口。
  if (s.native) return { usable: false, text: "" };
  const doc = s.doc;
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

/** 整页位图（图库用）：按长边 ≤ maxLong 渲染，与切片分开管理（图库看的是整页，不是切片）：
 *  切片按 PAGE_CACHE_MAX 张扫描淘汰，整页图按 fullOrder 的小 LRU（FULL_PAGE_CACHE_MAX 张）淘汰。
 *  仍登记进位图表（合成 token）→ 关书/换书时一并 revoke，不留悬挂 blob。 */
export async function renderPageImageFull(bookId: number, page: number, maxW = 1400, maxPx = 12_000_000): Promise<string> {
  const s = await ensurePdf(bookId);
  const doc = s.doc;
  const gen = generation;
  // 尺寸必须**先算出来**再当缓存键：键里含最终宽高，尺寸策略一变就不会命中旧图。
  // 为什么不能只按长边夹：15、Nginx.pdf 是 1693×11221pt，按长边 2000 只得到 302×2000，
  // 适配看着还行，一放大就是 302 源像素硬撑（必糊）。改按宽度 1400 定 scale，再用总像素
  // 1200 万收紧 → 该页约 1345×8919（≈1199 万像素），400%/800% 下文字仍清楚。
  let baseW = 1;
  let baseH = 1;
  if (s.native) {
    // 原生通道的页尺寸来自系统渲染器（只用于比例，与 pt 语义一致）。
    const pt = await pageGeometry(bookId, page);
    baseW = Math.max(1, pt.ptW);
    baseH = Math.max(1, pt.ptH);
  } else {
    const probe = await doc.getPage(page + 1);
    try {
      const vp0 = probe.getViewport({ scale: 1 });
      baseW = Math.max(1, vp0.width);
      baseH = Math.max(1, vp0.height);
    } finally {
      probe.cleanup();
    }
  }
  let w = Math.min(maxW, baseW); // 不放大原图
  let h = (w * baseH) / baseW;
  if (w * h > maxPx) {
    const k = Math.sqrt(maxPx / (w * h));
    w = Math.max(1, Math.floor(w * k));
    h = Math.max(1, Math.floor(h * k));
  }
  const token = `full#${page}@${w}x${h}`;
  const hit = bitmaps.get(token);
  if (hit) {
    // 命中：只刷新它在整页图 LRU 里的新鲜度，不产生淘汰（槽容量是常量，永远够放这一张）
    fullOrder = fullPageLruStep(fullOrder, token, FULL_PAGE_CACHE_MAX).order;
    return hit.url;
  }
  return withSlot(async () => {
    const url = s.native
      ? await nativeFullUrl(bookId, page, baseW, baseH, w, h)
      : await pdfjsFullUrl(doc, page, baseW, w);
    if (gen !== generation) {
      revoke(url);
      throw new Error("已切换书籍");
    }
    bitmaps.set(token, { url, page });
    // 整页图加钉子：切片的 sweep() 不动它 —— 它的生死只由这张容量 3 的小 LRU 决定，
    // 否则会被 PAGE_CACHE_MAX 那套按张数扫走（图库看着看着就白屏）。
    pinnedTokens.add(token);
    const step = fullPageLruStep(fullOrder, token, FULL_PAGE_CACHE_MAX);
    fullOrder = step.order;
    for (const t of step.evicted) {
      const e = bitmaps.get(t);
      if (!e) continue;
      revoke(e.url);
      bitmaps.delete(t);
      pinnedTokens.delete(t);
    }
    sweep();
    return url;
  });
}

/** pdf.js 的整页位图（图库用）。原生通道见 nativeFullUrl。 */
async function pdfjsFullUrl(doc: PDFDocumentProxy, page: number, baseW: number, w: number): Promise<string> {
  const p = await doc.getPage(page + 1);
  try {
    const viewport = p.getViewport({ scale: w / baseW });
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(viewport.width));
    canvas.height = Math.max(1, Math.round(viewport.height));
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 上下文不可用");
    await p.render({ canvasContext: ctx, viewport }).promise;
    const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
    if (!blob) throw new Error("整页位图生成失败");
    return URL.createObjectURL(blob);
  } finally {
    p.cleanup();
  }
}
/** 渲染第 1 页为封面图（WebP data URL），长边缩到 longEdge 像素。
 *  独立于切片缓存（分辨率不同、只渲染一次），失败时抛错由调用方静默处理。
 *  直接出最终缩略图尺寸：后端 setBookCoverImage 只落盘，不再解码缩放。 */
export async function renderCoverImage(bookId: number, longEdge = 320): Promise<string> {
  const s = await ensurePdf(bookId);
  const gen = generation;
  // 原生通道一律不补封面：走原生的是 >1.9GiB 的文件，系统渲染器要把整份文件映射进进程
  // （实测 3.31GiB 那册打开后宿主工作集 3.5GB），为一张缩略图付这个代价不值得，书库继续用默认封面。
  if (s.native) throw pdfError("pdf_no_cover");
  return withSlot(async () => {
    const page = await s.doc.getPage(1);
    // 画布在 try 外声明：finally 里要归还它的后备存储，块作用域里声明会取不到。
    const canvas = document.createElement("canvas");
    let url: string;
    try {
      const base = page.getViewport({ scale: 1 });
      // 长边（竖版封面就是页高）对齐缩略图尺寸：这里出的就是最终像素。
      const target = Math.max(1, longEdge);
      const scale = Math.max(0.02, target / Math.max(1, Math.max(base.width, base.height)));
      const viewport = page.getViewport({ scale });
      canvas.width = Math.max(1, Math.round(viewport.width));
      canvas.height = Math.max(1, Math.round(viewport.height));
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("canvas 上下文不可用");
      await page.render({ canvasContext: ctx, viewport }).promise;
      // 直接出 WebP：后端只落盘、不再解码缩放（见 set_book_cover_image）。
      url = canvas.toDataURL("image/webp", 0.9);
    } finally {
      page.cleanup();
      // 立刻归还画布后备存储：整页位图是这条链路里最大的单笔占用，
      // 书库补封面时一本接一本，晚一拍就会被下一次分配叠上去。
      canvas.width = 0;
      canvas.height = 0;
    }
    if (gen !== generation) throw new Error("已切换书籍");
    return url;
  });
}

/** 「重新提取封面」（PDF）：按目标缩略图尺寸重渲染并落盘，返回新的封面文件名。
 *  与后端 reextract_cover 的分工：那个只处理 epub/mobi 的内嵌封面，PDF 只能由前端栅格化。 */
export async function regeneratePdfCover(bookId: number, longEdge = 320): Promise<string> {
  const url = await renderCoverImage(bookId, longEdge);
  const b64 = url.slice(url.indexOf(",") + 1);
  return await api.setBookCoverImage(bookId, b64);
}

/* ---------- 原生通道渲染辅助（仅 PDF_NATIVE_MIN_BYTES 以上的文件走这条）---------- */

/** 「页内一条」：条带换算成源矩形（页面坐标）与目标像素，交给系统渲染器出 PNG。
 *  换算与 pdf.js 分支同源：scale = 条带目标像素高 / 条带页面高，宽度按页宽同比。 */
async function nativeSliceUrl(bookId: number, req: SliceRequest, pt: PageSize): Promise<string> {
  const bandPt = Math.max(0.5, pt.ptH * (req.f1 - req.f0));
  const scale = Math.max(0.01, req.bitH / bandPt);
  const destW = Math.max(1, Math.round(pt.ptW * scale));
  const destH = Math.max(1, Math.round(req.bitH));
  const bytes = await api.pdfNativeRenderPage(
    bookId,
    req.page,
    0,
    pt.ptH * req.f0,
    pt.ptW,
    bandPt,
    destW,
    destH,
  );
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

/** 整页位图（图库用）：源矩形就是整页。 */
async function nativeFullUrl(
  bookId: number,
  page: number,
  baseW: number,
  baseH: number,
  w: number,
  h: number,
): Promise<string> {
  const bytes = await api.pdfNativeRenderPage(
    bookId,
    page,
    0,
    0,
    baseW,
    baseH,
    Math.max(1, Math.round(w)),
    Math.max(1, Math.round(h)),
  );
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

/** 释放当前 PDF：回收全部位图、销毁文档（关书/换书/组件销毁时调用；非 PDF 时是空操作）。 */
export function releaseAll(): void {
  generation++;
  // 换书/关书：取消在途 range 请求，别让上一本的字节继续往这里送
  session?.range?.abort();
  for (const e of bitmaps.values()) revoke(e.url);
  bitmaps.clear();
  pinnedTokens.clear();
  fullOrder = [];
  geometry.clear();
  const s = session;
  session = null;
  sessionPromise = null;
  sessionBookId = -1;
  if (nativeSession) {
    nativeSession = null;
    // 原生通道没有 pdf.js 文档：显式让 Rust 侧释放文档句柄（换书/关书都要走这条）
    void api.pdfNativeClose().catch(() => {});
  }
  if (s && !s.native) void s.doc.destroy().catch(() => {});
}
