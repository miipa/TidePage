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
  /** 该书自己的封面版本：提取/更换封面时 +1，令 per-book 封面 URL 缓存失效。 */
  cover_ver: number;
  /** 反规范化的阅读进度（0–100，量纲同 progress.pct）：由写入路径维护，供封面进度环与状态过滤。 */
  read_pct: number;
  /** 手动状态覆盖：0=自动推导 / 1=手动已读 / 2=手动未读。 */
  read_state: number;
  /** 最近打开时间（打开动作写入）：状态推导的「打开」列，供前端 effectiveState 与 SQL 同源。 */
  last_opened: number;
}

/** 分页返回：total 为过滤后总数，rows 为当前窗口。 */
export interface BookPage {
  total: number;
  rows: BookRow[];
}

export interface TocEntry {
  title: string;
  index: number;
  chars: number;
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
