use crate::covers;
use crate::db::{self, Db};
use crate::models::*;
use crate::txt;
use futures_util::StreamExt;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct CachedEpub {
    pub doc: crate::epub::EpubDoc,
    pub data: Arc<Vec<u8>>,
}

pub struct AppState {
    pub db: Db,
    pub app: AppHandle,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub last_fs_event: AtomicU64,
    pub rules_ver: AtomicU64,
    pub txt_cache: Mutex<HashMap<i64, (u64, Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>)>>,
    pub epub_cache: Mutex<HashMap<i64, Arc<CachedEpub>>>,
    pub http: reqwest::Client,
}

const EXT_BOOK: &[&str] = &["txt", "epub", "mobi", "azw3", "prc"];

pub fn ext_of(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .filter(|e| EXT_BOOK.contains(&e.as_str()))
}

fn file_mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn quick_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn get_rules(state: &AppState) -> String {
    let conn = state.db.0.lock().unwrap();
    db::get_setting(&conn, "txt_rules").unwrap_or_else(|| crate::default_pattern().into())
}

pub async fn scan_folders(state: &AppState) {
    let folders = {
        let conn = state.db.0.lock().unwrap();
        db::list_folders(&conn)
    };
    let mut found: HashMap<String, (i64, i64, String, String)> = HashMap::new();
    for folder in &folders {
        for entry in walkdir::WalkDir::new(folder)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path().to_string_lossy().into_owned();
            if let Some(ext) = ext_of(&path) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as i64;
                let mtime = file_mtime(entry.path());
                let title = quick_title(&path);
                found.insert(path, (size, mtime, title, ext));
            }
        }
    }

    let to_insert: Vec<(String, i64, i64, String, String)> = {
        let conn = state.db.0.lock().unwrap();
        found
            .iter()
            .filter(|(path, (size, mtime, _, _))| {
                match db::get_book_by_path(&conn, path) {
                    Some(b) => b.size != *size || b.mtime != *mtime,
                    None => true,
                }
            })
            .map(|(p, (s, m, t, e))| (p.clone(), *s, *m, t.clone(), e.clone()))
            .collect()
    };

    {
        let conn = state.db.0.lock().unwrap();
        let all = db::all_book_paths(&conn);
        let stale: Vec<String> = all
            .into_iter()
            .filter(|(_, p)| {
                !found.contains_key(p)
                    && folders
                        .iter()
                        .any(|f| std::path::Path::new(p).starts_with(std::path::Path::new(f)))
            })
            .map(|(_, p)| p)
            .collect();
        if !stale.is_empty() {
            db::delete_books(&conn, &stale).ok();
            drop(conn);
            state.app.emit("library-changed", ()).ok();
        }
    }

    if to_insert.is_empty() {
        let needs_meta = {
            let conn = state.db.0.lock().unwrap();
            !db::books_needing_meta(&conn).is_empty()
        };
        if needs_meta {
            spawn_meta_worker(state.app.clone());
        }
        return;
    }

    {
        let conn = state.db.0.lock().unwrap();
        for (path, size, mtime, title, ext) in &to_insert {
            db::upsert_book(&conn, path, title, ext, *size, *mtime).ok();
        }
    }
    state.app.emit("library-changed", ()).ok();
    spawn_meta_worker(state.app.clone());
}

pub fn install_watcher(state: &Arc<AppState>) {
    let folders = {
        let conn = state.db.0.lock().unwrap();
        db::list_folders(&conn)
    };
    let state2 = state.clone();
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if res.is_ok() {
                state2.last_fs_event.store(db::now_ms() as u64, Ordering::Relaxed);
            }
        })
        .ok();
    if let Some(w) = watcher.as_mut() {
        for f in &folders {
            let _ = notify::Watcher::watch(w, Path::new(f), notify::RecursiveMode::Recursive);
        }
    }
    *state.watcher.lock().unwrap() = watcher;
}

pub fn spawn_watcher_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_handled: u64 = db::now_ms() as u64;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(2000));
            let state = match app.try_state::<Arc<AppState>>() {
                Some(s) => s.inner().clone(),
                None => continue,
            };
            let last = state.last_fs_event.load(Ordering::Relaxed);
            if last > last_handled && (db::now_ms() as u64) - last > 2500 {
                last_handled = last;
                tauri::async_runtime::block_on(scan_folders(&state));
            }
        }
    });
}

pub fn spawn_meta_worker(app: AppHandle) {
    std::thread::spawn(move || {
        let state = match app.try_state::<Arc<AppState>>() {
            Some(s) => s.inner().clone(),
            None => return,
        };
        let busy = {
            let conn = state.db.0.lock().unwrap();
            db::get_setting(&conn, "meta_busy").unwrap_or_default()
        };
        if busy == "1" {
            return;
        }
        {
            let conn = state.db.0.lock().unwrap();
            db::set_setting(&conn, "meta_busy", "1");
        }
        let covers_dir = covers::covers_dir(&app);
        let chunk = 12;
        loop {
            let batch = {
                let conn = state.db.0.lock().unwrap();
                db::books_needing_meta(&conn)
            };
            if batch.is_empty() {
                break;
            }
            for slice in batch.chunks(chunk) {
                let results: Vec<(i64, String, String, Option<String>, i64, i64)> = slice
                    .par_iter()
                    .map(|(id, path, format)| {
                        let (title, author, cover, chars, text_chars) =
                            extract_meta(path, format, &covers_dir);
                        (*id, title, author, cover, chars, text_chars)
                    })
                    .collect();
                {
                    let conn = state.db.0.lock().unwrap();
                    for (id, title, author, cover, chars, text_chars) in results {
                        db::set_book_meta(&conn, id, &title, &author, cover.as_deref(), chars, text_chars)
                            .ok();
                    }
                }
                state.app.emit("library-meta", ()).ok();
            }
        }
        {
            let conn = state.db.0.lock().unwrap();
            db::set_setting(&conn, "meta_busy", "");
        }
    });
}

fn extract_meta(
    path: &str,
    format: &str,
    covers_dir: &Path,
) -> (String, String, Option<String>, i64, i64) {
    let fallback = quick_title(path);
    match format {
        "epub" => {
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(_) => return (fallback, String::new(), None, 0, 0),
            };
            match crate::epub::parse(data.clone()) {
                Ok(doc) => {
                    let cover_file = doc.cover_href.as_ref().and_then(|href| {
                        crate::epub::read_cover(&data, href).and_then(|bytes| {
                            let name = format!("{}.png", db::path_id(path));
                            let out = covers_dir.join(&name);
                            covers::make_thumbnail(&bytes, &out).ok()?;
                            Some(name)
                        })
                    });
                    let cover_file = cover_file.or_else(|| {
                        crate::epub::find_cover_fallback(&data, &doc.spine).and_then(|bytes| {
                            let name = format!("{}.png", db::path_id(path));
                            let out = covers_dir.join(&name);
                            covers::make_thumbnail(&bytes, &out).ok()?;
                            Some(name)
                        })
                    });
                    let cover_file = cover_file.or_else(|| {
                        let name = format!("{}.svg", db::path_id(path));
                        let out = covers_dir.join(&name);
                        covers::write_placeholder(&out, &doc.title, &doc.author).ok()?;
                        Some(name)
                    });
                    (
                        if doc.title.is_empty() { fallback } else { doc.title },
                        doc.author,
                        cover_file,
                        doc.total_chars,
                        doc.total_text_chars,
                    )
                }
                Err(_) => {
                    let name = format!("{}.svg", db::path_id(path));
                    let out = covers_dir.join(&name);
                    covers::write_placeholder(&out, &fallback, "").ok();
                    (fallback, String::new(), Some(name), 0, 0)
                }
            }
        }
        "mobi" | "azw3" | "prc" => {
            // 一次读取/一次解析同时拿到正文与封面，避免对大书重复整读两遍
            let (doc_res, cover_bytes) = crate::mobi::parse_with_cover(path);
            match doc_res {
                Ok(doc) => {
                    // 优先书内真实封面（EXTH201/202/129 或首图），失败才回退占位 SVG
                    let cover_file = cover_bytes
                        .and_then(|bytes| {
                            let name = format!("{}.png", db::path_id(path));
                            let out = covers_dir.join(&name);
                            covers::make_thumbnail(&bytes, &out).ok()?;
                            Some(name)
                        })
                        .or_else(|| {
                            let name = format!("{}.svg", db::path_id(path));
                            let out = covers_dir.join(&name);
                            covers::write_placeholder(&out, &doc.title, &doc.author).ok()?;
                            Some(name)
                        });
                    let text = crate::epub::html_to_text(&doc.html);
                    let chars = text.chars().count() as i64;
                    let text_chars = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
                    (
                        if doc.title.is_empty() { fallback } else { doc.title },
                        doc.author,
                        cover_file,
                        chars,
                        text_chars,
                    )
                }
                Err(_) => (fallback, String::new(), None, 0, 0),
            }
        }
        _ => {
            let name = format!("{}.svg", db::path_id(path));
            let out = covers_dir.join(&name);
            covers::write_placeholder(&out, &fallback, "").ok();
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(_) => return (fallback, String::new(), Some(name), 0, 0),
            };
            let (text, _) = crate::txt::decode_bytes(&data);
            let chars = text.chars().count() as i64;
            let text_chars = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
            (fallback, String::new(), Some(name), chars, text_chars)
        }
    }
}

