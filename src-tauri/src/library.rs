use crate::covers;
use crate::db::{self, Db};
use crate::models::*;
use crate::txt;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct CachedEpub {
    pub doc: crate::epub::EpubDoc,
    pub data: Arc<Vec<u8>>,
    /// zip 条目名 → 索引（含 basename 兜底），解析一次复用，避免每章重跑 build_ci。
    pub ci: crate::epub::CiMap,
    /// 解析时该文件的指纹（大小, mtime）。命中缓存前必须与磁盘现读指纹一致，
    /// 否则「同一路径的文件被就地替换」后正文与目录会一直是旧文件的内容。
    pub stamp: Option<(i64, i64)>,
}

/// TXT/MOBI 解析缓存条目。
/// 只有 rules_ver（编码/切分规则）发现不了「同一路径的文件被替换」，必须再比文件指纹。
/// Clone：读缓存时先克隆一份出锁，再做指纹比对（避免持锁期间读盘/解析）。
#[derive(Clone)]
pub struct CachedText {
    pub ver: u64,
    pub stamp: Option<(i64, i64)>,
    pub doc: Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>,
}

/// EPUB 字数统计缓存条目：文件指纹 + 每章字数 + 全书 total/pure。
/// 指纹不匹配就整条作废，避免「清了 DB 又被旧缓存回填回去」。
pub struct EpubStats {
    pub stamp: Option<(i64, i64)>,
    pub per_chapter: Vec<i64>,
    pub total: i64,
    pub pure: i64,
}

pub struct AppState {
    pub db: Db,
    pub app: AppHandle,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub last_fs_event: AtomicU64,
    pub rules_ver: AtomicU64,
    pub txt_cache: Mutex<HashMap<i64, CachedText>>,
    pub epub_cache: Mutex<HashMap<i64, Arc<CachedEpub>>>,
    /// EPUB 字数统计缓存：key = book_id → EpubStats（带文件指纹）。
    /// 「全书总页数」（book_char_counts）与书详情/本书统计（get_book_word_count）共用同一份，
    /// 避免同一本书被完整解析两次；容量很小（4），因为一条就够几 MB 的逐章数组。
    pub epub_stats_cache: Mutex<HashMap<i64, Arc<EpubStats>>>,
    /// 已渲染章节 LRU：key = (book_id, chapter_index) → ChapterData。
    /// 避免每次 get_chapter 重开 zip/重跑正则/重复 base64 内联图片；随书切换/规则变化清理。
    pub chapter_cache: Mutex<ChapterCache>,
    /// 全书图片列表缓存：key = book_id。
    pub image_cache: Mutex<HashMap<i64, Arc<Vec<BookImage>>>>,
    // staged: 通用兜底 HTTP client（见 lib.rs:220-221 的蓄意预留说明）；实际各处 HTTP 各自建 client。
    #[allow(dead_code)]
    pub http: reqwest::Client,
    /// 封面增量扫描 worker 是否在跑（进程内标记，天然不会跨崩溃残留）。
    /// 单遍串行：处理完全部 `meta_done=0` 即退出（扫完即停、不驻留、不轮询）。
    pub scan_running: std::sync::atomic::AtomicBool,
    /// 请求 worker 尽快退出（清缓存前避免删封面目录与写封面并发；或应用退出）。
    pub scan_stop: std::sync::atomic::AtomicBool,
    /// worker 累计启动次数（仅诊断用）。
    pub scan_start_count: AtomicU64,
    /// 本轮扫描已处理数 / 待处理总数（进度条分子分母，worker 启动时按 DB 实测赋值）。
    pub cover_scan_done: AtomicU64,
    pub cover_scan_total: AtomicU64,
    /// 封面默认底图 id 与解析规则版本，参与缓存失效（F5）。
    pub cover_style_ver: AtomicU64,
}

/// 简单的按书维度 LRU 章节缓存：容量按条数限制，随书切换/规则变化整体失效。
/// 用 Vec 记录访问顺序（章节数不大，线性查找成本可忽略），命中即移到末尾。
pub struct ChapterCache {
    map: HashMap<(i64, i64, bool), Arc<ChapterData>>,
    order: std::collections::VecDeque<(i64, i64, bool)>,
    max: usize,
}

impl ChapterCache {
    pub fn new(max: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: std::collections::VecDeque::new(),
            max,
        }
    }

    fn touch(&mut self, key: (i64, i64, bool)) {
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    pub fn get(&mut self, key: (i64, i64, bool)) -> Option<Arc<ChapterData>> {
        let v = self.map.get(&key).cloned()?;
        self.touch(key);
        Some(v)
    }

    pub fn put(&mut self, key: (i64, i64, bool), val: Arc<ChapterData>) {
        self.map.insert(key, val);
        self.touch(key);
        while self.order.len() > self.max {
            if let Some(old) = self.order.pop_front() {
                // 可能已被 touch 更新到队尾，只有仍在 map 中才真正淘汰
                if !self.order.iter().any(|k| *k == old) {
                    self.map.remove(&old);
                }
            }
        }
    }

    /// 换书/改规则时整体失效（保持内存可控、避免跨书串味）。
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

const EXT_BOOK: &[&str] = &["txt", "epub", "mobi", "azw3", "prc"];

/// MOBI 家族格式（mobi/azw3/prc）判定：集中一处，避免多处散落的字符串比较漂移。
pub fn is_mobi_ext(ext: &str) -> bool {
    ext == "mobi" || ext == "azw3" || ext == "prc"
}

pub fn ext_of(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .filter(|e| EXT_BOOK.contains(&e.as_str()))
}

fn file_mtime(p: &Path) -> i64 {
    file_stamp_of(p).map(|s| s.1).unwrap_or(0)
}

/// 文件指纹：(大小, mtime 毫秒)。读缓存前用它判断「同一路径的文件是否被就地替换」——
/// 只按 book_id 命中缓存，会让正文、目录、字数全部停在旧文件上（id 由路径派生，替换后不变）。
/// 取不到（不存在/无权限）返回 None：调用方一律按「不可复用」处理，宁可重算。
fn file_stamp_of(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some((m.len() as i64, mtime))
}

fn file_stamp(path: &str) -> Option<(i64, i64)> {
    file_stamp_of(Path::new(path))
}

/// 缓存里的文件指纹是否仍然可用：两侧都得有值且完全一致。
/// 缓存侧缺失 = 当初就没拿到指纹（这种条目不该入缓存）；当前侧缺失 = 现在读不到文件。
/// 两种情况都不复用，绝不把「没有指纹」当成永远有效。
fn stamp_matches(cached: Option<(i64, i64)>, current: Option<(i64, i64)>) -> bool {
    matches!((cached, current), (Some(a), Some(b)) if a == b)
}

/// DB 里缓存的字数是否可用：必须有值，且行上的 size/mtime 与磁盘当前指纹一致。
/// 抽成纯函数便于单测（不碰文件系统）。
/// 为什么 DB 短路也要校验：扫描（upsert_book 更新 size/mtime）不一定在用户打开书详情之前跑到，
/// 只靠「扫描时清列」会漏掉这一段窗口。
fn db_chars_usable(
    total_chars: i64,
    book_size: i64,
    book_mtime: i64,
    disk: Option<(i64, i64)>,
) -> bool {
    if total_chars <= 0 {
        return false;
    }
    match disk {
        Some((size, mtime)) => size == book_size && mtime == book_mtime,
        None => false,
    }
}

fn quick_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 从文件名解析 (书名, 作者)：去扩展名、去常见括号后缀。
/// 兜底保证不抛异常：空名 → "未命名"；纯数字/多重括号 → 退回原文件名。
pub fn parse_filename(path: &str) -> (String, String) {
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let raw = stem.trim().to_string();
    if raw.is_empty() {
        return ("未命名".to_string(), String::new());
    }

    let mut title = raw.clone();
    let mut author = String::new();

    // 常见书源命名：书名 (作者) / 书名 [作者] / 《书名》作者 / 书名 - 作者 / 书名_作者
    // 取最后一个括号组作为作者；括号内容若过长或含"出版/出版社/校对/扫描"等则忽略。
    let bracket_pairs = [('(', ')'), ('[', ']'), ('【', '】'), ('（', '）')];
    for (open, close) in bracket_pairs {
        if let Some(o) = title.rfind(open) {
            if let Some(c) = title[o..].find(close) {
                let inner = title[o + open.len_utf8()..o + c].trim().to_string();
                let ok = !inner.is_empty()
                    && inner.chars().count() <= 20
                    && !inner.contains("出版")
                    && !inner.contains("校对")
                    && !inner.contains("扫描")
                    && !inner.contains("录入")
                    && !inner.contains("简介");
                if ok {
                    author = inner.clone();
                    title = title[..o].trim().to_string();
                }
            }
        }
    }

    // 《书名》书名号：去掉书名号，其中内容作为书名（按 char 安全切片）。
    if title.starts_with('《') {
        let chars: Vec<char> = title.chars().collect();
        if let Some(end) = chars.iter().position(|&c| c == '》') {
            title = chars[1..end].iter().collect::<String>().trim().to_string();
        }
    }

    // 书名 - 作者 / 书名_作者 形式（作者位较短时）
    if author.is_empty() {
        for sep in [" - ", "_", "—"] {
            if let Some(idx) = title.find(sep) {
                let right = title[idx + sep.len()..].trim();
                if !right.is_empty() && right.chars().count() <= 16 {
                    author = right.to_string();
                    title = title[..idx].trim().to_string();
                    break;
                }
            }
        }
    }

    let title = title.trim().to_string();
    // 兜底：解析后为空 → 退回原文件名；仍无 → "未命名"
    let title = if title.is_empty() {
        if raw.is_empty() { "未命名".to_string() } else { raw }
    } else {
        title
    };
    (title, author)
}

pub fn get_rules(state: &AppState) -> String {
    let conn = state.db.0.lock().unwrap();
    db::get_setting(&conn, "txt_rules").unwrap_or_else(|| crate::default_pattern().into())
}

pub async fn scan_folders(state: &AppState, progress_scope: Option<Vec<String>>) {
    let folders = {
        let conn = state.db.0.lock().unwrap();
        db::list_folders(&conn)
    };
    let progress_on = progress_scope.is_some();
    // ---- 阶段一：无锁遍历目录，收集 (path → size, mtime, title, ext) ----
    let folders_walk = folders.clone();
    let prog_app = state.app.clone();
    let (found, scanned): (HashMap<String, (i64, i64, String, String)>, usize) =
        tauri::async_runtime::spawn_blocking(move || {
            let mut found: HashMap<String, (i64, i64, String, String)> = HashMap::new();
            // scanned 只统计「本次进度范围内」的条目：不把其它已登记目录的条目算进去。
            let mut scanned: usize = 0;
            let emit = |scanned: usize, added: usize, running: bool| {
                prog_app
                    .emit(
                        "folder-scan-progress",
                        serde_json::json!({ "running": running, "scanned": scanned, "added": added }),
                    )
                    .ok();
            };
            if progress_on {
                emit(0, 0, true);
            }
            let mut last_emit = std::time::Instant::now();
            for folder in &folders_walk {
                for entry in walkdir::WalkDir::new(folder)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let path = entry.path().to_string_lossy().into_owned();
                    let scoped = progress_on
                        && progress_scope
                            .as_ref()
                            .map(|sc| sc.iter().any(|s| Path::new(&path).starts_with(Path::new(s))))
                            .unwrap_or(false);
                    if scoped {
                        scanned += 1;
                    }
                    if entry.file_type().is_file() {
                        if let Some(ext) = ext_of(&path) {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as i64;
                            let mtime = file_mtime(entry.path());
                            let title = quick_title(&path);
                            found.insert(path, (size, mtime, title, ext));
                        }
                    }
                    // 心跳：范围内每 200 条或至少每 ~300ms 发一帧（即使条目数没到 200）。
                    if progress_on
                        && (last_emit.elapsed().as_millis() >= 300 || (scoped && scanned % 200 == 0))
                    {
                        last_emit = std::time::Instant::now();
                        emit(scanned, 0, true);
                    }
                }
            }
            (found, scanned)
        })
        .await
        .unwrap_or_default();

    // ---- 阶段二：一次性取锁读 `SELECT path FROM books` 入 HashSet，立即释放锁 ----
    let existing: std::collections::HashSet<String> = {
        let conn = state.db.0.lock().unwrap();
        db::all_paths_set(&conn)
    };

    // ---- 阶段三：无锁求差集 ----
    // 新增文件 = found - existing（不再逐条 get_book_by_path 判定 size/mtime；
    // 存量文件的实时变更由文件监听 install_watcher 负责，启动扫描只补新增）。
    let new_files: Vec<(String, i64, i64, String, String)> = found
        .iter()
        .filter(|(p, _)| !existing.contains(p.as_str()))
        .map(|(p, (s, m, t, e))| (p.clone(), *s, *m, t.clone(), e.clone()))
        .collect();
    // 失效文件 = existing 中位于已登记目录、但磁盘上已不存在（不在 found）的。
    let stale: Vec<String> = existing
        .iter()
        .filter(|p| {
            !found.contains_key(p.as_str())
                && folders
                    .iter()
                    .any(|f| std::path::Path::new(p).starts_with(std::path::Path::new(f)))
        })
        .cloned()
        .collect();

    // ---- 阶段四：分批写入，每 2000 条一个事务，批间释放锁并 yield_now() ----
    let mut changed = false;
    for chunk in stale.chunks(2000) {
        let r: Result<(), String> = (|| {
            let mut guard = state.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            db::delete_books(&tx, chunk)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })();
        if r.is_ok() {
            changed = true;
        } else if let Err(e) = r {
            eprintln!("[scan] delete_books tx failed: {e}");
        }
        tokio::task::yield_now().await;
    }

    let mut new_ids: Vec<i64> = Vec::with_capacity(new_files.len());
    for chunk in new_files.chunks(2000) {
        let r: Result<Vec<i64>, String> = (|| {
            let mut guard = state.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            let mut ids = Vec::with_capacity(chunk.len());
            for (path, size, mtime, title, ext) in chunk {
                if let Ok(id) = db::upsert_book(&tx, path, title, ext, *size, *mtime) {
                    ids.push(id);
                }
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(ids)
        })();
        match r {
            Ok(ids) => {
                new_ids.extend(ids);
                changed = true;
            }
            Err(e) => eprintln!("[scan] upsert tx failed: {e}"),
        }
        tokio::task::yield_now().await;
    }

    if changed {
        state.app.emit("library-changed", ()).ok();
    }
    // 导入目录落库后：新书 meta_done=0，拉起增量扫描（若空闲）。
    if !new_ids.is_empty() {
        ensure_scan_worker(&state.app);
    }
    // 扫描结束发 running:false，并带上本次新增书籍数（供前端提示）。
    if progress_on {
        state
            .app
            .emit(
                "folder-scan-progress",
                serde_json::json!({ "running": false, "scanned": scanned, "added": new_files.len() }),
            )
            .ok();
    }
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
                tauri::async_runtime::block_on(scan_folders(&state, None));
            }
        }
    });
}

/// 发一次进度事件（`meta-progress`，phase=meta）。前端底部进度条据此显示 `done/total`。
fn emit_scan_progress(app: &AppHandle, done: u64, total: u64) {
    app.emit(
        "meta-progress",
        serde_json::json!({ "phase": "meta", "done": done, "total": total }),
    )
    .ok();
}

/// 确保增量扫描 worker 在跑：进程内单例。已在跑则 no-op。
/// 调用点：启动、导入书籍/目录、打开书籍、列表可见项插队、清封面缓存后。
///
/// 退出竞态修复（方案 A 要求 5）：
/// 线程体顺序为 `on_startup → store(false, Release) → 再查一次是否仍有待办 → 有则递归拉起`。
/// 抢占用 `swap(true, AcqRel)`：若在「on_startup 收尾判空」与「store(false)」之间有新入队
/// （打开书 / 列表插队 / 导入），本次释放后的复查能兜住并重新拉起，绝不出现「有待办却无 worker」。
pub fn ensure_scan_worker(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = match app.try_state::<Arc<AppState>>() {
            Some(s) => s.inner().clone(),
            None => return,
        };
        // 进程内单例：swap 返回 true 说明已有 worker 在跑。
        if state.scan_running.swap(true, Ordering::AcqRel) {
            return;
        }
        state.scan_start_count.fetch_add(1, Ordering::SeqCst);
        state.scan_stop.store(false, Ordering::SeqCst);

        on_startup(state.clone());

        // 释放单例标记（Release：on_startup 期间的写对随后的复查/新 worker 可见）。
        state.scan_running.store(false, Ordering::Release);

        // 主动停止（清缓存/退出）时不再复查、不再自拉起。
        if state.scan_stop.load(Ordering::Acquire) {
            return;
        }
        // 复查：释放后是否仍有待办（普通待办或优先项）。有则递归拉起（会重新 swap 抢占）。
        let has_work = {
            let conn = state.db.0.lock().unwrap();
            db::count_meta_pending(&conn) > 0 || db::next_priority(&conn).is_some()
        };
        if has_work {
            ensure_scan_worker(&app);
        }
    });
}

