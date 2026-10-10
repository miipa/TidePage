// 单本阅读覆盖设置：按书籍 id 持久化（localStorage），与全局 config 分离。
// 作用域严格限定当前书：字号 / 图片缩放 / 适应宽度 / 图片显示 / 屏蔽图片 / 覆盖书籍排版。
export interface BookOverride {
  /** 正文字号（px），覆盖全局 */
  fontSize?: number;
  /** 图片缩放倍率（0.3~2.5），覆盖全局/漫画默认 */
  imgScale?: number;
  /** 图片适应阅读器宽度 */
  fitWidth?: boolean;
  /** 图片适应阅读器高度（与 fitWidth 互斥） */
  fitHeight?: boolean;
  /** 是否显示正文图片（false=全部关闭） */
  showImages?: boolean;
  /** 已屏蔽图片的 src 列表 */
  blockedImages?: string[];
  /** 是否使用阅读器全局样式（覆盖书籍自带 CSS） */
  overrideBookCss?: boolean;
  /** 手动切换漫画模式（true/false 覆盖自动检测；未设置则用自动检测结果） */
  mangaMode?: boolean;

  // ===== 阅读配色方案（"仅应用到本书"时写入的整套排版快照）=====
  // 任一 scheme* 字段存在即表示本书已单独套用方案；清除这些字段即回到全局。
  schemeTheme?: string;
  schemeBg?: string;
  schemeFg?: string;
  schemeFontFamily?: string;
  schemeLatinFont?: string;
  schemeLineHeight?: number;
  schemeParaSpacing?: number;
  schemeIndent?: string;
  schemeTextAlign?: string;
  schemeBgImage?: string;
  schemeBgOpacity?: number;
  schemeCssExtra?: string;
  schemeFontWeight?: number;
}

const KEY = "tidepage.bookcfg.v1";

function loadMap(): Record<string, BookOverride> {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const o = JSON.parse(raw);
    return o && typeof o === "object" ? (o as Record<string, BookOverride>) : {};
  } catch {
    return {};
  }
}

export const bookOverrides = $state<{ map: Record<string, BookOverride> }>({ map: loadMap() });

/** 读取某本书的覆盖设置（响应式：读写 bookOverrides.map 的键会被追踪）。 */
export function getBookOverride(id: number | null | undefined): BookOverride {
  if (!id) return {};
  return bookOverrides.map[String(id)] ?? {};
}

function save() {
  try {
    localStorage.setItem(KEY, JSON.stringify(bookOverrides.map));
  } catch {
    /* ignore */
  }
}

/** 合并写入某本书的覆盖设置；值为 undefined 的键会被移除（回退全局）。 */
export function setBookOverride(id: number, patch: Partial<BookOverride>): void {
  if (!id) return;
  const key = String(id);
  const cur = bookOverrides.map[key] ?? {};
  const next: BookOverride = { ...cur, ...patch };
  for (const k of Object.keys(next) as (keyof BookOverride)[]) {
    if (next[k] === undefined) delete next[k];
  }
  bookOverrides.map[key] = next;
  save();
}
