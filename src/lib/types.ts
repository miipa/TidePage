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

export interface MoonProgress {
  file: string;
  title: string;
  chapter: string | null;
  pct: number | null;
}
