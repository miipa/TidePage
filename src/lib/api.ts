import { invoke } from "@tauri-apps/api/core";
import type { BookImage, BookPage, BookRow, BookSearchHit, BookSession, BookmarkRow, ChapterData, HighlightRow, MoonProgress, OpenResult, Progress, SearchHit, TocEntry } from "./types";

export const api = {
  listFolders: () => invoke<string[]>("list_folders"),
  addFolder: (path: string) => invoke<void>("add_folder", { path }),
  removeFolder: (path: string) => invoke<void>("remove_folder", { path }),
  listBooks: () => invoke<BookRow[]>("list_books"),
  listBooksPage: (query: string, sort: string, offset: number, limit: number) =>
    invoke<BookPage>("list_books_page", { query, sort, offset, limit }),
  /** 秩可寻址分页命令 1：返回小端 i64 原始字节流（ArrayBuffer，每 8 字节一个 books.id）。
   *  query 为空=全量；非空=与 list_books_page 同源的 title/author 过滤（搜索档同样 O(W)）。
   *  结果超过后端上限（30 万）时抛 "order_index_too_large:300000"，调用方须切 keyset 档。 */
  listOrderIndex: (sort: string, query: string) =>
    invoke<ArrayBuffer>("list_order_index", { sort, query }),
  /** 秩可寻址分页命令 2：按 ids 批量取行，返回长度/顺序恒等于 ids，缺行为 null。ids.len()>4096 抛错。 */
  listBooksByIds: (ids: number[]) => invoke<(BookRow | null)[]>("list_books_by_ids", { ids }),
  rescan: () => invoke<void>("rescan"),
  refreshLibrary: () => invoke<{ removed: number; added: number }>("refresh_library"),
  fileExists: (path: string) => invoke<boolean>("file_exists", { path }),
  openBook: (id: number) => invoke<OpenResult>("open_book", { id }),
  getChapter: (bookId: number, chapterIndex: number, keepStyles = false) =>
    invoke<ChapterData>("get_chapter", { bookId, chapterIndex, keepStyles }),
  listBookImages: (bookId: number) => invoke<BookImage[]>("list_book_images", { bookId }),
  saveProgress: (bookId: number, p: Progress) => invoke<void>("save_progress", { bookId, p }),
  loadProgress: (bookId: number) => invoke<Progress | null>("load_progress", { bookId }),
  deleteBooks: (paths: string[], fromDisk: boolean, keepNotes = true) =>
    invoke<void>("delete_books", { paths, fromDisk, keepNotes }),
  bookNoteInfo: (bookId: number) =>
    invoke<{ hasNotes: boolean; noteCount: number }>("book_note_info", { bookId }),
  listNoteBooks: () =>
    invoke<{ bookId: number; title: string; author: string; deleted: boolean; noteCount: number; lastAt: number }[]>(
      "list_note_books",
    ),
  listNotesOfBook: (bookId: number) =>
    invoke<{ id: number; chapterIndex: number; chapterTitle: string; text: string; note: string; createdAt: number }[]>(
      "list_notes_of_book",
      { bookId },
    ),
  setHighlightNote: (id: number, note: string) =>
    invoke<void>("set_highlight_note", { id, note }),
  appendHighlightNote: (id: number, addition: string) =>
    invoke<string>("append_highlight_note", { id, addition }),
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
  storageStats: () =>
    invoke<{
      data_dir: string;
      total_bytes: number;
      covers_bytes: number;
      res_bytes: number;
      fonts_bytes: number;
      db_bytes: number;
      cache_bytes: number;
    }>("storage_stats"),
  clearCache: (what: "covers" | "res" | "all") => invoke<number>("clear_cache", { what }),
  metaDebugStats: () => invoke<{ startCount: number; running: boolean; pending: number; priority: number }>("meta_debug_stats"),
  /** 封面扫描进度快照：前端挂载时主动拉取，初始化底栏进度条（消除启动事件早于挂载的竞态）。 */
  scanProgress: () => invoke<{ running: boolean; done: number; total: number; pending: number }>("scan_progress"),
  /** 方案 A：把列表可见的未扫描书籍提升为高优先级扫描目标（按视野顺序生成封面）。返回入队本数。 */
  prioritizeCovers: (ids: number[]) => invoke<number>("prioritize_covers", { ids }),
  setDefaultCover: (imagePath: string) => invoke<void>("set_default_cover", { imagePath }),
  clearDefaultCover: () => invoke<void>("clear_default_cover"),
  hasDefaultCover: () => invoke<boolean>("has_default_cover"),
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
  ttsVoices: (baseUrl: string, apiKey: string) =>
    invoke<{ id: string; name: string; type: string; language: string }[]>("tts_voices", {
      baseUrl,
      apiKey,
    }),
  ttsEnsureFixedVoice: (baseUrl: string, apiKey: string, language: string, instruct: string) =>
    invoke<{ id: string; created: boolean }>("tts_ensure_fixed_voice", {
      baseUrl,
      apiKey,
      language,
      instruct,
    }),
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

// 全局封面缓存破坏版本：换底图/恢复内置后 +1（后端 cover_style_ver 的镜像）。
let _coverBust = 0;
export function bumpCoverBust(): void {
  _coverBust++;
}

export function coverUrl(book: BookRow): string {
  // v = 全局分量(cover_style_ver) - per-book 分量(coverVer)。
  // per-book 分量保证：即使同一文件名，重新提取/更换封面后 URL 也会变，浏览器不会吃缓存。
  const v = `${_coverBust}-${book.cover_ver ?? 0}`;
  if (!book.cover_path) {
    // 无内嵌封面 → 共享默认封面端点（底图 + 实时叠字）。
    return `http://flowread.localhost/cover-default/${book.id}?v=${v}`;
  }
  // 有内嵌封面 → per-book 端点（/book-cover/<id>），按 id + coverVer 缓存破坏。
  return `http://flowread.localhost/book-cover/${book.id}?v=${v}`;
}
