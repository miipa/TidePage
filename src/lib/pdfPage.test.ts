import { describe, it, expect } from "vitest";
import {
  mergeOutlineTitles,
  pdfTextUsable,
  PDF_BITMAP_MAX_LONG,
  PDF_BITMAP_MAX_PIXELS,
  PDF_PX_PER_PT,
  buildPageToc,
  clampBitmapSize,
  pdfDisplayBox,
  pdfPageErrorHtml,
  pdfSliceRanges,
  retitlePdfPageToc,
} from "./pdfPage";

describe("pdfDisplayBox", () => {
  const a4 = { ptW: 595, ptH: 842 };

  it("按 4/3 换算与倍率算显示尺寸（A4 @1.5×）", () => {
    const b = pdfDisplayBox({ page: a4, containerW: 1796, containerH: 1198, imgScale: 1.5, fitWidth: false, fitHeight: false, horizontal: false });
    // 595×4/3=793.3 → ×1.5 = 1190；842×4/3=1122.7 → ×1.5 = 1684
    expect(b.w).toBe(1190);
    expect(b.h).toBe(1684);
    expect(b.k).toBeCloseTo(1.5, 5);
  });

  it("flow 下放大不夹到容器宽（超宽由横向滚动查看）", () => {
    // 595pt × 4/3 × 3 = 2380 > 容器 800：必须按真实倍率出尺寸，否则「放大」是空操作
    const b = pdfDisplayBox({ page: a4, containerW: 800, containerH: 1198, imgScale: 3, fitWidth: false, fitHeight: false, horizontal: false });
    expect(b.w).toBe(Math.round(595 * PDF_PX_PER_PT * 3));
    expect(b.k).toBeCloseTo(3, 5);
  });

  it("适宽优先于倍率：关掉适宽后三档倍率尺寸互不相同且 ≈ pt×4/3×倍率", () => {
    const w = (s: number) =>
      pdfDisplayBox({ page: a4, containerW: 1791, containerH: 1198, imgScale: s, fitWidth: false, fitHeight: false, horizontal: false }).w;
    expect(w(1)).toBe(793);
    expect(w(1.5)).toBe(1190);
    expect(w(2)).toBe(1587);
    expect(new Set([w(1), w(1.5), w(2)]).size).toBe(3);
    const fit = pdfDisplayBox({ page: a4, containerW: 1791, containerH: 1198, imgScale: 2, fitWidth: true, fitHeight: false, horizontal: false });
    expect(fit.w).toBe(1791);
  });

  it("fitWidth / fitHeight 直接按容器反推（与既有漫画语义一致）", () => {
    const fw = pdfDisplayBox({ page: a4, containerW: 1000, containerH: 1198, imgScale: 1.5, fitWidth: true, fitHeight: false, horizontal: false });
    expect(fw.w).toBe(1000);
    const fh = pdfDisplayBox({ page: a4, containerW: 1000, containerH: 1198, imgScale: 1.5, fitWidth: false, fitHeight: true, horizontal: false });
    expect(fh.h).toBe(1198);
  });

  it("横向布局用「容器高 × 倍率」定尺寸", () => {
    const b = pdfDisplayBox({ page: a4, containerW: 1796, containerH: 1198, imgScale: 1.5, fitWidth: false, fitHeight: false, horizontal: true });
    expect(b.h).toBe(1797);
  });

  it("非法输入兜底不产生 0/NaN", () => {
    const b = pdfDisplayBox({ page: { ptW: 0, ptH: 0 }, containerW: 0, containerH: 0, imgScale: 0, fitWidth: false, fitHeight: false, horizontal: false });
    expect(b.w).toBeGreaterThan(0);
    expect(b.h).toBeGreaterThan(0);
    expect(Number.isFinite(b.k)).toBe(true);
  });
});

describe("clampBitmapSize", () => {
  it("常规页：位图 = 显示尺寸 × DPR（无上采样）", () => {
    expect(clampBitmapSize(1190, 1684, 1)).toMatchObject({ bitW: 1190, bitH: 1684, scale: 1 });
    expect(clampBitmapSize(1190, 1684, 1.5)).toMatchObject({ bitW: 1785, bitH: 2526 });
  });

  it("长边超上限时降分辨率（不改显示尺寸）", () => {
    const r = clampBitmapSize(2000, 40000, 1);
    expect(Math.max(r.bitW, r.bitH)).toBeLessThanOrEqual(PDF_BITMAP_MAX_LONG);
    expect(r.bitH / r.bitW).toBeCloseTo(20, 1);
  });

  it("总像素超上限时继续降", () => {
    const r = clampBitmapSize(4000, 4000, 2);
    expect(r.bitW * r.bitH).toBeLessThanOrEqual(PDF_BITMAP_MAX_PIXELS * 1.02);
  });

  it("DPR 非法按 1 处理", () => {
    expect(clampBitmapSize(100, 100, 0).bitW).toBe(100);
    expect(clampBitmapSize(100, 100, Number.NaN).bitH).toBe(100);
  });
});

describe("pdfSliceRanges", () => {
  it("不超过阈值的页不切（1 条）", () => {
    expect(pdfSliceRanges(1000, 800)).toEqual([[0, 1]]);
  });

  it("11 屏高的超长页按屏数切分且覆盖整页", () => {
    const r = pdfSliceRanges(11 * 800, 800, 1.5);
    expect(r.length).toBeGreaterThan(1);
    expect(r[0][0]).toBe(0);
    expect(r[r.length - 1][1]).toBeCloseTo(1, 10);
    for (let i = 1; i < r.length; i++) expect(r[i][0]).toBeCloseTo(r[i - 1][1], 10);
  });

  it("每条高度 ≤ 阈值（除最后一条补齐）", () => {
    const h = 11 * 800;
    const r = pdfSliceRanges(h, 800, 1.5);
    for (const [f0, f1] of r) expect((f1 - f0) * h).toBeLessThanOrEqual(800 * 1.5 + 1);
  });
});