fn push_cache<K, V>(map: &Mutex<HashMap<K, V>>, key: K, value: V, max: usize)
where
    K: std::hash::Hash + Eq + Copy,
{
    let mut m = map.lock().unwrap();
    if m.len() >= max {
        if let Some(first) = m.keys().next().copied() {
            m.remove(&first);
        }
    }
    m.insert(key, value);
}

pub fn load_txt(
    state: &AppState,
    path: &str,
    book_id: i64,
) -> Result<Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>, String> {
    let ver = state.rules_ver.load(Ordering::Relaxed);
    if let Some(cached) = state.txt_cache.lock().unwrap().get(&book_id) {
        if cached.0 == ver {
            return Ok(cached.1.clone());
        }
    }
    let pattern = get_rules(state);
    let doc = txt::decode_and_split(path, Some(&pattern))?;
    let result = Arc::new((doc.text, doc.ranges, doc.toc));
    push_cache(&state.txt_cache, book_id, (ver, result.clone()), 3);
    Ok(result)
}

pub fn load_epub(state: &AppState, path: &str, book_id: i64) -> Result<Arc<CachedEpub>, String> {
    if let Some(c) = state.epub_cache.lock().unwrap().get(&book_id) {
        return Ok(c.clone());
    }
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let doc = crate::epub::parse(data.clone())?;
    let cached = Arc::new(CachedEpub {
        doc,
        data: Arc::new(data),
    });
    push_cache(&state.epub_cache, book_id, cached.clone(), 3);
    Ok(cached)
}

fn safe_slice(s: &str, start: usize, end: usize) -> String {
    let start = start.min(s.len());
    let end = end.min(s.len()).max(start);
    let mut s_i = start;
    while s_i < end && !s.is_char_boundary(s_i) {
        s_i += 1;
    }
    let mut e_i = end;
    while e_i < s.len() && !s.is_char_boundary(e_i) {
        e_i += 1;
    }
    s[s_i..e_i].to_string()
}

#[tauri::command]
pub fn list_folders(state: State<'_, Arc<AppState>>) -> Vec<String> {
    let conn = state.db.0.lock().unwrap();
    db::list_folders(&conn)
}

#[tauri::command]
pub async fn add_folder(state: State<'_, Arc<AppState>>, path: String) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err("文件夹不存在".into());
    }
    {
        let conn = state.db.0.lock().unwrap();
        db::add_folder(&conn, &path)?;
    }
    let st = state.inner().clone();
    tokio::spawn(async move {
        scan_folders(&st).await;
    });
    Ok(())
}

