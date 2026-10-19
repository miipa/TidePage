import { invoke } from "@tauri-apps/api/core";
import { coverTheme, defaultCoverBust, secretsReady } from "./stores.svelte";
import type { AiTestResult, BookImage, BookPage, BookRow, BookSearchHit, BookSession, BookmarkRow, ChapterData, HighlightRow, OpenResult, Progress, SearchHit, TocEntry, UpdateInfo } from "./types";

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
  /** 只取 id 的分批入口（每批 ≤500）。整库重建 FTS 之类「按 id 遍历全库」的操作用它，
   *  不要用 listBooks：后者把 17 万条整行物化并序列化，一次 IPC 就是几十 MB。 */
  listBookIdsPage: (offset: number, limit: number) => invoke<number[]>("list_book_ids_page", { offset, limit }),
  /** 书库挂载并取到首屏行后调用一次：启动日志据此判断「上次是否走到书库」，
   *  连续失败时下一次启动自动跳过封面重建（见 Rust memlog 模块）。 */
  libraryReady: () => invoke<void>("library_ready"),
  listBooksPage: (query: string, sort: string, offset: number, limit: number, formats?: string[], states?: string[], desc?: boolean, folder?: string) =>
    invoke<BookPage>("list_books_page", { query, sort, offset, limit, formats: formats ?? null, states: states ?? null, desc: desc ?? null, folder: folder ?? null }),
  /** 秩可寻址分页命令 1：返回小端 i64 原始字节流（ArrayBuffer，每 8 字节一个 books.id）。
   *  query 为空=全量；非空=与 list_books_page 同源的 title/author 过滤（搜索档同样 O(W)）。
   *  formats/states：分类筛选（格式取「或」、状态取「或」、两组取「且」），空/省略=不约束该维度。
   *  folder：文件夹单选作用域（路径前缀），省略/null=全库。
   *  结果超过后端上限（30 万）时抛 "order_index_too_large:300000"，调用方须切 keyset 档。 */
  listOrderIndex: (sort: string, query: string, formats?: string[], states?: string[], desc?: boolean, folder?: string) =>
    invoke<ArrayBuffer>("list_order_index", { sort, query, formats: formats ?? null, states: states ?? null, desc: desc ?? null, folder: folder ?? null }),
  /** Task B：书库当前实际存在的格式列表（去重、固定排序 count DESC, format ASC）。 */
  listFormats: () => invoke<string[]>("list_formats"),
  /** Task F：手动设置阅读状态覆盖。state 取 "read" | "unread" | "auto"。只写 read_state，不动真实进度。 */
  setReadState: (bookId: number, state: "read" | "unread" | "auto") =>
    invoke<void>("set_read_state", { bookId, stateStr: state }),
  /** 秩可寻址分页命令 2：按 ids 批量取行，返回长度/顺序恒等于 ids，缺行为 null。ids.len()>4096 抛错。 */
  listBooksByIds: (ids: number[]) => invoke<(BookRow | null)[]>("list_books_by_ids", { ids }),
  rescan: () => invoke<void>("rescan"),
  /**
   * 对账刷新。后端在"待删行超阈值"时**一条都不删**，返回 `needs_confirm=true` + 摘要；
   * 前端拿摘要问用户，用户确认后再以 `confirm=true` 调一次。
   */
  refreshLibrary: (confirm = false) =>
    invoke<{
      removed: number;
      added: number;
      /** 被挂起等待确认的待删条数（>0 表示需要二次确认）。 */
      pending: number;
      /** 因父目录不可列举被跳过的条数（>0 且 removed=0 表示本次什么也没删）。 */
      skipped: number;
      needs_confirm: boolean;
      /** 待删行的主要父目录（最多 5 条，降序）。 */
      dirs: { dir: string; count: number }[];
      /** 快照总行数。 */
      total: number;
    }>("refresh_library", { confirm }),
  /**
   * 上次扫描的异常态（`last_scan_stale`，后端每次扫描结束重算）：书库页首屏提示条的唯一来源。
   * 返回原始 JSON 字符串（`state` / `refused` / `pending` / `skipped` / `unfinishedDirs` / `dirs` / `at`）。
   */
  getLastScanStale: () => invoke<string | null>("get_last_scan_stale"),
  /** 三态：ok（存在）/ missing（文件不在，目录可列举）/ unreachable（目录当时不可访问）。 */
  fileExists: (path: string) => invoke<"ok" | "missing" | "unreachable">("file_exists", { path }),
  pathIsDir: (path: string) => invoke<boolean>("path_is_dir", { path }),
  openBook: (id: number) => invoke<OpenResult>("open_book", { id }),
  /** 全书每章字数（EPUB；非 EPUB 返回空数组）：「全书总页数」的估算来源。 */
  bookCharCounts: (bookId: number) => invoke<number[]>("book_char_counts", { bookId }),
  getChapter: (bookId: number, chapterIndex: number, keepStyles = false) =>
    invoke<ChapterData>("get_chapter", { bookId, chapterIndex, keepStyles }),
  /** 读取整本 PDF 的原始字节（仅 pdf）：返回 ArrayBuffer，交前端 pdf.js 解析渲染。 */
  readBookBytes: (bookId: number) => invoke<ArrayBuffer>("read_book_bytes", { bookId }),
  /** PDF 字节总数（决定整读还是 range 分块）与区间读取（pdf.js range 传输用）。 */
  pdfFileSize: (bookId: number) => invoke<number>("pdf_file_size", { bookId }),
  pdfReadRange: (bookId: number, begin: number, end: number) =>
    invoke<ArrayBuffer>("pdf_read_range", { bookId, begin, end }),
  /* ---- 超大 PDF 的 Windows 原生渲染通道（pdf.js 对 >2GiB 的文件无解，见 Rust pdfnative.rs）---- */
  /** 打开原生文档，返回页数（同一本书复用已在内存里的文档句柄）。 */
  pdfNativeOpen: (bookId: number) => invoke<number>("pdf_native_open", { bookId }),
  /** 单页尺寸（页面坐标系，左上原点）：只用于比例与切片换算。 */
  pdfNativePageGeometry: (bookId: number, page: number) =>
    invoke<[number, number]>("pdf_native_page_geometry", { bookId, page }),
  /** 渲染页面的一块区域为 PNG 字节：x/y/w/h 为页面坐标，destW/destH 为目标像素尺寸。 */
  pdfNativeRenderPage: (
    bookId: number,
    page: number,
    x: number,
    y: number,
    w: number,
    h: number,
    destW: number,
    destH: number,
  ) => invoke<ArrayBuffer>("pdf_native_render_page", { bookId, page, x, y, w, h, destW, destH }),
  /** 关闭当前原生文档（关书/换书时调用）。 */
  pdfNativeClose: () => invoke<void>("pdf_native_close"),

  /** PDF 某页纯文本回传（前端 pdf.js 抽取）：写入 chapter_text，供划词全文搜索/FTS/朗读复用。
   *  乱码页/扫描版不回传（见 pdfTextUsable），后端不解析 PDF。 */
  setPdfPageText: (bookId: number, pageIndex: number, title: string, text: string) =>
    invoke<void>("set_pdf_page_text", { bookId, pageIndex, title, text }),
  /** PDF 首页封面：前端传按目标缩略图尺寸渲染好的 WebP（base64，不含 data URL 前缀），
   *  后端只落盘并通知书库刷新；返回落盘文件名（重新提取封面要用它更新 cover_path）。 */
  setBookCoverImage: (bookId: number, webpBase64: string) =>
    invoke<string>("set_book_cover_image", { bookId, webpBase64 }),
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
  /** 只读：拉取云端本书进度（TidePage/progress/{path_id}.json），不写库、不 PUT。无则 null。 */
  webdavReadProgress: needsSecrets((base: string, user: string, pass: string, bookPath: string) =>
    invoke<Progress | null>("webdav_read_progress", { base, user, pass, bookPath })),
  webdavUploadBook: needsSecrets((base: string, user: string, pass: string, bookPath: string) =>
    invoke<string>("webdav_upload_book", { base, user, pass, bookPath })),
  webdavDownloadBooks: needsSecrets((base: string, user: string, pass: string, targetDir: string) =>
    invoke<string[]>("webdav_download_books", { base, user, pass, targetDir })),
  /** 单本书从云端 TidePage/books/ 下载到 targetDir，并按扩展名自动加入书库。 */
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
  /** 上传设置备份到 TidePage/settings/{name}，返回远端路径。 */
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
  // keep：仍被引用的背景图文件名（全局 bgImage 与各方案的 schemeBgImage）；其余旧图按时间窗回收。
  setBgImage: (imagePath: string, keep: string[] = []) => invoke<string>("set_background_image", { imagePath, keep }),
  clearBgImage: (keep: string[] = []) => invoke<void>("clear_background_image", { keep }),
  randomBook: () => invoke<BookRow | null>("random_book"),
  /** 从当前筛选结果随机一书（query/sort/formats/states/folder 与列表同源）。 */
  randomBookFiltered: (query: string, sort: string, formats: string[], states: string[], folder?: string) =>
    invoke<BookRow | null>("random_book_filtered", { query, sort, formats, states, folder: folder ?? null }),
  reextractCover: (bookId: number) => invoke<string>("reextract_cover", { bookId }),
  setCoverScan: (enabled: boolean) => invoke<void>("set_cover_scan", { enabled }),
  pdfBooksMissingCover: () => invoke<number[]>("pdf_books_missing_cover"),
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
    proxyMode?: string; proxyUrl?: string; proxyType?: string;
  }) => invoke<void>("ai_chat", args)),
  aiTest: needsSecrets((args: {
    provider: string; baseUrl: string; apiKey: string; model: string;
    deployment?: string; apiVersion?: string;
    proxyMode: string; proxyUrl: string; proxyType: string;
  }) => invoke<AiTestResult>("ai_test", args)),
  ttsSynth: needsSecrets((args: {
    provider: string; baseUrl: string; apiKey: string;
    secretKey?: string; appId?: string; cluster?: string;
    model: string; voice: string; format: string; text: string;
    lang?: string;
    proxyMode?: string; proxyUrl?: string; proxyType?: string;
  }) => invoke<string>("tts_synth", args)),
  ttsVoices: needsSecrets((
    provider: string,
    baseUrl: string,
    apiKey: string,
    secretKey = "",
    appId = "",
    cluster = "",
    proxyMode = "system",
    proxyUrl = "",
    proxyType = "http",
  ) =>
    invoke<{ id: string; name: string; type: string; language: string }[]>("tts_voices", {
      provider,
      baseUrl,
      apiKey,
      secretKey,
      appId,
      cluster,
      proxyMode,
      proxyUrl,
      proxyType,
    })),
  ttsEnsureFixedVoice: needsSecrets((
    baseUrl: string,
    apiKey: string,
    language: string,
    instruct: string,
    proxyMode = "system",
    proxyUrl = "",
    proxyType = "http",
  ) =>
    invoke<{ id: string; created: boolean }>("tts_ensure_fixed_voice", {
      baseUrl,
      apiKey,
      language,
      instruct,
      proxyMode,
      proxyUrl,
      proxyType,
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
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
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
    return `http://tidepage.localhost/cover-default/${book.id}?v=${v}${q}&db=${defaultCoverBust.n}`;
  }
  // 有内嵌封面 → per-book 端点（/book-cover/<id>），按 id + coverVer 缓存破坏。
  return `http://tidepage.localhost/book-cover/${book.id}?v=${v}`;
}