/// 增量扫描主循环（单遍、串行、DB 驱动）。设计见实现提示词 A/B 节：
/// - 开头回收崩溃残留（meta_done=3 → 0）（B4）；
/// - 每一本处理前先查优先表，命中优先处理（B3）；
/// - 否则分批 `SELECT ... WHERE meta_done=0 ORDER BY id LIMIT 200`（A2）；
/// - 每本：重读状态(B6) → CAS 认领(0→3, B5) → 解析(不持 DB 锁) → 独立事务写 1/2 + 删优先表(A3/B5)；
/// - 批间 sleep 80ms；每 1000 次提交 wal_checkpoint(PASSIVE)；整轮结束 TRUNCATE（A7）；
/// - 每批比较前后 `COUNT(meta_done=0)`，未下降则打印并终止（A6）；
/// - 每本发 `cover-updated`{id}（B7），进度节流发 `meta-progress`。
fn on_startup(state: Arc<AppState>) {
    let covers_dir = covers::covers_dir(&state.app);
    const BATCH: i64 = 200;
    const EMIT_EVERY: u64 = 16;
    const CKPT_EVERY: u64 = 1000;

    // B4：回收上次中断残留的「处理中」。
    let recovered = {
        let conn = state.db.0.lock().unwrap();
        db::recover_processing(&conn)
    };

    // 进度分母：本轮启动时的待处理总数（+ 已知优先项）。分子从 0 计。
    let total0 = {
        let conn = state.db.0.lock().unwrap();
        db::count_meta_pending(&conn)
    } as u64;
    state.cover_scan_done.store(0, Ordering::SeqCst);
    state.cover_scan_total.store(total0, Ordering::SeqCst);
    emit_scan_progress(&state.app, 0, total0);
    if recovered > 0 {
        eprintln!("[scan] recovered {recovered} rows from meta_done=3 -> 0");
    }

    let mut commits: u64 = 0;
    let mut since_emit: u64 = 0;
    // A6：用「连续两批的待办 id 集合」比较进展，取代批次前后的全量 count_meta_pending。
    let mut last_batch_ids: Vec<i64> = Vec::new();

    // 处理单本（**只提取封面**）：按扩展名最前置分流。
    // 非容器格式：不打开文件、不做任何读取，直接 meta_done=1。
    // 容器格式：CAS 认领 → 无锁只提取封面 → 独立事务写 cover_path + meta_done。
    // allow 为 CAS 允许的原状态集合：启动批次 [0]，打开插队重试 [0,2]。
    let process_one = |id: i64, path: &str, format: &str, allow: &[i64]| -> Option<bool> {
        let ext = format.to_ascii_lowercase();

        // ---- 分流最前置：非容器格式不碰文件（不 File::open / 不 fs::read）----
        if !has_cover_container(&ext) {
            let wrote = {
                let conn = state.db.0.lock().unwrap();
                match db::meta_done_of(&conn, id) {
                    Some(md) if allow.contains(&md) => {}
                    _ => { db::delete_priority(&conn, id); return None; }
                }
                // 直接写完成，不动 cover_path、不读文件。
                let r = db::mark_meta(&conn, id, 1, None);
                db::delete_priority(&conn, id);
                match r {
                    Ok(()) => true,
                    Err(e) => { eprintln!("[scan] mark_meta(1) failed id={id}: {e}"); false }
                }
            };
            if wrote {
                state.app.emit("cover-updated", serde_json::json!({ "id": id })).ok();
            }
            return Some(false);
        }

        // ---- 容器格式：先在同一锁内重读状态(B6) + CAS 认领(0/2→3) ----
        {
            let conn = state.db.0.lock().unwrap();
            match db::meta_done_of(&conn, id) {
                Some(md) if allow.contains(&md) => {}
                _ => { db::delete_priority(&conn, id); return None; }
            }
            if !db::claim_meta(&conn, id, allow) {
                // next_priority 的 WHERE 含 meta_done=3：认领失败必删优先行，杜绝空转。
                db::delete_priority(&conn, id);
                return None;
            }
        }
        // 解析在**不持 DB 锁**时进行（全库共用一个 Mutex<Connection>，持锁解析会冻结 UI）。
        // 只提取封面，绝不读全文、不产出 title/author/chars。
        // 看门狗：个别损坏/异常文件可能让封面解析长时间不返回（如 zip 中央目录条目损坏触发
        // zip-rs 回扫整文件）。在独立线程中提取、最多等 5s。失败**按类型**区分：
        //   预算字节/调用次数耗尽 → Pathological（真病态）；超时且预算未耗尽 → Timeout（多因 I/O 慢）；
        //   panic → Panic；其它 → Error。分类由 `CoverReadBudget::exhausted()` 等类型信息决定，
        //   **不依赖错误消息字符串**。
        // **不留泄漏线程**：超时时 `budget.cancel()`，worker 内受限 reader 下一次 read 立即 Err 退出。
        let budget = Arc::new(crate::epub::CoverReadBudget::default());
        let cover: Result<Option<String>, CoverFail> = {
            let (txr, rxr) = std::sync::mpsc::channel();
            let p = path.to_string();
            let e = ext.clone();
            let cd = covers_dir.clone();
            let b = budget.clone();
            std::thread::spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    extract_cover_only(&p, &e, &cd, &b)
                }));
                let out = match r {
                    Err(_) => Err(CoverFail::Panic),
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(msg)) => {
                        eprintln!("[scan] extract_cover_only failed id={id}: {msg}");
                        Err(if b.exhausted() {
                            CoverFail::Pathological
                        } else {
                            CoverFail::Error
                        })
                    }
                };
                let _ = txr.send(out);
            });
            match rxr.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(r) => r,
                Err(_) => {
                    budget.cancel(); // 取消：令 worker 的受限 reader 立即报错退出
                    Err(CoverFail::Timeout)
                }
            }
        };
        let mut got_cover = false;
        let mut wrote_done = false;
        {
            let conn = state.db.0.lock().unwrap();
            match &cover {
                Ok(name) => {
                    match db::set_cover_path(&conn, id, name.as_deref()) {
                        Ok(()) => {
                            got_cover = name.is_some();
                            match db::mark_meta(&conn, id, 1, None) {
                                Ok(()) => wrote_done = true,
                                Err(e) => eprintln!("[scan] mark_meta(1) failed id={id}: {e}"),
                            }
                        }
                        Err(e) => {
                            eprintln!("[scan] set_cover_path failed id={id}: {e}");
                            // 写库失败：归类为 cover-error（允许重试），并保留日志详情。
                            if let Err(e2) = db::mark_meta(&conn, id, 2, Some(CoverFail::Error.token())) {
                                eprintln!("[scan] mark_meta(2) failed id={id}: {e2}");
                            }
                        }
                    }
                }
                Err(kind) => {
                    eprintln!("[scan] cover fail id={id}: {}", kind.token());
                    if let Err(e2) = db::mark_meta(&conn, id, 2, Some(kind.token())) {
                        eprintln!("[scan] mark_meta(2) failed id={id}: {e2}");
                    }
                }
            }
            db::delete_priority(&conn, id);
        }
        // B7 + 要求6：cover-updated 仅在 meta_done 成功写为 1 时发送。
        if wrote_done {
            state.app.emit("cover-updated", serde_json::json!({ "id": id })).ok();
        }
        Some(got_cover)
    };

    // 优先表：处理其中一本（打开书籍插队，允许对失败项 0/2 重试）。返回是否命中。
    // 命中同样计入进度分子（优先项也是 meta_done=0，已包含在启动时的 total0 内）并按节流
    // 发 meta-progress —— 否则「只服务优先表」期间 `cover_scan_done` 不动、事件不发，
    // 底栏进度条会长时间不出现/不推进（封面却仍在插队生成）。
    let drain_one_priority = |commits: &mut u64, since_emit: &mut u64| -> bool {
        let hit = {
            let conn = state.db.0.lock().unwrap();
            db::next_priority(&conn)
        };
        match hit {
            Some((id, path, format)) => {
                if process_one(id, &path, &format, &[0, 2]).is_some() {
                    *commits += 1;
                    let done = state.cover_scan_done.fetch_add(1, Ordering::SeqCst) + 1;
                    *since_emit += 1;
                    let total = state.cover_scan_total.load(Ordering::SeqCst);
                    if *since_emit >= EMIT_EVERY {
                        *since_emit = 0;
                        emit_scan_progress(&state.app, done, total);
                        state.app.emit("library-meta", ()).ok();
                    }
                }
                true
            }
            None => false,
        }
    };

    'outer: loop {
        if state.scan_stop.load(Ordering::SeqCst) {
            break;
        }
        // B3：每一本处理前先查优先表；命中则先把优先项清空。
        while drain_one_priority(&mut commits, &mut since_emit) {
            if state.scan_stop.load(Ordering::SeqCst) {
                break 'outer;
            }
            if commits % CKPT_EVERY == 0 && commits > 0 {
                let conn = state.db.0.lock().unwrap();
                let _ = conn.pragma_update(None, "wal_checkpoint", "PASSIVE");
            }
        }

        // A2：分批取未处理项。
        let batch = {
            let conn = state.db.0.lock().unwrap();
            db::next_meta_batch(&conn, BATCH)
        };
        if batch.is_empty() {
            // 要求6：普通待办为 0 时，必须先确认优先表也空（drain 返回 None）才允许退出，
            // 避免恰好此刻有「列表插队 / 打开书」写入而漏处理。
            if drain_one_priority(&mut commits, &mut since_emit) {
                continue;
            }
            break; // 普通待办与优先表都空 → 退出。
        }
        // A6（局部比较，取代全量 count_meta_pending）：若本批 id 集合与上一批完全相同，
        // 说明这批没有任何一本被推进（卡住），终止本轮防死循环；否则记录为最新一批。
        let cur_batch_ids: Vec<i64> = batch.iter().map(|(id, _, _)| *id).collect();
        if cur_batch_ids == last_batch_ids {
            eprintln!(
                "[scan] batch id set unchanged (len={}), aborting round",
                cur_batch_ids.len()
            );
            break;
        }
        last_batch_ids = cur_batch_ids;

        for (id, path, format) in &batch {
            if state.scan_stop.load(Ordering::SeqCst) {
                break 'outer;
            }
            // B3：每本之前再查优先表（打开书籍可随时插队）。
            while drain_one_priority(&mut commits, &mut since_emit) {}
            if process_one(*id, path, format, &[0]).is_some() {
                commits += 1;
                let done = state.cover_scan_done.fetch_add(1, Ordering::SeqCst) + 1;
                since_emit += 1;
                let total = state.cover_scan_total.load(Ordering::SeqCst);
                if since_emit >= EMIT_EVERY {
                    since_emit = 0;
                    emit_scan_progress(&state.app, done, total);
                    state.app.emit("library-meta", ()).ok();
                }
                if commits % CKPT_EVERY == 0 {
                    let conn = state.db.0.lock().unwrap();
                    let _ = conn.pragma_update(None, "wal_checkpoint", "PASSIVE");
                }
            }
        }

        // A7：批间限流，避免满速占盘。
        std::thread::sleep(std::time::Duration::from_millis(80));
    }

    // 整轮结束：TRUNCATE 收缩 WAL；发最终进度 + scan-finished。
    {
        let conn = state.db.0.lock().unwrap();
        let _ = conn.pragma_update(None, "wal_checkpoint", "TRUNCATE");
    }
    let done = state.cover_scan_done.load(Ordering::SeqCst);
    let total = state.cover_scan_total.load(Ordering::SeqCst);
    emit_scan_progress(&state.app, done, total);
    state.app.emit("library-meta", ()).ok();
    state.app
        .emit("scan-finished", serde_json::json!({ "done": done, "total": total }))
        .ok();
}

/// 打开书籍时调用（B2）：
/// - meta_done==1 / 书不存在 → 直接返回；
/// - `meta_error` 归类为 `Pathological`（真病态，重试还会再病态）→ **拒绝插队**；
/// - `Timeout`（多为 I/O 慢）→ 允许重试，但 `fail_count` 达到 `MAX_COVER_TIMEOUT_RETRIES` 后不再插队；
/// - 其余（无错/Error/Panic）→ 插队重试。
/// 归类经 `CoverFail` 类型判定，非错误消息字符串匹配。
pub fn on_book_opened(state: &Arc<AppState>, book_id: i64) {
    {
        let conn = state.db.0.lock().unwrap();
        match db::meta_state_of(&conn, book_id) {
            None => return,                    // 书不存在
            Some((1, _, _)) => return,         // 已完成
            Some((_, Some(err), fail_count)) => {
                match CoverFail::from_token(&err) {
                    Some(CoverFail::Pathological) => return, // 病态：不插队
                    Some(CoverFail::Timeout) if fail_count > MAX_COVER_TIMEOUT_RETRIES => {
                        return; // 超时重试已达上限（允许 2 次重试）
                    }
                    _ => {}
                }
                if let Err(e) = db::enqueue_priority(&conn, book_id) {
                    eprintln!("[scan] enqueue_priority failed id={book_id}: {e}");
                    return;
                }
            }
            Some((_, None, _)) => {
                if let Err(e) = db::enqueue_priority(&conn, book_id) {
                    eprintln!("[scan] enqueue_priority failed id={book_id}: {e}");
                    return;
                }
            }
        }
    }
    ensure_scan_worker(&state.app);
}

/// 停止封面扫描 worker 并等待其退出（最多约 3 秒），用于清缓存前避免删/写并发。
fn stop_cover_worker_and_wait(state: &Arc<AppState>) {
    state.scan_stop.store(true, Ordering::SeqCst);
    for _ in 0..60 {
        if !state.scan_running.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// 读取默认封面底图为 data URL（设置里可更换）；无则 None（用内置渐变）。
fn default_cover_data_url(app: &AppHandle) -> Option<String> {
    let p = covers::default_cover_path(app);
    let bytes = std::fs::read(&p).ok()?;
    // 底图统一存为 `default-cover.img`，扩展名不可靠：按字节魔数嗅探真实类型，
    // 保证 png/jpg/jpeg/webp/gif/bmp 底图都能被正确声明 MIME（避免 jpg/webp 被当 png）。
    let mime = crate::sniff_mime(&bytes);
    Some(format!("data:{};base64,{}", mime, covers::b64(&bytes)))
}

/// 外壳主题传入的默认封面配色（来自 cover URL 查询串；空则回落内置观感）。
#[derive(Default, Clone)]
pub struct DcTheme {
    pub g1: String,
    pub g2: String,
    pub g3: String,
    /// 书名字色。
    pub tc: String,
}

fn nonempty(v: &str, fallback: &str) -> String {
    if v.trim().is_empty() { fallback.to_string() } else { v.trim().to_string() }
}

/// F2：渲染共享默认封面（底图 + 实时叠字）。
/// 书名/作者取 DB 实时值（无则用文件名解析），文字不落盘、不缓存。
/// `theme` 为外壳主题配色：无自定义底图时决定渐变/字色（随主题变化、保证易读）；
/// 有自定义底图时字色改用「设置底图时分析出的自适应色」（读书名区亮度定深/浅）。
pub fn render_default_cover(state: &Arc<AppState>, book_id: i64, theme: &DcTheme) -> String {
    let (title, author, dc_text, dc_light) = {
        let conn = state.db.0.lock().unwrap();
        let (t, a) = match db::get_book(&conn, book_id) {
            Some(b) => {
                let t = if b.title.trim().is_empty() {
                    parse_filename(&b.path).0
                } else {
                    b.title
                };
                let a = if b.author.trim().is_empty() {
                    parse_filename(&b.path).1
                } else {
                    b.author
                };
                (t, a)
            }
            None => ("未命名".to_string(), String::new()),
        };
        let dc_text = db::get_setting(&conn, "dc_text").filter(|s| !s.trim().is_empty());
        let dc_light = db::get_setting(&conn, "dc_text_light").map(|s| s == "1");
        (t, a, dc_text, dc_light)
    };

    let base = default_cover_data_url(&state.app);
    let style = if base.is_some() {
        // 自定义底图：字色用「设置时分析出的自适应色」；缺失则回落柔和浅色 + 压暗遮罩。
        let text = dc_text.unwrap_or_else(|| "#EFEBE4".to_string());
        let text_is_light = dc_light.unwrap_or(true);
        covers::CoverStyle {
            base_href: base,
            g1: String::new(),
            g2: String::new(),
            g3: String::new(),
            accent: text.clone(),
            text,
            text_is_light,
        }
    } else {
        // 主题渐变：字色/渐变随外壳主题变化；遮罩方向按字色明暗自适应。
        let text = nonempty(&theme.tc, "#f4ecdc");
        let text_is_light = covers::hex_is_light(&text);
        covers::CoverStyle {
            base_href: None,
            g1: nonempty(&theme.g1, "#36485c"),
            g2: nonempty(&theme.g2, "#212d3c"),
            g3: nonempty(&theme.g3, "#141c26"),
            accent: text.clone(),
            text,
            text_is_light,
        }
    };
    covers::default_cover_svg_styled(&title, &author, &style)
}

/// 封面提取来源（单一事实源）：容器格式 → 具体提取分支；其余 → None（非容器，不读文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverSource {
    Epub,
    Cbz,
    Mobi,
}

fn cover_source(ext: &str) -> Option<CoverSource> {
    match ext {
        "epub" => Some(CoverSource::Epub),
        "cbz" => Some(CoverSource::Cbz),
        "mobi" | "azw" | "azw3" => Some(CoverSource::Mobi),
        _ => None,
    }
}

/// 具备"可提取内嵌封面"的容器格式集合。**与 `extract_cover_only` 严格对齐**：
/// 直接用 `cover_source` 判定，二者不可能错位。返回 false 的格式（txt/fb2/cbr 等）扫描时**完全不读文件**。
fn has_cover_container(ext: &str) -> bool {
    cover_source(ext).is_some()
}

/// 封面字节上限：超过即判为异常，拒绝进入缩略图流程。
const MAX_COVER_BYTES: usize = 32 * 1024 * 1024;

/// 封面提取失败归类（**带类型**，不以错误消息字符串匹配来区分）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoverFail {
    /// 命中受限 reader 的字节/调用预算：真正病态（如 zip 因中央目录损坏回扫整文件）。
    Pathological,
    /// 看门狗超时且预算未耗尽：多为 I/O 慢/争用，可重试。
    Timeout,
    /// 解析线程 panic。
    Panic,
    /// 其它解析/写入错误（缺 EOCD、格式不支持、DRM、写库失败等）。
    Error,
}

impl CoverFail {
    /// 持久化到 `meta_error` 的规范标记。
    pub fn token(self) -> &'static str {
        match self {
            CoverFail::Pathological => "cover-pathological",
            CoverFail::Timeout => "cover-timeout",
            CoverFail::Panic => "cover-panic",
            CoverFail::Error => "cover-error",
        }
    }

    /// 由 `meta_error` 标记解析回类型。这是 DB 边界的标记↔类型的确定性映射，
    /// 非"错误消息字符串匹配"。
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "cover-pathological" => CoverFail::Pathological,
            "cover-timeout" => CoverFail::Timeout,
            "cover-panic" => CoverFail::Panic,
            "cover-error" => CoverFail::Error,
            _ => return None,
        })
    }
}

/// 打开书籍时允许的 `cover-timeout` 重试次数上限（超过即不再插队重复超时）。
pub const MAX_COVER_TIMEOUT_RETRIES: i64 = 2;