#[tauri::command]
pub fn remove_folder(state: State<'_, Arc<AppState>>, path: String) -> Result<(), String> {
    {
        let conn = state.db.0.lock().unwrap();
        db::remove_folder(&conn, &path)?;
        let base = path.trim_end_matches(['/', '\\']);
        let esc = base.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let like = format!("{}%", esc);
        let mut stmt = conn
            .prepare("SELECT path FROM books WHERE path LIKE ?1 ESCAPE '\\'")
            .map_err(|e| e.to_string())?;
        let doomed: Vec<String> = stmt
            .query_map(rusqlite::params![like], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        drop(stmt);
        if !doomed.is_empty() {
            db::delete_books(&conn, &doomed).ok();
        }
    }
    state.app.emit("library-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub fn list_books(state: State<'_, Arc<AppState>>) -> Vec<BookRow> {
    let conn = state.db.0.lock().unwrap();
    db::list_books(&conn)
}

/// 超大书库分页：过滤/排序下推 SQLite，单次 IPC 最多 500 行 + total。
/// 用 spawn_blocking 放到阻塞线程池，避免长查询占用主线程（异步命令不阻塞 UI）。
#[tauri::command]
pub async fn list_books_page(
    state: State<'_, Arc<AppState>>,
    query: String,
    sort: String,
    offset: i64,
    limit: i64,
) -> Result<BookPage, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_books_page(&conn, &query, &sort, offset, limit)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rescan(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let st = state.inner().clone();
    tokio::spawn(async move {
        scan_folders(&st).await;
    });
    Ok(())
}

#[derive(serde::Serialize)]
pub struct RefreshResult {
    pub removed: usize,
    pub added: usize,
}

/// 任务2 核心：一次性对账（纯目录遍历，不新增任何文件系统监听）。
/// 1) 记录路径在磁盘上不存在的 → 从索引移除并计数；
/// 2) 扫描集 = 存活记录所在目录 ∪ 已登记书库根目录及其**一级子目录**；
///    逐目录只扫一层，磁盘存在但索引里没有的书籍新增入库（不做全递归）；
/// 3) 返回 {removed, added}。不触碰正在阅读的窗口/reader 状态（不强关）。
pub fn refresh_library_core(conn: &rusqlite::Connection) -> RefreshResult {
    let records = db::all_book_paths(conn);
    let mut missing: Vec<String> = Vec::new();
    let mut survivors: Vec<String> = Vec::new();
    for (_, p) in &records {
        if Path::new(p).exists() {
            survivors.push(p.clone());
        } else {
            missing.push(p.clone());
        }
    }
    if !missing.is_empty() {
        let _ = db::delete_books(conn, &missing);
    }

    let mut scan_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for p in &survivors {
        if let Some(d) = Path::new(p.as_str()).parent() {
            scan_dirs.insert(d.to_path_buf());
        }
    }
    // 已登记书库根：并入根目录本身 + 其一级子目录（新落在子目录的书也能被发现）。
    for root in db::list_folders(conn) {
        let rp = PathBuf::from(&root);
        if !rp.is_dir() {
            continue;
        }
        scan_dirs.insert(rp.clone());
        if let Ok(rd) = std::fs::read_dir(&rp) {
            for entry in rd.flatten() {
                let sub = entry.path();
                if sub.is_dir() {
                    scan_dirs.insert(sub);
                }
            }
        }
    }

    let mut added = 0usize;
    for dir in &scan_dirs {
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in rd.flatten() {
            let fp = entry.path();
            if !fp.is_file() {
                continue;
            }
            let path = fp.to_string_lossy().into_owned();
            let ext = match ext_of(&path) {
                Some(e) => e,
                None => continue,
            };
            if db::get_book_by_path(conn, &path).is_some() {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as i64;
            let mtime = file_mtime(&fp);
            let title = quick_title(&path);
            if db::upsert_book(conn, &path, &title, &ext, size, mtime).is_ok() {
                added += 1;
            }
        }
    }
    RefreshResult { removed: missing.len(), added }
}

#[tauri::command]
pub async fn refresh_library(state: State<'_, Arc<AppState>>) -> Result<RefreshResult, String> {
    let res = {
        let conn = state.db.0.lock().unwrap();
        refresh_library_core(&conn)
    };
    if res.removed > 0 || res.added > 0 {
        state.app.emit("library-changed", ()).ok();
    }
    // A：刷新只做索引对账，不触发任何元数据/AI 任务；新入书 meta_done 保持未生成（=0）。
    Ok(res)
}

/// 任务3：打开入口先查文件是否存在（不存在则由前端弹询问，绝不进入空白阅读）。
#[tauri::command]
pub fn file_exists(path: String) -> bool {
    !path.is_empty() && Path::new(&path).exists()
}

#[tauri::command]
pub async fn open_book(state: State<'_, Arc<AppState>>, id: i64) -> Result<OpenResult, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, id).ok_or("书籍不存在")?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let st = state.inner().clone();
    let (toc, docs, warn) = if ext == "epub" {
        let cached = load_epub(&st, &book.path, id)?;
        let docs = cached
            .doc
            .toc
            .iter()
            .map(|t| {
                cached
                    .doc
                    .spine
                    .get(t.index)
                    .map(|s| s.rsplit('/').next().unwrap_or(s).to_string())
                    .unwrap_or_default()
            })
            .collect();
        (cached.doc.toc.clone(), docs, None)
    } else if ext == "txt" {
        let cached = load_txt(&st, &book.path, id)?;
        (cached.2.clone(), Vec::new(), None)
    } else {
        let doc = mobi_doc_for(&st, &book.path, id)?;
        let toc = doc.toc.clone();
        let html = doc.html.clone();
        let parts = doc.parts.clone();
        let ver = st.rules_ver.load(Ordering::Relaxed);
        push_cache(&st.txt_cache, id, (ver, Arc::new((html, parts, toc.clone()))), 3);
        (toc, Vec::new(), doc.warning)
    };
    Ok(OpenResult {
        book,
        toc,
        docs,
        encoding_warning: warn,
    })
}

#[tauri::command]
pub async fn get_chapter(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    chapter_index: i64,
) -> Result<ChapterData, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let idx = chapter_index.max(0) as usize;
    let st = state.inner().clone();

    if ext == "epub" {
        let cached = load_epub(&st, &book.path, book_id)?;
        let spine_idx = cached
            .doc
            .toc
            .get(idx)
            .map(|t| t.index)
            .unwrap_or(idx);
        let doc_path = cached.doc.spine.get(spine_idx).ok_or("章节文件缺失")?;
        let html = crate::epub::chapter_html(&cached.data, doc_path)?;
        let chars = cached.doc.spine_chars.get(spine_idx).copied().unwrap_or(0);
        let title = cached
            .doc
            .toc
            .get(idx)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| format!("第 {} 节", idx + 1));
        return Ok(ChapterData { title, html, chars });
    }

    let is_mobi = ext == "mobi" || ext == "azw3" || ext == "prc";
    let ver = st.rules_ver.load(Ordering::Relaxed);
    let cached_hit = {
        let m = st.txt_cache.lock().unwrap();
        m.get(&book_id).filter(|c| c.0 == ver).map(|c| c.1.clone())
    };
    let (content, ranges, toc) = match cached_hit {
        Some(c) => c.as_ref().clone(),
        None => {
            let entry = if is_mobi {
                let doc = mobi_doc_for(&st, &book.path, book_id)?;
                Arc::new((doc.html.clone(), doc.parts.clone(), doc.toc.clone()))
            } else {
                load_txt(&st, &book.path, book_id)?
            };
            push_cache(&st.txt_cache, book_id, (ver, entry.clone()), 3);
            entry.as_ref().clone()
        }
    };
    let (start, end) = ranges.get(idx).copied().ok_or("章节越界")?;
    let body = safe_slice(&content, start, end);
    let title = toc
        .get(idx)
        .map(|t| t.title.clone())
        .unwrap_or_default();
    let chars = toc.get(idx).map(|t| t.chars).unwrap_or(0);
    let html = if ext == "txt" {
        txt::render_html_with_title(&body)
    } else {
        body
    };
    Ok(ChapterData { title, html, chars })
}

#[tauri::command]
pub fn save_progress(state: State<'_, Arc<AppState>>, book_id: i64, p: Progress) -> Result<(), String> {
    let mut p = p;
    p.updated_at = db::now_ms();
    p.device = "flowread-pc".into();
    let conn = state.db.0.lock().unwrap();
    db::set_progress(&conn, book_id, &p)?;
    Ok(())
}

#[tauri::command]
pub fn load_progress(state: State<'_, Arc<AppState>>, book_id: i64) -> Option<Progress> {
    let conn = state.db.0.lock().unwrap();
    db::get_progress(&conn, book_id)
}

#[tauri::command]
pub fn delete_books(
    state: State<'_, Arc<AppState>>,
    paths: Vec<String>,
    from_disk: bool,
) -> Result<(), String> {
    {
        let conn = state.db.0.lock().unwrap();
        db::delete_books(&conn, &paths)?;
    }
    if from_disk {
        for p in &paths {
            trash::delete(p).map_err(|e| format!("删除失败: {e:?}"))?;
        }
    }
    state.app.emit("library-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub async fn preview_split(path: String, pattern: String) -> Result<Vec<TocEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let pattern = if pattern.trim().is_empty() { crate::default_pattern().to_string() } else { pattern };
        let doc = txt::decode_and_split(&path, Some(&pattern))?;
        Ok(doc.toc.into_iter().take(300).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn save_split_rules(state: State<'_, Arc<AppState>>, pattern: String) -> Result<(), String> {
    {
        let conn = state.db.0.lock().unwrap();
        db::set_setting(&conn, "txt_rules", &pattern);
    }
    state.rules_ver.fetch_add(1, Ordering::Relaxed);
    state.txt_cache.lock().unwrap().clear();
    Ok(())
}

/// 用当前设置解析 MOBI/AZW3：强制编码 + 图片提取到 res/<book_id>/
pub fn mobi_doc_for(st: &Arc<AppState>, path: &str, book_id: i64) -> Result<crate::mobi::MobiDoc, String> {
    let force = {
        let conn = st.db.0.lock().unwrap();
        db::get_setting(&conn, "mobi.encoding").unwrap_or_default()
    };
    let force = {
        let t = force.trim().to_ascii_lowercase();
        if t.is_empty() || t == "auto" { None } else { Some(t) }
    };
    let dir = st
        .app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("res")
        .join(book_id.to_string());
    let prefix = format!("http://flowread.localhost/res/{book_id}/");
    let doc = crate::mobi::parse_full(path, force.as_deref(), Some((&dir, &prefix)))?;
    if std::env::var("FLOWREAD_MOBI_DIAG").is_ok() {
        eprintln!(
            "[mobi] {path} imgs={} res_urls={} placeholder={}",
            doc.image_count, doc.res_urls, doc.placeholder_count
        );
    }
    Ok(doc)
}

#[tauri::command]
pub fn set_mobi_encoding(state: State<'_, Arc<AppState>>, value: String) -> Result<(), String> {
    let v = value.trim().to_ascii_lowercase();
    {
        let conn = state.db.0.lock().unwrap();
        db::set_setting(&conn, "mobi.encoding", &v);
    }
    state.rules_ver.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn convert_book(
    path: String,
    target: String,
    pattern: String,
    out_path: String,
    author: Option<String>,
    cover_path: Option<String>,
    css: Option<String>,
    title: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ext = ext_of(&path).unwrap_or_default();
        let is_mobi_like = ext == "mobi" || ext == "azw3" || ext == "prc";
        if (ext == "txt" && target == "epub")
            || (is_mobi_like && (target == "txt" || target == "epub"))
            || (ext == "epub" && target == "txt")
        {
            let mut book = if ext == "txt" {
                crate::convert::txt_to_epub_data(&path, &pattern)?
            } else if is_mobi_like {
                crate::convert::mobi_to_ebook_data(&path)?
            } else {
                crate::convert::epub_to_txt_data(&path)?
            };
            if let Some(t) = &title {
                if !t.trim().is_empty() {
                    book.title = t.trim().to_string();
                }
            }
            if let Some(a) = &author {
                if !a.trim().is_empty() {
                    book.author = a.trim().to_string();
                }
            }
            if target == "txt" {
                crate::convert::write_txt(&book, &out_path)?;
                return Ok(out_path);
            }
            let cover_bytes = match &cover_path {
                Some(p) if !p.is_empty() => {
                    let e = ext_of_img(p);
                    Some((std::fs::read(p).map_err(|e| format!("封面读取失败: {e}"))?, e))
                }
                _ => None,
            };
            let opts = crate::convert::EpubOptions {
                cover: cover_bytes.as_ref().map(|(b, e)| (b.as_slice(), e.as_str())),
                author: author.as_deref(),
                css: css.as_deref(),
            };
            crate::convert::write_epub(&book, &opts, &out_path)?;
            return Ok(out_path);
        }
        Err(format!("{ext} → {target} 暂不支持"))
    })
    .await
    .map_err(|e| format!("转换任务失败: {e}"))?
}

fn ext_of_img(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into())
}

#[tauri::command]
pub async fn pick_file(title: String, filters: Vec<FilterSpec>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = rfd::FileDialog::new().set_title(&title);
        for f in filters {
            dlg = dlg.add_filter(&f.name, &f.exts);
        }
        dlg.pick_file().map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[derive(serde::Deserialize)]
pub struct FilterSpec {
    pub name: String,
    pub exts: Vec<String>,
}

#[tauri::command]
pub async fn pick_folder(title: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_title(&title)
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub async fn pick_save(title: String, ext: String, default_name: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dlg = rfd::FileDialog::new()
            .set_title(&title)
            .add_filter(&ext.to_uppercase(), &[&ext])
            .set_file_name(&default_name);
        let path = dlg.save_file()?;
        let p = if path.to_string_lossy().to_lowercase().ends_with(&format!(".{ext}")) {
            path
        } else {
            std::path::PathBuf::from(format!("{}.{}", path.to_string_lossy(), ext))
        };
        Some(p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[derive(serde::Deserialize)]
pub struct AiExportItemIn {
    pub title: String,
    pub html: String,
}

#[tauri::command]
pub async fn export_ai(
    path: String,
    target: String,
    title: String,
    author: String,
    items: Vec<AiExportItemIn>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let items: Vec<crate::convert::AiExportItem> = items
            .into_iter()
            .map(|i| crate::convert::AiExportItem { title: i.title, html: i.html })
            .collect();
        if target == "epub" {
            crate::convert::ai_to_epub(&title, &author, &items, &path)?;
        } else {
            crate::convert::ai_to_txt(&title, &author, &items, &path)?;
        }
        Ok(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn set_custom_cover(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    image_path: String,
) -> Result<(), String> {
    let data = std::fs::read(&image_path).map_err(|e| e.to_string())?;
    let dir = covers::covers_dir(&state.app);
    let name = format!("custom-{book_id}.png");
    let out = dir.join(&name);
    covers::make_thumbnail(&data, &out).map_err(|e| format!("图片处理失败: {e}"))?;
    let rel = name;
    let conn = state.db.0.lock().unwrap();
    conn.execute(
        "UPDATE books SET cover_path=?2 WHERE id=?1",
        rusqlite::params![book_id, rel],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn webdav_test(base: String, user: String, pass: String) -> Result<bool, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.test().await
}

#[tauri::command]
pub async fn webdav_sync_progress(
    state: State<'_, Arc<AppState>>,
    base: String,
    user: String,
    pass: String,
    book_id: i64,
    book_path: String,
    local: Option<Progress>,
) -> Result<Progress, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.ensure_dir("FlowRead").await.ok();
    let remote_key = format!("FlowRead/progress/{}.json", db::path_id(&book_path));
    let local_p = local.filter(|p| p.updated_at > 0);

    let remote_p: Option<Progress> = match c.get(&remote_key).await {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    };

    let merged = match (&local_p, &remote_p) {
        (Some(l), Some(r)) => {
            if r.updated_at > l.updated_at {
                r.clone()
            } else {
                l.clone()
            }
        }
        (Some(l), None) => l.clone(),
        (None, Some(r)) => r.clone(),
        (None, None) => return Err("没有可同步的进度".into()),
    };

    if remote_p.is_none() || remote_p.as_ref().map(|r| r.updated_at) != local_p.as_ref().map(|l| l.updated_at) {
        let data = serde_json::to_vec(&merged).map_err(|e| e.to_string())?;
        c.put(&remote_key, data).await?;
    }

    {
        let conn = state.db.0.lock().unwrap();
        db::set_progress(&conn, book_id, &merged)?;
    }
    Ok(merged)
}

#[tauri::command]
pub async fn webdav_upload_book(
    base: String,
    user: String,
    pass: String,
    book_path: String,
) -> Result<String, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.ensure_dir("FlowRead").await?;
    c.ensure_dir("FlowRead/books").await?;
    let name = Path::new(&book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or("路径无效")?;
    let data = std::fs::read(&book_path).map_err(|e| e.to_string())?;
    c.put(&format!("FlowRead/books/{name}"), data).await?;
    Ok(format!("FlowRead/books/{name}"))
}

#[tauri::command]
pub async fn webdav_download_books(
    state: State<'_, Arc<AppState>>,
    base: String,
    user: String,
    pass: String,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let items = c.list("FlowRead/books").await?;
    let target = PathBuf::from(&target_dir);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let mut imported = Vec::new();
    for (name, is_dir, _size, _mtime) in items {
        if is_dir || name.is_empty() {
            continue;
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        if !EXT_BOOK.contains(&ext.as_str()) {
            continue;
        }
        let out = target.join(&name);
        if out.exists() {
            continue;
        }
        if let Some(data) = c.get(&format!("FlowRead/books/{name}")).await? {
            std::fs::write(&out, &data).map_err(|e| e.to_string())?;
            imported.push(out.to_string_lossy().into_owned());
        }
    }
    if !imported.is_empty() {
        scan_folders(&state).await;
    }
    Ok(imported)
}

#[tauri::command]
pub async fn webdav_moon_progress(
    base: String,
    user: String,
    pass: String,
) -> Result<Vec<MoonProgress>, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let mut out = Vec::new();
    for cand in [
        "MoonReader/progress.data",
        "moonreader/progress.data",
        "Books/progress.data",
    ] {
        if let Ok(Some(data)) = c.get(cand).await {
            let text = crate::dav::decode_progress_content(&data);
            out = crate::dav::parse_moonreader_content(&text);
            if !out.is_empty() {
                break;
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn webdav_moon_write(
    base: String,
    user: String,
    pass: String,
    book_path: String,
    title: String,
    chapter_title: String,
    pct: f64,
) -> Result<bool, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let candidates = [
        "MoonReader/progress.data",
        "moonreader/progress.data",
        "Books/progress.data",
    ];
    let mut target = String::from("MoonReader/progress.data");
    let mut content = String::new();
    for cand in candidates {
        if let Ok(Some(data)) = c.get(cand).await {
            content = crate::dav::decode_progress_content(&data);
            target = cand.to_string();
            break;
        }
    }
    let book_name = Path::new(&book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let book_stem = book_name
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| book_name.clone());
    let pct_frac = format!("{:.6}", (pct.clamp(0.0, 100.0) / 100.0));
    let book_low = book_stem.to_lowercase();
    let mut lines: Vec<String> = Vec::new();
    let mut updated = false;
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let e = crate::dav::parse_moonreader_line(t);
        let ef = e.file.rsplit(['/', '\\']).next().unwrap_or(&e.file).to_lowercase();
        let es = ef.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| ef.clone());
        let matched = e.title.trim() == title.trim()
            || ef == book_name.to_lowercase()
            || es == book_low
            || (es.len() >= 3 && book_low.contains(&es));
        if matched {
            let file_s = if e.file.is_empty() { book_name.clone() } else { e.file.clone() };
            let title_s = if e.title.is_empty() { title.clone() } else { e.title.clone() };
            lines.push(format!(
                "file={file_s}*book.title={title_s}*chapter.title={chapter_title}*book.percent={pct_frac}"
            ));
            updated = true;
        } else {
            lines.push(t.to_string());
        }
    }
    if !updated {
        lines.push(format!(
            "file={book_name}*book.title={title}*chapter.title={chapter_title}*book.percent={pct_frac}"
        ));
    }
    let out = lines.join("\n");
    c.ensure_dir("MoonReader").await?;
    c.put(&target, out.into_bytes()).await?;
    Ok(true)
}

#[derive(serde::Serialize)]
pub struct DavEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    pub mtime: i64,
}

#[tauri::command]
pub async fn webdav_list(
    base: String,
    user: String,
    pass: String,
    path: String,
) -> Result<Vec<DavEntry>, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let items = c.list(&path).await?;
    Ok(items
        .into_iter()
        .map(|(name, is_dir, size, mtime)| DavEntry { name, is_dir, size, mtime })
        .collect())
}

const MOON_CACHE_DIRS: [&str; 4] = [
    "Apps/books/.Moon+/Cache",
    "Apps/Books/.Moon+/Cache",
    "Apps/books",
    "Apps/Books",
];

fn moon_norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '.' && *c != '_' && *c != '，' && *c != ',')
        .collect::<String>()
        .to_lowercase()
}

/// 在静读天下目录中为某本书查找对应的 .po 进度文件，返回其 WebDAV 路径
async fn find_moon_po(
    c: &crate::dav::DavClient,
    book_path: &str,
    title: &str,
) -> Option<String> {
    let book_name = Path::new(book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let book_stem = book_name
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| book_name.clone());
    let bn = moon_norm(&book_name);
    let bs = moon_norm(&book_stem);
    let tt = moon_norm(title);
    let mut best: Option<(u32, String)> = None;
    for dir in MOON_CACHE_DIRS {
        let items = match c.list(dir).await {
            Ok(i) => i,
            Err(_) => continue,
        };
        for (name, is_dir, _size, _mtime) in items {
            if is_dir || !name.ends_with(".po") {
                continue;
            }
            let base = &name[..name.len() - 3];
            let b = moon_norm(base);
            let score: u32 = if b == bn || base == book_name {
                100
            } else if b == bs {
                90
            } else if bn.contains(&b) && b.len() >= 4 {
                50
            } else if b.starts_with(&bs) && bs.len() >= 3 {
                40
            } else if tt.len() >= 4 && (b.contains(&tt) || tt.contains(&b)) {
                30
            } else {
                0
            };
            if score == 0 {
                continue;
            }
            let path = format!("{dir}/{name}");
            if best.as_ref().map(|(s, _)| *s) < Some(score) {
                best = Some((score, path));
            }
        }
        if let Some((s, _)) = &best {
            if *s >= 90 {
                break;
            }
        }
    }
    best.map(|(_, p)| p)
}

#[derive(serde::Serialize)]
pub struct MoonPoRead {
    pub found: bool,
    pub percent: f64,
    pub chapter_at: String,
    pub updated_at: i64,
}

#[tauri::command]
pub async fn webdav_moon_read_po(
    base: String,
    user: String,
    pass: String,
    book_path: String,
    title: String,
) -> Result<MoonPoRead, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let empty = || MoonPoRead {
        found: false,
        percent: 0.0,
        chapter_at: String::new(),
        updated_at: 0,
    };
    let po_path = match find_moon_po(&c, &book_path, &title).await {
        Some(p) => p,
        None => return Ok(empty()),
    };
    let data = match c.get(&po_path).await {
        Ok(Some(d)) => d,
        _ => return Ok(empty()),
    };
    let text = crate::dav::decode_progress_content(&data);
    let line = text.lines().next().unwrap_or("");
    let (pct, chapter_at, updated_at) = crate::dav::parse_po_line(line);
    Ok(MoonPoRead {
        found: pct.is_some(),
        percent: pct.unwrap_or(0.0),
        chapter_at,
        updated_at,
    })
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[tauri::command]
pub async fn webdav_moon_write_po(
    base: String,
    user: String,
    pass: String,
    book_path: String,
    title: String,
    chapter_at: String,
    pct: f64,
    char_offset: i64,
) -> Result<bool, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let book_name = Path::new(&book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = find_moon_po(&c, &book_path, &title).await;
    let default_dir = "Apps/books/.Moon+/Cache".to_string();
    let mut po_path = existing.clone().unwrap_or_else(|| {
        format!("{default_dir}/{book_name}.po")
    });

    // 若重名书籍已在别处，读取它的 id 以便保留
    let mut book_id = String::new();
    if let Some(p) = &existing {
        if let Ok(Some(data)) = c.get(p).await {
            let text = crate::dav::decode_progress_content(&data);
            if let Some(line) = text.lines().next() {
                if let Some(hash_idx) = line.find('#') {
                    let tail = &line[hash_idx + 1..];
                    if let Some(col) = tail.find(':') {
                        book_id = tail[..col].to_string();
                    }
                }
            }
        }
        // 与现有文件同目录，避免新建到默认目录造成重复
        if let Some(stripped) = p.strip_suffix(&format!("/{book_name}.po")) {
            po_path = format!("{stripped}/{book_name}.po");
        }
    }
    if book_id.is_empty() {
        book_id = format!("{:x}", fnv1a(book_name.as_bytes()));
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let pct_c = pct.clamp(0.0, 100.0);
    let line = format!(
        "{now_ms}*{chapter_at}@{char_offset}#{book_id}:{pct_c:.2}%"
    );
    let dir = po_path
        .rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_else(|| "Apps/books/.Moon+/Cache".to_string());
    c.ensure_dir(&dir).await?;
    c.put(&po_path, line.into_bytes()).await?;
    Ok(true)
}

#[tauri::command]
pub async fn webdav_moon_po_count(
    base: String,
    user: String,
    pass: String,
) -> Result<i64, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    for dir in MOON_CACHE_DIRS {
        if let Ok(items) = c.list(dir).await {
            let n = items
                .iter()
                .filter(|(name, is_dir, _, _)| !is_dir && name.ends_with(".po"))
                .count() as i64;
            if n > 0 {
                return Ok(n);
            }
        }
    }
    Ok(0)
}

#[tauri::command]
pub async fn webdav_download_file(
    state: State<'_, Arc<AppState>>,
    base: String,
    user: String,
    pass: String,
    remote_path: String,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let data = c.get(&remote_path).await?.ok_or("远程文件不存在")?;
    let name = remote_path.rsplit('/').next().unwrap_or("").to_string();
    if name.is_empty() || name.contains("..") || name.contains('\\') {
        return Err("无效文件名".into());
    }
    let target = PathBuf::from(&target_dir);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let out = target.join(&name);
    std::fs::write(&out, &data).map_err(|e| e.to_string())?;
    let ext = ext_of(&name).unwrap_or_default();
    if EXT_BOOK.contains(&ext.as_str()) {
        let m = crate::bookmeta::from_filename(&name);
        let size = std::fs::metadata(&out).map(|m| m.len() as i64).unwrap_or(0);
        {
            let conn = state.db.0.lock().unwrap();
            let id = db::upsert_book(
                &conn,
                &out.to_string_lossy().into_owned(),
                &m.title,
                &ext,
                size,
                file_mtime(&out),
            )?;
            if !m.author.is_empty() {
                conn.execute(
                    "UPDATE books SET author=?2 WHERE id=?1 AND author=''",
                    rusqlite::params![id, m.author],
                )
                .ok();
            }
        }
        state.app.emit("library-changed", ()).ok();
    }
    Ok(vec![out.to_string_lossy().into_owned()])
}

#[tauri::command]
pub async fn webdav_download_progress(
    state: State<'_, Arc<AppState>>,
    session: String,
    base: String,
    user: String,
    pass: String,
    remote_path: String,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let g = state.app.clone();
    let resp = match c.get_stream(&remote_path).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err("远程文件不存在".into()),
        Err(e) => return Err(e),
    };
    let total: u64 = resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let emit = |received: u64, _color: &str, done: bool, err: Option<String>| {
        let ev = format!("wd-dl:{session}");
        let _ = g.emit(
            &ev,
            serde_json::json!({ "received": received, "total": total, "done": done, "err": err }),
        );
    };

    let mut buf: Vec<u8> = Vec::with_capacity(total as usize);
    let mut stream = resp.bytes_stream();
    let mut received: u64 = 0;
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                received += chunk.len() as u64;
                buf.extend_from_slice(&chunk);
                if total == 0 || received % (1 << 20) < (1 << 16) {
                    emit(received, "auto", false, None);
                }
            }
            Some(Err(e)) => {
                emit(received, "red", true, Some(e.to_string()));
                return Err(e.to_string());
            }
            None => break,
        }
    }

    let name = remote_path.rsplit('/').next().unwrap_or("").to_string();
    if name.is_empty() || name.contains("..") || name.contains('\\') {
        return Err("无效文件名".into());
    }
    let target = PathBuf::from(&target_dir);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let out = target.join(&name);
    std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    let ext = ext_of(&name).unwrap_or_default();
    if EXT_BOOK.contains(&ext.as_str()) {
        let m = crate::bookmeta::from_filename(&name);
        let size = std::fs::metadata(&out).map(|m| m.len() as i64).unwrap_or(0);
        {
            let conn = state.db.0.lock().unwrap();
            let id = db::upsert_book(
                &conn,
                &out.to_string_lossy().into_owned(),
                &m.title,
                &ext,
                size,
                file_mtime(&out),
            )?;
            if !m.author.is_empty() {
                conn.execute(
                    "UPDATE books SET author=?2 WHERE id=?1 AND author=''",
                    rusqlite::params![id, m.author],
                )
                .ok();
            }
        }
        state.app.emit("library-changed", ()).ok();
    }
    emit(received, "green", true, None);
    Ok(vec![out.to_string_lossy().into_owned()])
}

#[tauri::command]
pub fn set_setting_kv(
    state: State<'_, Arc<AppState>>,
    key: String,
    value: String,
) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::set_setting(&conn, &key, &value);
    Ok(())
}

