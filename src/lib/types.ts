export interface BookRow {
  id: number;
  path: string;
  title: string;
  author: string;
  format: string;
  size: number;
  mtime: number;
  added_at: number;
  cover_path: string | null;
  meta_done: number;
  last_read: number;
  /** 全书字数（EPUB 由 get_book_word_count/book_char_counts 首次统计后回填；未统计为 0）。 */
  total_chars: number;
  /** 全书纯文字字数（去掉空白后的字数，同上）。 */
  total_text_chars: number;
  /** 该书自己的封面版本：提取/更换封面时 +1，令 per-book 封面 URL 缓存失效。 */
  cover_ver: number;
  /** 反规范化的阅读进度（0–100，量纲同 progress.pct）：由写入路径维护，供封面进度环与状态过滤。 */
  read_pct: number;
  /** 手动状态覆盖：0=自动推导 / 1=手动已读 / 2=手动未读。 */
  read_state: number;
  /** 最近打开时间（打开动作写入）：状态推导的「打开」列，供前端 effectiveState 与 SQL 同源。 */
  last_opened: number;
}

/** 分页返回：total 为过滤后总数（-1 表示本次未计数，见 api.listBooksPage includeTotal），rows 为当前窗口。 */
export interface BookPage {
  total: number;
  rows: BookRow[];
}

export interface TocEntry {
  title: string;
  index: number;
  chars: number;
  /** PDF 自带目录（outline）的嵌套层级：仅用于目录面板缩进；其余格式不设此字段。 */
  depth?: number;
}

export interface ChapterData {
  title: string;
  html: string;
  chars: number;
}

/** 全书图片查看器：一张图片的引用。 */
export interface BookImage {
  url: string;
  chapter_index: number;
  chapter_title: string;
}

export interface BookSession {
  day: string;
  started_at: number;
  seconds: number;
  chars: number;
}

export interface Progress {
  chapter_index: number;
  chapter_ratio: number;
  pct: number;
  updated_at: number;
  device: string;
}

export interface OpenResult {
  book: BookRow;
  toc: TocEntry[];
  docs: string[];
  encoding_warning?: string | null;
  images?: BookImage[];
  /** 是否检测为漫画书籍（图片远多于文本；用于漫画图片大小默认值与首次引导） */
  is_manga?: boolean;
  /** 图片页型书籍：自动按图片页式排版并默认隐藏文字层（与漫画同源布局） */
  image_pages?: boolean;
}

export interface SearchHit {
  book_id: number;
  book_title: string;
  chapter_index: number;
  chapter_title: string;
  snippet: string;
}

export interface BookSearchHit {
  chapter_index: number;
  chapter_title: string;
  snippet: string;
}

export interface HighlightRow {
  id: number;
  book_id: number;
  book_title: string;
  chapter_index: number;
  chapter_title: string;
  pos_start: number;
  pos_end: number;
  text: string;
  style: string;
  color: string;
  note: string;
  created_at: number;
}

export interface BookmarkRow {
  id: number;
  chapter_index: number;
  title: string;
  excerpt: string;
  created_at: number;
}

/** 连接测试用的结构化片段（`ai.rs` 的 `AiPiece`）：**稳定 id + 动态素材**，界面句子一律前端拼。
 *
 *  - `code`：稳定 ASCII id，与文案表一一对应（如 `ai.routeDirect`）；空串表示这段没有固定
 *    说法（代理地址、上游返回文本等），直接显示 `param` 原文。
 *  - `param`：本片段自身的原文素材，同时作为单素材译文的 `{p0}`。
 *  - `params`：该译文的动态素材，按序填进 `{p0}` / `{p1}`…；素材本身也可以是片段（可嵌套）。
 *  翻译用 `aiPieceText()`（src/lib/i18n/index.svelte.ts）。 */
export interface AiPiece {
  code: string;
  param: string;
  params: AiPiece[];
}

/** ai_test 的返回：**只有状态与素材，没有拼好的句子**（界面文案一律前端按当前语言拼）。
 *  ok 走结构化 `route` / `diag`；config 带 `code` 以便查表；allFailed 的 `lines` 逐条结构化，
 *  `detail` 只是语言中立的原文明细（供日志/复制，界面不渲染）。 */
export type AiTestResult =
  | { status: "ok"; route: AiPiece; provider: string; reply: string; diag: AiPiece[] }
  | { status: "config"; code: string; detail: string }
  | { status: "allFailed"; detail: string; lines: AiPiece[] };

/** `check_update` 的返回：最新 Release 信息（`null` 表示仓库尚无任何 Release）。 */
export interface UpdateInfo {
  latest: string;
  url: string;
  name: string;
  notes: string;
  published_at: string;
}
