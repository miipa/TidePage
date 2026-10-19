/**
 * PDF `FlateDecode` 页面的解码单元（可独立测试）。
 *
 * 为什么单独成模块：直读通道里「解压 → 预测还原 → 像素解包」这三步是纯计算，
 * 与 DOM/canvas 无关；放在 pdf.ts 里只能靠肉眼验，抽出来后 vitest 能直接构造输入断言像素。
 *
 * 约定：后端只回**未解压 zlib 字节** + `/Predictor`、`/Width`、`/Height`、
 * `BitsPerComponent`、`ColorSpace`（Indexed 另带调色板）。这里负责把字节变成 RGBA。
 */

/** 解压：PDF `FlateDecode` 是 zlib 包装；畸形文件常见裸 deflate，失败再试一次。 */
export async function inflatePdfStream(bytes: ArrayBuffer | Uint8Array): Promise<Uint8Array> {
  const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const tryOne = async (fmt: "deflate" | "deflate-raw"): Promise<Uint8Array | null> => {
    try {
      const ds = new DecompressionStream(fmt);
      const stream = new Blob([buf as unknown as BlobPart]).stream().pipeThrough(ds);
      const out = await new Response(stream).arrayBuffer();
      return new Uint8Array(out);
    } catch {
      return null;
    }
  };
  const a = await tryOne("deflate");
  if (a && a.length) return a;
  const b = await tryOne("deflate-raw");
  if (b && b.length) return b;
  throw new Error("解压失败");
}

/**
 * 预测还原：
 * - `predictor <= 1`：原样返回；
 * - `predictor === 2`：TIFF 预测（每行逐字节累加）；
 * - `predictor >= 10`：PNG 预测（每行首字节是过滤类型 0..4，支持 None/Sub/Up/Average/Paeth）。
 *
 * `columns` 是「每行样本数」（PNG 预测里就是图像的宽），`colors` 是每像素分量数，
 * `bpc` 是位深 —— 三者决定行字节数 `ceil(columns*colors*bpc/8)`。
 */
export function undoPredictor(
  data: Uint8Array,
  predictor: number,
  columns: number,
  colors: number,
  bpc: number,
): Uint8Array {
  if (predictor <= 1) return data;
  const bpp = Math.max(1, Math.ceil((colors * bpc) / 8));
  const rowBytes = Math.ceil((columns * colors * bpc) / 8);
  if (rowBytes <= 0) return data;
  if (predictor === 2) {
    const out = data.slice();
    for (let r = 0; r * rowBytes < out.length; r++) {
      const base = r * rowBytes;
      for (let i = bpp; i < rowBytes && base + i < out.length; i++) {
        out[base + i] = (out[base + i] + out[base + i - bpp]) & 0xff;
      }
    }
    return out;
  }
  // PNG 预测：每行 = 1 字节过滤类型 + rowBytes 字节数据
  const rows = Math.ceil(data.length / (rowBytes + 1));
  const out = new Uint8Array(rows * rowBytes);
  for (let r = 0; r < rows; r++) {
    const srcOff = r * (rowBytes + 1);
    if (srcOff >= data.length) break;
    const ft = data[srcOff];
    const dstOff = r * rowBytes;
    for (let i = 0; i < rowBytes; i++) {
      const raw = data[srcOff + 1 + i] ?? 0;
      const a = i >= bpp ? out[dstOff + i - bpp] : 0;
      const b = r > 0 ? out[(r - 1) * rowBytes + i] : 0;
      const c = r > 0 && i >= bpp ? out[(r - 1) * rowBytes + i - bpp] : 0;
      let v = raw;
      switch (ft) {
        case 0:
          break;
        case 1:
          v = raw + a;
          break;
        case 2:
          v = raw + b;
          break;
        case 3:
          v = raw + ((a + b) >> 1);
          break;
        case 4: {
          const pa = Math.abs(b - c);
          const pb = Math.abs(a - c);
          const pc = Math.abs(a + b - 2 * c);
          const pr = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
          v = raw + pr;
          break;
        }
        default:
          v = raw;
      }
      out[dstOff + i] = v & 0xff;
    }
  }
  return out;
}

/** 页面像素参数（与后端 `pdf_scan_page` 的字段一一对应）。 */
export interface RawPageMeta {
  width: number;
  height: number;
  bpc: number;
  color_space: string;
  predictor: number;
  palette?: number[] | Uint8Array;
}

export interface RgbaPage {
  width: number;
  height: number;
  /** RGBA，长度 = width*height*4。 */
  rgba: Uint8ClampedArray;
}

/**
 * 「已解压字节 + 参数」→ RGBA 像素。支持：
 * - `/Indexed`（1/2/4/8bpc，调色板来自 `/ColorSpace` 第四项）；
 * - `/DeviceGray`（8bpc）；
 * - `/DeviceRGB`（8bpc）。
 */
export function decodeRawPage(meta: RawPageMeta, inflated: Uint8Array): RgbaPage {
  const w = Math.max(1, Math.floor(meta.width));
  const h = Math.max(1, Math.floor(meta.height));
  const bpc = meta.bpc || 8;
  const colors = meta.color_space === "Indexed" ? 1 : meta.color_space === "DeviceGray" ? 1 : 3;
  const rowBytes = Math.ceil((w * colors * bpc) / 8);
  const data = undoPredictor(inflated, meta.predictor, w, colors, bpc);
  const pal = meta.palette instanceof Uint8Array ? meta.palette : Uint8Array.from(meta.palette ?? []);
  const maxV = (1 << bpc) - 1;
  const sample = (row: number, x: number, ch: number): number => {
    if (bpc === 8) {
      return data[row * rowBytes + x * colors + ch] ?? 0;
    }
    // 4bpc 等：高半字节在前
    const bitPos = (x * colors + ch) * bpc;
    const byte = data[row * rowBytes + (bitPos >> 3)] ?? 0;
    const shift = 8 - bpc - (bitPos & 7);
    return (byte >> shift) & maxV;
  };
  const rgba = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const o = (y * w + x) * 4;
      if (meta.color_space === "Indexed") {
        const idx = sample(y, x, 0);
        rgba[o] = pal[idx * 3] ?? 0;
        rgba[o + 1] = pal[idx * 3 + 1] ?? 0;
        rgba[o + 2] = pal[idx * 3 + 2] ?? 0;
      } else if (colors === 1) {
        const g = Math.round((sample(y, x, 0) * 255) / maxV);
        rgba[o] = g;
        rgba[o + 1] = g;
        rgba[o + 2] = g;
      } else {
        rgba[o] = sample(y, x, 0);
        rgba[o + 1] = sample(y, x, 1);
        rgba[o + 2] = sample(y, x, 2);
      }
      rgba[o + 3] = 255;
    }
  }
  return { width: w, height: h, rgba };
}