#[tauri::command]
pub fn get_setting_kv(state: State<'_, Arc<AppState>>, key: String) -> Option<String> {
    let conn = state.db.0.lock().unwrap();
    db::get_setting(&conn, &key)
}

#[tauri::command]
pub fn app_data_dir(state: State<'_, Arc<AppState>>) -> String {
    use tauri::Manager;
    state
        .app
        .path()
        .app_data_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[tauri::command]
pub fn add_bookmark(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    chapter_index: i64,
    title: String,
    excerpt: String,
) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::add_bookmark(&conn, book_id, chapter_index, &title, &excerpt)
}

#[derive(serde::Serialize)]
pub struct BookmarkRow {
    pub id: i64,
    pub chapter_index: i64,
    pub title: String,
    pub excerpt: String,
    pub created_at: i64,
}

#[tauri::command]
pub fn list_bookmarks(state: State<'_, Arc<AppState>>, book_id: i64) -> Vec<BookmarkRow> {
    let conn = state.db.0.lock().unwrap();
    db::list_bookmarks(&conn, book_id)
        .into_iter()
        .map(|(id, chapter_index, title, excerpt, created_at)| BookmarkRow {
            id,
            chapter_index,
            title,
            excerpt,
            created_at,
        })
        .collect()
}

#[tauri::command]
pub fn delete_bookmark(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::delete_bookmark(&conn, id)
}