/// **只提取封面**：按 `cover_source` 分流各自最小读取路径，仅返回封面文件名（不含 title/author/chars）。
/// - Epub：container.xml → OPF → 封面条目（其余不解压）；
/// - Cbz：zip 内第一张图片；
/// - Mobi(含 azw/azw3)：header/EXTH 定位的单条封面记录（seek，不读全文）；
/// - None（非容器）：不读文件，返回 Ok(None)。
fn extract_cover_only(
    path: &str,
    ext: &str,
    covers_dir: &Path,
    budget: &Arc<crate::epub::CoverReadBudget>,
) -> Result<Option<String>, String> {
    let bytes: Option<Vec<u8>> = match cover_source(ext) {
        Some(CoverSource::Epub) => {
            // 提前判定：只读文件尾 ≤1KiB 的 EOCD，中央目录 >8MiB / 无 EOCD 直接 Err，
            // 不进入 ZipArchive::new（挡住部分畸形/超大 CD 的文件）。
            crate::epub::precheck_eocd(path)?;
            crate::epub::extract_cover_file(path, budget)?
        }
        Some(CoverSource::Cbz) => {
            crate::epub::precheck_eocd(path)?;
            crate::epub::extract_cbz_cover_file(path, budget)?
        }
        Some(CoverSource::Mobi) => crate::mobi::extract_cover_seek(path),
        None => return Ok(None), // 非容器：不读文件
    };
    match bytes {
        Some(b) => {
            if b.len() > MAX_COVER_BYTES {
                return Err(format!(
                    "封面字节数 {} 超过上限 {} (32MiB)，跳过缩略图",
                    b.len(),
                    MAX_COVER_BYTES
                ));
            }
            let name = format!("{}.png", db::path_id(path));
            let out = covers_dir.join(&name);
            covers::make_thumbnail(&b, &out)?;
            Ok(Some(name))
        }
        None => Ok(None),
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

/// EPUB 字数统计（每章字数 + 全书 total/pure）：先查共享缓存，未命中则读盘并完整解析一次，随后回填 DB。
///
/// 为什么共享缓存：book_char_counts（全书总页数）与 get_book_word_count（书详情/本书统计）
/// 要的是同一份统计，各解析一次就是把整本书解析两遍（数千章是秒级）。
/// 为什么回填 DB：books.total_chars > 0 是这两个命令的短路条件，回填后同一本书不再重算。
///
/// 本函数是阻塞的（读盘 + rayon 逐章统计），**只能在 spawn_blocking 里调用**。
fn epub_stats_for(state: &Arc<AppState>, book_id: i64, path: &str) -> Result<Arc<EpubStats>, String> {
    let stamp = file_stamp(path);
    let hit = state.epub_stats_cache.lock().unwrap().get(&book_id).cloned();
    if let Some(h) = hit {
        if stamp_matches(h.stamp, stamp) {
            return Ok(h);
        }
        // 文件换过了：旧条目立刻移除，避免它被其它入口（book_char_counts / 书详情）再读走。
        state.epub_stats_cache.lock().unwrap().remove(&book_id);
    }
    let data = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    let (per_chapter, total, pure) = crate::epub::epub_word_stats(&data)?;
    let stats = Arc::new(EpubStats { stamp, per_chapter, total, pure });
    // 指纹取不到就不入缓存：否则这条「无法校验」的条目会一直命中。
    if stamp.is_some() {
        push_cache(&state.epub_stats_cache, book_id, stats.clone(), 4);
    }
    backfill_book_chars(state, book_id, stats.total, stats.pure);
    Ok(stats)
}

/// 把统计出来的字数回填到 books 的两列（尽力而为：失败只记日志，不影响本次返回）。
fn backfill_book_chars(state: &Arc<AppState>, book_id: i64, total: i64, pure: i64) {
    let wrote = {
        let conn = state.db.0.lock().unwrap();
        db::set_book_chars(&conn, book_id, total, pure)
    };
    if let Err(e) = wrote {
        eprintln!("[chars] backfill failed book={book_id}: {e}");
    }
}

pub fn load_txt(
    state: &AppState,
    path: &str,
    book_id: i64,
) -> Result<Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>, String> {
    let ver = state.rules_ver.load(Ordering::Relaxed);
    // 命中必须同时满足「规则版本一致」与「文件指纹一致」：只比版本的话，
    // 同一路径的文件被就地替换后正文会一直是旧文件的。
    let stamp = file_stamp(path);
    let hit = state.txt_cache.lock().unwrap().get(&book_id).cloned();
    if let Some(c) = hit {
        if c.ver == ver && stamp_matches(c.stamp, stamp) {
            return Ok(c.doc.clone());
        }
        state.txt_cache.lock().unwrap().remove(&book_id);
    }
    let pattern = get_rules(state);
    // 手动兜底:正文字符编码设置(与 MOBI 共用 `mobi.encoding` 键)也作用于 TXT。
    // 为空 / "auto" 时传 None,走 decode_bytes 的自动检测(常用汉字密度简繁择优)。
    let forced = {
        let conn = state.db.0.lock().unwrap();
        db::get_setting(&conn, "mobi.encoding").unwrap_or_default()
    };
    let forced = {
        let t = forced.trim().to_ascii_lowercase();
        if t.is_empty() || t == "auto" { None } else { Some(t) }
    };
    let doc = txt::decode_and_split(path, Some(&pattern), forced.as_deref())?;
    let result = Arc::new((doc.text, doc.ranges, doc.toc));
    if stamp.is_some() {
        push_cache(
            &state.txt_cache,
            book_id,
            CachedText { ver, stamp, doc: result.clone() },
            3,
        );
    }
    Ok(result)
}

pub fn load_epub(state: &AppState, path: &str, book_id: i64) -> Result<Arc<CachedEpub>, String> {
    // 指纹校验：文件被就地替换后必须重解析（否则正文/toc/spine 全是旧文件的）。
    let stamp = file_stamp(path);
    let hit = state.epub_cache.lock().unwrap().get(&book_id).cloned();
    if let Some(c) = hit {
        if stamp_matches(c.stamp, stamp) {
            return Ok(c.clone());
        }
        state.epub_cache.lock().unwrap().remove(&book_id);
    }
    let t0 = std::time::Instant::now();
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    // 借用解析（Cursor 只借用 data），避免对 600MB+ 大书再复制一份；解析完再 Arc 移交所有权。
    let (doc, ci) = crate::epub::parse_with_ci(&data)?;
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[open] load_epub book={book_id} parse+ci={:?} spine={} toc={}",
            t0.elapsed(), doc.spine.len(), doc.toc.len()
        );
    }
    let cached = Arc::new(CachedEpub {
        doc,
        data: Arc::new(data),
        ci,
        stamp,
    });
    if stamp.is_some() {
        push_cache(&state.epub_cache, book_id, cached.clone(), 3);
    }
    Ok(cached)
}

/// 按需读取 EPUB 内图片（供 `epubimg/` 协议路由使用）：优先用缓存，未命中则读盘解析。
/// 返回 (mime, bytes)；找不到返回 None。
pub fn read_epub_image(state: &AppState, book_id: i64, target: &str) -> Option<(String, Vec<u8>)> {
    let cached = state.epub_cache.lock().unwrap().get(&book_id).cloned();
    let cached = match cached {
        Some(c) => c,
        None => {
            let path = {
                let conn = state.db.0.lock().unwrap();
                db::get_book(&conn, book_id)?.path
            };
            load_epub(state, &path, book_id).ok()?
        }
    };
    let bytes = crate::epub::read_entry(&cached.data, &cached.ci, target)?;
    Some((crate::epub::mime_for_path(target).to_string(), bytes))
}

/// 全书图片列表（按 spine/章节顺序，按解析出的 zip 路径去重）。
/// 用于图片查看器浏览整本书所有图片。EPUB 走 zip 内路径；MOBI/AZW3 走已提取的 res 目录。
#[tauri::command]
pub async fn list_book_images(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<Vec<BookImage>, String> {
    if let Some(hit) = state.image_cache.lock().unwrap().get(&book_id) {
        return Ok(hit.as_ref().clone());
    }
    let st = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || collect_book_images(&st, book_id))
        .await
        .map_err(|e| e.to_string())??;
    state
        .image_cache
        .lock()
        .unwrap()
        .insert(book_id, Arc::new(result.clone()));
    Ok(result)
}

fn collect_book_images(state: &AppState, book_id: i64) -> Result<Vec<BookImage>, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let mut out: Vec<BookImage> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if ext == "epub" {
        let cached = load_epub(state, &book.path, book_id)?;
        let doc = &cached.doc;
        // 直接使用 OPF manifest 的图片清单（瞬时、绝不扫描全书章节）。
        let mut targets: Vec<String> = Vec::new();
        if let Some(c) = &doc.cover_href {
            if seen.insert(c.clone()) {
                targets.push(c.clone());
            }
        }
        for href in &doc.image_hrefs {
            if seen.insert(href.clone()) {
                targets.push(href.clone());
            }
        }
        for (i, target) in targets.iter().enumerate() {
            out.push(BookImage {
                url: crate::epub::epub_image_url(book_id, target),
                chapter_index: i as i64,
                chapter_title: if i == 0 { "封面".to_string() } else { String::new() },
            });
        }
    } else if is_mobi_ext(&ext) {
        // 已提取到 appdata/res/<book_id>/ 的图片
        let dir = crate::data_dir(&state.app).join("res").join(book_id.to_string());
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut files: Vec<String> = rd
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            files.sort();
            for f in files {
                let url = format!("http://tidepage.localhost/res/{book_id}/{}", f);
                if seen.insert(url.clone()) {
                    out.push(BookImage { url, chapter_index: -1, chapter_title: String::new() });
                }
            }
        }
    }
    Ok(out)
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
        scan_folders(&st, Some(vec![path])).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn remove_folder(state: State<'_, Arc<AppState>>, path: String) -> Result<(), String> {
    let st = state.inner().clone();
    let diag = std::env::var("TIDEPAGE_OP_DIAG").is_ok();
    let t0 = std::time::Instant::now();
    {
        let conn = st.db.0.lock().unwrap();
        db::remove_folder(&conn, &path)?;
    }
    let base = path.trim_end_matches(['/', '\\']).to_string();
    let folder = Path::new(&base).to_path_buf();
    // 用可走索引的前缀范围代替 LIKE 全表扫描：base <= path < base+U+10FFFF。
    let hi = format!("{}{}", base, '\u{10FFFF}');
    let t_query = std::time::Instant::now();
    let rows: Vec<String> = {
        let conn = st.db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path FROM books WHERE path >= ?1 AND path < ?2")
            .map_err(|e| e.to_string())?;
        stmt.query_map(rusqlite::params![base, hi], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    };
    // 前缀边界修正：`C:\Books` 不应命中 `C:\Books2`。
    let row_count = rows.len();
    let doomed: Vec<String> = rows
        .into_iter()
        .filter(|p| Path::new(p).starts_with(&folder))
        .collect();
    if diag {
        eprintln!(
            "[remove_folder] query={:?} rows={} doomed={}",
            t_query.elapsed(),
            row_count,
            doomed.len()
        );
    }
    // 分批删除，每批一个事务；批间释放 db 锁并 yield，避免长持锁阻塞窗口取行等 DB 读取。
    // 与 scan_folders 阶段四同款：异步命令体直接跑（非同步命令/主线程），批间让出。
    let t_del = std::time::Instant::now();
    for chunk in doomed.chunks(2000) {
        let r: Result<(), String> = (|| {
            let mut guard = st.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            db::delete_books_policy(&tx, chunk, true)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(e) = r {
            eprintln!("[remove_folder] delete tx failed: {e}");
        }
        tokio::task::yield_now().await;
    }
    if diag {
        eprintln!(
            "[remove_folder] delete={:?} total={:?}",
            t_del.elapsed(),
            t0.elapsed()
        );
    }
    state.app.emit("library-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub async fn list_books(state: State<'_, Arc<AppState>>) -> Result<Vec<BookRow>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_books(&conn)
    })
    .await
    .map_err(|e| e.to_string())
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
    formats: Option<Vec<String>>,
    states: Option<Vec<String>>,
    desc: Option<bool>,
    folder: Option<String>,
) -> Result<BookPage, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_books_page(&conn, &query, &sort, offset, limit, &formats, &states, desc.unwrap_or(false), &folder)
    })
    .await
    .map_err(|e| e.to_string())
}

/// 秩可寻址分页有序索引上限：超过则前端必须切 keyset 档（禁止静默截断）。
const MAX_ORDER_INDEX_ROWS: i64 = 300_000;

