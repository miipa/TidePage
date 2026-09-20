import { invoke } from "@tauri-apps/api/core";
import type { BookPage, BookRow, BookSearchHit, BookSession, BookmarkRow, ChapterData, HighlightRow, MoonProgress, OpenResult, Progress, SearchHit, TocEntry } from "./types";

export const api = {
  listFolders: () => invoke<string[]>("list_folders"),
  addFolder: (path: string) => invoke<void>("add_folder", { path }),
  removeFolder: (path: string) => invoke<void>("remove_folder", { path }),
  listBooks: () => invoke<BookRow[]>("list_books"),
  listBooksPage: (query: string, sort: string, offset: number, limit: number) =>
    invoke<BookPage>("list_books_page", { query, sort, offset, limit }),
  rescan: () => invoke<void>("rescan"),
  refreshLibrary: () => invoke<{ removed: number; added: number }>("refresh_library"),
  fileExists: (path: string) => invoke<boolean>("file_exists", { path }),
  openBook: (id: number) => invoke<OpenResult>("open_book", { id }),
  getChapter: (bookId: number, chapterIndex: number) =>
    invoke<ChapterData>("get_chapter", { bookId, chapterIndex }),
  saveProgress: (bookId: number, p: Progress) => invoke<void>("save_progress", { bookId, p }),
  loadProgress: (bookId: number) => invoke<Progress | null>("load_progress", { bookId }),
  deleteBooks: (paths: string[], fromDisk: boolean) =>
    invoke<void>("delete_books", { paths, fromDisk }),
  previewSplit: (path: string, pattern: string) =>
    invoke<TocEntry[]>("preview_split", { path, pattern }),
  saveSplitRules: (pattern: string) => invoke<void>("save_split_rules", { pattern }),
  convertBook: (
    path: string, target: string, outPath: string,
    opts?: { title?: string; author?: string; coverPath?: string; css?: string; pattern?: string },
  ) =>
    invoke<string>("convert_book", {
      path, target, outPath,
      title: opts?.title || null,
      author: opts?.author || null,
      coverPath: opts?.coverPath || null,
      css: opts?.css || null,
      pattern: opts?.pattern ?? "",
    }),
  pickFile: (title: string, filters: { name: string; exts: string[] }[]) =>
    invoke<string | null>("pick_file", { title, filters }),
  pickFolder: (title: string) => invoke<string | null>("pick_folder", { title }),
  pickSave: (title: string, ext: string, defaultName: string) =>
    invoke<string | null>("pick_save", { title, ext, defaultName }),
  exportAi: (path: string, target: string, title: string, author: string, items: { title: string; html: string }[]) =>
    invoke<string>("export_ai", { path, target, title, author, items }),
  setCustomCover: (bookId: number, imagePath: string) =>
    invoke<void>("set_custom_cover", { bookId, imagePath }),
  webdavTest: (base: string, user: string, pass: string) =>
    invoke<boolean>("webdav_test", { base, user, pass }),
  webdavSyncProgress: (
    base: string, user: string, pass: string,
    bookId: number, bookPath: string, local: Progress | null,
  ) => invoke<Progress>("webdav_sync_progress", { base, user, pass, bookId, bookPath, local }),
  webdavUploadBook: (base: string, user: string, pass: string, bookPath: string) =>
    invoke<string>("webdav_upload_book", { base, user, pass, bookPath }),
  webdavDownloadBooks: (base: string, user: string, pass: string, targetDir: string) =>
    invoke<string[]>("webdav_download_books", { base, user, pass, targetDir }),
  webdavMoonProgress: (base: string, user: string, pass: string) =>
    invoke<MoonProgress[]>("webdav_moon_progress", { base, user, pass }),
  webdavMoonWrite: (base: string, user: string, pass: string, bookPath: string, title: string, chapterTitle: string, pct: number) =>
    invoke<boolean>("webdav_moon_write", { base, user, pass, bookPath: bookPath, title, chapterTitle, pct }),
  webdavMoonReadPo: (base: string, user: string, pass: string, bookPath: string, title: string) =>
    invoke<{ found: boolean; percent: number; chapter_at: string; updated_at: number }>("webdav_moon_read_po", { base, user, pass, bookPath, title }),
  webdavMoonWritePo: (base: string, user: string, pass: string, bookPath: string, title: string, chapterAt: string, pct: number, charOffset: number) =>
    invoke<boolean>("webdav_moon_write_po", { base, user, pass, bookPath, title, chapterAt, pct, charOffset }),
  webdavMoonPoCount: (base: string, user: string, pass: string) =>
    invoke<number>("webdav_moon_po_count", { base, user, pass }),
  webdavList: (base: string, user: string, pass: string, path: string) =>
    invoke<{ name: string; is_dir: boolean; size: number; mtime: number }[]>("webdav_list", { base, user, pass, path }),
  webdavDownloadFile: (base: string, user: string, pass: string, remotePath: string, targetDir: string) =>
    invoke<string[]>("webdav_download_file", { base, user, pass, remotePath, targetDir }),
  webdavDownloadProgress: (base: string, user: string, pass: string, remotePath: string, targetDir: string, session: string) =>
    invoke<string[]>("webdav_download_progress", { base, user, pass, remotePath, targetDir, session }),
  setSetting: (key: string, value: string) => invoke<void>("set_setting_kv", { key, value }),
  getSetting: (key: string) => invoke<string | null>("get_setting_kv", { key }),
  setMobiEncoding: (value: string) => invoke<void>("set_mobi_encoding", { value }),
  getAppDataDir: () => invoke<string>("app_data_dir"),
  addBookmark: (bookId: number, chapterIndex: number, title: string, excerpt: string) =>
    invoke<void>("add_bookmark", { bookId, chapterIndex, title, excerpt }),
  listBookmarks: (bookId: number) => invoke<BookmarkRow[]>("list_bookmarks", { bookId }),
  deleteBookmark: (id: number) => invoke<void>("delete_bookmark", { id }),
  addStats: (bookId: number, seconds: number) => invoke<void>("add_stats", { bookId, seconds }),
  getStats: () => invoke<[string, number][]>("get_stats"),
  setBgImage: (imagePath: string) => invoke<string>("set_background_image", { imagePath }),
  clearBgImage: () => invoke<void>("clear_background_image"),
  randomBook: () => invoke<BookRow | null>("random_book"),
  reextractCover: (bookId: number) => invoke<string>("reextract_cover", { bookId }),
  startSession: (bookId: number) => invoke<number>("start_reading_session", { bookId }),
  updateSession: (sessionId: number, seconds: number, chars: number) =>
    invoke<void>("update_reading_session", { sessionId, seconds, chars }),
  getBookSessions: (bookId: number) => invoke<BookSession[]>("get_book_sessions", { bookId }),
  bookWordCount: (bookId: number) => invoke<{ total: number; pure: number }>("get_book_word_count", { bookId }),
  listSystemFonts: () => invoke<{ family: string }[]>("list_system_fonts"),
  installFontFile: (path: string) => invoke<string>("install_font_file", { path }),
  listCustomFonts: () => invoke<{ family: string; file: string }[]>("list_custom_fonts"),
  deleteCustomFont: (name: string) => invoke<void>("delete_custom_font", { name }),
  aiChat: (args: {
    id: string; provider: string; baseUrl: string; apiKey: string;
    model: string; system: string; messages: [string, string][];
    deployment?: string; apiVersion?: string;
    proxyMode?: string; proxyUrl?: string;
  }) => invoke<void>("ai_chat", args),
  aiTest: (args: {
    provider: string; baseUrl: string; apiKey: string; model: string;
    deployment?: string; apiVersion?: string;
    proxyMode: string; proxyUrl: string;
  }) => invoke<string>("ai_test", args),
  ttsSynth: (args: {
    provider: string; baseUrl: string; apiKey: string; model: string;
    deployment?: string; apiVersion?: string;
    proxyMode?: string; proxyUrl?: string;
    voice: string; format: string; text: string;
  }) => invoke<string>("tts_synth", args),
  addDroppedBook: (path: string) => invoke<BookRow>("add_dropped_book", { path }),
  searchAll: (query: string) => invoke<SearchHit[]>("search_all", { query }),
  searchInBook: (bookId: number, query: string) =>
    invoke<BookSearchHit[]>("search_in_book", { bookId, query }),
  ensureBookText: (bookId: number) => invoke<number>("ensure_book_text_cmd", { bookId }),
  addHighlight: (args: {
    bookId: number; chapterIndex: number; chapterTitle: string;
    start: number; end: number; text: string; style: string; color: string; note: string;
  }) => invoke<number>("add_highlight", args),
  listHighlights: (bookId: number) => invoke<HighlightRow[]>("list_highlights", { bookId }),
  searchHighlights: (query: string) => invoke<HighlightRow[]>("search_highlights", { query }),
  deleteHighlight: (id: number) => invoke<void>("delete_highlight", { id }),
  updateHighlight: (id: number, style: string, color: string, note: string) =>
    invoke<void>("update_highlight", { id, style, color, note }),
  updateBookMeta: (bookId: number, title: string, author: string) =>
    invoke<void>("update_book_meta", { bookId, title, author }),
  writeBookMeta: (bookId: number, title: string, author: string) =>
    invoke<void>("write_book_meta", { bookId, title, author }),
  suggestMeta: (path: string) =>
    invoke<{ title: string; author: string }>("suggest_meta", { path }),
  rebuildFts: () => invoke<number>("rebuild_fts"),
  indexBookFts: (bookId: number) => invoke<number>("index_book_fts", { bookId }),
  revealInExplorer: (path: string) => invoke<void>("reveal_in_explorer", { path }),
};

export function coverUrl(book: BookRow): string {
  const name = book.cover_path || `${book.id}.none`;
  return `http://flowread.localhost/covers/${encodeURIComponent(name)}?v=${book.mtime || book.added_at || 0}`;
}
