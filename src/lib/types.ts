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