/// 命令 1（秩可寻址分页）：返回当前 `sort` + 可选 `query` 下**有序 books.id 序列**，
/// 编码为小端 i64 原始字节流（每 8 字节一个 id）；前端得到 ArrayBuffer 后建 BigInt64Array。
/// - 结果行数 > `MAX_ORDER_INDEX_ROWS` 返回 `Err("order_index_too_large:300000")`（边收集边中止）；
/// - `sort` 白名单外一律回落 "added"；ORDER BY 为字面量（见 `db::order_index_ids`）；
/// - `query` 非空时施加与 `list_books_page` 逐字同源的 WHERE，搜索档同样 O(W) 跳秩（不再回落 OFFSET）。
#[tauri::command]
pub async fn list_order_index(
    state: State<'_, Arc<AppState>>,
    sort: String,
    query: Option<String>,
    formats: Option<Vec<String>>,
    states: Option<Vec<String>>,
    desc: Option<bool>,
    folder: Option<String>,
) -> Result<tauri::ipc::Response, String> {
    // 白名单归一（未知回落 added），避免把任意字符串带入下游。
    let sort = match sort.as_str() {
        "added" | "recent" | "title" | "lastread" | "size" => sort,
        _ => "added".to_string(),
    };
    let desc = desc.unwrap_or(false);
    let query = query.unwrap_or_default();
    let st = state.inner().clone();
    let bytes: Vec<u8> = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let conn = st.db.0.lock().unwrap();
        // 边收集边计数：越限立即 Err（禁止静默截断），不为超大结果集构建巨大 Vec。
        let ids = db::order_index_ids(&conn, &sort, &query, MAX_ORDER_INDEX_ROWS as usize, &formats, &states, desc, &folder)?;
        let mut buf = Vec::with_capacity(ids.len() * 8);
        for id in ids {
            buf.extend_from_slice(&id.to_le_bytes());
        }
        Ok(buf)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Task B：书库当前实际存在的格式列表（去重、固定排序）。供前端「分类」面板动态生成格式标签。
#[tauri::command]
pub async fn list_formats(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_formats(&conn)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Task F：手动设置阅读状态覆盖。stateStr 取 "read" / "unread" / "auto"（read→1，unread→2，auto→0）。
/// 只写 read_state，不动 read_pct（真实进度不破坏）。前端据是否处于状态筛选决定 reload / 就地刷新。
#[tauri::command]
pub async fn set_read_state(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    state_str: String,
) -> Result<(), String> {
    let v: i64 = match state_str.as_str() {
        "read" => 1,
        "unread" => 2,
        "auto" => 0,
        _ => return Err(format!("unknown read_state: {state_str}")),
    };
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::set_read_state(&conn, book_id, v)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 命令 2（秩可寻址分页）：按 `ids` 批量取行。返回长度/顺序恒等于 `ids`，
/// 不存在的 id 以 `null` 占位（保持秩对齐）。`ids.len() > 4096` 返回 `Err("too_many_ids:N")`。
#[tauri::command]
pub async fn list_books_by_ids(
    state: State<'_, Arc<AppState>>,
    ids: Vec<i64>,
) -> Result<Vec<Option<BookRow>>, String> {
    if ids.len() > 4096 {
        return Err(format!("too_many_ids:{}", ids.len()));
    }
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::books_by_ids_aligned(&conn, &ids)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rescan(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let st = state.inner().clone();
    tokio::spawn(async move {
        scan_folders(&st, None).await;
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
// staged: 对账算法的唯一有测试的实现（命令 refresh_library 目前内联重写同逻辑且无测试；最干净的做法是让命令委托至此以复用其 5 项测试，属行为相邻重构，另行评审）。
#[allow(dead_code)]
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

/// 扫描进度事件负载（phase: check=对账缺失 / scan=扫描新增 / done=完成）。
#[derive(serde::Serialize, Clone)]
pub struct RefreshProgress {
    pub phase: String,
    pub done: usize,
    pub total: usize,
}

/// 刷新（带进度）：分步执行，**不长时间持有 DB 锁**，因此扫描期间阅读/翻页/打开书等其它 IPC 不受阻。
/// 与 `refresh_library_core` 对账规则一致：缺失记录移除、存活记录所在目录 ∪ 书库根及其一级子目录只扫一层。
#[tauri::command]
pub async fn refresh_library(state: State<'_, Arc<AppState>>) -> Result<RefreshResult, String> {
    let st = state.inner().clone();
    let emit = |phase: &str, done: usize, total: usize| {
        st.app
            .emit(
                "refresh-progress",
                RefreshProgress { phase: phase.into(), done, total },
            )
            .ok();
    };

    // 阶段 1：快照记录与书库根（短暂持锁），文件系统存在性检查不持锁。
    let (records, folders) = {
        let conn = st.db.0.lock().unwrap();
        (db::all_book_paths(&conn), db::list_folders(&conn))
    };
    emit("check", 0, records.len());
    let mut missing: Vec<String> = Vec::new();
    let mut survivors: Vec<String> = Vec::new();
    for (i, (_, p)) in records.iter().enumerate() {
        if Path::new(p).exists() {
            survivors.push(p.clone());
        } else {
            missing.push(p.clone());
        }
        if i % 200 == 0 || i + 1 == records.len() {
            emit("check", i + 1, records.len());
        }
    }
    if !missing.is_empty() {
        let conn = st.db.0.lock().unwrap();
        let _ = db::delete_books(&conn, &missing);
    }

    // 扫描目录：存活记录所在目录 ∪ 书库根及其一级子目录。
    let mut scan_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for p in &survivors {
        if let Some(d) = Path::new(p.as_str()).parent() {
            scan_dirs.insert(d.to_path_buf());
        }
    }
    for root in &folders {
        let rp = PathBuf::from(root);
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

    // 阶段 2：遍历目录（不持锁），只收集索引中不存在的书；最后一次性入库。
    let known: std::collections::HashSet<String> =
        records.iter().map(|(_, p)| p.clone()).collect();
    let dirs: Vec<PathBuf> = scan_dirs.into_iter().collect();
    emit("scan", 0, dirs.len());
    let mut new_books: Vec<(String, String, String, i64, i64)> = Vec::new();
    for (di, dir) in dirs.iter().enumerate() {
        if let Ok(rd) = std::fs::read_dir(dir) {
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
                if known.contains(&path) || new_books.iter().any(|(p, ..)| p == &path) {
                    continue;
                }
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as i64;
                let mtime = file_mtime(&fp);
                let title = quick_title(&path);
                new_books.push((path, title, ext, size, mtime));
            }
        }
        emit("scan", di + 1, dirs.len());
    }

    let mut added = 0usize;
    let mut new_ids: Vec<i64> = Vec::new();
    if !new_books.is_empty() {
        let conn = st.db.0.lock().unwrap();
        for (path, title, ext, size, mtime) in &new_books {
            if let Ok(id) = db::upsert_book(&conn, path, title, ext, *size, *mtime) {
                added += 1;
                new_ids.push(id);
            }
        }
    }
    if !missing.is_empty() || added > 0 {
        st.app.emit("library-changed", ()).ok();
    }
    emit("done", 1, 1);
    // 导入目录落库后：新书 meta_done=0，拉起增量扫描（若空闲）。
    if !new_ids.is_empty() {
        ensure_scan_worker(&st.app);
    }
    Ok(RefreshResult { removed: missing.len(), added })
}

/// 任务3：打开入口先查文件是否存在（不存在则由前端弹询问，绝不进入空白阅读）。
#[tauri::command]
pub fn file_exists(path: String) -> bool {
    !path.is_empty() && Path::new(&path).exists()
}

/// 拖拽入书库时判定路径是否为目录（目录直接登记为书库根，文件走单本导入）。
#[tauri::command]
pub fn path_is_dir(path: String) -> bool {
    !path.is_empty() && Path::new(&path).is_dir()
}

/// 漫画正文阈值：单章「非空白字符数 − 该章目录标题非空白字符数」超过它就判为非漫画。
/// 取 40 的依据：纯漫画页若在章节文档里带 <h1>第N话</h1> 之类标题，已被标题长度抵扣，剩余
/// 通常为 0~个位数（拟声词、页边页码）；而最精简的正文段落，一句话也远超 40 个非空白字符。
/// 40 因而能可靠挡住「带插图的普通小说」，又不会误杀带少量拟声词的漫画。
const MANGA_BODY_CHAR_LIMIT: i64 = 40;

/// 漫画判定时允许解压的字节预算（8MB）。
/// 预算耗尽即停止扫描，当前结果**放行**（视为漫画）：已经连续解压 8MB 的 spine XHTML 都没有
/// 出现一段正文，对普通小说极不可能（其第一章正文通常几十 KB 内就触发提前退出）；
/// 放行同时保证超大漫画集判定不会因遍历上万文档而变慢/卡住打开。
const MANGA_SCAN_BUDGET: usize = 8 * 1024 * 1024;

/// 图片门槛（廉价前置条件，绝不读正文）：图片数够多（≥30）且「大致每个内容文档≈一张图」。
/// - EPUB：以 spine（内容文档）数为分母。一图一页漫画图/spine≈1，故 ≥ spine*0.8 即过门槛；
///   旧的 toc 比例*1.5 对「每张图一个 xhtml 页」结构会漏判（图/章≈1），此处保留为「或」条件。
/// - MOBI/TXT：无 spine 清单（spine_n 传 0），仅按原 toc 比例*1.5（保持原行为）。
fn manga_image_gate(images: usize, spine_n: usize, toc_n: usize, ext: &str) -> bool {
    images >= 30
        && ((ext == "epub" && (images as f64) >= (spine_n.max(1) as f64) * 0.8)
            || (images as f64) >= (toc_n.max(1) as f64) * 1.5)
}

/// 漫画判据（纯函数，便于单测）：图片门槛 + 「全书每章几乎没有正文」。
/// `max_body_chars`：全书单章「非空白字符数 − 目录标题非空白字符数」的最大值，
/// 由 open_book 扫描正文得到（怎么取文本留在 open_book/下面的探针里，这里只做判定）。
/// 带插图的普通小说第一章正文就会让该值远超阈值 → false。
fn is_manga_like(images: usize, spine_n: usize, toc_n: usize, ext: &str, max_body_chars: i64) -> bool {
    manga_image_gate(images, spine_n, toc_n, ext) && max_body_chars <= MANGA_BODY_CHAR_LIMIT
}

/// 单章正文量：去掉所有空白后的字符数，再减去该章目录标题的非空白字符数 —— 标题本来就
/// 出现在章节文档里，不能当作正文证据。
fn body_chars_excluding_title(text: &str, title: &str) -> i64 {
    let body = text.chars().filter(|c| !c.is_whitespace()).count() as i64;
    let t = title.chars().filter(|c| !c.is_whitespace()).count() as i64;
    body - t
}

/// EPUB 正文探针：复用 open_book 已读进内存的 zip 字节（cached.data），不再读盘。
/// 返回 (单章正文最大值, 实际解压字节数)。
fn manga_body_chars_epub(data: &[u8], spine: &[String], toc: &[TocEntry]) -> (i64, usize) {
    // spine 下标 → 目录标题（同一文档被多个目录项指向时取第一个）。
    let mut title_by_spine: Vec<&str> = vec![""; spine.len()];
    for t in toc {
        if t.index < spine.len() && title_by_spine[t.index].is_empty() {
            title_by_spine[t.index] = t.title.as_str();
        }
    }
    let mut max_body = 0i64;
    let used = crate::epub::scan_spine_text(data, spine, MANGA_SCAN_BUDGET, |i, text| {
        let v = body_chars_excluding_title(text, title_by_spine.get(i).copied().unwrap_or(""));
        if v > max_body {
            max_body = v;
        }
        // 出现一页正常正文即可断定「不是漫画」，立刻停（普通小说第一章就触发）。
        v <= MANGA_BODY_CHAR_LIMIT
    });
    (max_body, used)
}

/// MOBI/AZW3/PRC 正文探针：正文已整篇在内存（doc.html + parts），逐章切片求最大值。
fn manga_body_chars_mobi(html: &str, parts: &[(usize, usize)], toc: &[TocEntry]) -> i64 {
    let mut max_body = 0i64;
    for (t, (s, e)) in toc.iter().zip(parts) {
        let raw = safe_slice(html, *s, *e);
        let text = crate::epub::html_to_text(&raw);
        let v = body_chars_excluding_title(&text, &t.title);
        if v > max_body {
            max_body = v;
        }
        if max_body > MANGA_BODY_CHAR_LIMIT {
            break;
        }
    }
    max_body
}

#[tauri::command]
pub async fn open_book(state: State<'_, Arc<AppState>>, id: i64) -> Result<OpenResult, String> {
    // 打开未扫描的书：把该本封面扫描提升为最高优先级（处理完当前这一本后立即执行）。
    on_book_opened(state.inner(), id);
    let book = {
        let conn = state.db.0.lock().unwrap();
        let b = db::get_book(&conn, id).ok_or("书籍不存在")?;
        // 记录“最后打开时间”，让这本书进入「按最近阅读」（即使随后解析失败也能看到）。
        db::touch_book_opened(&conn, id);
        b
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let st = state.inner().clone();
    let t_open = std::time::Instant::now();
    // 换书整体失效已渲染章节缓存，避免跨书串味与无界增长。
    st.chapter_cache.lock().unwrap().clear();
    // spine（内容文档）数量：漫画判定分母。EPUB 分支填入；TXT/MOBI 无 spine 清单保持 0。
    let mut spine_n: usize = 0;
    // 漫画正文探针的原料：仅在「图片门槛通过」后才真正使用，普通书不为它付文本读取成本。
    // EPUB 直接持有 Arc<CachedEpub>（廉价浅拷贝），复用已读入内存的 zip 字节，不再读盘。
    let mut epub_probe: Option<Arc<CachedEpub>> = None;
    let mut mobi_probe: Option<(String, Vec<(usize, usize)>, Vec<TocEntry>)> = None;
    let (toc, docs, warn) = if ext == "epub" {
        // 解析 7000+ 章的大书可达数秒：放到阻塞线程池，避免占用 async 运行时
        // 导致其它 IPC（预取、进度保存等）排队卡死。
        let st2 = st.clone();
        let bpath = book.path.clone();
        let cached = tauri::async_runtime::spawn_blocking(move || load_epub(&st2, &bpath, id))
            .await
            .map_err(|e| e.to_string())??;
        spine_n = cached.doc.spine.len();
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
        epub_probe = Some(cached.clone());
        (cached.doc.toc.clone(), docs, None)
    } else if ext == "txt" {
        // TXT 整读+正则切分同样可能耗时：与 EPUB 一样放阻塞线程池。
        let st2 = st.clone();
        let bpath = book.path.clone();
        let cached = tauri::async_runtime::spawn_blocking(move || load_txt(&st2, &bpath, id))
            .await
            .map_err(|e| e.to_string())??;
        (cached.2.clone(), Vec::new(), None)
    } else {
        // MOBI/AZW3 解压 + 切分：放阻塞线程池。
        let st2 = st.clone();
        let bpath = book.path.clone();
        let doc = tauri::async_runtime::spawn_blocking(move || mobi_doc_for(&st2, &bpath, id))
            .await
            .map_err(|e| e.to_string())??;
        // 拆包取所有权，避免再 clone 一份可能很大的 html（push_cache 本来就要一份）。
        let crate::mobi::MobiDoc { html, toc, parts, warning, .. } = doc;
        let ver = st.rules_ver.load(Ordering::Relaxed);
        let stamp = file_stamp(&book.path);
        if stamp.is_some() {
            push_cache(
                &st.txt_cache,
                id,
                CachedText {
                    ver,
                    stamp,
                    doc: Arc::new((html.clone(), parts.clone(), toc.clone())),
                },
                3,
            );
        }
        mobi_probe = Some((html, parts, toc.clone()));
        (toc, Vec::new(), warning)
    };
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[open] open_book id={id} ext={ext} total={:?} toc={}",
            t_open.elapsed(),
            toc.len()
        );
    }
    // 全书图片列表：随 open_book 一次算好并缓存，图片查看器无需再发命令（避免打开后
    // 再调用命令带来的排队/卡顿）。EPUB 用 OPF 清单（瞬时），MOBI 用已提取的 res 目录。
    let images = collect_book_images(&st, id).unwrap_or_default();
    st.image_cache.lock().unwrap().insert(id, Arc::new(images.clone()));
    // 漫画检测分两步，先廉价后昂贵：
    //  1) 图片门槛：图片数远多于章节数（每页基本一张图）。不过门槛直接 false，**不读正文**，
    //     带少量插图的普通书在这里就以零成本被排除（这是成本控制的关键）。
    //  2) 正文检查：门槛通过后逐章取纯文本，只有「每章除标题外几乎没有别的文字」才判漫画，
    //     否则普通小说（正文很长）立刻 false —— 旧实现完全不看正文，故带插图的正常小说会误判。
    let toc_n = toc.len().max(1);
    let mut body_chars = 0i64;
    let mut scanned_bytes = 0usize;
    let mut is_manga = false;
    if manga_image_gate(images.len(), spine_n, toc_n, &ext) {
        if ext == "epub" {
            if let Some(c) = &epub_probe {
                let (b, used) = manga_body_chars_epub(&c.data, &c.doc.spine, &c.doc.toc);
                body_chars = b;
                scanned_bytes = used;
            }
        } else if let Some((html, parts, t)) = &mobi_probe {
            body_chars = manga_body_chars_mobi(html, parts, t);
        }
        is_manga = is_manga_like(images.len(), spine_n, toc_n, &ext, body_chars);
    }
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[open] is_manga id={id} images={} spine={} toc={} body_chars={} scanned={} => {is_manga}",
            images.len(),
            spine_n,
            toc.len(),
            body_chars,
            scanned_bytes
        );
    }
    Ok(OpenResult {
        book,
        toc,
        docs,
        encoding_warning: warn,
        images,
        is_manga,
    })
}

#[tauri::command]
pub async fn get_chapter(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    chapter_index: i64,
    keep_styles: bool,
) -> Result<ChapterData, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let idx = chapter_index.max(0) as usize;
    let st = state.inner().clone();

    // 已渲染章节缓存：命中直接返回（零解析成本，快速连续翻页/来回翻页瞬时）。
    if let Some(hit) = st
        .chapter_cache
        .lock()
        .unwrap()
        .get((book_id, idx as i64, keep_styles))
    {
        return Ok(hit.as_ref().clone());
    }

    if ext == "epub" {
        // 解析/渲染放到阻塞线程池，避免占用 async 运行时拖死其它 IPC。
        let st2 = st.clone();
        let bpath = book.path.clone();
        let t_render = std::time::Instant::now();
        let data = tauri::async_runtime::spawn_blocking(move || -> Result<ChapterData, String> {
            let cached = load_epub(&st2, &bpath, book_id)?;
            let doc = &cached.doc;
            // 章节 = 从本章目录项指向的 spine 下标，到下一章目录项（或末尾）为止：
            // 支持「一章被拆成多个小文件」的 EPUB（目录只指向首个小文件）。
            let start = doc
                .toc
                .get(idx)
                .map(|t| t.index)
                .unwrap_or(idx)
                .min(doc.spine.len());
            let end = doc
                .toc_end
                .get(idx)
                .copied()
                .unwrap_or(start + 1)
                .max(start + 1)
                .min(doc.spine.len());
            // 复用缓存的 CiMap 与按需图片 URL（不内联 base64），连续渲染整段章节。
            let docs: Vec<String> = (start..end)
                .filter_map(|si| doc.spine.get(si).cloned())
                .collect();
            let mut html = crate::epub::chapter_html_multi_ci(
                &cached.data,
                Some(&cached.ci),
                &docs,
                Some(book_id),
                keep_styles,
            )
            .unwrap_or_default();
            let mut chars: i64 = (start..end)
                .map(|si| doc.spine_chars.get(si).copied().unwrap_or(0))
                .sum();
            if html.is_empty() {
                // 区间异常时兜底返回本章首文件，避免整章空白。
                if let Some(dp) = doc.spine.get(start) {
                    html = crate::epub::chapter_html_ci(
                        &cached.data,
                        Some(&cached.ci),
                        dp,
                        Some(book_id),
                        keep_styles,
                    )?;
                    chars = doc.spine_chars.get(start).copied().unwrap_or(0);
                }
            }
            let title = doc
                .toc
                .get(idx)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| format!("第 {} 节", idx + 1));
            Ok(ChapterData { title, html, chars })
        })
        .await
        .map_err(|e| e.to_string())??;
        if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
            eprintln!(
                "[open] get_chapter book={book_id} idx={idx} render={:?} html={}B",
                t_render.elapsed(),
                data.html.len()
            );
        }
        st.chapter_cache
            .lock()
            .unwrap()
            .put((book_id, idx as i64, keep_styles), Arc::new(data.clone()));
        return Ok(data);
    }

    let is_mobi = is_mobi_ext(&ext);
    let ver = st.rules_ver.load(Ordering::Relaxed);
    // 与 load_txt 同一套判据：规则版本 + 文件指纹都对上才复用。
    let stamp = file_stamp(&book.path);
    let cached_hit = {
        let m = st.txt_cache.lock().unwrap();
        m.get(&book_id)
            .filter(|c| c.ver == ver && stamp_matches(c.stamp, stamp))
            .map(|c| c.doc.clone())
    };
    let (content, ranges, toc) = match cached_hit {
        Some(c) => c.as_ref().clone(),
        None => {
            // 缓存未命中才解析：TXT/MOBI 的解析放阻塞线程池，避免占用 async 运行时。
            let st2 = st.clone();
            let bpath = book.path.clone();
            let entry = tauri::async_runtime::spawn_blocking(
                move || -> Result<Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>, String> {
                    if is_mobi {
                        let doc = mobi_doc_for(&st2, &bpath, book_id)?;
                        Ok(Arc::new((doc.html.clone(), doc.parts.clone(), doc.toc.clone())))
                    } else {
                        load_txt(&st2, &bpath, book_id)
                    }
                },
            )
            .await
            .map_err(|e| e.to_string())??;
            // 指纹取不到就不入缓存：无法校验的条目会一直命中。
            if stamp.is_some() {
                push_cache(
                    &st.txt_cache,
                    book_id,
                    CachedText { ver, stamp, doc: entry.clone() },
                    3,
                );
            }
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
    let data = ChapterData { title, html, chars };
    st.chapter_cache
        .lock()
        .unwrap()
        .put((book_id, idx as i64, keep_styles), Arc::new(data.clone()));
    Ok(data)
}

#[tauri::command]
pub fn save_progress(state: State<'_, Arc<AppState>>, book_id: i64, p: Progress) -> Result<(), String> {
    let mut p = p;
    p.updated_at = db::now_ms();
    p.device = "tidepage-pc".into();
    let conn = state.db.0.lock().unwrap();
    db::set_progress(&conn, book_id, &p)?;
    Ok(())
}

#[tauri::command]
pub fn load_progress(state: State<'_, Arc<AppState>>, book_id: i64) -> Option<Progress> {
    let conn = state.db.0.lock().unwrap();
    db::get_progress(&conn, book_id)
}

/// 删除书籍。`fromDisk`=删除本地文件；`keepNotes`=保留带笔记的划线（默认 true）。
/// 前端需先调用 `book_note_info` 判断是否有笔记并弹窗让用户选择。
#[tauri::command]
pub async fn delete_books(
    state: State<'_, Arc<AppState>>,
    paths: Vec<String>,
    from_disk: bool,
    keep_notes: Option<bool>,
) -> Result<(), String> {
    let keep = keep_notes.unwrap_or(true);
    let st = state.inner().clone();
    let app = state.app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        // 与 remove_folder 对齐：分批(2000/批) + 每批独立事务，消除「每本 7~8 条 DELETE
        // 各自自动提交 = 8N 次 fsync」的大批量删除慢。批间释放 db 锁，避免长持锁冻结其它读。
        // 本闭包已在 spawn_blocking 的阻塞线程池内（非主线程），分批即可，无需 async yield。
        for chunk in paths.chunks(2000) {
            let mut guard = st.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            db::delete_books_policy(&tx, chunk, keep)?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        if from_disk {
            for p in &paths {
                trash::delete(p).map_err(|e| format!("删除失败: {e:?}"))?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    app.emit("library-changed", ()).ok();
    app.emit("notes-changed", ()).ok();
    Ok(())
}

/// 查该书是否有带笔记的划线（供前端删除前弹窗判定）。
#[tauri::command]
pub fn book_note_info(state: State<'_, Arc<AppState>>, book_id: i64) -> serde_json::Value {
    let conn = state.db.0.lock().unwrap();
    let has = db::book_has_notes(&conn, book_id);
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND note!=''",
            rusqlite::params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    serde_json::json!({ "hasNotes": has, "noteCount": n })
}

/// 笔记界面：按书分组列出「有划线或添加过感想」的书（有划线即入列）。
/// noteCount = 其中带感想的条数；hlCount = 划线条数（含无感想）。
#[tauri::command]
pub fn list_note_books(state: State<'_, Arc<AppState>>) -> Vec<serde_json::Value> {
    let conn = state.db.0.lock().unwrap();
    db::list_note_books(&conn)
        .into_iter()
        .map(|(id, title, author, deleted, hl, notes, last, cover, cover_ver)| {
            serde_json::json!({
                "bookId": id, "title": title, "author": author,
                "deleted": deleted, "hlCount": hl, "noteCount": notes, "lastAt": last,
                "coverPath": cover, "coverVer": cover_ver
            })
        })
        .collect()
}

/// 某书的笔记明细。
#[tauri::command]
pub fn list_notes_of_book(state: State<'_, Arc<AppState>>, book_id: i64) -> Vec<serde_json::Value> {
    let conn = state.db.0.lock().unwrap();
    db::list_notes_of_book(&conn, book_id)
        .into_iter()
        .map(|(id, ch, ct, text, note, at)| {
            serde_json::json!({
                "id": id, "chapterIndex": ch, "chapterTitle": ct,
                "text": text, "note": note, "createdAt": at
            })
        })
        .collect()
}

/// 设置某条划线的感想（note）。
#[tauri::command]
pub fn set_highlight_note(state: State<'_, Arc<AppState>>, id: i64, note: String) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::set_highlight_note(&conn, id, &note)?;
    drop(conn);
    state.app.emit("notes-changed", ()).ok();
    Ok(())
}

/// 追加感想（AI 添加到笔记：不覆盖已有）。
#[tauri::command]
pub fn append_highlight_note(
    state: State<'_, Arc<AppState>>,
    id: i64,
    addition: String,
) -> Result<String, String> {
    let conn = state.db.0.lock().unwrap();
    let next = db::append_highlight_note(&conn, id, &addition)?;
    drop(conn);
    state.app.emit("notes-changed", ()).ok();
    Ok(next)
}

#[tauri::command]
pub async fn preview_split(path: String, pattern: String) -> Result<Vec<TocEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let pattern = if pattern.trim().is_empty() { crate::default_pattern().to_string() } else { pattern };
        let doc = txt::decode_and_split(&path, Some(&pattern), None)?;
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
    state.chapter_cache.lock().unwrap().clear();
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
    let dir = crate::data_dir(&st.app)
        .join("res")
        .join(book_id.to_string());
    let prefix = format!("http://tidepage.localhost/res/{book_id}/");
    let doc = crate::mobi::parse_full(path, force.as_deref(), Some((&dir, &prefix)))?;
    if std::env::var("TIDEPAGE_MOBI_DIAG").is_ok() {
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
    // 编码变化影响 TXT/MOBI 的解码结果:清掉已缓存的解析与已渲染章节,令重开书籍即生效
    // (与 save_split_rules 一致;否则 chapter_cache 不按 rules_ver 失效会返回旧编码的旧渲染)。
    state.txt_cache.lock().unwrap().clear();
    state.chapter_cache.lock().unwrap().clear();
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
        let is_mobi_like = is_mobi_ext(&ext);
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
    {
        let conn = state.db.0.lock().unwrap();
        conn.execute(
            "UPDATE books SET cover_path=?2, cover_ver=cover_ver+1 WHERE id=?1",
            rusqlite::params![book_id, rel],
        )
        .map_err(|e| e.to_string())?;
    }
    // 该本封面已变：通知前端刷新，per-book URL 因 cover_ver+1 而失效重取。
    state.app.emit("book-updated", ()).ok();
    Ok(())
}

/// 默认底图变更后的统一收尾（F2 修正版）。
///
/// 换底图 / 恢复内置【只影响共享底图那 1 份文件】：
/// - 绝不删除 `covers/` 目录里 per-book 的封面（有内嵌封面的书不受影响）；
/// - 绝不重置任何书的 `cover_path`/`meta_done`（不回落默认底图）；
/// - 不清理 covers 目录、不重启元数据 worker。
/// 只需版本号 +1（令共享默认封面缓存与前端 URL 失效）并通知前端刷新书库即可。
fn invalidate_covers_after_base_change(state: &Arc<AppState>) {
    state.cover_style_ver.fetch_add(1, Ordering::SeqCst);
    state.app.emit("library-changed", ()).ok();
}

/// 默认底图原子写入：校验可解码 → 写同目录临时文件 → 原子替换。
/// 任何失败都清理临时文件、且**不触碰**已存在的 out（失败不残留半步）。
fn write_default_cover_atomic(out: &Path, data: &[u8]) -> Result<(), String> {
    image::load_from_memory(data).map_err(|e| format!("不是有效图片: {e}"))?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("img.tmp");
    if let Err(e) = std::fs::write(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("写入底图失败: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, out) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换底图失败: {e}"));
    }
    Ok(())
}

/// 设置默认封面底图（校验 + 缩放到封面尺寸 + 分析书名区亮度 + 原子替换）。
///
/// 关键：把用户底图**下采样到 320×480 PNG** 再落盘——
///   1) 修复「设置大图后书库封面全部空白（需重启）」：默认封面是把底图整张 base64 内嵌进
///      每本书的 SVG，大图会让每张封面 SVG 体积暴涨（几 MB），可见网格一次并发几十张时
///      webview 内存/解码顶不住而渲染空白；缩到 320×480 后单张 SVG 仅几十 KB，秒开不空白。
///   2) 同时分析底图「书名显示区域」（底部 42%）平均亮度，自动决定叠字用深色还是浅色，
///      结果存入 settings（dc_text / dc_text_light），渲染时直接取用（无需每次重解码大图）。
#[tauri::command]
pub fn set_default_cover(state: State<'_, Arc<AppState>>, image_path: String) -> Result<(), String> {
    let data = std::fs::read(&image_path).map_err(|e| format!("读取图片失败: {e}"))?;
    let img = image::load_from_memory(&data).map_err(|e| format!("不是有效图片: {e}"))?;
    // 下采样到封面比例（2:3 居中裁切填满），再编码为 PNG。
    let cover = img.resize_to_fill(320, 480, image::imageops::FilterType::Lanczos3);
    let rgba = cover.to_rgba8();
    let mut png: Vec<u8> = Vec::new();
    cover
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("编码封面失败: {e}"))?;

    let out = covers::default_cover_path(&state.app);
    write_default_cover_atomic(&out, &png)?;

    // 分析书名区亮度 → 自适应字色（偏亮配柔和深色、偏暗配柔和浅色；非纯黑纯白、护眼）。
    let region_light = covers::region_is_light(&rgba);
    let (text, is_light) = if region_light {
        ("#2A2723", false)
    } else {
        ("#EFEBE4", true)
    };
    {
        let conn = state.db.0.lock().unwrap();
        db::set_setting(&conn, "dc_text", text);
        db::set_setting(&conn, "dc_text_light", if is_light { "1" } else { "0" });
    }
    invalidate_covers_after_base_change(state.inner());
    Ok(())
}

/// 清除默认封面底图（恢复内置渐变），并让缓存失效重算。
#[tauri::command]
pub fn clear_default_cover(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let out = covers::default_cover_path(&state.app);
    if out.exists() {
        std::fs::remove_file(&out).map_err(|e| format!("移除底图失败: {e}"))?;
    }
    {
        // 清掉自适应字色记录：回到主题渐变后由主题字色接管。
        let conn = state.db.0.lock().unwrap();
        db::set_setting(&conn, "dc_text", "");
        db::set_setting(&conn, "dc_text_light", "");
    }
    invalidate_covers_after_base_change(state.inner());
    Ok(())
}

/// 默认封面底图是否存在（供设置页显示状态）。
#[tauri::command]
pub fn has_default_cover(state: State<'_, Arc<AppState>>) -> bool {
    covers::default_cover_path(&state.app).is_file()
}

#[tauri::command]
pub async fn webdav_test(base: String, user: String, pass: String) -> Result<bool, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.test().await
}

/// 上传设置备份到云端 `TidePage/settings/{name}`（data 为前端组装的 JSON）。
/// name 必须是单一文件名（非空、不含 `/`、`\`、`..`）；远端只写 TidePage/settings/ 下文件。
/// 返回写入的远端完整路径。
#[tauri::command]
pub async fn webdav_backup_settings(
    base: String,
    user: String,
    pass: String,
    name: String,
    data: String,
) -> Result<String, String> {
    if !valid_backup_name(&name) {
        return Err("备份文件名无效".into());
    }
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.ensure_dir("TidePage").await?;
    c.ensure_dir("TidePage/settings").await?;
    let path = format!("TidePage/settings/{name}");
    c.put(&path, data.into_bytes()).await?;
    Ok(path)
}

/// 读取云端指定设置备份；文件不存在（404）返回 None。
#[tauri::command]
pub async fn webdav_restore_settings(
    base: String,
    user: String,
    pass: String,
    name: String,
) -> Result<Option<String>, String> {
    if !valid_backup_name(&name) {
        return Err("备份文件名无效".into());
    }
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    match c.get(&format!("TidePage/settings/{name}")).await? {
        Some(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "云端设置备份不是合法文本".to_string()),
        None => Ok(None),
    }
}

/// 备份文件名合法性：非空、单一文件名、不含路径穿越。
fn valid_backup_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
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
    c.ensure_dir("TidePage").await.ok();
    // 确保自有同步进度子目录存在（严格服务器对未建父目录的深路径 PUT 会 409）。
    c.ensure_dir("TidePage/progress").await.ok();
    let remote_key = format!("TidePage/progress/{}.json", db::path_id(&book_path));
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

/// 只读：从云端读取本书进度（TidePage/progress/{path_id}.json），仅供「同步前对比」使用。
/// **绝不写库、绝不 PUT、绝不 set_progress**；文件不存在（404）或解析失败一律返回 None。
#[tauri::command]
pub async fn webdav_read_progress(
    base: String,
    user: String,
    pass: String,
    book_path: String,
) -> Result<Option<Progress>, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    let remote_key = format!("TidePage/progress/{}.json", db::path_id(&book_path));
    // get()：404 → Ok(None)，传输错误 → Err；两种失败都按「云端无进度」处理。
    Ok(match c.get(&remote_key).await {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    })
}

#[tauri::command]
pub async fn webdav_upload_book(
    base: String,
    user: String,
    pass: String,
    book_path: String,
) -> Result<String, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.ensure_dir("TidePage").await?;
    c.ensure_dir("TidePage/books").await?;
    let name = Path::new(&book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or("路径无效")?;
    let data = std::fs::read(&book_path).map_err(|e| e.to_string())?;
    c.put(&format!("TidePage/books/{name}"), data).await?;
    Ok(format!("TidePage/books/{name}"))
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
    let items = c.list("TidePage/books").await?;
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
        if let Some(data) = c.get(&format!("TidePage/books/{name}")).await? {
            std::fs::write(&out, &data).map_err(|e| e.to_string())?;
            imported.push(out.to_string_lossy().into_owned());
        }
    }
    if !imported.is_empty() {
        scan_folders(&state, None).await;
    }
    Ok(imported)
}

/// 单本书从云端下载到本地：远端键为 TidePage/books/{file_name}，复用 webdav_download_file
/// 的「下载 → 按扩展名 upsert_book → emit library-changed」逻辑。返回落地路径。
#[tauri::command]
pub async fn webdav_download_book(
    state: State<'_, Arc<AppState>>,
    base: String,
    user: String,
    pass: String,
    book_path: String,
    target_dir: String,
) -> Result<String, String> {
    let name = Path::new(&book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or("路径无效")?;
    let remote_path = format!("TidePage/books/{name}");
    let mut files = webdav_download_file(state, base, user, pass, remote_path, target_dir).await?;
    Ok(if files.is_empty() { String::new() } else { files.remove(0) })
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

/// 删除云端文件/目录（DELETE）。
#[tauri::command]
pub async fn webdav_delete(
    base: String,
    user: String,
    pass: String,
    path: String,
) -> Result<(), String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.delete(&path).await
}

/// 复制云端文件/目录（COPY，Overwrite: F 不覆盖同名项）。
#[tauri::command]
pub async fn webdav_copy(
    base: String,
    user: String,
    pass: String,
    src: String,
    dst: String,
) -> Result<(), String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.copy(&src, &dst).await
}

/// 移动/重命名云端文件/目录（MOVE，Overwrite: F）。
#[tauri::command]
pub async fn webdav_move(
    base: String,
    user: String,
    pass: String,
    src: String,
    dst: String,
) -> Result<(), String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass);
    c.move_path(&src, &dst).await
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
    crate::data_dir(&state.app).to_string_lossy().into_owned()
}

