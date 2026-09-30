import { invoke } from "@tauri-apps/api/core";
import { coverTheme, defaultCoverBust, secretsReady } from "./stores.svelte";
import type { BookImage, BookPage, BookRow, BookSearchHit, BookSession, BookmarkRow, ChapterData, HighlightRow, OpenResult, Progress, SearchHit, TocEntry } from "./types";

/** 依赖凭据的封装统一先等凭据回填完成再 invoke，避免启动瞬间用到空密钥。
 *  仅延后 invoke；调用方传入的参数保持不变（命令签名不变）。 */
function needsSecrets<A extends unknown[], R>(fn: (...args: A) => Promise<R>) {
  return async (...args: A): Promise<R> => {
    await secretsReady;
    return fn(...args);
  };
}

export const api = {
  listFolders: () => invoke<string[]>("list_folders"),
  addFolder: (path: string) => invoke<void>("add_folder", { path }),
  removeFolder: (path: string) => invoke<void>("remove_folder", { path }),
  listBooks: () => invoke<BookRow[]>("list_books"),
  listBooksPage: (query: string, sort: string, offset: number, limit: number, formats?: string[], states?: string[], desc?: boolean) =>
    invoke<BookPage>("list_books_page", { query, sort, offset, limit, formats: formats ?? null, states: states ?? null, desc: desc ?? null }),
  /** 秩可寻址分页命令 1：返回小端 i64 原始字节流（ArrayBuffer，每 8 字节一个 books.id）。
   *  query 为空=全量；非空=与 list_books_page 同源的 title/author 过滤（搜索档同样 O(W)）。
   *  formats/states：分类筛选（格式取「或」、状态取「或」、两组取「且」），空/省略=不约束该维度。
   *  结果超过后端上限（30 万）时抛 "order_index_too_large:300000"，调用方须切 keyset 档。 */
  listOrderIndex: (sort: string, query: string, formats?: string[], states?: string[], desc?: boolean) =>
    invoke<ArrayBuffer>("list_order_index", { sort, query, formats: formats ?? null, states: states ?? null, desc: desc ?? null }),
  /** Task B：书库当前实际存在的格式列表（去重、固定排序 count DESC, format ASC）。 */
  listFormats: () => invoke<string[]>("list_formats"),
  /** Task F：手动设置阅读状态覆盖。state 取 "read" | "unread" | "auto"。只写 read_state，不动真实进度。 */
  setReadState: (bookId: number, state: "read" | "unread" | "auto") =>
    invoke<void>("set_read_state", { bookId, stateStr: state }),
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
    invoke<
      {
        bookId: number; title: string; author: string; deleted: boolean;
        /** 划线条数（含无感想的划线） */ hlCount: number;
        /** 其中带感想的条数 */ noteCount: number;
        lastAt: number;
        /** 内嵌封面文件名（空 = 无内嵌封面，走共享默认封面端点） */ coverPath: string;
        coverVer: number;
      }[]
    >("list_note_books"),
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
  webdavTest: needsSecrets((base: string, user: string, pass: string) =>
    invoke<boolean>("webdav_test", { base, user, pass })),
  webdavSyncProgress: needsSecrets((
    base: string, user: string, pass: string,
    bookId: number, bookPath: string, local: Progress | null,
  ) => invoke<Progress>("webdav_sync_progress", { base, user, pass, bookId, bookPath, local })),
  /** 只读：拉取云端本书进度（FlowRead/progress/{path_id}.json），不写库、不 PUT。无则 null。 */
  webdavReadProgress: needsSecrets((base: string, user: string, pass: string, bookPath: string) =>
    invoke<Progress | null>("webdav_read_progress", { base, user, pass, bookPath })),
  webdavUploadBook: needsSecrets((base: string, user: string, pass: string, bookPath: string) =>
    invoke<string>("webdav_upload_book", { base, user, pass, bookPath })),
  webdavDownloadBooks: needsSecrets((base: string, user: string, pass: string, targetDir: string) =>
    invoke<string[]>("webdav_download_books", { base, user, pass, targetDir })),
  /** 单本书从云端 FlowRead/books/ 下载到 targetDir，并按扩展名自动加入书库。 */
  webdavDownloadBook: needsSecrets((base: string, user: string, pass: string, bookPath: string, targetDir: string) =>
    invoke<string>("webdav_download_book", { base, user, pass, bookPath, targetDir })),
  webdavList: needsSecrets((base: string, user: string, pass: string, path: string) =>
    invoke<{ name: string; is_dir: boolean; size: number; mtime: number }[]>("webdav_list", { base, user, pass, path })),
  /** 删除云端文件/目录。 */
  webdavDelete: needsSecrets((base: string, user: string, pass: string, path: string) =>
    invoke<void>("webdav_delete", { base, user, pass, path })),
  /** 复制云端文件/目录（不覆盖同名项）。 */
  webdavCopy: needsSecrets((base: string, user: string, pass: string, src: string, dst: string) =>
    invoke<void>("webdav_copy", { base, user, pass, src, dst })),
  /** 移动/重命名云端文件/目录（不覆盖同名项）。 */
  webdavMove: needsSecrets((base: string, user: string, pass: string, src: string, dst: string) =>
    invoke<void>("webdav_move", { base, user, pass, src, dst })),
  webdavDownloadFile: needsSecrets((base: string, user: string, pass: string, remotePath: string, targetDir: string) =>
    invoke<string[]>("webdav_download_file", { base, user, pass, remotePath, targetDir })),
  webdavDownloadProgress: needsSecrets((base: string, user: string, pass: string, remotePath: string, targetDir: string, session: string) =>
    invoke<string[]>("webdav_download_progress", { base, user, pass, remotePath, targetDir, session })),
  /** 上传设置备份到 FlowRead/settings/{name}，返回远端路径。 */
  webdavBackupSettings: needsSecrets((base: string, user: string, pass: string, name: string, data: string) =>
    invoke<string>("webdav_backup_settings", { base, user, pass, name, data })),
  /** 读取云端指定设置备份；无则 null。 */
  webdavRestoreSettings: needsSecrets((base: string, user: string, pass: string, name: string) =>
    invoke<string | null>("webdav_restore_settings", { base, user, pass, name })),
  setSetting: (key: string, value: string) => invoke<void>("set_setting_kv", { key, value }),
  getSetting: (key: string) => invoke<string | null>("get_setting_kv", { key }),
  /** 凭据加密落盘（DPAPI）：传入 JSON 明文串，Rust 侧加密后写入 settings.secret.config。 */
  secretWrite: (plain: string) => invoke<void>("secret_write", { plain }),
  /** 读回并解密凭据 JSON；无记录返回 null。 */
  secretRead: () => invoke<string | null>("secret_read"),
  /** 本机设备名（用于备份文件名）。 */
  hostName: () => invoke<string>("host_name"),
  /** 用口令派生密钥加密（AES-256-GCM），返回可入 JSON 的封装串。 */
  backupSeal: (plain: string, passphrase: string) => invoke<string>("backup_seal", { plain, passphrase }),
  /** 用口令解密封装串；口令不符/损坏时 reject。 */
  backupOpen: (blob: string, passphrase: string) => invoke<string>("backup_open", { blob, passphrase }),
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
  /** 从当前筛选结果随机一书（query/sort/formats/states 与列表同源）。 */
  randomBookFiltered: (query: string, sort: string, formats: string[], states: string[]) =>
    invoke<BookRow | null>("random_book_filtered", { query, sort, formats, states }),
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
  aiChat: needsSecrets((args: {
    id: string; provider: string; baseUrl: string; apiKey: string;
    model: string; system: string; messages: [string, string][];
    deployment?: string; apiVersion?: string;
    proxyMode?: string; proxyUrl?: string;
  }) => invoke<void>("ai_chat", args)),
  aiTest: needsSecrets((args: {
    provider: string; baseUrl: string; apiKey: string; model: string;
    deployment?: string; apiVersion?: string;
    proxyMode: string; proxyUrl: string;
  }) => invoke<string>("ai_test", args)),
  ttsSynth: needsSecrets((args: {
    provider: string; baseUrl: string; apiKey: string;
    secretKey?: string; appId?: string; cluster?: string;
    model: string; voice: string; format: string; text: string;
    lang?: string;
    proxyMode?: string; proxyUrl?: string;
  }) => invoke<string>("tts_synth", args)),
  ttsVoices: needsSecrets((
    provider: string,
    baseUrl: string,
    apiKey: string,
    secretKey = "",
    appId = "",
    cluster = "",
  ) =>
    invoke<{ id: string; name: string; type: string; language: string }[]>("tts_voices", {
      provider,
      baseUrl,
      apiKey,
      secretKey,
      appId,
      cluster,
    })),
  ttsEnsureFixedVoice: needsSecrets((baseUrl: string, apiKey: string, language: string, instruct: string) =>
    invoke<{ id: string; created: boolean }>("tts_ensure_fixed_voice", {
      baseUrl,
      apiKey,
      language,
      instruct,
    })),
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

export function coverUrl(book: Pick<BookRow, "id" | "cover_path" | "cover_ver">): string {
  // v = 全局分量(cover_style_ver) - per-book 分量(coverVer)。
  // per-book 分量保证：即使同一文件名，重新提取/更换封面后 URL 也会变，浏览器不会吃缓存。
  const v = `${_coverBust}-${book.cover_ver ?? 0}`;
  if (!book.cover_path) {
    // 无内嵌封面 → 共享默认封面端点（底图 + 实时叠字）。
    // 追加外壳主题配色（g1/g2/g3/tc）：读 coverTheme（响应式）——切主题即改 URL、自动重取、
    // 默认封面颜色随主题变化；且这些参数令 URL 随主题唯一，规避 webview 对自定义协议的旧缓存。
    const t = coverTheme;
    const q =
      `&g1=${encodeURIComponent(t.g1)}&g2=${encodeURIComponent(t.g2)}` +
      `&g3=${encodeURIComponent(t.g3)}&tc=${encodeURIComponent(t.tc)}`;
    // 追加默认底图缓存破坏分量（仅此端点）：换/恢复默认底图时 bumpDefaultCoverBust() 自增，
    // 内部读 defaultCoverBust.n 建立响应式依赖，令无内嵌封面的 <img> 即时重取新底图。
    // /book-cover/ 端点不追加此参数，故有内嵌封面的书不受换底图影响、不闪白。
    return `http://flowread.localhost/cover-default/${book.id}?v=${v}${q}&db=${defaultCoverBust.n}`;
  }
  // 有内嵌封面 → per-book 端点（/book-cover/<id>），按 id + coverVer 缓存破坏。
  return `http://flowread.localhost/book-cover/${book.id}?v=${v}`;
}