describe("buildPageToc / retitlePdfPageToc", () => {
  it("一页一项：index 从 0 连续、chars 恒为 0", () => {
    expect(buildPageToc(2, (n) => `第 ${n} 页`)).toEqual([
      { title: "第 1 页", index: 0, chars: 0 },
      { title: "第 2 页", index: 1, chars: 0 },
    ]);
  });

  it("页数非法或非正时返回空目录", () => {
    expect(buildPageToc(0, String)).toEqual([]);
    expect(buildPageToc(-5, String)).toEqual([]);
    expect(buildPageToc(Number.NaN, String)).toEqual([]);
  });

  it("按前端文案重写标题，index/chars 原样保留", () => {
    expect(retitlePdfPageToc([{ title: "第 7 页", index: 6, chars: 12 }], (n) => `Page ${n}`)).toEqual([
      { title: "Page 7", index: 6, chars: 12 },
    ]);
  });

  it("空目录返回空", () => {
    expect(retitlePdfPageToc([], String)).toEqual([]);
  });
});

describe("pdfTextUsable", () => {
  it("真实中文/英文文本可用", () => {
    expect(pdfTextUsable("page00走进数码单反镜头随着数码单反相机的价格越来越平民化")).toBe(true);
    expect(pdfTextUsable("This is a real English paragraph extracted from a PDF file.")).toBe(true);
    expect(pdfTextUsable("我能承受吗目录1“只有我们感觉到并承受环境的痛苦")).toBe(true);
  });

  it("扫描版（空文本）不可用", () => {
    expect(pdfTextUsable("")).toBe(false);
    expect(pdfTextUsable("   ")).toBe(false);
    expect(pdfTextUsable("短")).toBe(false);
  });

  it("缺 ToUnicode 的乱码不可用（实测该类文件控制字符占比 25%）", () => {
    // 真实样本形态：HTTP → KWWS，斜杠 → 控制字符
    const garbled = "1JLQ[" + "\u001d\u0012\u0012WHQJLQH\u0011WDREDR\u0011RUJ\u0012ERRN\u0012".repeat(8);
    expect(pdfTextUsable(garbled)).toBe(false);
  });

  it("非中文外文文档必须可用（西里尔/希腊/阿拉伯）", () => {
    expect(pdfTextUsable("Это обычный русский текст документа, извлечённый из PDF-файла.")).toBe(true);
    expect(pdfTextUsable("Αυτό είναι ένα κανονικό ελληνικό κείμενο από αρχείο PDF.")).toBe(true);
    expect(pdfTextUsable("هذا نص عربي عادي مستخرج من ملف PDF للقراءة.")).toBe(true);
  });

  it("纯英文/代码文本可用", () => {
    expect(pdfTextUsable("const a = 1; // normal code and english text from a pdf page")).toBe(true);
    expect(pdfTextUsable("0123456789 0123456789 0123456789 0123456789")).toBe(true);
  });

  it("替换字符 / 私用区码位过多不可用", () => {
    expect(pdfTextUsable("\uFFFD".repeat(30) + "abcdefgh")).toBe(false);
    expect(pdfTextUsable("\uE000\uE001\uE002\uE003\uE004\uE005\uE006\uE007\uE008\uE009abcdefgh")).toBe(false);
  });

  it("非 ASCII 占多数但非文字码位时不可用", () => {
    expect(pdfTextUsable("\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u200b\u200c\u200d")).toBe(false);
  });
});

describe("mergeOutlineTitles", () => {
  const base = buildPageToc(4, (n) => `第 ${n} 页`);

  it("命中页用 outline 标题与层级，其余页保持页码", () => {
    const out = mergeOutlineTitles(base, [{ title: "第一章", page: 1, depth: 0 }, { title: "1.1 小节", page: 2, depth: 1 }], (n) => `Page ${n}`);
    expect(out.map((e) => e.title)).toEqual(["Page 1", "第一章", "1.1 小节", "Page 4"]);
    expect(out.map((e) => e.depth)).toEqual([undefined, 0, 1, undefined]);
    // index 语义不变（数组下标即页号）
    expect(out.map((e) => e.index)).toEqual([0, 1, 2, 3]);
  });

  it("越界/未解析(page=-1)的条目被忽略，不影响其余页", () => {
    const out = mergeOutlineTitles(base, [{ title: "坏的", page: -1, depth: 0 }, { title: "越界", page: 99, depth: 0 }], (n) => `Page ${n}`);
    expect(out.map((e) => e.title)).toEqual(["Page 1", "Page 2", "Page 3", "Page 4"]);
  });

  it("空 outline 时等价于纯页码目录（并统一按传入文案重标）", () => {
    expect(mergeOutlineTitles(base, [], (n) => `Page ${n}`)).toEqual(buildPageToc(4, (n) => `Page ${n}`));
  });
});

describe("PDF 页占位 HTML", () => {
  it("渲染失败给固定高度且不含图片", () => {
    const html = pdfPageErrorHtml();
    expect(html).toContain("height:60vh");
    expect(html).not.toContain("<img");
  });
});