/// 递归统计目录大小（字节）与文件数。目录不存在返回 (0, 0)。
fn dir_size(dir: &Path) -> (u64, u64) {
    let mut bytes: u64 = 0;
    let mut files: u64 = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(p),
                Ok(ft) if ft.is_file() => {
                    files += 1;
                    if let Ok(m) = e.metadata() {
                        bytes += m.len();
                    }
                }
                _ => {}
            }
        }
    }
    (bytes, files)
}

/// 存储占用：数据目录总量 + 各子目录（封面/提取资源/字体/数据库）。
#[derive(serde::Serialize)]
pub struct StorageStats {
    pub data_dir: String,
    pub total_bytes: u64,
    pub covers_bytes: u64,
    pub res_bytes: u64,
    pub fonts_bytes: u64,
    pub db_bytes: u64,
    /// 「可清理缓存」= 封面 + 提取资源（可再生，删除后重新打开/刷新会重建）。
    pub cache_bytes: u64,
}

#[tauri::command]
pub fn storage_stats(state: State<'_, Arc<AppState>>) -> StorageStats {
    let dir = crate::data_dir(&state.app);
    let (covers_bytes, _) = dir_size(&dir.join("covers"));
    let (res_bytes, _) = dir_size(&dir.join("res"));
    let (fonts_bytes, _) = dir_size(&dir.join("fonts"));
    let (total_bytes, _) = dir_size(&dir);
    let db_bytes = ["tidepage.db", "tidepage.db-wal", "tidepage.db-shm"]
        .iter()
        .map(|n| std::fs::metadata(dir.join(n)).map(|m| m.len()).unwrap_or(0))
        .sum();
    StorageStats {
        data_dir: dir.to_string_lossy().into_owned(),
        total_bytes,
        covers_bytes,
        res_bytes,
        fonts_bytes,
        db_bytes,
        cache_bytes: covers_bytes + res_bytes,
    }
}

/// 诊断：worker 启动次数 / 是否在跑 / 待处理数（仅用于验证；非手动补齐入口）。
#[tauri::command]
pub fn meta_debug_stats(state: State<'_, Arc<AppState>>) -> serde_json::Value {
    let (pending, priority) = {
        let conn = state.db.0.lock().unwrap();
        let pending = db::count_meta_pending(&conn);
        let priority: i64 = conn
            .query_row("SELECT COUNT(*) FROM cover_priority", [], |r| r.get(0))
            .unwrap_or(0);
        (pending, priority)
    };
    serde_json::json!({
        "startCount": state.scan_start_count.load(Ordering::SeqCst),
        "running": state.scan_running.load(Ordering::SeqCst),
        "pending": pending,
        "priority": priority,
    })
}

/// 当前封面扫描进度快照（done/total/pending）。
/// 供前端**挂载时主动拉取**以初始化底栏进度条：启动时那条 `meta-progress done=0` 是在
/// Rust setup 阶段发出的，常早于 webview 挂载、事件无从接收；仅靠事件会导致底栏空着。
#[tauri::command]
pub fn scan_progress(state: State<'_, Arc<AppState>>) -> serde_json::Value {
    let pending = {
        let conn = state.db.0.lock().unwrap();
        db::count_meta_pending(&conn)
    };
    let done = state.cover_scan_done.load(Ordering::SeqCst);
    let total = state.cover_scan_total.load(Ordering::SeqCst).max(done);
    serde_json::json!({
        "running": state.scan_running.load(Ordering::SeqCst),
        "done": done,
        "total": total,
        "pending": pending,
    })
}

/// 方案 A：把「列表可见的未扫描书籍」提升为高优先级扫描目标（按用户视野顺序生成封面）。
/// - 单事务批量入队，复用 `enqueue_priority`（ON CONFLICT 刷新 queued_at → 幂等，重复滚动不膨胀）；
/// - 逐本校验 `meta_done != 1` 才入队（已完成的不入队）；
/// - 一次最多处理 1000 个 id；
/// - 结束后 `ensure_scan_worker`（worker 空闲则拉起，`next_priority` 按 queued_at DESC 取最近关注项）。
#[tauri::command]
pub fn prioritize_covers(state: State<'_, Arc<AppState>>, ids: Vec<i64>) -> Result<u64, String> {
    let ids: Vec<i64> = ids.into_iter().take(1000).collect();
    if ids.is_empty() {
        return Ok(0);
    }
    let enqueued = {
        // 最外层锁不 panic：Mutex 中毒也返回 Err 给前端。
        let mut guard = state
            .db
            .0
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        // 事务开启失败 → Err（不吞错）。
        let tx = guard.transaction().map_err(|e| e.to_string())?;
        let mut n = 0u64;
        for id in &ids {
            // 仅对仍需处理的书入队（meta_done != 1；不存在的书跳过，避免孤儿优先项）。
            if matches!(db::meta_done_of(&tx, *id), Some(md) if md != 1) {
                match db::enqueue_priority(&tx, *id) {
                    Ok(()) => n += 1,
                    Err(e) => eprintln!("[scan] enqueue_priority failed id={id}: {e}"),
                }
            }
        }
        // commit 失败 → Err，且下面不打印成功日志、不拉起 worker。
        tx.commit().map_err(|e| e.to_string())?;
        n
    };
    if enqueued > 0 {
        eprintln!("[scan] prioritize_covers enqueued {enqueued}/{} visible ids", ids.len());
        ensure_scan_worker(&state.app);
    }
    Ok(enqueued)
}


