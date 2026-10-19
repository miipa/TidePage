import { describe, expect, it } from "vitest";
import { decodeRawPage, inflatePdfStream, undoPredictor } from "./pdfFlate";

/** PNG 预测（Predictor ≥10）单行构造：`[filter, bytes...]`。 */
function pngRow(filter: number, bytes: number[]): Uint8Array {
  return Uint8Array.from([filter, ...bytes]);
}

describe("undoPredictor：PNG 预测（Predictor=12）", () => {
  // 每行 4 字节、bpp=1（灰度 8bpc、宽 4）
  it("None（filter 0）原样还原", () => {
    const out = undoPredictor(pngRow(0, [1, 2, 3, 4]), 12, 4, 1, 8);
    expect(Array.from(out)).toEqual([1, 2, 3, 4]);
  });

  it("Sub（filter 1）按左邻累加", () => {
    // 原始行 [10,20,30,40] ⇒ Sub 编码 [10,10,10,10]
    const out = undoPredictor(pngRow(1, [10, 10, 10, 10]), 12, 4, 1, 8);
    expect(Array.from(out)).toEqual([10, 20, 30, 40]);
  });

  it("Up（filter 2）按上一行同列累加", () => {
    const row0 = pngRow(0, [5, 6, 7, 8]);
    const row1 = pngRow(2, [1, 1, 1, 1]);
    const data = Uint8Array.from([...row0, ...row1]);
    const out = undoPredictor(data, 12, 4, 1, 8);
    expect(Array.from(out.slice(4))).toEqual([6, 7, 8, 9]);
  });

  it("Average（filter 3）按 (左+上)/2 累加", () => {
    const row0 = pngRow(0, [10, 10, 10, 10]);
    // 目标第 1 行 [20,20,20,20]，Average 预测 = floor((左 + 上)/2)：
    // i=0: 左=0, 上=10 ⇒ 5 ⇒ raw=15；i≥1: 左=20, 上=10 ⇒ 15 ⇒ raw=5
    const row1 = pngRow(3, [15, 5, 5, 5]);
    const data = Uint8Array.from([...row0, ...row1]);
    const out = undoPredictor(data, 12, 4, 1, 8);
    expect(Array.from(out.slice(4))).toEqual([20, 20, 20, 20]);
  });

  it("Paeth（filter 4）按左/上/左上最近者累加", () => {
    const row0 = pngRow(0, [10, 20, 30, 40]);
    // 目标第 1 行 [11,21,31,41]：a=左(11…)，b=上(10…)，c=左上(10…)
    // i=0: a=0,b=10,c=0 ⇒ paeth=10 ⇒ raw=11-10=1
    // i=1: a=11,b=20,c=10 ⇒ p=11+20-10=21, pa=|20-10|=10, pb=|11-10|=1, pc=|21-20|=1 ⇒ 选 b=20 ⇒ raw=1
    // i=2: a=21,b=30,c=20 ⇒ 同上 ⇒ b=30 ⇒ raw=1
    // i=3: a=31,b=40,c=30 ⇒ b=40 ⇒ raw=1
    const row1 = pngRow(4, [1, 1, 1, 1]);
    const data = Uint8Array.from([...row0, ...row1]);
    const out = undoPredictor(data, 12, 4, 1, 8);
    expect(Array.from(out.slice(4))).toEqual([11, 21, 31, 41]);
  });
});

describe("undoPredictor：TIFF 预测（Predictor=2）", () => {
  it("逐字节累加还原 RGB 行", () => {
    // 原始 [10,20,30, 15,25,35] ⇒ 差分 [10,20,30, 5,5,5]（bpp=3，跨像素差分）
    const out = undoPredictor(Uint8Array.from([10, 20, 30, 5, 5, 5]), 2, 2, 3, 8);
    expect(Array.from(out)).toEqual([10, 20, 30, 15, 25, 35]);
  });
});

describe("decodeRawPage", () => {
  it("Indexed + 4bpc：半字节索引映射到调色板", () => {
    // 宽 4、高 1，4bpc ⇒ 每字节 2 像素：0x01 0x23 ⇒ 索引 [0,1,2,3]
    const palette = Uint8Array.from([
      255, 0, 0, // 0 红
      0, 255, 0, // 1 绿
      0, 0, 255, // 2 蓝
      255, 255, 255, // 3 白
    ]);
    const page = decodeRawPage(
      { width: 4, height: 1, bpc: 4, color_space: "Indexed", predictor: 1, palette },
      Uint8Array.from([0x01, 0x23]),
    );
    expect(page.width).toBe(4);
    expect(page.height).toBe(1);
    expect(Array.from(page.rgba)).toEqual([
      255, 0, 0, 255,
      0, 255, 0, 255,
      0, 0, 255, 255,
      255, 255, 255, 255,
    ]);
  });

  it("DeviceGray 8bpc：单通道复制到 RGB", () => {
    const page = decodeRawPage(
      { width: 2, height: 1, bpc: 8, color_space: "DeviceGray", predictor: 1 },
      Uint8Array.from([7, 200]),
    );
    expect(Array.from(page.rgba)).toEqual([7, 7, 7, 255, 200, 200, 200, 255]);
  });

  it("DeviceRGB + PNG Sub 预测：先还原再解包", () => {
    // 宽 2、高 1、RGB 8bpc ⇒ 行 6 字节；目标 [10,20,30, 15,25,35]
    // Sub 编码（bpp=3）：[10,20,30, 5,5,5]
    const page = decodeRawPage(
      { width: 2, height: 1, bpc: 8, color_space: "DeviceRGB", predictor: 12 },
      Uint8Array.from([1, 10, 20, 30, 5, 5, 5]),
    );
    expect(Array.from(page.rgba)).toEqual([10, 20, 30, 255, 15, 25, 35, 255]);
  });
});

describe("inflatePdfStream", () => {
  it("zlib 包装（deflate）可解", async () => {
    const src = new TextEncoder().encode("q 1 0 0 1 0 0 cm /Im1 Do Q");
    // 用浏览器/Node 自带压缩产生合法 zlib 流
    const cs = new CompressionStream("deflate");
    const zipped = new Uint8Array(
      await new Response(new Blob([src]).stream().pipeThrough(cs)).arrayBuffer(),
    );
    const out = await inflatePdfStream(zipped);
    expect(new TextDecoder().decode(out)).toBe("q 1 0 0 1 0 0 cm /Im1 Do Q");
  });
});