#[tauri::command]
pub fn add_stats(state: State<'_, Arc<AppState>>, book_id: i64, seconds: i64) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::add_stats(&conn, book_id, seconds)
}

#[tauri::command]
pub fn get_stats(state: State<'_, Arc<AppState>>) -> Vec<(String, i64)> {
    let conn = state.db.0.lock().unwrap();
    db::get_stats(&conn)
}

#[tauri::command]
pub async fn ai_chat(
    app: AppHandle,
    id: String,
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
    system: String,
    messages: Vec<(String, String)>,
    proxy_mode: Option<String>,
    proxy_url: Option<String>,
    deployment: Option<String>,
    api_version: Option<String>,
) -> Result<(), crate::ai::AiError> {
    let proxy_mode = proxy_mode.unwrap_or_else(|| "auto".into());
    let cfg = crate::ai::AiConfig {
        provider,
        base_url,
        api_key,
        model,
        deployment: deployment.unwrap_or_default(),
        api_version: api_version.unwrap_or_default(),
    };
    let http = crate::ai::build_client(&proxy_mode, proxy_url.as_deref().unwrap_or(""))
        .map_err(crate::ai::AiError::config)?;
    crate::ai::stream_chat(app, http, id, cfg, system, messages).await
}

#[tauri::command]
pub async fn ai_test(
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
    proxy_mode: Option<String>,
    proxy_url: Option<String>,
    deployment: Option<String>,
    api_version: Option<String>,
) -> Result<String, String> {
    crate::ai::test_connection(
        provider,
        base_url,
        api_key,
        model,
        deployment.unwrap_or_default(),
        api_version.unwrap_or_default(),
        proxy_mode.unwrap_or_else(|| "auto".into()),
        proxy_url.unwrap_or_default(),
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn tts_synth(
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
    deployment: Option<String>,
    api_version: Option<String>,
    proxy_mode: Option<String>,
    proxy_url: Option<String>,
    voice: String,
    format: String,
    text: String,
) -> Result<String, crate::ai::AiError> {
    let cfg = crate::ai::AiConfig {
        provider,
        base_url,
        api_key,
        model,
        deployment: deployment.unwrap_or_default(),
        api_version: api_version.unwrap_or_default(),
    };
    let http = crate::ai::build_client(proxy_mode.as_deref().unwrap_or("auto"), proxy_url.as_deref().unwrap_or(""))
        .map_err(crate::ai::AiError::config)?;
    let bytes = crate::ai::synthesize(http, cfg, &voice, &format, &text).await?;
    use base64::Engine as _;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 读取自定义 TTS 服务（如本地 VoxCPM）提供的音色列表：GET {base}/v1/voices。
#[tauri::command]
pub async fn tts_voices(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("未填写服务地址".into());
    }
    let root = base.strip_suffix("/v1").unwrap_or(base);
    // 兼容常见写法：VoiceStudio 用 /v1/audio/voices，其它服务用 /v1/voices
    let candidates = [
        format!("{root}/v1/audio/voices"),
        format!("{root}/v1/voices"),
    ];
    let client = crate::ai::build_client("auto", "").map_err(|e| e)?;
    let mut last_err = String::from("未找到音色接口");
    for url in candidates {
        let mut req = client.get(&url);
        if !api_key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("请求失败：{e}");
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = format!("HTTP {}", resp.status());
            continue;
        }
        let v: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        };
        let mut out: Vec<String> = Vec::new();
        // data: [...]  或  voices: [...]
        let arr = v
            .get("data")
            .and_then(|d| d.as_array())
            .or_else(|| v.get("voices").and_then(|d| d.as_array()));
        if let Some(arr) = arr {
            for it in arr {
                let id = it
                    .get("id")
                    .and_then(|x| x.as_str())
                    .or_else(|| it.get("voice_id").and_then(|x| x.as_str()))
                    .or_else(|| it.get("name").and_then(|x| x.as_str()))
                    .or_else(|| it.as_str());
                if let Some(id) = id {
                    if !out.iter().any(|x| x == id) {
                        out.push(id.to_string());
                    }
                }
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
        last_err = "服务未返回音色列表".into();
    }
    Err(last_err)
}

#[tauri::command]
pub fn search_all(state: State<'_, Arc<AppState>>, query: String) -> Vec<SearchHit> {
    let conn = state.db.0.lock().unwrap();
    if !db::has_fts(&conn) || query.chars().count() < 2 {
        return vec![];
    }
    let q = query.replace('"', "\"\"");
    let sql = "SELECT book_id, chapter_index, title, snippet(fts, 3, '<mark>', '</mark>', '…', 12) FROM fts WHERE body MATCH ?1 LIMIT 80";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([&q], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })
    .map(|rows| {
        rows.filter_map(|r| r.ok())
            .filter_map(|(book_id, chapter_index, title, snippet)| {
                let book = db::get_book(&conn, book_id)?;
                Some(SearchHit {
                    book_id,
                    book_title: book.title,
                    chapter_index,
                    chapter_title: title,
                    snippet,
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// 提取整本书每章的纯文本（章节号、标题、正文文本）。
/// txt/mobi 先剥离 HTML 标签，保证与阅读界面渲染出的文字一致，选区匹配更可靠。
fn collect_book_chapters(state: &Arc<AppState>, book_id: i64) -> Result<Vec<(i64, String, String)>, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let chapters: Vec<(i64, String, String)> = if ext == "txt" {
        let plain = {
            let data = std::fs::read(&book.path).map_err(|e| e.to_string())?;
            let (t, _) = txt::decode_bytes(&data);
            t.replace("\r\n", "\n").replace('\r', "\n")
        };
        let pattern = {
            let conn = state.db.0.lock().unwrap();
            db::get_setting(&conn, "txt_rules").unwrap_or_else(|| crate::default_pattern().into())
        };
        let doc = txt::split_custom(&plain, &pattern)?;
        doc.toc
            .iter()
            .zip(&doc.ranges)
            .map(|(t, (s, e))| (t.index as i64, t.title.clone(), safe_slice(&plain, *s, *e)))
            .collect()
    } else if ext == "epub" {
        let data = std::fs::read(&book.path).map_err(|e| e.to_string())?;
        let doc = crate::epub::parse(data.clone())?;
        let spine_len = doc.spine.len();
        let mut toc_pos_by_spine: Vec<Option<i64>> = vec![None; spine_len];
        for (i, t) in doc.toc.iter().enumerate() {
            if t.index < spine_len && toc_pos_by_spine[t.index].is_none() {
                toc_pos_by_spine[t.index] = Some(i as i64);
            }
        }
        doc.spine
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let ti = toc_pos_by_spine[i].unwrap_or(i as i64);
                let title = doc
                    .toc
                    .iter()
                    .find(|t| t.index == i)
                    .map(|t| t.title.clone())
                    .unwrap_or_default();
                (ti, title, crate::epub::doc_text(&data, d))
            })
            .collect()
    } else {
        let doc = mobi_doc_for(state, &book.path, book_id)?;
        doc.toc
            .iter()
            .zip(&doc.parts)
            .map(|(t, (s, e))| {
                let raw = safe_slice(&doc.html, *s, *e);
                (t.index as i64, t.title.clone(), crate::epub::html_to_text(&raw))
            })
            .collect()
    };
    Ok(chapters)
}

/// 确保某本书的纯文本章节已缓存到 chapter_text（划词全文搜索用）。
fn ensure_book_text(state: &Arc<AppState>, book_id: i64) -> Result<usize, String> {
    {
        let conn = state.db.0.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM chapter_text WHERE book_id=?1",
                [book_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if n > 0 {
            return Ok(n as usize);
        }
    }
    let chapters = collect_book_chapters(state, book_id)?;
    let mut conn = state.db.0.lock().unwrap();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (idx, title, body) in &chapters {
        tx.execute(
            "INSERT OR REPLACE INTO chapter_text(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
            rusqlite::params![book_id, idx, title, body],
        )
        .ok();
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(chapters.len())
}

#[tauri::command]
pub fn ensure_book_text_cmd(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<i64, String> {
    ensure_book_text(state.inner(), book_id).map(|n| n as i64)
}

/// 生成带 <mark> 高亮的片段（HTML 转义正文，只保留命中两侧少量上下文）。
fn highlight_snippet(chars: &[char], bs: usize, be: usize) -> String {
    let ctx = 24usize;
    let from = bs.saturating_sub(ctx);
    let to = (be + 1 + ctx).min(chars.len());
    let mut s = String::new();
    if from > 0 {
        s.push('…');
    }
    for (k, ch) in chars[from..to].iter().enumerate() {
        let idx = from + k;
        if idx == bs {
            s.push_str("<mark>");
        }
        match ch {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            c => s.push(*c),
        }
        if idx == be {
            s.push_str("</mark>");
        }
    }
    if to < chars.len() {
        s.push('…');
    }
    s
}

/// 单本书内全文检索：不依赖 FTS5，直接扫描缓存的纯文本章节。
/// 忽略所有空白并做 ASCII 不区分大小写匹配，避免选区换行/空格与索引原文不一致导致漏命中。
#[tauri::command]
pub fn search_in_book(state: State<'_, Arc<AppState>>, book_id: i64, query: String) -> Vec<BookSearchHit> {
    let compact_q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if compact_q.is_empty() {
        return vec![];
    }
    if ensure_book_text(state.inner(), book_id).is_err() {
        return vec![];
    }
    let rows: Vec<(i64, String, String)> = {
        let conn = state.db.0.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT chapter_index, title, body FROM chapter_text WHERE book_id=?1 ORDER BY chapter_index",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([book_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rs| rs.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    };

    const MAX_HITS: usize = 200;
    const PER_CHAPTER: usize = 5;
    let qlen = compact_q.len();
    let mut out: Vec<BookSearchHit> = Vec::new();

    for (idx, title, body) in rows {
        if out.len() >= MAX_HITS {
            break;
        }
        let chars: Vec<char> = body.chars().collect();
        let mut compact: Vec<char> = Vec::with_capacity(chars.len());
        let mut map: Vec<usize> = Vec::with_capacity(chars.len());
        for (i, ch) in chars.iter().enumerate() {
            if !ch.is_whitespace() {
                compact.push(ch.to_ascii_lowercase());
                map.push(i);
            }
        }
        let mut i = 0usize;
        let mut per = 0usize;
        while i + qlen <= compact.len() {
            if compact[i..i + qlen] == compact_q[..] {
                let bs = map[i];
                let be = map[i + qlen - 1];
                out.push(BookSearchHit {
                    chapter_index: idx,
                    chapter_title: title.clone(),
                    snippet: highlight_snippet(&chars, bs, be),
                });
                per += 1;
                i += qlen.max(1);
                if out.len() >= MAX_HITS || per >= PER_CHAPTER {
                    break;
                }
            } else {
                i += 1;
            }
        }
    }
    out
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn add_highlight(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    chapter_index: i64,
    chapter_title: String,
    start: i64,
    end: i64,
    text: String,
    style: String,
    color: String,
    note: String,
) -> Result<i64, String> {
    let conn = state.db.0.lock().unwrap();
    db::add_highlight(
        &conn,
        book_id,
        chapter_index,
        &chapter_title,
        start,
        end,
        &text,
        &style,
        &color,
        &note,
    )
}

#[tauri::command]
pub fn list_highlights(state: State<'_, Arc<AppState>>, book_id: i64) -> Vec<HighlightRow> {
    let conn = state.db.0.lock().unwrap();
    db::list_highlights(&conn, book_id)
}

#[tauri::command]
pub fn search_highlights(state: State<'_, Arc<AppState>>, query: String) -> Vec<HighlightRow> {
    let conn = state.db.0.lock().unwrap();
    db::search_highlights(&conn, &query)
}

#[tauri::command]
pub fn delete_highlight(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::delete_highlight(&conn, id)
}

#[tauri::command]
pub fn update_highlight(
    state: State<'_, Arc<AppState>>,
    id: i64,
    style: String,
    color: String,
    note: String,
) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::update_highlight(&conn, id, &style, &color, &note)
}

#[tauri::command]
pub fn update_book_meta(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    title: String,
    author: String,
) -> Result<(), String> {
    {
        let conn = state.db.0.lock().unwrap();
        db::update_book_title_author(&conn, book_id, &title, &author)?;
    }
    state.app.emit("library-changed", ()).ok();
    Ok(())
}

/// 把书名/作者直接写回原始电子书文件（当前支持 EPUB，直接覆盖，不生成备份），
/// 同时更新数据库显示。
#[tauri::command]
pub fn write_book_meta(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    title: String,
    author: String,
) -> Result<(), String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    if ext != "epub" {
        return Err("写回原文件目前仅支持 EPUB；其他格式请先用「转换格式」转为 EPUB".into());
    }
    crate::epub::write_meta(&book.path, &title, &author)?;
    {
        let conn = state.db.0.lock().unwrap();
        db::update_book_title_author(&conn, book_id, &title, &author)?;
    }
    state.app.emit("library-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub fn rebuild_fts(state: State<'_, Arc<AppState>>) -> Result<i64, String> {
    let conn = state.db.0.lock().unwrap();
    if !db::has_fts(&conn) {
        return Err("SQLite 编译未包含 FTS5".into());
    }
    db::ensure_fts(&conn)?;
    let n = conn
        .execute("DELETE FROM fts", [])
        .map_err(|e| e.to_string())?;
    Ok(n as i64)
}

#[tauri::command]
pub fn index_book_fts(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<i64, String> {
    let needs_index = {
        let conn = state.db.0.lock().unwrap();
        if !db::has_fts(&conn) {
            return Ok(0);
        }
        db::ensure_fts(&conn)?;
        let existing: i64 = conn
            .query_row(
                "SELECT count(*) FROM fts WHERE book_id=?1",
                [book_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        existing == 0
    };
    if !needs_index {
        // 已建过 FTS：顺手确保纯文本缓存存在（划词全文搜索不依赖 FTS）
        let _ = ensure_book_text(state.inner(), book_id);
        return Ok(0);
    }

    let chapters = collect_book_chapters(state.inner(), book_id)?;
    let text_empty = {
        let conn = state.db.0.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM chapter_text WHERE book_id=?1",
                [book_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        n == 0
    };

    let mut conn = state.db.0.lock().unwrap();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (idx, title, body) in &chapters {
        tx.execute(
            "INSERT INTO fts(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
            rusqlite::params![book_id, idx, title, body],
        )
        .ok();
        if text_empty {
            tx.execute(
                "INSERT OR REPLACE INTO chapter_text(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
                rusqlite::params![book_id, idx, title, body],
            )
            .ok();
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(chapters.len() as i64)
}

#[tauri::command]
pub fn reveal_in_explorer(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if let Some(dir) = Path::new(&path).parent() {
        app.opener()
            .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn covers_dir_of(app: &AppHandle) -> PathBuf {
    covers::covers_dir(app)
}

#[tauri::command]
pub fn reextract_cover(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<String, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let covers_dir = covers::covers_dir(&state.app);
    let ext = ext_of(&book.path).unwrap_or_default();
    let name = format!("{}.png", db::path_id(&book.path));
    let out = covers_dir.join(&name);

    let ok = if ext == "epub" {
        let data = std::fs::read(&book.path).map_err(|e| e.to_string())?;
        let doc = crate::epub::parse(data.clone()).ok();
        let bytes = doc
            .as_ref()
            .and_then(|d| d.cover_href.as_ref().and_then(|h| crate::epub::read_cover(&data, h)))
            .or_else(|| doc.as_ref().map(|d| crate::epub::find_cover_fallback(&data, &d.spine)).flatten());
        match bytes {
            Some(b) => covers::make_thumbnail(&b, &out).is_ok(),
            None => false,
        }
    } else if ext == "mobi" || ext == "azw3" || ext == "prc" {
        // MOBI/AZW3：EXTH201/202/129 或 first_image_index 起首张图
        crate::mobi::extract_cover_path(&book.path)
            .map(|b| covers::make_thumbnail(&b, &out).is_ok())
            .unwrap_or(false)
    } else {
        false
    };

    if ok {
        let rel = name;
        let conn = state.db.0.lock().unwrap();
        conn.execute(
            "UPDATE books SET cover_path=?2 WHERE id=?1",
            rusqlite::params![book_id, rel],
        )
        .map_err(|e| e.to_string())?;
        Ok(rel)
    } else {
        Err("该书未能自动提取封面，请右键「设置封面」手动指定图片".into())
    }
}

#[tauri::command]
pub fn set_background_image(state: State<'_, Arc<AppState>>, image_path: String) -> Result<String, String> {
    let data = std::fs::read(&image_path).map_err(|e| e.to_string())?;
    let dir = covers::covers_dir(&state.app);
    std::fs::write(dir.join("background.img"), &data).map_err(|e| e.to_string())?;
    Ok("background.img".into())
}

#[tauri::command]
pub fn clear_background_image(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let dir = covers::covers_dir(&state.app);
    std::fs::remove_file(dir.join("background.img")).ok();
    Ok(())
}

#[tauri::command]
pub fn random_book(state: State<'_, Arc<AppState>>) -> Option<BookRow> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let conn = state.db.0.lock().unwrap();
    let books = db::list_books(&conn);
    if books.is_empty() {
        return None;
    }
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    Some(books[n % books.len()].clone())
}

#[tauri::command]
pub fn start_reading_session(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<i64, String> {
    let conn = state.db.0.lock().unwrap();
    db::start_session(&conn, book_id)
}

#[tauri::command]
pub fn update_reading_session(
    state: State<'_, Arc<AppState>>,
    session_id: i64,
    seconds: i64,
    chars: i64,
) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::update_session(&conn, session_id, seconds, chars)
}

#[tauri::command]
pub fn get_book_sessions(state: State<'_, Arc<AppState>>, book_id: i64) -> Vec<SessionRow> {
    let conn = state.db.0.lock().unwrap();
    db::get_book_sessions(&conn, book_id)
        .into_iter()
        .map(|(day, started_at, seconds, chars)| SessionRow {
            day,
            started_at,
            seconds,
            chars,
        })
        .collect()
}

#[tauri::command]
pub fn add_dropped_book(state: State<'_, Arc<AppState>>, path: String) -> Result<BookRow, String> {
    let p = Path::new(&path);
    if !p.is_file() {
        return Err("文件不存在".into());
    }
    let ext = ext_of(&path).ok_or_else(|| "不支持的文件类型".to_string())?;
    let m = crate::bookmeta::from_filename(
        &p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    );
    let size = std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0);
    let book = {
        let conn = state.db.0.lock().unwrap();
        let id = db::upsert_book(&conn, &path, &m.title, &ext, size, file_mtime(p))?;
        if !m.author.is_empty() {
            conn.execute(
                "UPDATE books SET author=?2 WHERE id=?1 AND author=''",
                rusqlite::params![id, m.author],
            )
            .ok();
        }
        db::get_book(&conn, id)
    };
    state.app.emit("library-changed", ()).ok();
    book.ok_or_else(|| "添加失败".to_string())
}

#[tauri::command]
pub fn suggest_meta(state: State<'_, Arc<AppState>>, path: String) -> crate::bookmeta::SuggestMeta {
    let known = {
        let conn = state.db.0.lock().unwrap();
        db::get_book_by_path(&conn, &path).map(|b| (b.title, b.author, b.meta_done))
    };
    if let Some((title, author, done)) = known {
        if done == 1 && !author.is_empty() {
            return crate::bookmeta::SuggestMeta { title, author };
        }
    }
    let ext = ext_of(&path).unwrap_or_default();
    let mut head = String::new();
    if ext == "txt" || ext.is_empty() {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open(&path) {
            let mut buf = Vec::new();
            if f.take(8192).read_to_end(&mut buf).is_ok() {
                let (text, _) = crate::txt::decode_bytes(&buf);
                head = text;
            }
        }
    }
    crate::bookmeta::suggest(&path, Some(&head))
}

#[derive(serde::Serialize)]
pub struct WordCount {
    pub total: i64,
    pub pure: i64,
}

#[tauri::command]
pub async fn get_book_word_count(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<WordCount, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    if book.total_chars > 0 {
        return Ok(WordCount {
            total: book.total_chars,
            pure: book.total_text_chars,
        });
    }
    let ext = ext_of(&book.path).unwrap_or_default();
    let st = state.inner().clone();
    let is_mobi = ext == "mobi" || ext == "azw3" || ext == "prc";
    if ext == "epub" {
        let cached = load_epub(&st, &book.path, book_id)?;
        return Ok(WordCount {
            total: cached.doc.total_chars,
            pure: cached.doc.total_text_chars,
        });
    }
    let (content, _r, _t) = if is_mobi {
        let doc = mobi_doc_for(&st, &book.path, book_id)?;
        (doc.html, Vec::new(), Vec::new())
    } else {
        load_txt(&st, &book.path, book_id)?.as_ref().clone()
    };
    let text = if is_mobi {
        crate::epub::html_to_text(&content)
    } else {
        content
    };
    let total = text.chars().count() as i64;
    let pure = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
    Ok(WordCount { total, pure })
}


#[cfg(test)]
mod refresh_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_root(tag: &str) -> PathBuf {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let d = std::env::temp_dir().join(format!("fr_refresh_{}_{}_{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 必验 A/B：删文件 → removed 计数正确；目录新增文件 → added 计数正确；二次刷新幂等 0/0。
    #[test]
    fn refresh_removes_missing_and_discovers_new() {
        let root = tmp_root("ab");
        let a = root.join("shelfA");
        let b = root.join("shelfB");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let one = a.join("one.txt");
        let two = a.join("two.txt");
        let extra = a.join("extra.txt");
        let keep = b.join("keep.txt");
        let dead = b.join("dead.txt");
        for f in [&one, &two, &keep, &dead] { std::fs::write(f, b"x").unwrap(); }
        std::fs::write(&extra, b"y").unwrap(); // 磁盘存在但未入库

        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        db::upsert_book(&conn, &sp(&one), "one", "txt", 1, 0).unwrap();
        db::upsert_book(&conn, &sp(&two), "two", "txt", 1, 0).unwrap();
        db::upsert_book(&conn, &sp(&keep), "keep", "txt", 1, 0).unwrap();
        db::upsert_book(&conn, &sp(&dead), "dead", "txt", 1, 0).unwrap();

        std::fs::remove_file(&dead).unwrap(); // 索引里有、磁盘已删
        let r1 = refresh_library_core(&conn);
        assert_eq!(r1.removed, 1, "deleted file must be removed from index");
        assert_eq!(r1.added, 1, "new file on disk must be added to index");
        assert!(db::get_book_by_path(&conn, &sp(&dead)).is_none());
        assert!(db::get_book_by_path(&conn, &sp(&extra)).is_some());

        let r2 = refresh_library_core(&conn);
        assert_eq!((r2.removed, r2.added), (0, 0), "second refresh must be idempotent");

        std::fs::remove_file(&two).unwrap();
        let r3 = refresh_library_core(&conn);
        assert_eq!((r3.removed, r3.added), (1, 0));
        assert!(db::get_book_by_path(&conn, &sp(&two)).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 非书籍扩展名不得入库；不存在索引记录的目录不会被扫。
    #[test]
    fn refresh_ignores_non_book_and_unindexed_dirs() {
        let root = tmp_root("ign");
        let a = root.join("shelfA");
        let other = root.join("nowhere");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let one = a.join("one.txt");
        std::fs::write(&one, b"x").unwrap();
        std::fs::write(a.join("note.md"), b"#").unwrap();      // 非书籍扩展名
        let orphan = other.join("orphan.txt");
        std::fs::write(&orphan, b"z").unwrap();                 // 无任何记录指向该目录
        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        db::upsert_book(&conn, &sp(&one), "one", "txt", 1, 0).unwrap();
        let r = refresh_library_core(&conn);
        assert_eq!((r.removed, r.added), (0, 0));
        assert!(db::get_book_by_path(&conn, &sp(&a.join("note.md"))).is_none());
        assert!(db::get_book_by_path(&conn, &sp(&orphan)).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 必改1(B)：书库根的一级子目录中“未入库过”的新书，刷新时能被发现；二级不递归。
    #[test]
    fn refresh_discovers_new_books_in_root_subdir() {
        let root = tmp_root("rootsub");
        let shelf = root.join("shelf");
        std::fs::create_dir_all(&shelf).unwrap();
        let conn = db::init(&root.join("t.db")).unwrap();
        db::add_folder(&conn, &shelf.to_string_lossy()).unwrap(); // 登记书库根
        // 根下一级子目录里放一本“未入库过”的新书
        let sub = shelf.join("未入库过的一级目录");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("new1.txt"), b"a").unwrap();
        // 根目录本身直接放的书
        std::fs::write(shelf.join("inroot.txt"), b"b").unwrap();
        // 二级子目录（不得被非递归扫描发现）
        let deep = sub.join("deeper");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.txt"), b"c").unwrap();

        let r = refresh_library_core(&conn);
        assert_eq!(r.removed, 0);
        assert_eq!(r.added, 2, "根+一级子目录的新书应被识别");
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        assert!(db::get_book_by_path(&conn, &sp(&sub.join("new1.txt"))).is_some());
        assert!(db::get_book_by_path(&conn, &sp(&shelf.join("inroot.txt"))).is_some());
        assert!(db::get_book_by_path(&conn, &sp(&deep.join("deep.txt"))).is_none(), "二级不递归");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 分页查询：total/rows、排序白名单、LIMIT 上限、offset、LIKE 转义。
    #[test]
    fn list_books_page_filters_sorts_and_pages() {
        let root = tmp_root("page");
        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        let data = [
            ("a.txt", "Alpha", 100i64),
            ("b.txt", "beta", 200),
            ("c.txt", "Gamma", 300),
            ("d.txt", "100%_marker", 400),
            ("e.txt", "delta", 500),
            ("f.txt", "Epsilon", 600),
            ("g.txt", "zeta", 700),
        ];
        for (name, title, mtime) in data {
            let p = root.join(name);
            std::fs::write(&p, b"x").unwrap();
            db::upsert_book(&conn, &sp(&p), title, "txt", 1, mtime).unwrap();
        }

        let p1 = db::list_books_page(&conn, "", "recent", 0, 500);
        assert_eq!(p1.total, 7);
        assert_eq!(p1.rows.len(), 7);
        assert_eq!(p1.rows[0].title, "zeta", "mtime DESC 最新在前");

        let p2 = db::list_books_page(&conn, "", "recent", 2, 3);
        assert_eq!((p2.total, p2.rows.len()), (7, 3));
        assert_eq!(p2.rows[0].title, "delta", "offset=2 应从第三新开始");

        let p3 = db::list_books_page(&conn, "", "added", 0, 9999);
        assert_eq!(p3.rows.len(), 7, "limit 超上限被 clamp 到 500，本应有 7 行");

        let p4 = db::list_books_page(&conn, "epsilon", "title", 0, 10);
        assert_eq!((p4.total, p4.rows.len()), (1, 1), "标题搜索应大小写不敏感");
        assert_eq!(p4.rows[0].title, "Epsilon");

        let p5 = db::list_books_page(&conn, "%_marker", "title", 0, 10);
        assert_eq!(p5.total, 1, "% 与 _ 必须按字面匹配");
        assert_eq!(p5.rows[0].title, "100%_marker");

        let p6 = db::list_books_page(&conn, "%", "title", 0, 10);
        assert_eq!(p6.total, 1, "纯通配符不得匹配全部");

        let p7 = db::list_books_page(&conn, "", "title", 0, 10);
        assert_eq!(p7.rows[0].title, "100%_marker", "title COLLATE NOCASE 升序");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 16 万本规模的分页性能基线（默认忽略；`cargo test -- --ignored --nocapture` 运行）。
    #[test]
    #[ignore]
    fn list_books_page_160k_perf() {
        use std::time::Instant;
        let root = tmp_root("perf160k");
        let conn = db::init(&root.join("t.db")).unwrap();
        {
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..160_000i64 {
                tx.execute(
                    "INSERT INTO books(id,path,title,author,format,size,mtime,added_at,meta_done)
                     VALUES(?1,?2,?3,?4,'txt',1,?5,?6,0)",
                    rusqlite::params![
                        i,
                        format!("C:/fake/lib/{}.txt", i),
                        format!("书籍标题 Book {:06}", i),
                        format!("作者{}", i % 1000),
                        1_700_000_000_000i64 + i,
                        1_700_000_000_000i64 + i
                    ],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }

        let t = Instant::now();
        let p1 = db::list_books_page(&conn, "", "added", 0, 500);
        println!("[perf160k] added page0: {:?} total={} rows={}", t.elapsed(), p1.total, p1.rows.len());

        let t = Instant::now();
        let p2 = db::list_books_page(&conn, "", "title", 0, 500);
        println!("[perf160k] title page0: {:?} rows={}", t.elapsed(), p2.rows.len());

        let t = Instant::now();
        let p3 = db::list_books_page(&conn, "", "recent", 159_000, 500);
        println!("[perf160k] recent offset=159000: {:?} rows={}", t.elapsed(), p3.rows.len());

        let t = Instant::now();
        let p3b = db::list_books_page(&conn, "", "added", 159_000, 500);
        println!("[perf160k] added offset=159000: {:?} rows={}", t.elapsed(), p3b.rows.len());

        let t = Instant::now();
        let p3c = db::list_books_page(&conn, "", "title", 159_000, 500);
        println!("[perf160k] title offset=159000: {:?} rows={}", t.elapsed(), p3c.rows.len());

        let t = Instant::now();
        let p3d = db::list_books_page(&conn, "", "lastread", 0, 500);
        println!("[perf160k] lastread page0: {:?} rows={}", t.elapsed(), p3d.rows.len());

        let t = Instant::now();
        let p4 = db::list_books_page(&conn, "Book 159", "title", 0, 500);
        println!("[perf160k] search 'Book 159': {:?} total={} rows={}", t.elapsed(), p4.total, p4.rows.len());

        assert_eq!(p1.total, 160_000);
        assert_eq!(p4.total, 1000, "Book 159000..159999 共 1000 本");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod epub_meta_tests {
    use std::io::Write;

    fn write_min_epub(path: &std::path::Path) {
        let f = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let stored =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("mimetype", stored).unwrap();
        zw.write_all(b"application/epub+zip").unwrap();
        zw.start_file("META-INF/container.xml", deflated).unwrap();
        zw.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        zw.start_file("OEBPS/content.opf", deflated).unwrap();
        zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title id="t">Old Title</dc:title><dc:creator id="c">Old Author</dc:creator></metadata><manifest/><spine/></package>"#).unwrap();
        zw.finish().unwrap();
    }

    #[test]
    fn write_meta_overwrites_title_and_author() {
        let dir = std::env::temp_dir().join(format!("fr_epub_meta_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("b.epub");
        write_min_epub(&p);
        let ps = p.to_string_lossy().into_owned();
        crate::epub::write_meta(&ps, "新标题 & <tag>", "新作者").unwrap();
        let data = std::fs::read(&p).unwrap();
        let doc = crate::epub::parse(data).unwrap();
        assert_eq!(doc.title, "新标题 & <tag>");
        assert_eq!(doc.author, "新作者");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