/// 清理可再生的缓存目录。`what`: "covers" | "res" | "all"。
/// 清理后把受影响书的 cover_path/meta_done 复位，让下次刷新/打开重新生成。
#[tauri::command]
pub fn clear_cache(state: State<'_, Arc<AppState>>, what: String) -> Result<u64, String> {
    let dir = crate::data_dir(&state.app);
    let targets: Vec<&str> = match what.as_str() {
        "covers" => vec!["covers"],
        "res" => vec!["res"],
        "all" => vec!["covers", "res"],
        other => return Err(format!("未知缓存类型: {other}")),
    };
    // 含封面清理时：先停 worker 并等其退出，避免删文件与写封面并发（半成品残留）。
    if targets.contains(&"covers") {
        stop_cover_worker_and_wait(state.inner());
    }
    let mut freed: u64 = 0;
    for name in &targets {
        let p = dir.join(name);
        let (b, _) = dir_size(&p);
        if p.exists() {
            std::fs::remove_dir_all(&p).map_err(|e| format!("清理 {name} 失败: {e}"))?;
        }
        std::fs::create_dir_all(&p).ok();
        freed += b;
    }
    // 清理封面后，把元数据标记复位，下次扫描/打开会重建封面。
    if targets.contains(&"covers") {
        // 换底图/清缓存让旧封面缓存整体失效（F5）：规则版本 +1 参与缓存 key。
        state.cover_style_ver.fetch_add(1, Ordering::SeqCst);
        {
            let conn = state.db.0.lock().unwrap();
            conn.execute("UPDATE books SET cover_path=NULL, meta_done=0", [])
                .map_err(|e| e.to_string())?;
        }
        // worker 已停止：重新拉起增量扫描（全局互斥，若已在跑则 no-op）。
        ensure_scan_worker(&state.app);
    }
    if targets.contains(&"res") {
        // 提取资源清空后，已缓存的章节/图片需要失效。
        state.chapter_cache.lock().unwrap().clear();
        state.image_cache.lock().unwrap().clear();
    }
    state.app.emit("library-changed", ()).ok();
    Ok(freed)
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
    proxy_type: Option<String>,
    deployment: Option<String>,
    api_version: Option<String>,
) -> Result<(), crate::ai::AiError> {
    let proxy_mode = proxy_mode.unwrap_or_else(|| "system".into());
    let cfg = crate::ai::AiConfig {
        provider,
        base_url,
        api_key,
        model,
        deployment: deployment.unwrap_or_default(),
        api_version: api_version.unwrap_or_default(),
    };
    let http = crate::ai::build_client(
        &proxy_mode,
        proxy_url.as_deref().unwrap_or(""),
        proxy_type.as_deref(),
        &cfg.base_url,
    )?;
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
    proxy_type: Option<String>,
    deployment: Option<String>,
    api_version: Option<String>,
) -> crate::ai::AiTestResult {
    crate::ai::test_connection(
        provider,
        base_url,
        api_key,
        model,
        deployment.unwrap_or_default(),
        api_version.unwrap_or_default(),
        proxy_mode.unwrap_or_else(|| "system".into()),
        proxy_url.unwrap_or_default(),
        proxy_type.unwrap_or_default(),
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn tts_synth(
    provider: String,
    base_url: String,
    api_key: String,
    secret_key: Option<String>,
    app_id: Option<String>,
    cluster: Option<String>,
    model: String,
    voice: String,
    format: String,
    text: String,
    lang: Option<String>,
    proxy_mode: Option<String>,
    proxy_url: Option<String>,
    proxy_type: Option<String>,
) -> Result<String, crate::ai::AiError> {
    let params = crate::ai::TtsParams {
        provider: &provider,
        base_url: &base_url,
        api_key: &api_key,
        secret_key: secret_key.as_deref().unwrap_or(""),
        app_id: app_id.as_deref().unwrap_or(""),
        cluster: cluster.as_deref().unwrap_or(""),
        model: &model,
        voice: &voice,
        format: &format,
        lang: lang.as_deref().unwrap_or(""),
    };
    let proxy_mode = proxy_mode.as_deref().unwrap_or("system");
    let proxy_url = proxy_url.as_deref().unwrap_or("");
    let proxy_type = proxy_type.as_deref();
    let http = crate::ai::build_client(proxy_mode, proxy_url, proxy_type, &base_url)?;
    let bytes = crate::ai::synthesize_tts(http, proxy_mode, proxy_url, proxy_type, &params, &text).await?;
    use base64::Engine as _;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 自定义 TTS 服务返回的单个音色。
/// `kind` 的取值：`profile`=服务端保存的固定克隆音色（推荐，整章音色一致）；
/// `openai_alias`=OpenAI 别名（映射到服务端默认音色，对克隆引擎可能逐句随机）；
/// 其它/空=服务端自定义预设。
#[derive(serde::Serialize)]
pub struct TtsVoiceInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub language: String,
}

/// 读取 TTS 服务提供的音色列表，按用户显式选择的协议分派：
///  - openai：GET {base}/v1/audio/voices（兼容 /v1/voices）；
///  - breeze：GET {base}/api/voices；
///  - edge：官方音色清单接口；
///  - 其余协议不支持音色列表，返回空数组（由前端提示手填）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn tts_voices(
    provider: String,
    base_url: String,
    api_key: String,
    secret_key: Option<String>,
    app_id: Option<String>,
    cluster: Option<String>,
    proxy_mode: Option<String>,
    proxy_url: Option<String>,
    proxy_type: Option<String>,
) -> Result<Vec<TtsVoiceInfo>, String> {
    let _ = (&secret_key, &app_id, &cluster);
    let provider = provider.trim().to_ascii_lowercase();
    let base = base_url.trim().trim_end_matches('/');
    let proxy_mode = proxy_mode.as_deref().unwrap_or("system");
    let proxy_url = proxy_url.as_deref().unwrap_or("");
    let proxy_type = proxy_type.as_deref();
    if crate::ai::is_breeze_provider(&provider) {
        if base.is_empty() {
            return Err("未填写服务地址".into());
        }
        // breeze 是本地/局域网网关，固定直连（见 breeze_client），不吃全局代理。
        return list_breeze_voices(base, &api_key).await;
    }
    match provider.as_str() {
        "edge" => list_edge_voices(proxy_mode, proxy_url, proxy_type).await,
        // 缺省与 openai 一致；VoiceStudio 亦为 OpenAI 兼容；未知协议也按 OpenAI 兼容尝试。
        "" | "openai" | "voicestudio" => {
            if base.is_empty() {
                return Err("未填写服务地址".into());
            }
            list_openai_voices(base, &api_key, proxy_mode, proxy_url, proxy_type).await
        }
        _ => Ok(vec![]),
    }
}

/// OpenAI 兼容音色列表：GET {base}/v1/audio/voices（兼容 /v1/voices）。
async fn list_openai_voices(
    base: &str,
    api_key: &str,
    proxy_mode: &str,
    proxy_url: &str,
    proxy_type: Option<&str>,
) -> Result<Vec<TtsVoiceInfo>, String> {
    let root = base.strip_suffix("/v1").unwrap_or(base);
    // 兼容常见写法：VoiceStudio 用 /v1/audio/voices，其它服务用 /v1/voices
    let candidates = [
        format!("{root}/v1/audio/voices"),
        format!("{root}/v1/voices"),
    ];
    // 与 tts_synth 用同一份全局代理：否则用户选「直连/手动代理」时会出现
    // 「合成能出声、音色列表拉不到」的自相矛盾。
    let client = crate::ai::build_client(proxy_mode, proxy_url, proxy_type, base).map_err(|e| e.message)?;
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
        let mut out: Vec<TtsVoiceInfo> = Vec::new();
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
                let Some(id) = id else { continue };
                let id = id.trim();
                if id.is_empty() || out.iter().any(|x| x.id == id) {
                    continue;
                }
                let name = it
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or(id)
                    .to_string();
                let kind = it
                    .get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let language = it
                    .get("language")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(TtsVoiceInfo {
                    id: id.to_string(),
                    name,
                    kind,
                    language,
                });
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
        last_err = "服务未返回音色列表".into();
    }
    Err(last_err)
}

/// Edge TTS 官方音色清单（无需密钥）：取 ShortName 作 id/name、Locale 作语言。
async fn list_edge_voices(
    proxy_mode: &str,
    proxy_url: &str,
    proxy_type: Option<&str>,
) -> Result<Vec<TtsVoiceInfo>, String> {
    const URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list?trustedclienttoken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";
    // 固定公网端点：同样跟随全局代理设置，不另起一套语义。
    let client = crate::ai::build_client(proxy_mode, proxy_url, proxy_type, URL).map_err(|e| e.message)?;
    let resp = client.get(URL).send().await.map_err(|e| format!("请求失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let arr: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    let mut out: Vec<TtsVoiceInfo> = Vec::new();
    for it in &arr {
        let name = it
            .get("ShortName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() || out.iter().any(|x| x.id == name) {
            continue;
        }
        let language = it
            .get("Locale")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        out.push(TtsVoiceInfo {
            id: name.to_string(),
            name: name.to_string(),
            kind: String::new(),
            language,
        });
    }
    if out.is_empty() {
        return Err("服务未返回音色列表".into());
    }
    Ok(out)
}

/// 读取 Breeze-TTS-2 本地网关中保存的克隆音色。网关返回
/// `{"voices":[{"id","name","reference_text",...}]}`，全部视为固定音色。
/// 依次尝试候选网关根地址（配置地址 + 7863 回退，修正误填上游 8080 端口）；
/// 本机 / 局域网服务一律直连（绕过系统代理）。
async fn list_breeze_voices(base: &str, api_key: &str) -> Result<Vec<TtsVoiceInfo>, String> {
    let client = crate::ai::breeze_client(base).map_err(|e| e.message)?;
    let candidates = crate::ai::breeze_root_candidates(base);
    let mut last_err = String::from("服务未返回音色列表");
    for root in &candidates {
        let voices_url = format!("{root}/api/voices");
        let mut req = client.get(&voices_url);
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
        let empty: Vec<serde_json::Value> = Vec::new();
        let arr = v
            .get("voices")
            .and_then(|x| x.as_array())
            .or_else(|| v.get("data").and_then(|x| x.as_array()))
            .unwrap_or(&empty);
        let mut out: Vec<TtsVoiceInfo> = Vec::new();
        for it in arr {
            let id = it.get("id").and_then(|x| x.as_str()).unwrap_or("").trim();
            if id.is_empty() || out.iter().any(|x| x.id == id) {
                continue;
            }
            let name = it
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or(id)
                .to_string();
            let reference = it
                .get("reference_text")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let language = if reference
                .chars()
                .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            {
                "Chinese"
            } else if reference.chars().any(|c| c.is_ascii_alphabetic()) {
                "English"
            } else {
                ""
            };
            out.push(TtsVoiceInfo {
                id: id.to_string(),
                name,
                kind: "profile".to_string(),
                language: language.to_string(),
            });
        }
        if !out.is_empty() {
            return Ok(out);
        }
        last_err = "服务未返回音色列表".into();
    }
    Err(last_err)
}

#[derive(serde::Serialize)]
pub struct EnsureVoiceResult {
    pub id: String,
    pub created: bool,
}

/// VoiceStudio 专用：确保存在一个「固定音色（design profile）」并返回其 id。
///
/// 背景：VoiceStudio + OmniVoice 是零样本克隆引擎。OpenAI 别名（alloy…）会走
/// 「Auto」模式（模型自己挑说话人），逐句请求会换人；只有带固定参考音频的
/// profile 才能锁定说话人。若用户没有同语言的固定音色，这里通过 VoiceStudio
/// 的 `/design/describe` + `/profiles`(kind=design) 现场创建一个（内部用固定
/// 种子渲染身份样本），从而无需英文参考音频即可得到稳定的中文音色。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn tts_ensure_fixed_voice(
    base_url: String,
    api_key: String,
    language: String,
    instruct: String,
    proxy_mode: Option<String>,
    proxy_url: Option<String>,
    proxy_type: Option<String>,
) -> Result<EnsureVoiceResult, String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("未填写服务地址".into());
    }
    let client = crate::ai::build_client(
        proxy_mode.as_deref().unwrap_or("system"),
        proxy_url.as_deref().unwrap_or(""),
        proxy_type.as_deref(),
        base,
    )
    .map_err(|e| e.message)?;
    let key = api_key.trim().to_string();
    let auth = |req: reqwest::RequestBuilder| -> reqwest::RequestBuilder {
        if key.is_empty() {
            req
        } else {
            req.header("Authorization", format!("Bearer {key}"))
        }
    };
    const NAME_PREFIX: &str = "TidePage";

    // 1) 复用已存在的同语言 TidePage 固定音色（避免重复创建）。
    if let Ok(resp) = auth(client.get(format!("{base}/profiles"))).send().await {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                let mut rows: Vec<&serde_json::Value> = Vec::new();
                if let Some(a) = v.as_array() {
                    rows.extend(a.iter());
                } else if v.is_object() {
                    rows.push(&v);
                }
                for it in rows {
                    let id = it.get("id").and_then(|x| x.as_str()).unwrap_or("");
                    let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
                    let lang = it.get("language").and_then(|x| x.as_str()).unwrap_or("");
                    let kind = it.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                    let ref_audio = it
                        .get("ref_audio_path")
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    if !id.is_empty()
                        && name.starts_with(NAME_PREFIX)
                        && kind == "design"
                        && lang.eq_ignore_ascii_case(&language)
                        && !ref_audio.is_empty()
                    {
                        return Ok(EnsureVoiceResult {
                            id: id.to_string(),
                            created: false,
                        });
                    }
                }
            }
        }
    }

    // 2) 把 instruct 解析成 VoiceStudio 需要的 vd_states（失败则用空对象）。
    let vd_states = match auth(client.post(format!("{base}/design/describe")))
        .json(&serde_json::json!({ "description": instruct }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("attrs").cloned())
            .unwrap_or_else(|| serde_json::json!({})),
        _ => serde_json::json!({}),
    };

    // 3) 创建 design profile（VoiceStudio 会用固定种子渲染身份样本作为参考音频）。
    let form = reqwest::multipart::Form::new()
        .text("name", format!("{NAME_PREFIX} {language} Voice"))
        .text("kind", "design")
        .text("language", language.clone())
        .text("instruct", instruct.clone())
        .text("seed", "42")
        .text("vd_states", vd_states.to_string());
    let resp = auth(client.post(format!("{base}/profiles")))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("创建固定音色失败：{e}"))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let brief: String = txt.chars().take(200).collect();
        return Err(format!("创建固定音色失败：HTTP {} {brief}", status.as_u16()));
    }
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or("服务未返回音色 id")?
        .to_string();
    Ok(EnsureVoiceResult { id, created: true })
}

#[tauri::command]
pub async fn search_all(state: State<'_, Arc<AppState>>, query: String) -> Result<Vec<SearchHit>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || search_all_inner(&st, query))
        .await
        .map_err(|e| e.to_string())
}

fn search_all_inner(state: &Arc<AppState>, query: String) -> Vec<SearchHit> {
    {
        let conn = state.db.0.lock().unwrap();
        if !db::has_fts(&conn) || query.chars().count() < 2 {
            return vec![];
        }
    }
    // 直接用 FTS5 snippet() 生成片段：不再逐条 ensure_book_text（整书解析），首次搜索显著变快。
    // snippet() 不做 HTML 转义，故用正文不会出现的哨兵字符包裹命中，取出后先在 Rust 侧
    // 按 highlight_snippet 同款规则转义（& < >），再把哨兵还原为 <mark>/</mark>。
    // 书名用 JOIN 直接取，不再逐条 get_book。
    let rows: Vec<(i64, i64, String, String, String)> = {
        let conn = state.db.0.lock().unwrap();
        let q = query.replace('"', "\"\"");
        let sql = "SELECT fts.book_id, fts.chapter_index, fts.title, COALESCE(b.title,''), \
                   snippet(fts, 3, ?2, ?3, '…', 12) \
                   FROM fts LEFT JOIN books b ON b.id=fts.book_id \
                   WHERE fts.body MATCH ?1 LIMIT 80";
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(rusqlite::params![&q, "\u{1}", "\u{2}"], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map(|rs| rs.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    };
    rows.into_iter()
        .map(|(book_id, chapter_index, chapter_title, book_title, raw)| SearchHit {
            book_id,
            book_title,
            chapter_index,
            chapter_title,
            snippet: snippet_highlight(&raw),
        })
        .collect()
}

/// 把 FTS5 snippet() 的原始片段转成安全 HTML：先转义 & < >，再把哨兵 \u{1}/\u{2}
/// 还原为 <mark>/</mark>（哨兵仅在转义之后替换，不会与转义结果冲突）。
fn snippet_highlight(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len() + 16);
    for ch in raw.chars() {
        match ch {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '\u{1}' => s.push_str("<mark>"),
            '\u{2}' => s.push_str("</mark>"),
            c => s.push(c),
        }
    }
    s
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
        // 复用解析缓存（load_txt）：不再重新 fs::read + 解码 + 正则切分。
        let cached = load_txt(state, &book.path, book_id)?;
        let (plain, ranges, toc) = cached.as_ref();
        toc.iter()
            .zip(ranges)
            .map(|(t, (s, e))| (t.index as i64, t.title.clone(), safe_slice(plain, *s, *e)))
            .collect()
    } else if ext == "epub" {
        // 复用解析缓存（load_epub）：不再重新 fs::read + parse。
        let cached = load_epub(state, &book.path, book_id)?;
        let doc = &cached.doc;
        let spine_len = doc.spine.len();
        // 一次性建好「spine 下标 → 章节号 / 标题」，避免逐章线性查找目录（大书 O(n²)）。
        let mut toc_pos_by_spine: Vec<Option<i64>> = vec![None; spine_len];
        let mut title_by_spine: Vec<String> = vec![String::new(); spine_len];
        for (i, t) in doc.toc.iter().enumerate() {
            if t.index < spine_len {
                if toc_pos_by_spine[t.index].is_none() {
                    toc_pos_by_spine[t.index] = Some(i as i64);
                }
                if title_by_spine[t.index].is_empty() {
                    title_by_spine[t.index] = t.title.clone();
                }
            }
        }
        // 复用单个 zip/CiMap 提取全章文本（而非每章重建索引）。
        let texts = crate::epub::collect_texts(&cached.data, &doc.spine);
        doc.spine
            .iter()
            .enumerate()
            .map(|(i, _)| {
                (
                    toc_pos_by_spine[i].unwrap_or(i as i64),
                    title_by_spine[i].clone(),
                    texts.get(i).cloned().unwrap_or_default(),
                )
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
pub async fn ensure_book_text_cmd(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<i64, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || ensure_book_text(&st, book_id).map(|n| n as i64))
        .await
        .map_err(|e| e.to_string())?
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
pub async fn search_in_book(state: State<'_, Arc<AppState>>, book_id: i64, query: String) -> Result<Vec<BookSearchHit>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || search_in_book_inner(&st, book_id, query))
        .await
        .map_err(|e| e.to_string())
}

fn search_in_book_inner(state: &Arc<AppState>, book_id: i64, query: String) -> Vec<BookSearchHit> {
    let compact_q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if compact_q.is_empty() {
        return vec![];
    }
    if ensure_book_text(state, book_id).is_err() {
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
    // 创建划线时 upsert 该书指纹（大小+首尾64KB sha256）与书名/作者快照。
    // 书此时必然存在，写入最稳；每书一行，不冗余进 highlight。
    if let Some(b) = db::get_book(&conn, book_id) {
        let (size, mtime, hash) = db::file_fingerprint(&b.path);
        let (t, a) = if b.title.trim().is_empty() {
            let (ft, fa) = parse_filename(&b.path);
            (ft, fa)
        } else {
            (b.title.clone(), b.author.clone())
        };
        db::upsert_book_fingerprint(&conn, book_id, size, mtime, &hash, &t, &a).ok();
    }
    let r = db::add_highlight(
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
    );
    drop(conn);
    // 新增划线会把该书带进笔记窗口（有划线即入列），通知已打开的笔记窗口刷新。
    state.app.emit("notes-changed", ()).ok();
    r
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
    let r = db::delete_highlight(&conn, id);
    drop(conn);
    // 删掉某书最后一条划线后，该书应立即从笔记窗口消失。
    state.app.emit("notes-changed", ()).ok();
    r
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
    let r = db::update_highlight(&conn, id, &style, &color, &note);
    drop(conn);
    // 感想/样式变动会影响笔记窗口的条数与内容。
    state.app.emit("notes-changed", ()).ok();
    r
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
pub async fn rebuild_fts(state: State<'_, Arc<AppState>>) -> Result<i64, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        {
            let conn = st.db.0.lock().unwrap();
            if !db::has_fts(&conn) {
                return Err("SQLite 编译未包含 FTS5".into());
            }
            db::ensure_fts(&conn)?;
            // 先清空索引，再逐本重建（此前只 DELETE 不重建，会把索引清空）。
            conn.execute("DELETE FROM fts", []).map_err(|e| e.to_string())?;
        }
        let ids: Vec<i64> = {
            let conn = st.db.0.lock().unwrap();
            db::all_book_paths(&conn).into_iter().map(|(id, _)| id).collect()
        };
        let mut total = 0i64;
        for id in ids {
            // 单本失败（文件缺失等）跳过，不影响整体重建。
            if let Ok(n) = index_book_fts_inner(&st, id) {
                total += n;
            }
        }
        Ok(total)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn index_book_fts(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<i64, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || index_book_fts_inner(&st, book_id))
        .await
        .map_err(|e| e.to_string())?
}

fn index_book_fts_inner(state: &Arc<AppState>, book_id: i64) -> Result<i64, String> {
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
        let _ = ensure_book_text(state, book_id);
        return Ok(0);
    }

    let chapters = collect_book_chapters(state, book_id)?;
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
    // 在文件管理器中定位并选中该文件（而非打开父目录），语义更准确，
    // 也避免使用 opener 的 open_path 打开任意路径。
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())?;
    Ok(())
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
        let doc = crate::epub::parse(&data).ok();
        let bytes = doc
            .as_ref()
            .and_then(|d| d.cover_href.as_ref().and_then(|h| crate::epub::read_cover(&data, h)))
            .or_else(|| doc.as_ref().map(|d| crate::epub::find_cover_fallback(&data, &d.spine)).flatten());
        match bytes {
            Some(b) => covers::make_thumbnail(&b, &out).is_ok(),
            None => false,
        }
    } else if is_mobi_ext(&ext) {
        // MOBI/AZW3：EXTH201/202/129 或 first_image_index 起首张图
        crate::mobi::extract_cover_path(&book.path)
            .map(|b| covers::make_thumbnail(&b, &out).is_ok())
            .unwrap_or(false)
    } else {
        false
    };

    if ok {
        let rel = name;
        {
            let conn = state.db.0.lock().unwrap();
            conn.execute(
                "UPDATE books SET cover_path=?2, cover_ver=cover_ver+1 WHERE id=?1",
                rusqlite::params![book_id, rel],
            )
            .map_err(|e| e.to_string())?;
        }
        state.app.emit("book-updated", ()).ok();
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

/// 全库随机一本书（轻量：COUNT + 随机 offset；放阻塞线程池，避免同步持锁 O(N)）。
#[tauri::command]
pub async fn random_book(state: State<'_, Arc<AppState>>) -> Result<Option<BookRow>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::random_book_any(&conn)
    })
    .await
    .map_err(|e| e.to_string())
}

/// 从「当前筛选结果」中随机一本书（query/sort/formats/states 与列表同源）。
#[tauri::command]
pub async fn random_book_filtered(
    state: State<'_, Arc<AppState>>,
    query: String,
    sort: String,
    formats: Option<Vec<String>>,
    states: Option<Vec<String>>,
    folder: Option<String>,
) -> Result<Option<BookRow>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::random_book_filtered(&conn, &query, &sort, &formats, &states, &folder)
    })
    .await
    .map_err(|e| e.to_string())
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
    let (_id, book) = {
        let conn = state.db.0.lock().unwrap();
        let id = db::upsert_book(&conn, &path, &m.title, &ext, size, file_mtime(p))?;
        if !m.author.is_empty() {
            conn.execute(
                "UPDATE books SET author=?2 WHERE id=?1 AND author=''",
                rusqlite::params![id, m.author],
            )
            .ok();
        }
        (id, db::get_book(&conn, id))
    };
    state.app.emit("library-changed", ()).ok();
    // 导入书籍落库后：新书 meta_done=0，拉起增量扫描（若空闲）。
    ensure_scan_worker(&state.app);
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
        if let Ok(f) = std::fs::File::open(&path) {
            let mut buf = Vec::new();
            if f.take(8192).read_to_end(&mut buf).is_ok() {
                let (text, _) = crate::txt::decode_bytes(&buf, None);
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
    // DB 短路：字数已回填过**且文件没被换过**才直接用。
    // 光看 total_chars > 0 不够：同一路径的文件被就地替换后 id 不变、扫描也不一定跑过，
    // 只按值短路会把旧文件的字数一直报下去。
    let disk = file_stamp(&book.path);
    if db_chars_usable(book.total_chars, book.size, book.mtime, disk) {
        return Ok(WordCount {
            total: book.total_chars,
            pure: book.total_text_chars,
        });
    }
    let st = state.inner().clone();
    let path = book.path.clone();
    let ext = ext_of(&path).unwrap_or_default();
    // 三条现算分支全部放进阻塞线程池：EPUB 是完整解析（rayon 逐章文本统计，数千章可达数秒），
    // MOBI/TXT 也要读盘 + 全量解码。旧实现在 async fn 里直接同步调用 load_epub / mobi_doc_for /
    // load_txt —— 会占住 async 运行时线程，把这段时间内的其它 IPC 一并拖住
    // （open_book 对同类调用有明确注释要求放阻塞线程池）。
    tauri::async_runtime::spawn_blocking(move || -> Result<WordCount, String> {
        if ext == "epub" {
            // 与 book_char_counts 同源：命中同一份缓存时不会再解析第二遍。
            let stats = epub_stats_for(&st, book_id, &path)?;
            return Ok(WordCount {
                total: stats.total,
                pure: stats.pure,
            });
        }
        let is_mobi = is_mobi_ext(&ext);
        let (content, _r, _t) = if is_mobi {
            let doc = mobi_doc_for(&st, &path, book_id)?;
            (doc.html, Vec::new(), Vec::new())
        } else {
            load_txt(&st, &path, book_id)?.as_ref().clone()
        };
        let text = if is_mobi {
            crate::epub::html_to_text(&content)
        } else {
            content
        };
        let total = text.chars().count() as i64;
        let pure = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
        // 同样回填：下一次调用直接走 DB 短路。
        backfill_book_chars(&st, book_id, total, pure);
        Ok(WordCount { total, pure })
    })
    .await
    .map_err(|e| e.to_string())?
}


/// 全书每章字数（EPUB）：底栏「全书总页数」要用真实字数才能估算。
/// 打开书籍走的是快速解析（不做全书逐章统计，toc.chars 恒为 0），故按需补算一次；
/// 非 EPUB 返回空 Vec（前端据此放弃本次补算，不会在轮询里反复调用）。
///
/// 与 get_book_word_count 共用 epub_stats_cache：先跑过书详情/本书统计的书，
/// 这里直接命中缓存，不会再解析第二遍。
#[tauri::command]
pub async fn book_char_counts(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<Vec<i64>, String> {
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    if ext_of(&book.path).unwrap_or_default() != "epub" {
        return Ok(Vec::new());
    }
    let st = state.inner().clone();
    let path = book.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        epub_stats_for(&st, book_id, &path).map(|s| s.per_chapter.clone())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod cover_container_tests {
    use super::{cover_source, has_cover_container, CoverSource};

    /// 对齐性：has_cover_container 返回 true 的每个扩展名，extract_cover_only 都命中具体分支
    /// （cover_source 非 None），绝不落默认分支；fb2/cbr/txt 等归非容器。
    #[test]
    fn container_set_aligns_with_extract_branches() {
        for ext in ["epub", "mobi", "azw", "azw3", "cbz"] {
            assert!(has_cover_container(ext), "{ext} 应为容器格式");
            assert!(
                cover_source(ext).is_some(),
                "{ext} 必须命中具体提取分支，不能落默认分支"
            );
        }
        // 具体分支归属
        assert_eq!(cover_source("epub"), Some(CoverSource::Epub));
        assert_eq!(cover_source("cbz"), Some(CoverSource::Cbz));
        assert_eq!(cover_source("mobi"), Some(CoverSource::Mobi));
        assert_eq!(cover_source("azw"), Some(CoverSource::Mobi));
        assert_eq!(cover_source("azw3"), Some(CoverSource::Mobi));
        // fb2/cbr 已移出容器集合 → 非容器路径
        for ext in ["fb2", "cbr", "txt", "prc", "pdf", ""] {
            assert!(!has_cover_container(ext), "{ext} 不应为容器格式");
            assert!(cover_source(ext).is_none(), "{ext} 应落非容器路径");
        }
    }
}

#[cfg(test)]
mod default_cover_tests {
    use super::write_default_cover_atomic;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fr_dcover_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([(x * 30) as u8, (y * 30) as u8, 128]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn valid_png_is_written_atomically() {
        let d = tmp_dir("ok");
        let out = d.join("default-cover.img");
        let bytes = tiny_png();
        write_default_cover_atomic(&out, &bytes).unwrap();
        assert!(out.is_file());
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
        // 无残留临时文件
        assert!(!d.join("default-cover.img.tmp").exists());
    }

    #[test]
    fn invalid_bytes_do_not_touch_existing() {
        let d = tmp_dir("bad");
        let out = d.join("default-cover.img");
        let good = tiny_png();
        write_default_cover_atomic(&out, &good).unwrap();
        let before = std::fs::read(&out).unwrap();
        // 塞入损坏内容：应报错，且旧底图保持不变、无 tmp 残留。
        let err = write_default_cover_atomic(&out, b"not an image at all").unwrap_err();
        assert!(err.contains("不是有效图片"), "err={err}");
        assert_eq!(std::fs::read(&out).unwrap(), before, "旧底图被破坏");
        assert!(!d.join("default-cover.img.tmp").exists(), "残留临时文件");
    }

    #[test]
    fn creates_parent_dir() {
        let d = tmp_dir("mk");
        let out = d.join("nested").join("default-cover.img");
        write_default_cover_atomic(&out, &tiny_png()).unwrap();
        assert!(out.is_file());
    }

    /// Bug1/Bug2 回归：换底图只写共享底图那 1 份文件，绝不动同目录里其他书的封面。
    #[test]
    fn changing_base_does_not_touch_sibling_covers() {
        let d = tmp_dir("sib");
        // 模拟 covers 目录里「有内嵌封面书」的 per-book 封面文件。
        let per_book = d.join("deadbeef.png");
        let book_bytes = tiny_png();
        std::fs::write(&per_book, &book_bytes).unwrap();
        let base = d.join("default-cover.img");
        // 换底图：只写 default-cover.img。
        write_default_cover_atomic(&base, &tiny_png()).unwrap();
        assert!(base.is_file());
        // 其他书封面原封不动。
        assert!(per_book.is_file(), "per-book cover was deleted");
        assert_eq!(std::fs::read(&per_book).unwrap(), book_bytes, "per-book cover changed");
        // 无残留临时文件。
        assert!(!d.join("default-cover.img.tmp").exists());
    }

    /// 底图存为 .img，扩展名不可靠：MIME 必须按字节魔数嗅探。
    #[test]
    fn sniff_mime_recognizes_common_base_formats() {
        assert_eq!(crate::sniff_mime(&[0x89, b'P', b'N', b'G']), "image/png");
        assert_eq!(crate::sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(crate::sniff_mime(b"GIF89a"), "image/gif");
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend_from_slice(b"VP8 ");
        assert_eq!(crate::sniff_mime(&webp), "image/webp");
        assert_eq!(crate::sniff_mime(b"BM\0\0"), "image/bmp");
    }
}

#[cfg(test)]
mod filename_tests {
    use crate::library::parse_filename;

    fn p(s: &str) -> (String, String) {
        parse_filename(s)
    }

    #[test]
    fn normal_title_author_brackets() {
        assert_eq!(p("/x/三体 (刘慈欣).txt"), ("三体".into(), "刘慈欣".into()));
        assert_eq!(p("/x/活着[余华].epub"), ("活着".into(), "余华".into()));
    }

    #[test]
    fn book_marks_and_dash() {
        assert_eq!(p("/x/《围城》.txt"), ("围城".into(), String::new()));
        assert_eq!(p("/x/解忧杂货店 - 东野圭吾.txt"), ("解忧杂货店".into(), "东野圭吾".into()));
    }

    #[test]
    fn no_extension_still_works() {
        assert_eq!(p("/x/无扩展名书籍"), ("无扩展名书籍".into(), String::new()));
    }

    #[test]
    fn pure_numeric_name() {
        assert_eq!(p("/x/12345.txt"), ("12345".into(), String::new()));
    }

    #[test]
    fn empty_name_falls_back() {
        let (t, a) = p("");
        assert_eq!(t, "未命名");
        assert_eq!(a, "");
    }

    #[test]
    fn publisher_bracket_ignored() {
        // 含"出版"的括号不作为作者
        let (t, a) = p("/x/某书 (某某出版社).txt");
        assert_eq!(a, "");
        assert!(t.contains("某书"));
    }

    #[test]
    fn long_bracket_ignored() {
        // 括号内容过长（>20）不作作者
        let (_, a) = p("/x/书 (这是一个非常非常非常非常冗长的括号说明内容).txt");
        assert_eq!(a, "");
    }

    #[test]
    fn multi_bracket_takes_last() {
        let (t, a) = p("/x/书名 (系列) (作者名).txt");
        assert_eq!(a, "作者名");
        assert!(t.contains("书名"));
    }
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

        let p1 = db::list_books_page(&conn, "", "recent", 0, 500, &None, &None, true, &None);
        assert_eq!(p1.total, 7);
        assert_eq!(p1.rows.len(), 7);
        assert_eq!(p1.rows[0].title, "zeta", "mtime DESC 最新在前");

        let p2 = db::list_books_page(&conn, "", "recent", 2, 3, &None, &None, true, &None);
        assert_eq!((p2.total, p2.rows.len()), (7, 3));
        assert_eq!(p2.rows[0].title, "delta", "offset=2 应从第三新开始");

        let p3 = db::list_books_page(&conn, "", "added", 0, 9999, &None, &None, true, &None);
        assert_eq!(p3.rows.len(), 7, "limit 超上限被 clamp 到 500，本应有 7 行");

        let p4 = db::list_books_page(&conn, "epsilon", "title", 0, 10, &None, &None, false, &None);
        assert_eq!((p4.total, p4.rows.len()), (1, 1), "标题搜索应大小写不敏感");
        assert_eq!(p4.rows[0].title, "Epsilon");

        let p5 = db::list_books_page(&conn, "%_marker", "title", 0, 10, &None, &None, false, &None);
        assert_eq!(p5.total, 1, "% 与 _ 必须按字面匹配");
        assert_eq!(p5.rows[0].title, "100%_marker");

        let p6 = db::list_books_page(&conn, "%", "title", 0, 10, &None, &None, false, &None);
        assert_eq!(p6.total, 1, "纯通配符不得匹配全部");

        let p7 = db::list_books_page(&conn, "", "title", 0, 10, &None, &None, false, &None);
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
        let p1 = db::list_books_page(&conn, "", "added", 0, 500, &None, &None, true, &None);
        println!("[perf160k] added page0: {:?} total={} rows={}", t.elapsed(), p1.total, p1.rows.len());

        let t = Instant::now();
        let p2 = db::list_books_page(&conn, "", "title", 0, 500, &None, &None, false, &None);
        println!("[perf160k] title page0: {:?} rows={}", t.elapsed(), p2.rows.len());

        let t = Instant::now();
        let p3 = db::list_books_page(&conn, "", "recent", 159_000, 500, &None, &None, true, &None);
        println!("[perf160k] recent offset=159000: {:?} rows={}", t.elapsed(), p3.rows.len());

        let t = Instant::now();
        let p3b = db::list_books_page(&conn, "", "added", 159_000, 500, &None, &None, true, &None);
        println!("[perf160k] added offset=159000: {:?} rows={}", t.elapsed(), p3b.rows.len());

        let t = Instant::now();
        let p3c = db::list_books_page(&conn, "", "title", 159_000, 500, &None, &None, false, &None);
        println!("[perf160k] title offset=159000: {:?} rows={}", t.elapsed(), p3c.rows.len());

        let t = Instant::now();
        let p3d = db::list_books_page(&conn, "", "lastread", 0, 500, &None, &None, true, &None);
        println!("[perf160k] lastread page0: {:?} rows={}", t.elapsed(), p3d.rows.len());

        let t = Instant::now();
        let p4 = db::list_books_page(&conn, "Book 159", "title", 0, 500, &None, &None, false, &None);
        println!("[perf160k] search 'Book 159': {:?} total={} rows={}", t.elapsed(), p4.total, p4.rows.len());

        assert_eq!(p1.total, 160_000);
        assert_eq!(p4.total, 1000, "Book 159000..159999 共 1000 本");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 秩可寻址分页：order_index_ids 与 list_books_page 同源（rank→id 对齐），
    /// books_by_ids_aligned 按入参顺序对齐、缺行补 None。
    #[test]
    fn order_index_and_by_ids_align() {
        let root = tmp_root("orderidx");
        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        let data = [
            ("a.txt", "Alpha", 100i64, 500i64),
            ("b.txt", "beta", 200, 400),
            ("c.txt", "Gamma", 300, 300),
            ("d.txt", "delta", 400, 200),
            ("e.txt", "Epsilon", 500, 100),
        ];
        for (name, title, mtime, size) in data {
            let p = root.join(name);
            std::fs::write(&p, b"x").unwrap();
            db::upsert_book(&conn, &sp(&p), title, "txt", size, mtime).unwrap();
        }

        // 每个 sort × 每个方向：order_index_ids 的序列必须与 list_books_page 全量窗口逐行 id 一致。
        for sort in ["added", "recent", "title", "lastread", "size"] {
            for desc in [false, true] {
                let idx = db::order_index_ids(&conn, sort, "", 1_000_000, &None, &None, desc, &None).unwrap();
                let page = db::list_books_page(&conn, "", sort, 0, 500, &None, &None, desc, &None);
                let page_ids: Vec<i64> = page.rows.iter().map(|r| r.id).collect();
                assert_eq!(idx, page_ids, "order_index_ids 与 list_books_page 不同源: sort={sort} desc={desc}");
            }
        }

        // size：降序大→小、升序小→大（反向按钮两端都可用）。
        let big = db::list_books_page(&conn, "", "size", 0, 500, &None, &None, true, &None);
        let small = db::list_books_page(&conn, "", "size", 0, 500, &None, &None, false, &None);
        assert!(big.rows.len() >= 2 && big.rows[0].size >= big.rows[1].size, "size DESC 首行最大");
        assert!(small.rows.len() >= 2 && small.rows[0].size <= small.rows[1].size, "size ASC 首行最小");

        // 未知 sort 回落 added。
        assert_eq!(
            db::order_index_ids(&conn, "bogus", "", 1_000_000, &None, &None, true, &None).unwrap(),
            db::order_index_ids(&conn, "added", "", 1_000_000, &None, &None, true, &None).unwrap(),
            "未知 sort 必须回落 added"
        );

        // 过滤档：order_index_ids(query) 必须与 list_books_page(query) 逐行同源（rank→id 对齐）。
        for (sort, q, desc) in [("title", "delta", false), ("added", "a", true), ("recent", "e", true)] {
            let idx = db::order_index_ids(&conn, sort, q, 1_000_000, &None, &None, desc, &None).unwrap();
            let page = db::list_books_page(&conn, q, sort, 0, 500, &None, &None, desc, &None);
            let page_ids: Vec<i64> = page.rows.iter().map(|r| r.id).collect();
            assert_eq!(idx, page_ids, "过滤档不同源: sort={sort} q={q} desc={desc}");
            assert_eq!(idx.len() as i64, page.total, "过滤档条数应等于 total");
        }

        // 越限：max_rows 限制触发 Err，不静默截断。
        let err = db::order_index_ids(&conn, "added", "", 2, &None, &None, true, &None).unwrap_err();
        assert!(err.starts_with("order_index_too_large:"), "越限必须报错: {err}");

        // books_by_ids_aligned：长度/顺序恒等于入参，缺行补 None。
        let idx = db::order_index_ids(&conn, "title", "", 1_000_000, &None, &None, false, &None).unwrap();
        let missing = 424242i64; // 不存在
        let query_ids = vec![idx[2], missing, idx[0], idx[0]]; // 含重复与缺失
        let rows = db::books_by_ids_aligned(&conn, &query_ids);
        assert_eq!(rows.len(), query_ids.len(), "返回长度必须等于入参");
        assert_eq!(rows[0].as_ref().unwrap().id, idx[2]);
        assert!(rows[1].is_none(), "缺失 id 必须为 None 占位，不跳过");
        assert_eq!(rows[2].as_ref().unwrap().id, idx[0]);
        assert_eq!(rows[3].as_ref().unwrap().id, idx[0], "重复 id 各自命中");

        // 空入参返回空。
        assert!(db::books_by_ids_aligned(&conn, &[]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 分类筛选（格式 + 阅读状态）：build_book_filter 下推后 order_index_ids 与 list_books_page 仍逐行同源；
    /// 空数组=不约束；状态语义（未读/在读/已读 + 手动覆盖）与 effectiveState 一致。
    #[test]
    fn book_filter_states_and_formats_align() {
        let root = tmp_root("bookfilter");
        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        // 造 4 本：不同格式 + 不同状态。
        let mk = |name: &str, fmt: &str| {
            let p = root.join(name);
            std::fs::write(&p, b"x").unwrap();
            db::upsert_book(&conn, &sp(&p), name, fmt, 1, 100).unwrap()
        };
        let id_unread = mk("u.txt", "txt"); // 从未打开 → unread
        let id_reading = mk("r.epub", "epub"); // 打开过 + 进度未满 → reading
        let id_read = mk("d.epub", "epub"); // 进度已满 → read
        let id_manual = mk("m.mobi", "mobi"); // 手动已读（从未打开）→ read

        db::touch_book_opened(&conn, id_reading);
        let mut p = crate::models::Progress { updated_at: db::now_ms(), ..Default::default() };
        p.pct = 42.0;
        db::set_progress(&conn, id_reading, &p).unwrap();

        db::touch_book_opened(&conn, id_read);
        let mut p2 = crate::models::Progress { updated_at: db::now_ms(), ..Default::default() };
        p2.pct = 100.0;
        db::set_progress(&conn, id_read, &p2).unwrap();

        db::set_read_state(&conn, id_manual, 1).unwrap(); // 手动已读

        // 逐状态断言（用 order_index_ids 结果集合判定）。
        let ids_of = |states: Option<Vec<String>>| -> Vec<i64> {
            db::order_index_ids(&conn, "added", "", 1_000_000, &None, &states, true, &None).unwrap()
        };
        let has = |v: &[i64], id: i64| v.contains(&id);

        let unread = ids_of(Some(vec!["unread".into()]));
        assert!(has(&unread, id_unread) && !has(&unread, id_reading) && !has(&unread, id_read) && !has(&unread, id_manual), "unread 集合错误: {unread:?}");

        let reading = ids_of(Some(vec!["reading".into()]));
        assert!(has(&reading, id_reading) && !has(&reading, id_unread) && !has(&reading, id_read) && !has(&reading, id_manual), "reading 集合错误: {reading:?}");

        let read = ids_of(Some(vec!["read".into()]));
        assert!(has(&read, id_read) && has(&read, id_manual) && !has(&read, id_unread) && !has(&read, id_reading), "read 集合错误: {read:?}");

        // 多状态取「或」。
        let ru = ids_of(Some(vec!["read".into(), "unread".into()]));
        assert_eq!(ru.len(), 3, "read+unread 应 3 本: {ru:?}");

        // 空数组 = 不约束（回完整库 4 本）。
        assert_eq!(ids_of(Some(vec![])).len(), 4, "空状态数组必须不约束");
        assert_eq!(db::order_index_ids(&conn, "added", "", 1_000_000, &None, &None, true, &None).unwrap().len(), 4);

        // 格式过滤：epub 2 本。
        let epub = db::order_index_ids(&conn, "added", "", 1_000_000, &Some(vec!["epub".into()]), &None, true, &None).unwrap();
        assert_eq!(epub.len(), 2, "epub 应 2 本");

        // 格式 + 状态取「且」：epub 且 read → 仅 id_read。
        let epub_read = db::order_index_ids(&conn, "added", "", 1_000_000, &Some(vec!["epub".into()]), &Some(vec!["read".into()]), true, &None).unwrap();
        assert_eq!(epub_read, vec![id_read], "epub∩read 应仅 id_read");

        // 红线 3：任意过滤组合下 order_index_ids 与 list_books_page 逐行同源。
        for (fmts, sts) in [
            (Some(vec!["epub".into()]), None),
            (None, Some(vec!["read".into(), "reading".into()])),
            (Some(vec!["epub".into(), "mobi".into()]), Some(vec!["read".into()])),
        ] {
            let idx = db::order_index_ids(&conn, "title", "", 1_000_000, &fmts, &sts, false, &None).unwrap();
            let page = db::list_books_page(&conn, "", "title", 0, 500, &fmts, &sts, false, &None);
            let page_ids: Vec<i64> = page.rows.iter().map(|r| r.id).collect();
            assert_eq!(idx, page_ids, "过滤组合不同源: fmts={fmts:?} sts={sts:?}");
            assert_eq!(idx.len() as i64, page.total, "total 应等于索引长度");
        }

        // list_formats：3 种格式（txt/epub/mobi）。
        let formats = db::list_formats(&conn);
        assert_eq!(formats.len(), 3, "格式种类应为 3: {formats:?}");
        assert!(formats.contains(&"epub".to_string()));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// lastread 列由写入路径维护：打开书 / 保存进度后 last_read 单调取大，排序随之变化。
    #[test]
    fn last_read_column_maintained_and_orders() {
        let root = tmp_root("lastread");
        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        let mut ids = Vec::new();
        for name in ["a.txt", "b.txt", "c.txt"] {
            let p = root.join(name);
            std::fs::write(&p, b"x").unwrap();
            ids.push(db::upsert_book(&conn, &sp(&p), name, "txt", 1, 1).unwrap());
        }
        // 初始 last_read 全 0 → 顺序按 id DESC。
        let idx0 = db::order_index_ids(&conn, "lastread", "", 1_000_000, &None, &None, true, &None).unwrap();
        assert_eq!(idx0.len(), 3);

        // 打开中间那本 → 它的 last_read 变大，应排到最前。
        db::touch_book_opened(&conn, ids[1]);
        let idx1 = db::order_index_ids(&conn, "lastread", "", 1_000_000, &None, &None, true, &None).unwrap();
        assert_eq!(idx1[0], ids[1], "打开的书应排在按最近阅读的最前");

        // 保存另一本的进度（更大的 updated_at）→ 它应排到最前。
        let p = crate::models::Progress { updated_at: db::now_ms() + 10_000, ..Default::default() };
        db::set_progress(&conn, ids[2], &p).unwrap();
        let idx2 = db::order_index_ids(&conn, "lastread", "", 1_000_000, &None, &None, true, &None).unwrap();
        assert_eq!(idx2[0], ids[2], "刚更新进度的书应排在最前");

        // 该档不得出现 TEMP B-TREE（走 idx_books_lastread_id）。
        let plan: Vec<String> = {
            let mut stmt = conn
                .prepare("EXPLAIN QUERY PLAN SELECT id FROM books ORDER BY last_read DESC, id DESC")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert!(
            !plan.iter().any(|s| s.contains("TEMP B-TREE")),
            "lastread 档不得出现 TEMP B-TREE, plan={plan:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 文件夹作用域：folder 参数下推路径前缀范围后，只返回该目录内的书；
    /// 前缀边界（`…\A` 不得命中 `…\AB`）由末尾补分隔符保证；
    /// order_index_ids / random_book_filtered 与 list_books_page 逐字同源。
    #[test]
    fn list_books_page_folder_scope_and_prefix_boundary() {
        let root = tmp_root("folder");
        let conn = db::init(&root.join("t.db")).unwrap();
        let sp = |p: &Path| p.to_string_lossy().into_owned();
        let dir_a = root.join("A");
        let dir_ab = root.join("AB");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_ab).unwrap();
        // A 内两本、AB 内一本、root 直下一本。
        let a1 = dir_a.join("a1.txt");
        let a2 = dir_a.join("a2.txt");
        let ab1 = dir_ab.join("ab1.txt");
        let top = root.join("top.txt");
        for p in [&a1, &a2, &ab1, &top] {
            std::fs::write(p, b"x").unwrap();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            db::upsert_book(&conn, &sp(p), &name, "txt", 1, 1).unwrap();
        }

        // folder = A：仅 A 内两本，且绝不命中前缀相邻的 AB。
        let a = sp(&dir_a);
        let got = db::list_books_page(&conn, "", "added", 0, 500, &None, &None, true, &Some(a.clone()));
        assert_eq!(got.total, 2, "仅 A 内两本");
        assert!(got.rows.iter().all(|r| r.path.starts_with(&a)));
        assert!(!got.rows.iter().any(|r| r.path == sp(&ab1)), "A 不得命中 AB（前缀边界）");

        // folder = AB：仅 AB 内一本（验证边界另一侧）。
        let ab = sp(&dir_ab);
        let got_ab = db::list_books_page(&conn, "", "added", 0, 500, &None, &None, true, &Some(ab.clone()));
        assert_eq!(got_ab.total, 1);
        assert_eq!(got_ab.rows[0].path, sp(&ab1));

        // 不带 folder = 全库 4 本。
        let all = db::list_books_page(&conn, "", "added", 0, 500, &None, &None, true, &None);
        assert_eq!(all.total, 4);

        // 排序/随机同源：order_index_ids 与 random_book_filtered 同样限定在 A 内。
        let folder = Some(a.clone());
        let idx = db::order_index_ids(&conn, "added", "", 1_000_000, &None, &None, true, &folder).unwrap();
        assert_eq!(idx.len(), 2, "order_index_ids 也应只含 A 内两本");
        let picked = db::random_book_filtered(&conn, "", "added", &None, &None, &folder).unwrap();
        assert!(picked.path.starts_with(&a), "随机也需限定在 A 内: {}", picked.path);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 漫画判据：图片门槛 + 正文极简（本次收紧后，只有「每章除标题外几乎没有文字」才算漫画）。
    #[test]
    fn manga_detection_one_image_per_page() {
        // 全职猎人式：约 2438 图 / 2439 内容文档 + 无正文 → true（旧 toc*1.5 会漏判）。
        assert!(is_manga_like(2438, 2439, 2439, "epub", 0), "一图一页应判漫画");
        // 常见漫画：1.2 万图 / 75 章 + 无正文 → true。
        assert!(is_manga_like(12000, 12000, 75, "epub", 0));
        // 只有章节标题（正文量扣掉标题后为 0）→ true。
        assert!(is_manga_like(600, 600, 600, "epub", 0), "只有标题应判漫画");
        // 恰好等于阈值仍算漫画；超过一位即否。
        assert!(is_manga_like(600, 600, 600, "epub", MANGA_BODY_CHAR_LIMIT));
        assert!(!is_manga_like(600, 600, 600, "epub", MANGA_BODY_CHAR_LIMIT + 1));
        // 普通小说：少量插图 → false。
        assert!(!is_manga_like(5, 200, 200, "epub", 0));
        // 图不足 30 的边界 → false。
        assert!(!is_manga_like(29, 29, 29, "epub", 0));
        // 图多但远少于文档数（图文混排字书）→ 图片门槛即否。
        assert!(!is_manga_like(2000, 5000, 5000, "epub", 0));
        // 图片够多但每章有正常正文 → false：这是本次收紧的核心（带插图的普通小说不得误判）。
        assert!(!is_manga_like(600, 600, 600, "epub", 5000));
        assert!(!is_manga_like(12000, 0, 75, "mobi", 5000), "MOBI 有正文也不判漫画");
        // MOBI/TXT 无 spine 清单：仅按原 toc 比例*1.5（保持原行为）+ 无正文。
        assert!(!is_manga_like(5000, 0, 5000, "mobi", 0), "MOBI 保持原 toc 判据");
        assert!(is_manga_like(12000, 0, 75, "mobi", 0));
    }

    /// 单章正文量扣掉目录标题：标题本身就写在章节文档里，不能当正文证据。
    #[test]
    fn manga_body_chars_excludes_title() {
        assert_eq!(body_chars_excluding_title("", ""), 0);
        assert_eq!(body_chars_excluding_title("第1话", "第1话"), 0);
        assert_eq!(body_chars_excluding_title("第1话\n\n  ", "第1话"), 0);
        assert_eq!(body_chars_excluding_title("第1话 正文", "第1话"), 2);
        assert_eq!(body_chars_excluding_title("abcdefghij", ""), 10);
        assert_eq!(body_chars_excluding_title("abcdefghij", "abc"), 7);
    }

    /// 反例（本次收紧的核心保证）：带 30+ 张插图、但每章有正常正文的 EPUB，
    /// 图片门槛会通过，正文探针必须给出远超阈值的值，使 is_manga_like 判 false。
    /// 同时验证纯图 EPUB（正文只有标题）探针为 0、仍判漫画。
    #[test]
    fn manga_probe_rejects_illustrated_novel() {
        use std::io::Write as _;
        fn build_zip(para: &str) -> Vec<u8> {
            let mut buf = std::io::Cursor::new(Vec::new());
            {
                let mut zw = zip::ZipWriter::new(&mut buf);
                let opt = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                for i in 0..40usize {
                    let page = format!(
                        "<html><body><h1>第{}章</h1><p>{}</p><img src=\"img/{:03}.png\"/></body></html>",
                        i + 1,
                        para,
                        i
                    );
                    zw.start_file(format!("OEBPS/c{i:03}.xhtml"), opt).unwrap();
                    zw.write_all(page.as_bytes()).unwrap();
                }
                zw.finish().unwrap();
            }
            buf.into_inner()
        }
        let spine: Vec<String> = (0..40).map(|i| format!("OEBPS/c{i:03}.xhtml")).collect();
        let toc: Vec<TocEntry> = (0..40)
            .map(|i| TocEntry { title: format!("第{}章", i + 1), index: i, chars: 0 })
            .collect();

        // 带插图的普通小说：正文量远超阈值 → 不判漫画（一句正常正文就约 23 字，这里取 6 句）。
        let long_para = "这是一段正常的正文，用来模拟带插图的普通小说。".repeat(6);
        let novel = build_zip(&long_para);
        let (body, used) = manga_body_chars_epub(&novel, &spine, &toc);
        assert!(body > MANGA_BODY_CHAR_LIMIT, "普通小说正文量应超阈值，实际 {body}");
        assert!(used > 0, "探针应实际解压过字节");
        assert!(!is_manga_like(40, 40, 40, "epub", body), "带插图的普通小说不得判漫画");

        // 纯图（章节文档里只有标题）：正文量为 0 → 判漫画。
        let manga = build_zip("");
        let (body2, _) = manga_body_chars_epub(&manga, &spine, &toc);
        assert_eq!(body2, 0, "只有标题时正文量应为 0");
        assert!(is_manga_like(40, 40, 40, "epub", body2), "纯图应判漫画");
    }

    /// 端到端：用 _test/make_manga_false_fixture.py 生成的真实 EPUB 校验探针（按需运行）。
    ///   $env:FR_MANGA_FALSE_FIXTURE="<path.epub>"; cargo test manga_probe_against_python_fixture -- --nocapture
    #[test]
    fn manga_probe_against_python_fixture() {
        let Ok(path) = std::env::var("FR_MANGA_FALSE_FIXTURE") else {
            return;
        };
        let data = std::fs::read(&path).expect("读取夹具失败");
        let (doc, _ci) = crate::epub::parse_with_ci(&data).expect("解析夹具失败");
        let images = 40usize; // 夹具固定 40 张图 / 40 章
        assert!(
            manga_image_gate(images, doc.spine.len(), doc.toc.len(), "epub"),
            "带插图小说夹具应通过图片门槛：images=40 spine={} toc={}",
            doc.spine.len(),
            doc.toc.len()
        );
        let (body, _used) = manga_body_chars_epub(&data, &doc.spine, &doc.toc);
        assert!(body > MANGA_BODY_CHAR_LIMIT, "正文量应超阈值，实际 {body}");
        assert!(
            !is_manga_like(images, doc.spine.len(), doc.toc.len(), "epub", body),
            "带插图的普通小说不得判漫画"
        );
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
        let doc = crate::epub::parse(&data).unwrap();
        assert_eq!(doc.title, "新标题 & <tag>");
        assert_eq!(doc.author, "新作者");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod cache_stamp_tests {
    //! 任务六：读缓存 / DB 短路之前的文件指纹校验。
    //! 同一路径的文件被就地替换后 book_id 不变，只按 id 命中缓存（或只看 DB 有值）
    //! 会让正文、目录、字数全部停在旧文件上。这几个纯函数就是那道闸门。
    use super::*;

    #[test]
    fn stamp_matches_requires_both_sides_equal() {
        assert!(stamp_matches(Some((10, 20)), Some((10, 20))));
        assert!(!stamp_matches(Some((10, 20)), Some((11, 20))), "大小变了");
        assert!(!stamp_matches(Some((10, 20)), Some((10, 21))), "mtime 变了");
        assert!(!stamp_matches(Some((10, 20)), None), "当前取不到指纹");
        assert!(!stamp_matches(None, Some((10, 20))), "缓存条目没有指纹");
        assert!(!stamp_matches(None, None));
    }

    #[test]
    fn db_chars_usable_needs_value_and_matching_stamp() {
        assert!(db_chars_usable(100, 10, 20, Some((10, 20))));
        assert!(!db_chars_usable(0, 10, 20, Some((10, 20))), "没有值");
        assert!(!db_chars_usable(-5, 10, 20, Some((10, 20))), "负值视为无效");
        assert!(!db_chars_usable(100, 10, 20, Some((11, 20))), "size 与磁盘不一致");
        assert!(!db_chars_usable(100, 10, 20, Some((10, 21))), "mtime 与磁盘不一致");
        assert!(!db_chars_usable(100, 10, 20, None), "文件读不到时不拿旧值冒充");
    }

    #[test]
    fn file_stamp_reflects_size_and_missing_path() {
        let dir = std::env::temp_dir().join(format!("fr_stamp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, b"abc").unwrap();
        let s1 = file_stamp(p.to_str().unwrap()).expect("正常文件应有指纹");
        assert_eq!(s1.0, 3, "指纹第一项是文件大小");

        let missing = dir.join("nope.txt");
        assert!(
            file_stamp(missing.to_str().unwrap()).is_none(),
            "不存在的路径必须返回 None（调用方按不可复用处理）"
        );

        // 就地替换：内容长度变化必然改变指纹
        std::fs::write(&p, b"abcdefgh").unwrap();
        let s2 = file_stamp(p.to_str().unwrap()).unwrap();
        assert_eq!(s2.0, 8);
        assert_ne!(s1, s2, "内容变化后指纹必须不同");

        // file_mtime 与 file_stamp 同源：mtime 部分一致
        assert_eq!(file_mtime(&p), s2.1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
