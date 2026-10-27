use crate::covers;
use crate::db::{self, Db};
use crate::memlog;
use crate::models::*;
use crate::txt;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// 全书图片列表缓存条目：带文件指纹，同一路径的文件被就地替换后不再命中。
pub struct CachedImages {
    /// (size, mtime)：取自 file_stamp；None 表示取不到指纹，此时不做命中（每次都重算）。
    pub stamp: Option<(i64, i64)>,
    pub images: Arc<Vec<BookImage>>,
}

/// 全书图片列表缓存上限（按书数）。超限整体清空，实现简单且内存上限可预期。
const MAX_IMAGE_CACHE: usize = 32;

impl CachedEpub {
    /// C13：仅显式用户动作（链接跳转/全文文本等）可临时整本读取；超过 64MB 拒绝，
    /// 避免 990MB 书在非首屏路径再次瞬时读入整本。
    pub fn read_all(&self) -> Option<Vec<u8>> {
        let limit = 64 * 1024 * 1024u64;
        if std::fs::metadata(&self.path).map(|m| m.len() > limit).unwrap_or(true) {
            return None;
        }
        std::fs::read(&self.path).ok()
    }
}

pub struct CachedEpub {
    pub doc: crate::epub::EpubDoc,
    /// C13：只保留文件路径；zip 条目按需打开读取，不再常驻整本字节。
    pub path: String,
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
    /// 解析时的编码告警（MOBI 用）。随缓存保存：命中缓存时 open_book 仍要返回与冷解析一致的提示。
    pub warn: Option<String>,
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
    /// 文件监听收到的最近一批事件路径（保留最近 512 条，超出丢最旧）。
    /// watcher 循环据它把事件映射到「包含该路径的已登记目录」，只扫描受影响目录。
    pub fs_paths: Mutex<Vec<PathBuf>>,
    pub rules_ver: AtomicU64,
    pub txt_cache: Mutex<HashMap<i64, CachedText>>,
    pub epub_cache: Mutex<HashMap<i64, Arc<CachedEpub>>>,
    /// C1：EPUB 缓存的插入顺序（最旧在前），用于按字节权重淘汰。
    pub epub_cache_order: Mutex<Vec<i64>>,
    /// EPUB 字数统计缓存：key = book_id → EpubStats（带文件指纹）。
    /// 「全书总页数」（book_char_counts）与书详情/本书统计（get_book_word_count）共用同一份，
    /// 避免同一本书被完整解析两次；容量很小（4），因为一条就够几 MB 的逐章数组。
    pub epub_stats_cache: Mutex<HashMap<i64, Arc<EpubStats>>>,
    /// 已渲染章节 LRU：key = (book_id, chapter_index) → ChapterData。
    /// 避免每次 get_chapter 重开 zip/重跑正则/重复 base64 内联图片；随书切换/规则变化清理。
    pub chapter_cache: Mutex<ChapterCache>,
    /// 全书图片列表缓存：key = book_id → 带文件指纹的条目（见 CachedImages）。
    pub image_cache: Mutex<HashMap<i64, CachedImages>>,
    // staged: 通用兜底 HTTP client（见 lib.rs:220-221 的蓄意预留说明）；实际各处 HTTP 各自建 client。
    #[allow(dead_code)]
    pub http: reqwest::Client,
    /// 封面增量扫描 worker 是否在跑（进程内标记，天然不会跨崩溃残留）。
    /// 单遍：处理完全部 `meta_done=0` 即退出（扫完即停、不驻留、不轮询）。
    /// 批内并发（COVER_EXTRACT_CONCURRENCY，默认 2），批间仍按 path 字典序串行推进。
    pub scan_running: std::sync::atomic::AtomicBool,
    /// 请求 worker 尽快退出（清缓存前避免删封面目录与写封面并发；或应用退出）。
    pub scan_stop: std::sync::atomic::AtomicBool,
    /// R11-3：书库首屏就绪后置 true；就绪前的 ensure_scan_worker 一律不启动扫描。
    pub library_ready: std::sync::atomic::AtomicBool,
    /// R11-3：扫描暂停开关（进程内状态；resume 后原地继续，不重复处理已标记书籍）。
    pub scan_paused: std::sync::atomic::AtomicBool,
    /// B3：维护任务（VACUUM）互斥；与 FTS 重建/封面扫描互斥，避免长时间锁与并发写。
    pub maintenance_running: std::sync::atomic::AtomicBool,
    /// B2/B3：空闲 FTS optimize 正在执行（VACUUM 与手动重建都要避开它）。
    pub fts_optimize_running: std::sync::atomic::AtomicBool,
    /// R12-2：格式分组计数缓存（epoch, formats）；books 实际变更时 bump formats_epoch。
    pub formats_cache: Mutex<Option<(u64, Vec<String>)>>,
    pub formats_epoch: AtomicU64,
    /// worker 累计启动次数（仅诊断用）。
    pub scan_start_count: AtomicU64,
    /// 本轮扫描已处理数 / 待处理总数（进度条分子分母，worker 启动时按 DB 实测赋值）。
    pub cover_scan_done: AtomicU64,
    pub cover_scan_total: AtomicU64,
    /// 封面扫描总开关（首选项「封面扫描」）。false = **只跳过「提取封面」这一步**：
    /// 元数据/字数照旧写入、meta_done 照旧置 1 —— 否则整轮扫描会卡在缺封面的书上，
    /// 后面所有书的字数/元数据也跟着停。
    pub cover_scan_enabled: std::sync::atomic::AtomicBool,
    /// 本轮扫描启动时的 WS 基线（MB）：批开始前比较，超过基线 +50MB 就把并发降回 1。
    pub cover_scan_ws_base: AtomicU64,
    /// 封面默认底图 id 与解析规则版本，参与缓存失效（F5）。
    pub cover_style_ver: AtomicU64,
    /// 进程内 FTS 索引单实例闸门：自动索引 / 命令调用 / 全库重建共用，避免并发写出重复行。
    pub fts_index_running: AtomicBool,
    /// get_chapter 每章都要的书籍元信息缓存：book_id → (path, ext)。
    /// 命中路径完全不碰 state.db.0；删除书籍 / 移除目录 / 刷新书库后整体清空。
    pub book_meta: Mutex<HashMap<i64, (String, String)>>,
    /// 最近一次 open_book 的书 id：同书重复打开（弹出目录/面板窗）不失效章节缓存，只有真换书才整体清空。
    pub last_open_book: AtomicI64,
    /// 字数统计单飞表（P13）：book_id → 在途航班；并发调用同书统计时只有 leader 真正解析。
    pub epub_stats_flights: Mutex<HashMap<i64, Arc<Flight<Result<Arc<EpubStats>, String>>>>>,
    /// R8-1：MOBI/AZW3 图片清单进程内缓存（key=book_id，带源文件指纹，容量 4，淘汰最旧）。
    pub mobi_manifest_cache: Mutex<Vec<(i64, i64, i64, Arc<crate::mobi::ResManifest>)>>,
}

/// FTS 索引单实例闸门（RAII）：同一进程内只允许一个索引任务写库，
/// 避免自动索引与手动重建并发产生重复行。析构时复位，异常/提前 return 也不会卡住后续索引。
struct FtsIndexGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> FtsIndexGuard<'a> {
    /// 抢占闸门；已有任务在跑时返回 None（调用方按「跳过」处理）。
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self { flag })
    }
}

impl Drop for FtsIndexGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

/// 单飞航班（P13）：一个 key 的第一次调用成为 leader 执行计算，其余调用等 Condvar 复用结果。
pub struct Flight<V> {
    done: Mutex<Option<V>>,
    cv: Condvar,
}

impl<V> Default for Flight<V> {
    fn default() -> Self {
        Self { done: Mutex::new(None), cv: Condvar::new() }
    }
}

/// 单飞辅助（P13）：同 key 并发调用只让第一个执行 compute，其余等待并克隆同一结果。
/// 完成顺序：leader 写结果 → 从 map 移除 → notify_all（等待者不会拿到已删除的表项）；
/// leader panic 时同样移除表项并唤醒等待者，等待者回到循环重新竞争，不会永久挂起。
fn single_flight<V: Clone>(
    map: &Mutex<HashMap<i64, Arc<Flight<V>>>>,
    key: i64,
    compute: impl FnOnce() -> V,
) -> V {
    enum Role<V> {
        Leader,
        Follower(Arc<Flight<V>>),
    }
    loop {
        let role = {
            let mut m = map.lock().unwrap();
            match m.get(&key) {
                Some(f) => Role::Follower(f.clone()),
                None => {
                    let f = Arc::new(Flight::default());
                    m.insert(key, f.clone());
                    Role::Leader
                }
            }
        };
        match role {
            Role::Leader => {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute));
                match outcome {
                    Ok(value) => {
                        // 顺序：先写结果 → 再移除表项 → 最后唤醒。这样「写结果后、移除前」到达的
                        // 新调用仍会等到结果，不会在窗口期重复计算；等待者也不会拿到已删除的表项。
                        let flight = map.lock().unwrap().get(&key).cloned();
                        if let Some(f) = flight {
                            *f.done.lock().unwrap() = Some(value.clone());
                            map.lock().unwrap().remove(&key);
                            f.cv.notify_all();
                        }
                        return value;
                    }
                    Err(payload) => {
                        let flight = map.lock().unwrap().remove(&key);
                        if let Some(f) = flight {
                            f.cv.notify_all();
                        }
                        std::panic::resume_unwind(payload);
                    }
                }
            }
            Role::Follower(f) => loop {
                let mut done = f.done.lock().unwrap();
                if let Some(v) = done.as_ref() {
                    return v.clone();
                }
                // wait_timeout：即使 leader panic 时恰好错过 notify，也能定期复查表项，避免挂死。
                let (guard, _timeout) = f
                    .cv
                    .wait_timeout(done, std::time::Duration::from_millis(20))
                    .unwrap();
                done = guard;
                if let Some(v) = done.as_ref() {
                    return v.clone();
                }
                drop(done);
                // 判据必须是「表里当前航班是否仍是我等待的同一个 Arc」：leader panic 移除表项后，
                // 新调用可能已插入同 key 的新 Flight；只看 contains_key 会误以为仍是旧航班而继续
                // 等一个再也不会被唤醒的 Arc（线程挂起）。
                let still_ours = map
                    .lock()
                    .unwrap()
                    .get(&key)
                    .map_or(false, |cur| Arc::ptr_eq(cur, &f));
                if !still_ours {
                    break; // 回外层重新竞争（成为新 leader 或跟新航班）
                }
            },
        }
    }
}

/// 简单的按书维度 LRU 章节缓存：容量按条数限制，随书切换/规则变化整体失效。
/// 用 Vec 记录访问顺序（章节数不大，线性查找成本可忽略），命中即移到末尾。
pub struct ChapterCache {
    /// S1：值里带源文件指纹——同路径文件被就地替换后，旧正文不再命中（key 只有 book/idx/keep_styles）。
    map: HashMap<(i64, i64, bool), (Option<(i64, i64)>, Arc<ChapterData>)>,
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

    /// S1：命中必须同时校验文件指纹；不匹配则移除该条并按未命中处理。
    pub fn get(
        &mut self,
        key: (i64, i64, bool),
        stamp: Option<(i64, i64)>,
    ) -> Option<Arc<ChapterData>> {
        let (cached_stamp, val) = self.map.get(&key)?.clone();
        if !stamp_matches(cached_stamp, stamp) {
            self.map.remove(&key);
            self.order.retain(|k| *k != key);
            return None;
        }
        self.touch(key);
        Some(val)
    }

    pub fn put(&mut self, key: (i64, i64, bool), stamp: Option<(i64, i64)>, val: Arc<ChapterData>) {
        self.map.insert(key, (stamp, val));
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

const EXT_BOOK: &[&str] = &["txt", "epub", "mobi", "azw3", "prc", "pdf"];

/// 设置项键名：PDF 页数（`pdf.pages.<book_id>`）。Rust 不解析 PDF，页数只由前端
/// 用 pdf.js 解析后上报；后端只负责按它生成页码目录。集中此处避免键名漂移。
pub fn pdf_pages_key(book_id: i64) -> String {
    format!("pdf.pages.{book_id}")
}

/// PDF 页码目录：**一个 PDF 页 = 一个章节**（页码即 chapter_index）。
/// 页数只来自前端上报的设置项（后端不解析 PDF）；缺失/非法时返回空目录，
/// 前端用 pdf.js 解析出真实页数后会重建目录并回写该设置项。
fn pdf_page_toc(state: &AppState, book_id: i64) -> Vec<TocEntry> {
    let pages = {
        let conn = state.db.0.lock().unwrap();
        db::get_setting(&conn, &pdf_pages_key(book_id))
            .and_then(|v| v.trim().parse::<i64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(0)
    };
    (0..pages)
        .map(|i| TocEntry {
            title: format!("第 {} 页", i + 1),
            index: i as usize,
            chars: 0,
        })
        .collect()
}

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

pub(crate) fn file_stamp(path: &str) -> Option<(i64, i64)> {
    file_stamp_of(Path::new(path))
}

/// 缓存里的文件指纹是否仍然可用：两侧都得有值且完全一致。
/// 缓存侧缺失 = 当初就没拿到指纹（这种条目不该入缓存）；当前侧缺失 = 现在读不到文件。
/// 两种情况都不复用，绝不把「没有指纹」当成永远有效。
pub(crate) fn stamp_matches(cached: Option<(i64, i64)>, current: Option<(i64, i64)>) -> bool {
    matches!((cached, current), (Some(a), Some(b)) if a == b)
}

/// 解析缓存条目是否可复用：规则版本一致，且两侧文件指纹都有值且相等。
/// 抽成纯函数便于单测；load_txt / load_mobi / get_chapter 的内联快路径共用同一判据，避免漂移。
fn cache_entry_valid(
    entry_ver: u64,
    ver: u64,
    entry_stamp: Option<(i64, i64)>,
    file_stamp: Option<(i64, i64)>,
) -> bool {
    entry_ver == ver && stamp_matches(entry_stamp, file_stamp)
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

/// DB 里持久化的解析缓存（TXT 编码判定 / EPUB 字数统计）与磁盘文件是否仍匹配：
/// 行上 size+mtime 必须与当前指纹完全一致；取不到当前指纹一律不复用。
/// 抽成纯函数便于单测（与 db_chars_usable 同理，防「同路径文件被就地替换」后读到旧缓存）。
fn db_cache_stamp_usable(row_size: i64, row_mtime: i64, disk: Option<(i64, i64)>) -> bool {
    matches!(disk, Some((size, mtime)) if size == row_size && mtime == row_mtime)
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

/// 单次扫描自动删除的比例阈值：超过「登记数的 5%」或「200 条」即拒绝自动删除。
const STALE_AUTO_DELETE_MIN: usize = 200;
const STALE_AUTO_DELETE_PCT: usize = 5;

/// 失效文件删除的决策结果（纯数据，便于单测）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StalePlan {
    /// 连续两次都判缺失 → 本轮真删
    pub delete: Vec<String>,
    /// 首次判缺失 → 记进待确认
    pub pending: Vec<String>,
    /// 因所在目录本轮没走完而不参与判定的失效路径
    pub skipped: Vec<String>,
    /// 待确认里本轮重新出现的（文件回来了，要清掉登记）
    pub revived: Vec<String>,
    /// 规模超阈值：拒绝自动删除（全部转为待确认）
    pub refused: bool,
    pub limit: usize,
}

/// 失效文件判定的纯逻辑：两段确认 + 目录未走完跳过 + 规模阈值。
/// 为什么三件事必须一起做：目录被临时改名/不可读时，一次判定就会把整目录的书行删光。
pub(crate) fn plan_stale(
    stale_all: Vec<String>,
    pending: &std::collections::HashSet<String>,
    skip_folders: &[String],
    registered_in_scope: usize,
) -> StalePlan {
    let mut skipped = Vec::new();
    let mut stale = Vec::new();
    for path in stale_all {
        if skip_folders
            .iter()
            .any(|f| std::path::Path::new(&path).starts_with(std::path::Path::new(f)))
        {
            skipped.push(path);
        } else {
            stale.push(path);
        }
    }
    let limit = std::cmp::max(
        STALE_AUTO_DELETE_MIN,
        registered_in_scope * STALE_AUTO_DELETE_PCT / 100,
    );
    let refused = stale.len() > limit;
    let mut delete = Vec::new();
    let mut pending_now = Vec::new();
    for path in stale.iter() {
        if pending.contains(path) {
            delete.push(path.clone());
        } else {
            pending_now.push(path.clone());
        }
    }
    if refused {
        pending_now.append(&mut delete);
    }
    let stale_set: std::collections::HashSet<&String> = stale.iter().collect();
    let revived: Vec<String> = pending
        .iter()
        .filter(|p| !stale_set.contains(p))
        .cloned()
        .collect();
    StalePlan {
        delete,
        pending: pending_now,
        skipped,
        revived,
        refused,
        limit,
    }
}

/// 进程级扫描互斥：**串行化全部扫描**（rescan / add_folder / 文件监听 / WebDAV 导入）。
/// 不加锁时两轮扫描会并发：后完成的一轮读到前一轮刚写入 `stale_pending` 的条目，
/// 会把它当成「上一次已确认缺失」直接删书，绕过两段确认保护。
static SCAN_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 文件事件时间基准：用**单调时钟**而不是系统墙钟。
/// 系统时间回拨时墙钟差值会变成负数/长时间不满足静默判定，自动扫描会停。
static EVENT_CLOCK_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// 进程启动以来的单调毫秒数（文件事件回调与监听循环共用同一基准）。
fn event_now_ms() -> u64 {
    EVENT_CLOCK_START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// 发一帧目录扫描进度（仅作用域扫描使用；`phase` = walk / diff / insert / done）。
fn emit_folder_scan_progress(
    app: &AppHandle,
    origin: &str,
    phase: &str,
    scanned: usize,
    added: usize,
    running: bool,
) {
    app.emit(
        "folder-scan-progress",
        serde_json::json!({
            "running": running,
            "phase": phase,
            "scanned": scanned,
            "added": added,
            "origin": origin,
        }),
    )
    .ok();
}

/// 把一批文件路径映射到「包含它的已登记目录」（最长前缀），去重保序。
/// 用于文件监听与 WebDAV 导入：只扫描受影响的已登记目录，不做全量遍历。
fn registered_dirs_for_paths<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
    folders: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        let hit = folders
            .iter()
            .filter(|f| path.starts_with(Path::new(f.as_str())))
            .max_by_key(|f| f.len());
        if let Some(f) = hit {
            if !out.iter().any(|x| x == f) {
                out.push(f.clone());
            }
        }
    }
    out
}

/// 扫描来源：**只有 `user` 才允许弹提示**；`timer` 一律静默，
/// 异常态改由书库页首屏常驻提示条呈现（数据安全信号不能因为静默而消失）。
/// 必须由调用方显式传入，不能从 `progress_scope` 推导：`rescan`（用户动作）传的也是 None。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanOrigin {
    Timer,
    User,
}

impl ScanOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            ScanOrigin::Timer => "timer",
            ScanOrigin::User => "user",
        }
    }
}

/// `last_scan_stale` 的 DB 键：**提示条的唯一来源**。
/// 与 `stale_pending_*` 的分工（不得互相推导）：
/// - `stale_pending`（db.rs）只存"待确认删除"的 path，供清理逻辑判定哪些书可删；
///   `plan_stale` 已把 `skip_folders` 下的路径剔进 `plan.skipped`、不进 `plan.pending`，
///   所以「目录没走完」在它里面一条都看不到。
/// - 本键每次扫描**结束时重算并写入**完整判定（refused / unfinishedDirs / pending / skipped /
///   主要未完成目录 / 时间），是提示条读的唯一状态；从 `stale_pending` 读会得到
///   「目录改名时提示条显示一切正常」的错误结论。
pub const LAST_SCAN_STALE_KEY: &str = "last_scan_stale";

/// 扫描入口写"扫描中"（清掉上一次的结论，异常中断不会残留旧状态），结束写最终态。
fn write_scan_state(conn: &rusqlite::Connection, v: serde_json::Value) {
    db::set_setting(conn, LAST_SCAN_STALE_KEY, &v.to_string());
}

pub async fn scan_folders(state: &AppState, progress_scope: Option<Vec<String>>, origin: ScanOrigin) {
    // 串行化全部扫描：并发扫描会互相误判 `stale_pending`（见 SCAN_GUARD 注释）。
    let _scan_guard = SCAN_GUARD.lock().await;
    let origin_str = origin.as_str().to_string();
    let full_scan = progress_scope.is_none();
    // 两段式第一段：**全量扫描**进入即写入"扫描中"（清掉上一轮结论，异常中断不会残留旧状态）。
    // 作用域扫描只覆盖部分目录，不能把上一轮的全量结论清掉，故不预写。
    if full_scan {
        let conn = state.db.0.lock().unwrap();
        write_scan_state(
            &conn,
            serde_json::json!({ "state": "scanning", "origin": origin_str, "at": db::now_ms() }),
        );
    }
    let folders = {
        let conn = state.db.0.lock().unwrap();
        db::list_folders(&conn)
    };
    // 本次实际遍历的目录：Some(scope) 时只保留其中确实已登记的路径（去重、保序）；
    // None 时等于全部已登记目录（全量扫描，行为与旧版一致）。
    let walked_folders: Vec<String> = match progress_scope.as_ref() {
        Some(scope) => {
            let mut out: Vec<String> = Vec::new();
            for s in scope {
                if folders.iter().any(|f| f == s) && !out.contains(s) {
                    out.push(s.clone());
                }
            }
            out
        }
        None => folders,
    };
    // 嵌套登记目录去重：扫描只走最外层已登记目录，避免父目录与子目录被重复遍历。
    // stale/registered/pending 的作用域仍用 walked_folders（判定要覆盖全部登记路径）。
    let effective_walk: Vec<String> = walked_folders
        .iter()
        .filter(|f| {
            !walked_folders.iter().any(|other| {
                other != *f && Path::new(f.as_str()).starts_with(Path::new(other.as_str()))
            })
        })
        .cloned()
        .collect();
    let progress_on = progress_scope.is_some();
    memlog::stage("scan_folders:start");
    // ---- 阶段一：无锁遍历目录，收集 (path → size, mtime, title, ext) ----
    let prog_app = state.app.clone();
    let origin_inner = origin_str.clone();
    // 遍历在阻塞线程池里执行，需要把目录列表移进闭包；只遍历最外层目录（effective_walk）。
    let walk_folders = effective_walk.clone();
    // 返回四元组：found / scanned / 每目录命中文件数 / 遍历出错的目录与原因
    let (found, scanned, per_folder, walk_bad): (
        HashMap<String, (i64, i64, String, String)>,
        usize,
        HashMap<String, usize>,
        Vec<(String, String)>,
    ) = tauri::async_runtime::spawn_blocking(move || {
            let mut found: HashMap<String, (i64, i64, String, String)> = HashMap::new();
            let mut per_folder: HashMap<String, usize> = HashMap::new();
            let mut walk_bad: Vec<(String, String)> = Vec::new();
            // scanned = 本次已遍历的文件数（遍历范围只有 walked_folders）。
            let mut scanned: usize = 0;
            let emit = |phase: &str, scanned: usize, added: usize, running: bool| {
                emit_folder_scan_progress(&prog_app, &origin_inner, phase, scanned, added, running);
            };
            if progress_on {
                emit("walk", 0, 0, true);
            }
            let mut last_emit = std::time::Instant::now();
            for folder in &walk_folders {
                let mut folder_files: usize = 0;
                let mut walk_err: Option<String> = None;
                for entry in walkdir::WalkDir::new(folder).follow_links(false).into_iter() {
                    // 遍历出错（权限/目录被改名/IO）→ 记下来：该目录本轮不参与 stale 判定
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            walk_err.get_or_insert_with(|| e.to_string());
                            continue;
                        }
                    };
                    let path = entry.path().to_string_lossy().into_owned();
                    if entry.file_type().is_file() {
                        scanned += 1;
                        if let Some(ext) = ext_of(&path) {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as i64;
                            let mtime = file_mtime(entry.path());
                            let title = quick_title(&path);
                            found.insert(path, (size, mtime, title, ext));
                            folder_files += 1;
                        }
                    }

                    // 心跳：每 200 个文件或至少每 ~300ms 发一帧（即使文件数没到 200）。
                    if progress_on
                        && (last_emit.elapsed().as_millis() >= 300 || (scanned > 0 && scanned % 200 == 0))
                    {
                        last_emit = std::time::Instant::now();
                        emit("walk", scanned, 0, true);
                    }
                }
                per_folder.insert(folder.clone(), folder_files);
                if let Some(msg) = walk_err {
                    walk_bad.push((folder.clone(), msg));
                }
            }
            (found, scanned, per_folder, walk_bad)
        })
        .await
        .unwrap_or_default();

    memlog::stage_note("scan_folders:walked", &format!("found={}", found.len()));
    // 遍历结束：即将进入差集/失效判定，前端底栏切到「正在比对书库」。
    if progress_on {
        emit_folder_scan_progress(&state.app, &origin_str, "diff", scanned, 0, true);
    }
    // ---- 阶段二：一次性取锁读 books 的路径元信息与 ignored 集合，立即释放锁 ----
    // existing: path -> (id, size, mtime)；ignored: 「移出书库」的长期忽略路径。
    let (existing, ignored) = {
        let conn = state.db.0.lock().unwrap();
        (db::all_paths_meta(&conn), db::ignored_paths_all(&conn))
    };

    // ---- 阶段三：无锁求差集 ----
    // 新增文件 = 磁盘有、库中没有；被「移出书库」的路径长期忽略，不导回。
    let new_files: Vec<(String, i64, i64, String, String)> = found
        .iter()
        .filter(|(p, _)| !existing.contains_key(p.as_str()) && !ignored.contains(p.as_str()))
        .map(|(p, (s, m, t, e))| (p.clone(), *s, *m, t.clone(), e.clone()))
        .collect();
    // 存量文件指纹变化：同一路径但 size/mtime 与库中不同（同名替换）。
    // 只刷新指纹并重跑封面/文本，不当作新增重复插入。
    let changed_files: Vec<(i64, i64, i64)> = found
        .iter()
        .filter_map(|(p, (size, mtime, _, _))| {
            existing.get(p.as_str()).and_then(|(id, db_size, db_mtime)| {
                if db_size != size || db_mtime != mtime {
                    Some((*id, *size, *mtime))
                } else {
                    None
                }
            })
        })
        .collect();
    // 失效文件 = existing 中位于**本次 walked 目录**、但磁盘上已不存在（不在 found）的。
    // 必须收窄到 walked：作用域扫描没遍历其它目录，不能据此判定它们缺失。
    let stale_all: Vec<String> = existing
        .keys()
        .filter(|p| {
            !found.contains_key(p.as_str())
                && walked_folders
                    .iter()
                    .any(|f| std::path::Path::new(p).starts_with(std::path::Path::new(f)))
        })
        .cloned()
        .collect();

    // 每目录登记数：既用于「本轮该目录结果为空」的判定，也用于规模阈值的基数。
    // 只统计 walked 目录下的登记数（阈值基数与本次扫描范围同口径）。
    let mut registered_per_folder: HashMap<String, usize> = HashMap::new();
    for path in existing.keys() {
        if let Some(f) = walked_folders
            .iter()
            .find(|f| std::path::Path::new(path).starts_with(std::path::Path::new(f)))
        {
            *registered_per_folder.entry(f.clone()).or_insert(0) += 1;
        }
    }
    let registered_in_scope: usize = registered_per_folder.values().sum();
    // 未走完的目录：遍历出错，或本轮 0 个文件而库里登记着 —— 目录被改名/暂时不可读都长这样。
    // 这类目录本轮**不参与 stale 判定**，否则一次读不到就把整目录的书行删光。
    let mut skip_folders: Vec<String> = Vec::new();
    for (f, why) in walk_bad.iter() {
        memlog::stage_note(
            "scan_folders:unfinished",
            &format!("dir={f} why=walk-error: {why} 未走完，跳过 stale 判定"),
        );
        skip_folders.push(f.clone());
    }
    for f in effective_walk.iter() {
        let hit = per_folder.get(f).copied().unwrap_or(0);
        let reg = registered_per_folder.get(f).copied().unwrap_or(0);
        if hit == 0 && reg > 0 && !skip_folders.contains(f) {
            memlog::stage_note(
                "scan_folders:unfinished",
                &format!("dir={f} why=walked-empty registered={reg} 未走完，跳过 stale 判定"),
            );
            skip_folders.push(f.clone());
        }
    }
    let (pending_prev_all, books_before) = {
        let conn = state.db.0.lock().unwrap();
        (db::stale_pending_all(&conn), db::count_books(&conn))
    };
    // 两段确认的记忆也按扫描范围收窄：作用域扫描只判定本次 walked 目录下的待确认条目，
    // 不能把其它目录的 pending 当成「文件已回来」清掉（否则它们会一直循环在首次判定）。
    // 全量扫描保持旧行为：walked = 全部已登记目录，越界的 pending 一并清理。
    let pending_prev: std::collections::HashSet<String> = if full_scan {
        pending_prev_all
    } else {
        pending_prev_all
            .into_iter()
            .filter(|p| {
                walked_folders
                    .iter()
                    .any(|f| Path::new(p.as_str()).starts_with(Path::new(f)))
            })
            .collect()
    };
    let plan = plan_stale(stale_all, &pending_prev, &skip_folders, registered_in_scope);
    let stale_total = plan.pending.len() + plan.delete.len() + plan.skipped.len();
    // 目录分布：只看本轮将删/待确认，便于事后追
    let mut by_folder: HashMap<String, usize> = HashMap::new();
    for path in plan.pending.iter().chain(plan.delete.iter()) {
        if let Some(f) = walked_folders
            .iter()
            .find(|f| std::path::Path::new(path).starts_with(std::path::Path::new(f)))
        {
            *by_folder.entry(f.clone()).or_insert(0) += 1;
        }
    }
    memlog::stage_note(
        "scan_folders:diff",
        &format!(
            "existing={} new={} changed={} stale={} pending={} delete={} skipped={} refused={} limit={} books_before={}",
            existing.len(),
            new_files.len(),
            changed_files.len(),
            stale_total,
            plan.pending.len(),
            plan.delete.len(),
            plan.skipped.len(),
            plan.refused,
            plan.limit,
            books_before
        ),
    );
    for (f, n) in by_folder.iter() {
        memlog::stage_note("scan_folders:stale_by_dir", &format!("dir={f} n={n}"));
    }
    for path in plan.delete.iter().take(20) {
        memlog::stage_note("scan_folders:stale_del_sample", path);
    }
    if plan.refused {
        memlog::stage_note(
            "scan_folders:STALE_REFUSED",
            &format!(
                "count={} limit={} 只登记不删除（可能是目录被改名/暂时不可读）",
                plan.pending.len(),
                plan.limit
            ),
        );
    }
    {
        let conn = state.db.0.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = db::stale_pending_put(&conn, &plan.pending, now);
        let _ = db::stale_pending_drop(&conn, &plan.revived);
    }
    // ---- 阶段四：分批写入，每 2000 条一个事务，批间释放锁并 yield_now() ----
    let mut changed = false;
    for chunk in plan.delete.chunks(2000) {
        // 删除前先收集可再生资源（封面/res）：行删掉后就查不到了。
        let artifacts = {
            let conn = state.db.0.lock().unwrap();
            db::book_artifacts_by_paths(&conn, chunk)
        };
        let r: Result<(), String> = (|| {
            let mut guard = state.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            db::delete_books(&tx, chunk)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })();
        if r.is_ok() {
            changed = true;
            cleanup_book_artifacts_for(&state.app, &artifacts);
            // 扫描确认删除的书籍也要让元信息缓存失效（否则 get_chapter 会拿旧 path 当成书还在）。
            state.book_meta.lock().unwrap().clear();
        } else if let Err(e) = r {
            eprintln!("[scan] delete_books tx failed: {e}");
        }
        tokio::task::yield_now().await;
    }
    // 存量指纹刷新：每 2000 条一个独立事务，批间释放锁并 yield_now。
    for chunk in changed_files.chunks(2000) {
        let r: Result<(), String> = (|| {
            let mut guard = state.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            for (id, size, mtime) in chunk {
                db::refresh_book_fingerprint(&tx, *id, *size, *mtime)
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })();
        match r {
            Ok(()) => changed = true,
            Err(e) => eprintln!("[scan] refresh fingerprint tx failed: {e}"),
        }
        tokio::task::yield_now().await;
    }
    if !plan.delete.is_empty() {
        let conn = state.db.0.lock().unwrap();
        let _ = db::stale_pending_drop(&conn, &plan.delete);
    }

    // 两段式第二段：把本轮最终判定写进 `last_scan_stale`（提示条的唯一来源），
    // 并在同一份数据上决定要不要发 `folder-scan-stale`（事件里带 origin）。
    let stale = plan.refused || !skip_folders.is_empty();
    let top_dirs: Vec<String> = skip_folders.iter().take(3).map(|d| d.to_string()).collect();
    // - 全量扫描结束时照旧重算并写入（含恢复正常，提示条据此消失）；
    // - 作用域扫描只覆盖部分目录，仅在本次确实产生异常判定时写入，否则保留上一轮结论，
    //   避免一次局部扫描把全局异常态覆盖成「正常」。
    if full_scan || stale {
        let conn = state.db.0.lock().unwrap();
        let payload = serde_json::json!({
            "state": if stale { "stale" } else { "ok" },
            "origin": origin_str,
            "at": db::now_ms(),
            "refused": plan.refused,
            "pending": plan.pending.len(),
            "skipped": plan.skipped.len(),
            "limit": plan.limit,
            "unfinishedDirs": skip_folders.len(),
            "dirs": top_dirs,
        });
        write_scan_state(&conn, payload.clone());
        // `scan-stale-state` 与写入条件一致：全量扫描每次广播（含恢复），作用域扫描只在异常时广播。
        state.app.emit("scan-stale-state", payload).ok();
    }
    if stale {
        state
            .app
            .emit(
                "folder-scan-stale",
                serde_json::json!({
                    "refused": plan.refused,
                    "pending": plan.pending.len(),
                    "skipped": plan.skipped.len(),
                    "limit": plan.limit,
                    "unfinishedDirs": skip_folders.len(),
                    "dirs": top_dirs,
                    "origin": origin_str,
                }),
            )
            .ok();
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
        // 入库每批完成：前端底栏切到「正在写入书库」并显示累计已写入本数。
        if progress_on {
            emit_folder_scan_progress(&state.app, &origin_str, "insert", scanned, new_ids.len(), true);
        }
        tokio::task::yield_now().await;
    }

    // 留痕：新增插入完成后再数一次 books（原位置在插入前，after 不含本轮新书）。
    {
        let conn = state.db.0.lock().unwrap();
        let after = db::count_books(&conn);
        memlog::stage_note(
            "scan_folders:books",
            &format!(
                "before={books_before} after={after} added={} deleted={}",
                (after - books_before).max(0),
                (books_before - after).max(0)
            ),
        );
    }
    if changed {
        bump_formats_epoch(state);
        state.app.emit("library-changed", ()).ok();
    }
    // 新书或存量指纹变化都会产生 meta_done=0，拉起增量扫描（若空闲）。
    if !new_ids.is_empty() || !changed_files.is_empty() {
        ensure_scan_worker(&state.app);
    }
    // 扫描结束发 running:false，并带上本次新增书籍数（供前端提示）。
    if progress_on {
        state
            .app
            .emit(
                "folder-scan-progress",
                serde_json::json!({ "running": false, "phase": "done", "scanned": scanned, "added": new_files.len(), "origin": origin_str }),
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
    let mut watcher = match notify::recommended_watcher(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(ev) = res {
                // 单调时钟：系统墙钟回拨不会让自动扫描停掉。
                state2.last_fs_event.store(event_now_ms(), Ordering::Relaxed);
                let mut paths = state2.fs_paths.lock().unwrap();
                paths.extend(ev.paths);
                // 只保留最近 512 条：事件密集时避免无界增长。
                if paths.len() > 512 {
                    let drop_n = paths.len() - 512;
                    paths.drain(0..drop_n);
                }
            }
        },
    ) {
        Ok(w) => Some(w),
        Err(e) => {
            // 创建失败不再静默：没有监听就没有自动扫描，事后日志要能看出来。
            eprintln!("[watcher] create failed: {e}");
            None
        }
    };
    if let Some(w) = watcher.as_mut() {
        for f in &folders {
            if let Err(e) =
                notify::Watcher::watch(w, Path::new(f), notify::RecursiveMode::Recursive)
            {
                // 逐目录注册失败同样不能吞掉（外置盘/网络盘/权限受限目录最常见）。
                eprintln!("[watcher] watch failed dir={f} err={e}");
            }
        }
    }
    *state.watcher.lock().unwrap() = watcher;
}

pub fn spawn_watcher_loop(app: AppHandle) {
    std::thread::spawn(move || {
        // 事件先后与静默判定全部用单调时钟（系统时间回拨不影响）。
        let mut last_handled: u64 = event_now_ms();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(2000));
            let state = match app.try_state::<Arc<AppState>>() {
                Some(s) => s.inner().clone(),
                None => continue,
            };
            let last = state.last_fs_event.load(Ordering::Relaxed);
            // 静默判定（2.5 秒无新事件）保持现状。
            if last > last_handled && event_now_ms().saturating_sub(last) > 2500 {
                last_handled = last;
                // 取出并清空本批事件路径（锁内只做搬移，不读盘）。
                let paths: Vec<PathBuf> = {
                    let mut buf = state.fs_paths.lock().unwrap();
                    std::mem::take(&mut *buf)
                };
                let folders = {
                    let conn = state.db.0.lock().unwrap();
                    db::list_folders(&conn)
                };
                // 对每个路径找包含它的已登记目录（最长前缀）并去重。
                let dirs = registered_dirs_for_paths(paths.iter().map(|p| p.as_path()), &folders);
                // 映射不到任何已登记目录（非书库路径）→ 跳过，绝不回落成全量扫描。
                if dirs.is_empty() {
                    continue;
                }
                tauri::async_runtime::block_on(scan_folders(&state, Some(dirs), ScanOrigin::Timer));
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
    // 崩溃保护触发时进程内不会再清除该标记：直接不创建工作线程，杜绝「跳过 → 释放 → 复查 → 再拉起」自转链。
    if memlog::skip_cover_rebuild() {
        return;
    }
    // R11-3：首屏就绪前的启动调用一律不启动（library_ready 之后才会再拉起）。
    if let Some(st) = app.try_state::<Arc<AppState>>() {
        if !st.library_ready.load(Ordering::SeqCst) {
            return;
        }
    }
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

        let ran = on_startup(state.clone());

        // 释放单例标记（Release：on_startup 期间的写对随后的复查/新 worker 可见）。
        state.scan_running.store(false, Ordering::Release);

        // 主动停止（清缓存/退出）时不再复查、不再自拉起。
        if state.scan_stop.load(Ordering::Acquire) {
            return;
        }
        // 本轮被崩溃保护跳过：不复查、不自拉起，否则会形成永不停止的线程自转链。
        if !ran {
            memlog::stage_note("scan:skip-no-respawn", "本轮封面扫描被跳过，不再自动重试");
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

/// 增量扫描主循环（单遍、DB 驱动；批内并发 2、批间按 path 字典序顺序推进）。设计见实现提示词 A/B 节：
/// - 开头回收崩溃残留（meta_done=3 → 0）（B4）；
/// - 每一本处理前先查优先表，命中优先处理（B3）；
/// - 否则分批 `SELECT ... WHERE meta_done=0 ORDER BY path, id LIMIT 200`（A2；走 idx_books_meta_done_path，
///   同一目录的书在批内相邻 —— 机械盘/外置盘上减少目录跳跃）；
/// - 每本：重读状态(B6) → CAS 认领(0→3, B5) → 解析(不持 DB 锁) → 独立事务写 1/2 + 删优先表(A3/B5)；
/// - 批间 sleep 80ms；每 1000 次提交 wal_checkpoint(PASSIVE)；整轮结束 TRUNCATE（A7）；
/// - 每批比较前后 `COUNT(meta_done=0)`，未下降则打印并终止（A6）；
/// - 每本发 `cover-updated`{id}（B7），进度节流发 `meta-progress`。
/// 返回值：本轮是否真正执行了扫描（被崩溃保护跳过时返回 false，调用方据此不再自动重启 worker）。
fn on_startup(state: Arc<AppState>) -> bool {
    // 连续启动失败保护：上一次启动没走到书库 → 本轮不做封面重建（只记日志）。
    // 为什么整轮跳过而不是缩小批量：封面重建本身没错，错的是「在启动路径上做全库封面」——
    // 崩溃恢复优先，让用户先能进书库，下一轮（成功一次后）再继续补。
    if memlog::skip_cover_rebuild() {
        memlog::stage_note("cover:skipped", "previous start crashed before library");
        eprintln!("[scan] skipped this run: previous start did not reach library");
        return false;
    }
    memlog::stage("cover:start");
    let covers_dir = covers::covers_dir(&state.app);
    const BATCH: i64 = 200;
    const EMIT_EVERY: u64 = 16;
    const CKPT_EVERY: u64 = 1000;
    // 本轮的 WS 基线：并发闸门用它判断「是否已超出 基线+50MB」。
    state
        .cover_scan_ws_base
        .store(memlog::ws_mb().max(1), Ordering::SeqCst);

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

    let commits = std::sync::atomic::AtomicU64::new(0);
    let since_emit = std::sync::atomic::AtomicU64::new(0);
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
        // 手动模式：只跳过「提取封面」这一步 —— 仍照常把 meta_done 置 1（元数据/字数流程不变），
        // 也**不碰 cover_path**（已存在的封面不能被这轮扫描抹掉），更不打开文件。
        if !state.cover_scan_enabled.load(Ordering::SeqCst) {
            let wrote = {
                let conn = state.db.0.lock().unwrap();
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
                    // MOBI 定位读取与大图解码不响应取消：线程可能继续跑完，实际并发可能超过配额。
                    eprintln!("[scan] cover timeout id={id}: watchdog fired, worker may keep running");
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
    let drain_one_priority = |commits: &std::sync::atomic::AtomicU64,
                              since_emit: &std::sync::atomic::AtomicU64|
     -> bool {
        let hit = {
            let conn = state.db.0.lock().unwrap();
            db::next_priority(&conn)
        };
        match hit {
            Some((id, path, format)) => {
                if process_one(id, &path, &format, &[0, 2]).is_some() {
                    commits.fetch_add(1, Ordering::SeqCst);
                    let done = state.cover_scan_done.fetch_add(1, Ordering::SeqCst) + 1;
                    let n = since_emit.fetch_add(1, Ordering::SeqCst) + 1;
                    if n >= EMIT_EVERY {
                        since_emit.store(0, Ordering::SeqCst);
                        let total = state.cover_scan_total.load(Ordering::SeqCst);
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
        // R11-3：暂停时原地等待（150ms 轮询 scan_stop），resume 后从当前批次继续。
        while state.scan_paused.load(Ordering::SeqCst) && !state.scan_stop.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        if state.scan_stop.load(Ordering::SeqCst) {
            break;
        }
        // B3：每一本处理前先查优先表；命中则先把优先项清空。
        while drain_one_priority(&commits, &since_emit) {
            if state.scan_stop.load(Ordering::SeqCst) {
                break 'outer;
            }
            if commits.load(Ordering::Relaxed) % CKPT_EVERY == 0 && commits.load(Ordering::Relaxed) > 0 {
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
            if drain_one_priority(&commits, &since_emit) {
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

        // 顺序批流 + 并发：批内用 `conc` 个线程分摊（默认 2）。顺序仍由 `path` 决定，
        // 并发只是让 I/O 等待重叠；插队流在**每本之前**照样优先（线程内各自查优先表）。
        // 内存闸门：批开始前若 WS 已超过「基线 + 50MB」，本批退回并发 1。
        let conc = {
            let want = COVER_EXTRACT_CONCURRENCY;
            let base = state.cover_scan_ws_base.load(Ordering::SeqCst);
            let ws = memlog::ws_mb();
            if base > 0 && ws > base + 50 {
                memlog::stage_note("cover:concurrency", &format!("degraded=1 ws={ws} base={base}"));
                1
            } else {
                want
            }
        };
        memlog::stage_note("cover:concurrency", &format!("conc={conc} batch={}", batch.len()));
        {
            let cursor = std::sync::atomic::AtomicUsize::new(0);
            let batch_ref = &batch;
            std::thread::scope(|s| {
                for _ in 0..conc.min(batch_ref.len().max(1)) {
                    s.spawn(|| loop {
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= batch_ref.len() || state.scan_stop.load(Ordering::SeqCst) {
                            break;
                        }
                        while state.scan_paused.load(Ordering::SeqCst)
                            && !state.scan_stop.load(Ordering::SeqCst)
                        {
                            std::thread::sleep(std::time::Duration::from_millis(150));
                        }
                        if state.scan_stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let (id, path, format) = &batch_ref[i];
                        // B3：每本之前再查优先表（打开书籍可随时插队）。
                        while drain_one_priority(&commits, &since_emit) {}
                        if process_one(*id, path, format, &[0]).is_some() {
                            let c = commits.fetch_add(1, Ordering::SeqCst) + 1;
                            let done = state.cover_scan_done.fetch_add(1, Ordering::SeqCst) + 1;
                            let emit_n = since_emit.fetch_add(1, Ordering::SeqCst) + 1;
                            if emit_n >= EMIT_EVERY {
                                since_emit.store(0, Ordering::SeqCst);
                                let total = state.cover_scan_total.load(Ordering::SeqCst);
                                emit_scan_progress(&state.app, done, total);
                                state.app.emit("library-meta", ()).ok();
                            }
                            if c % CKPT_EVERY == 0 {
                                let conn = state.db.0.lock().unwrap();
                                let _ = conn.pragma_update(None, "wal_checkpoint", "PASSIVE");
                            }
                        }
                    });
                }
            });
        }
        // 批结束后再处理一轮优先表：线程退出前可能刚好有新插队项（避免它等下批）。
        while drain_one_priority(&commits, &since_emit) {}

        // 每批结束时释放批内中间对象（batch / cur_batch_ids 出作用域即 drop），
        // 并打一条「批次 + 工作集」日志：封面重建的内存必须停在「一本位图」量级。
        drop(batch);
        memlog::stage_note(
            "cover:batch",
            &format!(
                "commits={} last_batch={} ",
                commits.load(Ordering::Relaxed),
                last_batch_ids.len()
            ),
        );
        // A7：批间限流，避免满速占盘。
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    memlog::stage("cover:done");

    // 整轮结束：TRUNCATE 收缩 WAL；发最终进度 + scan-finished。
    {
        let conn = state.db.0.lock().unwrap();
        let _ = conn.pragma_update(None, "wal_checkpoint", "TRUNCATE");
    }
    // 最终帧收敛分母：轮内可能有书被删除/移出或状态被并发改写，或 A6 提前终止，
    // 旧 total0 会让进度永远到不了 100%。以「已处理 + 当前剩余待处理」作为最终 total。
    let remaining = {
        let conn = state.db.0.lock().unwrap();
        db::count_meta_pending(&conn)
    } as u64;
    let done = state.cover_scan_done.load(Ordering::SeqCst);
    let total = done + remaining;
    state.cover_scan_total.store(total, Ordering::SeqCst);
    emit_scan_progress(&state.app, done, total);
    // 空轮/no-op 不触发前端可见区刷新与优先 effect；有实际提交才发 library-meta。
    if commits.load(Ordering::Relaxed) > 0 {
        state.app.emit("library-meta", ()).ok();
    }
    state.app
        .emit("scan-finished", serde_json::json!({ "done": done, "total": total }))
        .ok();
    true
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

/// 扫描模式的持久化键：`auto` / `manual`。前端每次启动都会推一次 `set_cover_scan`，
/// 所以启动自愈能据此判断当前模式（`cover_scan_enabled` 的默认值是 true，推送前无法区分）。
/// 它只记录**模式**，不是「自愈已完成」标记 —— 自愈本身每进程只跑一次、且不写任何持久标记。
pub const COVER_SCAN_MODE_KEY: &str = "cover_scan_mode";

/// 本进程是否已执行过封面自愈（不落盘：每次启动都重新核对）。
static COVER_HEAL_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 标记「本进程已处理过封面自愈」（启动路径已跑过时调用，避免前端随后再推一次时重复执行）。
pub fn mark_cover_heal_done() {
    COVER_HEAL_DONE.store(true, Ordering::SeqCst);
}

/// 封面自愈的日志口径：读目录失败 / 目录为空 / 正常复位 三种情形都要可追溯；
/// 1B 的同 stem 清扫结果单独一行（保留了多少组必须可见，否则「为什么没清干净」无从判断）。
pub fn log_cover_heal(rep: &db::CoverHeal) {
    if rep.stem_groups > 0 {
        let dist = rep
            .stem_ref_ext
            .iter()
            .map(|(e, n)| format!("{e}={n}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "[covers] 同 stem 冗余 {} 组：已删 {} 个文件；无引用保守保留 {} 组；被引用扩展名 [{dist}]",
            rep.stem_groups, rep.stem_removed, rep.stem_kept
        );
    }
    if let Some(e) = &rep.dir_err {
        eprintln!("[covers] 读取封面目录失败，本轮跳过自愈（未复位任何行）：{e}");
        return;
    }
    if rep.reset == 0 {
        return;
    }
    if rep.scanned == 0 {
        // 目录存在但为空 = 外部删除后又被 create_dir_all 重建的形态，全量复位是正确行为。
        eprintln!(
            "[covers] 封面目录为空，已复位 {} 行（其中自选封面 {} 行），将按扫描顺序重建",
            rep.reset, rep.reset_custom
        );
    } else {
        eprintln!(
            "[covers] 封面文件缺失，已复位 {} 行（其中自选封面 {} 行；目录内 {} 个文件）",
            rep.reset, rep.reset_custom, rep.scanned
        );
    }
}

/// 停 worker → 自愈 → 重新拉起扫描。运行时入口（前端推 set_cover_scan）用这一条，
/// 避免与在途认领交叉（同 clear_cache 的停/起约定）。
fn heal_covers_now(state: &Arc<AppState>) {
    // 自愈链路保持原行为：超时也继续，不阻塞设置项切换。
    let _ = stop_cover_worker_and_wait(state);
    let rep = {
        let conn = state.db.0.lock().unwrap();
        let dir = covers::covers_dir(&state.app);
        db::heal_missing_covers(&conn, &dir)
    };
    log_cover_heal(&rep);
    ensure_scan_worker(&state.app);
}

/// 封面提取的并发度常量：**默认 2**。
///
/// 为什么默认只有 2：多数用户的书库在机械盘或外置盘上，封面重建的瓶颈是随机 I/O
/// （实测：单本 CPU（解码+缩放+重编码）≈11ms，而端到端 ≈473ms/本）；并发提高的是
/// "把等待重叠起来"的能力，在寻道受限的介质上开太大只会加剧抖动。
/// 自适应提升需要判断介质是否 SSD（DeviceIoControl 的 IncursSeekPenalty），
/// 那要新增 `Win32_System_IO` / `Win32_System_Ioctl` feature（= Cargo.toml 改动），
/// 本轮不做 —— 因此固定 2，不按核数自动放大。
///
/// 度量钩子已删除：调用点直接用本常量（不再读 DB 键）。
pub const COVER_EXTRACT_CONCURRENCY: usize = 2;

/// 停止封面扫描 worker 并等待其退出（最多约 10 秒）：覆盖单本 5 秒看门狗 + 批内线程收尾。
/// 返回 false = 超时未确认退出，调用方必须放弃删目录，避免与仍在写封面的 worker 并发。
fn stop_cover_worker_and_wait(state: &Arc<AppState>) -> bool {
    state.scan_stop.store(true, Ordering::SeqCst);
    for _ in 0..200 {
        if !state.scan_running.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("[covers] stop worker timeout (10s): still running, abort caller action");
    false
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
            // 够用就直出（原始字节 + 嗅探扩展名），否则回落解码/缩放/重编码；
            // 落盘后清理同一 stem 的其它扩展名旧文件（{path_id}.png ↔ {path_id}.webp）。
            let stem = db::path_id(path).to_string();
            let (name, direct) = covers::save_cover(covers_dir, &stem, &b)?;
            // 直出命中统计：每 20 张记一行（含字节数与是否直出），供验收口径复算命中率。
            let n = COVER_DIRECT_SEEN.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 20 == 0 {
                memlog::stage_note(
                    "cover:direct",
                    &format!("seen={n} direct={} bytes={} name={name}", u8::from(direct), b.len()),
                );
            }
            Ok(Some(name))
        }
        None => Ok(None),
    }
}

/// C1：EPUB 缓存按「字节权重 + 条目数」双上限淘汰；当前打开的书不淘汰（允许单本超限）。
const EPUB_CACHE_MAX_BYTES: usize = 1536 * 1024 * 1024;
const EPUB_CACHE_MAX_ITEMS: usize = 4;

/// 纯函数：计算需要淘汰的缓存条目（从 order 最旧开始，跳过 current/已不存在的 id）。
fn plan_epub_evictions(
    sizes: &[(i64, usize)],
    order: &[i64],
    current: i64,
    max_bytes: usize,
    max_items: usize,
) -> Vec<i64> {
    let mut present: HashMap<i64, usize> = sizes.iter().copied().collect();
    let mut total: usize = present.values().sum();
    let mut evicted = Vec::new();
    for id in order {
        if total <= max_bytes && present.len() <= max_items {
            break;
        }
        if *id == current || !present.contains_key(id) {
            continue;
        }
        if let Some(sz) = present.remove(id) {
            total = total.saturating_sub(sz);
            evicted.push(*id);
        }
    }
    evicted
}

/// C1：把 book_id 移到 LRU 末尾（命中缓存/重新插入都要更新，避免正在读的旧书被当作最旧淘汰）。
fn touch_epub_cache_order(order: &mut Vec<i64>, book_id: i64) {
    order.retain(|id| *id != book_id);
    order.push(book_id);
}

/// C1：插入 EPUB 缓存并按权重淘汰；当前书 book_id 永不被本轮淘汰。
fn push_epub_cache(state: &AppState, book_id: i64, value: Arc<CachedEpub>) {
    // S1：fs::metadata 是阻塞 IO（外置盘可能卡住），绝不能放在 epub_cache/order 两把锁内。
    // 第一段短锁：只做插入与 LRU 顺序更新，顺带快照 (id, path)。
    let entries: Vec<(i64, String)> = {
        let mut m = state.epub_cache.lock().unwrap();
        let mut order = state.epub_cache_order.lock().unwrap();
        m.insert(book_id, value);
        touch_epub_cache_order(&mut order, book_id);
        m.iter().map(|(k, v)| (*k, v.path.clone())).collect()
    };
    // 锁外逐项 stat 计算权重（路径在快照后可能被换/删，读不到按 0 估）。
    let sizes: Vec<(i64, usize)> = entries
        .iter()
        .map(|(k, p)| {
            (
                *k,
                std::fs::metadata(p)
                    .map(|x| x.len() as usize)
                    .unwrap_or(0),
            )
        })
        .collect();
    // 第三段短锁：重新取 order 做淘汰计划；只删仍存在的项，避免把快照后被重新插入的书误删。
    let mut m = state.epub_cache.lock().unwrap();
    let mut order = state.epub_cache_order.lock().unwrap();
    for victim in plan_epub_evictions(
        &sizes,
        &order,
        book_id,
        EPUB_CACHE_MAX_BYTES,
        EPUB_CACHE_MAX_ITEMS,
    ) {
        if m.contains_key(&victim) {
            m.remove(&victim);
            order.retain(|id| *id != victim);
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

/// 书籍元信息（path, ext）进程内缓存：get_chapter 每章都要它，旧实现每章抢一次 state.db.0
/// 全局锁读 books 行 —— 后台建索引持锁时会连带把翻章拖住。命中路径完全不碰 DB；
/// 未命中回退读取并回填。容量有限（1024），删除书籍 / 移除目录 / 刷新书库后整体清空。
fn cached_book_meta(state: &Arc<AppState>, book_id: i64) -> Option<(String, String)> {
    if let Some(v) = state.book_meta.lock().unwrap().get(&book_id).cloned() {
        return Some(v);
    }
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id)?
    };
    let ext = ext_of(&book.path).unwrap_or_default();
    let meta = (book.path, ext);
    push_cache(&state.book_meta, book_id, meta.clone(), 1024);
    Some(meta)
}

/// EPUB 字数统计（每章字数 + 全书 total/pure）：先查共享缓存，未命中则读盘并完整解析一次，随后回填 DB。
///
/// 为什么共享缓存：book_char_counts（全书总页数）与 get_book_word_count（书详情/本书统计）
/// 要的是同一份统计，各解析一次就是把整本书解析两遍（数千章是秒级）。
/// 为什么回填 DB：books.total_chars > 0 是这两个命令的短路条件，回填后同一本书不再重算。
///
/// 本函数是阻塞的（读盘 + rayon 逐章统计），**只能在 spawn_blocking 里调用**。
fn epub_stats_for(state: &Arc<AppState>, book_id: i64, path: &str) -> Result<Arc<EpubStats>, String> {
    // P13 单飞去重：book_char_counts 与 get_book_word_count 同时触发时，只让一个调用真正跑下面的
    // 整本统计，其余调用等待并复用同一结果（复测出现过一次 21,600ms 的重复解析）。
    single_flight(&state.epub_stats_flights, book_id, || {
        epub_stats_compute(state, book_id, path)
    })
}

/// epub_stats_for 的实际计算体（内存缓存 → DB 持久化短路 → 整本解析）。
fn epub_stats_compute(state: &Arc<AppState>, book_id: i64, path: &str) -> Result<Arc<EpubStats>, String> {
    let stamp = file_stamp(path);
    let hit = state.epub_stats_cache.lock().unwrap().get(&book_id).cloned();
    if let Some(h) = hit {
        if stamp_matches(h.stamp, stamp) {
            return Ok(h);
        }
        // 文件换过了：旧条目立刻移除，避免它被其它入口（book_char_counts / 书详情）再读走。
        state.epub_stats_cache.lock().unwrap().remove(&book_id);
    }
    // 持久化短路（P8）：上次已算过且文件指纹一致 → 反序列化逐章数组直接复用，不再读盘/整本解析。
    if let Some((size, mtime, total, pure, per_json)) = {
        let conn = state.db.0.lock().unwrap();
        db::get_epub_stats(&conn, book_id)
    } {
        if db_cache_stamp_usable(size, mtime, stamp) {
            if let Ok(per_chapter) = serde_json::from_str::<Vec<i64>>(&per_json) {
                let stats = Arc::new(EpubStats { stamp, per_chapter, total, pure });
                push_cache(&state.epub_stats_cache, book_id, stats.clone(), 4);
                return Ok(stats);
            }
        }
    }
    // C13：按文件路径统计字数，不整本读入常驻。
    let (per_chapter, total, pure) = crate::epub::epub_word_stats_file(std::path::Path::new(path))?;
    let stats = Arc::new(EpubStats { stamp, per_chapter, total, pure });
    // 指纹取不到就不入缓存/不写库：否则这条「无法校验」的条目会一直命中。
    if let Some((size, mtime)) = stamp {
        push_cache(&state.epub_stats_cache, book_id, stats.clone(), 4);
        // 逐章数组持久化：每个文件一辈子只算一次（跨重启短路）。
        if let Ok(per_json) = serde_json::to_string(&stats.per_chapter) {
            let conn = state.db.0.lock().unwrap();
            db::set_epub_stats(&conn, book_id, size, mtime, stats.total, stats.pure, &per_json);
        }
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
        if cache_entry_valid(c.ver, ver, c.stamp, stamp) {
            return Ok(c.doc.clone());
        }
        state.txt_cache.lock().unwrap().remove(&book_id);
    }
    // L2 磁盘解析缓存（P10）：规则版本 + 文件指纹都命中就直接复用，免去解码 + 切分。
    // 缓存本身不可信：read_cache 遇坏文件会删除并返回 None，走下面正常解析。
    if let Some((size, mtime)) = stamp {
        let key = crate::parsecache::CacheKey {
            kind: crate::parsecache::KIND_TXT,
            book_id,
            rules_ver: crate::parsecache::CACHE_RULES_VER,
            size,
            mtime,
        };
        if let Some(doc) = crate::parsecache::read_cache(&crate::parsecache::cache_dir(&state.app), key) {
            let result = Arc::new((doc.text, doc.ranges, doc.toc));
            push_cache(
                &state.txt_cache,
                book_id,
                CachedText { ver, stamp, doc: result.clone(), warn: doc.warn },
                3,
            );
            return Ok(result);
        }
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
    // 编码判定缓存（P7）：用户没强制编码时，指纹一致的判定直接当强制编码用，跳过三候选检测链；
    // 检测结果写回 DB（带文件指纹），同一文件后续启动检测耗时为 0。
    let cached_enc = if forced.is_none() {
        let conn = state.db.0.lock().unwrap();
        db::get_txt_encoding(&conn, book_id)
            .filter(|(size, mtime, _)| db_cache_stamp_usable(*size, *mtime, stamp))
            .map(|(_, _, enc)| enc)
            .filter(|enc| !enc.trim().is_empty())
    } else {
        None
    };
    let forced_eff = forced.clone().or(cached_enc);
    let (doc, enc) = txt::decode_and_split_detect(path, Some(&pattern), forced_eff.as_deref())?;
    // 指纹取不到就不写缓存：无法校验的判定会一直命中错误编码。
    if forced.is_none() {
        if let Some((size, mtime)) = stamp {
            let conn = state.db.0.lock().unwrap();
            db::set_txt_encoding(&conn, book_id, size, mtime, enc);
        }
    }
    let result = Arc::new((doc.text, doc.ranges, doc.toc));
    // 落盘（P10）：只在拿到文件指纹时写；写失败忽略（缓存只是加速手段，不影响打开）。
    if let Some((size, mtime)) = stamp {
        // L1 先入缓存：本次调用立即可用。
        push_cache(
            &state.txt_cache,
            book_id,
            CachedText { ver, stamp, doc: result.clone(), warn: None },
            3,
        );
        // L2 异步落盘（补丁）：TXT 正文 60~80MB，同步压缩写实测把首次打开从 3.2s 拖到 8.3s，
        // 且命中后还要解压；改为后台线程不压缩 + 流式写，不阻塞打开返回。
        // 线程只持有目录/键/结果 Arc，不借用 state；失败只记诊断日志。
        let key = crate::parsecache::CacheKey {
            kind: crate::parsecache::KIND_TXT,
            book_id,
            rules_ver: crate::parsecache::CACHE_RULES_VER,
            size,
            mtime,
        };
        crate::parsecache::spawn_write_txt_cache(
            crate::parsecache::cache_dir(&state.app),
            key,
            result.clone(),
        );
    }
    Ok(result)
}

/// MOBI/AZW3 解析：带 rules_ver + 文件指纹校验的内存缓存，open_book / get_chapter /
/// collect_book_chapters 共用同一条路径（旧实现三处各自解析，同一进程内整本 MOBI 会被解析两次以上）。
/// 缓存命中时不再解析、不再重新抽图（图片已在首次解析时落到 res/<book_id>/）。
/// 返回值带编码告警：告警随缓存保存，命中时与冷解析返回同样的 encoding_warning。
pub(crate) fn load_mobi(
    state: &Arc<AppState>,
    path: &str,
    book_id: i64,
) -> Result<(Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>, Option<String>), String> {
    let ver = state.rules_ver.load(Ordering::Relaxed);
    // 命中必须同时满足「规则版本一致」与「文件指纹一致」（与 load_txt 同一套判据）。
    let stamp = file_stamp(path);
    let hit = state.txt_cache.lock().unwrap().get(&book_id).cloned();
    if let Some(c) = hit {
        if cache_entry_valid(c.ver, ver, c.stamp, stamp) {
            return Ok((c.doc.clone(), c.warn.clone()));
        }
        state.txt_cache.lock().unwrap().remove(&book_id);
    }
    // L2 磁盘解析缓存（P10）：除规则版本 + 文件指纹外，还要求 res 图片目录与源文件一致
    //（否则缓存 HTML 里的图片 URL 会指向缺失文件）——不满足就按未命中重解析（重解析会重写图片）。
    let res_dir = crate::data_dir(&state.app).join("res").join(book_id.to_string());
    if let Some((size, mtime)) = stamp {
        if crate::mobi::res_dir_ready(&res_dir, stamp) {
            let key = crate::parsecache::CacheKey {
                kind: crate::parsecache::KIND_MOBI,
                book_id,
                rules_ver: crate::parsecache::CACHE_RULES_VER,
                size,
                mtime,
            };
            if let Some(doc) =
                crate::parsecache::read_cache(&crate::parsecache::cache_dir(&state.app), key)
            {
                let result = Arc::new((doc.text, doc.ranges, doc.toc));
                push_cache(
                    &state.txt_cache,
                    book_id,
                    CachedText {
                        ver,
                        stamp,
                        doc: result.clone(),
                        warn: doc.warn.clone(),
                    },
                    3,
                );
                return Ok((result, doc.warn));
            }
        }
    }
    let doc = mobi_doc_for(state, path, book_id)?;
    // 拆包取所有权，避免再 clone 一份可能很大的 html。
    let crate::mobi::MobiDoc { html, toc, parts, warning, .. } = doc;
    let result = Arc::new((html, parts, toc));
    // 指纹取不到就不入缓存/不落盘：无法校验的条目会一直命中。
    if let Some((size, mtime)) = stamp {
        // 落盘（P10）：res 标记已由 extract_images 在本次解析中原子写入；写缓存失败忽略。
        let key = crate::parsecache::CacheKey {
            kind: crate::parsecache::KIND_MOBI,
            book_id,
            rules_ver: crate::parsecache::CACHE_RULES_VER,
            size,
            mtime,
        };
        // R8-2：MOBI 解析缓存改为后台异步写（不压缩 + 流式），不阻塞打开返回；
        // 进程提前退出则下次按未命中重解析（可接受），失败只打诊断日志。
        crate::parsecache::spawn_write_cache(
            crate::parsecache::cache_dir(&state.app),
            key,
            result.clone(),
            warning.clone(),
        );
        push_cache(
            &state.txt_cache,
            book_id,
            CachedText { ver, stamp, doc: result.clone(), warn: warning.clone() },
            3,
        );
    }
    Ok((result, warning))
}

pub fn load_epub(state: &AppState, path: &str, book_id: i64) -> Result<Arc<CachedEpub>, String> {
    // 指纹校验：文件被就地替换后必须重解析（否则正文/toc/spine 全是旧文件的）。
    let stamp = file_stamp(path);
    let hit = state.epub_cache.lock().unwrap().get(&book_id).cloned();
    if let Some(c) = hit {
        if stamp_matches(c.stamp, stamp) {
            // C1：命中时把该书移到 LRU 末尾（缓存锁已在上面克隆后释放，这里只锁 order）。
            touch_epub_cache_order(&mut state.epub_cache_order.lock().unwrap(), book_id);
            return Ok(c.clone());
        }
        state.epub_cache.lock().unwrap().remove(&book_id);
        state.epub_cache_order.lock().unwrap().retain(|id| *id != book_id);
    }
    let t0 = std::time::Instant::now();
    // C13：File+ZipArchive 按需读取 central directory/OPF/导航，不把整本读入常驻。
    let (mut doc, ci) = crate::epub::parse_file_with_ci(std::path::Path::new(path))?;
    // P11：切章只用于「目录折叠 + 图片页型」的书；该书已有划线时关掉切分（划线按旧章号记录，
    // 切章会让位置错乱）。门控放在缓存前，同一进程内后续命中都是已门控的结果。
    if doc.split {
        // 门控细节：只有「有划线且尚未迁移过」才关闭切分。迁移标记存在说明该书切章后已按新章号
        // 使用过（用户之后新建的划线也是新章号），此时再关切分反而会让这些划线错位。
        let (has_highlights, migrated) = {
            let conn = state.db.0.lock().unwrap();
            let hl = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM highlights WHERE book_id=?1)",
                    [book_id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            let key = format!("epub.split_migrated.{book_id}");
            (hl, db::get_setting(&conn, &key).is_some())
        };
        if has_highlights && !migrated {
            doc.split = false;
            doc.split_toc.clear();
            doc.chapters.clear();
        }
    }
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[open] load_epub book={book_id} parse+ci={:?} spine={} toc={}",
            t0.elapsed(), doc.spine.len(), doc.toc.len()
        );
    }
    let cached = Arc::new(CachedEpub {
        doc,
        path: path.to_string(),
        ci,
        stamp,
    });
    if stamp.is_some() {
        push_epub_cache(state, book_id, cached.clone());
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
    let bytes = crate::epub::read_entry_file(std::path::Path::new(&cached.path), &cached.ci, target)?;
    Some((crate::epub::mime_for_path(target).to_string(), bytes))
}

/// 把全书图片列表写入缓存并返回调用方要用的列表（避免调用方再从缓存 clone 一次）。
/// 超上限整体清空：本缓存条目不可变、命中受文件指纹约束，逐条 LRU 的收益很小；
/// 清空实现简单、上限确定（最多 MAX_IMAGE_CACHE 本书的清单）。
fn image_cache_put(
    state: &AppState,
    book_id: i64,
    stamp: Option<(i64, i64)>,
    images: Vec<BookImage>,
    out: Vec<BookImage>,
) -> Vec<BookImage> {
    let mut map = state.image_cache.lock().unwrap();
    if map.len() >= MAX_IMAGE_CACHE {
        map.clear();
    }
    map.insert(
        book_id,
        CachedImages {
            stamp,
            images: Arc::new(images),
        },
    );
    out
}

/// 全书图片列表（按 spine/章节顺序，按解析出的 zip 路径去重）。
/// 用于图片查看器浏览整本书所有图片。EPUB 走 zip 内路径；MOBI/AZW3 走已提取的 res 目录。
#[tauri::command]
pub async fn list_book_images(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<Vec<BookImage>, String> {
    // 指纹取 DB 的 size/mtime：缓存条目有同值且为 Some 才命中；否则文件可能已就地替换。
    let stamp = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).map(|b| (b.size, b.mtime))
    };
    if let Some(s) = stamp {
        let hit = state
            .image_cache
            .lock()
            .unwrap()
            .get(&book_id)
            .map(|c| (c.stamp, c.images.clone()));
        if let Some((cached, images)) = hit {
            if cached == Some(s) {
                return Ok(images.as_ref().clone());
            }
        }
    }
    let st = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || collect_book_images(&st, book_id))
        .await
        .map_err(|e| e.to_string())??;
    // 缓存与返回各需一份：只 clone 一次（缓存内部持有 Arc）。
    let out = result.clone();
    Ok(image_cache_put(state.inner(), book_id, stamp, result, out))
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
        // 已提取到 appdata/res/<book_id>/ 的图片（R8-1：Lazy 模式优先用清单，旧目录回退扫描）
        let dir = crate::data_dir(&state.app).join("res").join(book_id.to_string());
        let mut files: Vec<String> = crate::mobi::res_image_names(&dir).unwrap_or_else(|| {
            std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| {
                            e.path().is_file()
                                && crate::mobi::is_res_image_name(
                                    &e.file_name().to_string_lossy(),
                                )
                        })
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        });
        {
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

/// H11：改 async + 阻塞线程池（主线程不再持全局 DB 锁做重 IO）。
/// Tauri 的 async 命令带引用入参时必须返回 Result，故返回 Ok(Vec)；
/// 前端 invoke 的返回值形态不变（数组），join 失败按空数组处理（与旧实现一致）。
#[tauri::command]
pub async fn list_folders(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    let st = state.inner().clone();
    let folders = tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_folders(&conn)
    })
    .await
    .unwrap_or_default();
    Ok(folders)
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
    // 运行期新增目录：重建监听列表，该目录后续变化才能触发自动扫描。
    install_watcher(state.inner());
    let st = state.inner().clone();
    tokio::spawn(async move {
        scan_folders(&st, Some(vec![path]), ScanOrigin::User).await;
    });
    Ok(())
}

/// 发一帧「移除目录」进度（delete → clean → done）；前端底栏据此显示文字进度。
fn emit_remove_progress(app: &AppHandle, phase: &str, done: usize, total: usize, running: bool) {
    app.emit(
        "remove-folder-progress",
        serde_json::json!({
            "running": running, "phase": phase, "done": done, "total": total,
        }),
    )
    .ok();
}

#[tauri::command]
pub async fn remove_folder(state: State<'_, Arc<AppState>>, path: String) -> Result<(), String> {
    let st = state.inner().clone();
    let diag = std::env::var("TIDEPAGE_OP_DIAG").is_ok();
    let t0 = std::time::Instant::now();
    // 入口第一行：必须在任何 `?` 之前 —— 否则"目录登记被删掉"这一步失败时，
    // 日志里连"用户点过移除目录"都看不到（这条链路历史上完全没有日志）。
    log_op("remove_folder", &format!("folder={path} start"));
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
    // 移除目录进度：删除 → 清理封面/res → 完成；失败路径也会发 running:false 收条。
    let total = doomed.len();
    emit_remove_progress(&state.app, "delete", 0, total, true);
    if doomed.is_empty() {
        // 没有书可删：立刻收条，继续后续的注销目录等流程。
        emit_remove_progress(&state.app, "done", 0, 0, false);
    }
    // 分批删除，每批一个事务；批间释放 db 锁并 yield，避免长持锁阻塞窗口取行等 DB 读取。
    // 与 scan_folders 阶段四同款：异步命令体直接跑（非同步命令/主线程），批间让出。
    // 先删书、后注销目录：任何一批失败都保留目录登记，避免「目录已注销、书还在」的幽灵行。
    let t_del = std::time::Instant::now();
    let mut deleted_ok = 0usize;
    let mut cleaned = 0usize;
    let mut first_err: Option<String> = None;
    // 删除阶段细粒度进度：事务内按 100 本切段、80ms 节流发帧（不增加事务数/fsync）。
    let mut last_emit = std::time::Instant::now();
    let mut deleted_in_batch = 0usize;
    for chunk in doomed.chunks(2000) {
        // 删除前先收集可再生资源（封面/res）：行删掉后就查不到了。
        let artifacts = {
            let conn = st.db.0.lock().unwrap();
            db::book_artifacts_by_paths(&conn, chunk)
        };
        let r: Result<(), String> = (|| {
            let mut guard = st.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            for sub in chunk.chunks(100) {
                // 同一个事务内分批删除：不增加 fsync，只为让前端数字连续变化。
                db::delete_books_policy(&tx, sub, true)?;
                deleted_in_batch += sub.len();
                // 进度帧允许领先于提交（纯展示；事务失败时随后会用已提交数收正）。
                if last_emit.elapsed() >= std::time::Duration::from_millis(80) {
                    emit_remove_progress(
                        &state.app,
                        "delete",
                        deleted_ok + deleted_in_batch,
                        total,
                        true,
                    );
                    last_emit = std::time::Instant::now();
                }
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })();
        match r {
            Ok(()) => {
                deleted_ok += chunk.len();
                deleted_in_batch = 0;
                // 每批提交后必发一帧，保证数字与实际提交进度对齐。
                emit_remove_progress(&state.app, "delete", deleted_ok, total, true);
                last_emit = std::time::Instant::now();
                // 资源清理：每 100 本一组，逐组推进 clean 相位。
                for group in artifacts.chunks(100) {
                    cleanup_book_artifacts(&st, group);
                    cleaned += group.len();
                    emit_remove_progress(&state.app, "clean", cleaned, total, true);
                }
            }
            Err(e) => {
                // 失败立即停止后续批次：先把数字回落到「已提交数」，再让外层发收条。
                eprintln!("[remove_folder] delete tx failed: {e}");
                emit_remove_progress(&state.app, "delete", deleted_ok, total, true);
                first_err = Some(e);
                break;
            }
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
    if let Some(e) = first_err {
        let failed = doomed.len() - deleted_ok;
        log_op(
            "remove_folder",
            &format!(
                "folder={path} failed err={e} prefix_rows={row_count} deleted_ok={deleted_ok} failed={failed} elapsed_ms={}",
                t0.elapsed().as_millis()
            ),
        );
        bump_formats_epoch(state.inner());
        state.app.emit("library-changed", ()).ok();
        emit_remove_progress(&state.app, "done", deleted_ok, total, false);
        return Err(format!(
            "移除目录失败：已删除 {deleted_ok} 本，{failed} 本未删除，目录未注销，可重试"
        ));
    }
    // 全部书删除成功后才注销目录；注销失败时书已移除、目录仍在（可重试移除）。
    {
        let conn = st.db.0.lock().unwrap();
        if let Err(e) = db::remove_folder(&conn, &path) {
            log_op(
                "remove_folder",
                &format!(
                    "folder={path} unregister_failed err={e} prefix_rows={row_count} deleted_ok={deleted_ok} elapsed_ms={}",
                    t0.elapsed().as_millis()
                ),
            );
            bump_formats_epoch(state.inner());
            state.app.emit("library-changed", ()).ok();
            emit_remove_progress(&state.app, "done", deleted_ok, total, false);
            return Err(format!("书籍已移除，但目录注销失败：{e}"));
        }
    }
    // 目录已注销：清掉该前缀下的「移出书库」忽略记录，重新添加同一目录会重新导入。
    {
        let conn = st.db.0.lock().unwrap();
        if let Err(e) = db::remove_ignored_under(&conn, &base) {
            log_op(
                "remove_folder",
                &format!(
                    "folder={path} remove_ignored_failed err={e} elapsed_ms={}",
                    t0.elapsed().as_millis()
                ),
            );
        }
        // 连带注销其下嵌套登记的子目录（失败只记日志，不影响主流程）。
        if let Err(e) = db::remove_nested_folders(&conn, &base) {
            log_op(
                "remove_folder",
                &format!(
                    "folder={path} remove_nested_failed err={e} elapsed_ms={}",
                    t0.elapsed().as_millis()
                ),
            );
        }
    }
    // 注销成功：监听列表同步摘掉该目录。
    install_watcher(state.inner());
    log_op(
        "remove_folder",
        &format!(
            "folder={path} done prefix_rows={row_count} deleted_ok={deleted_ok} failed=0 elapsed_ms={}",
            t0.elapsed().as_millis()
        ),
    );
    emit_remove_progress(&state.app, "done", deleted_ok, total, false);
    bump_formats_epoch(state.inner());
    state.app.emit("library-changed", ()).ok();
    // 目录下的书已删：元信息缓存整体失效（避免残留已删书的 path/ext）。
    st.book_meta.lock().unwrap().clear();
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
    include_total: Option<bool>,
    tags: Option<Vec<String>>,
) -> Result<BookPage, String> {
    memlog::watch("cmd:list_books_page");
    let st = state.inner().clone();
    let count_total = include_total.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_books_page_opts_tags(
            &conn,
            &query,
            &sort,
            offset,
            limit,
            &formats,
            &states,
            desc.unwrap_or(false),
            &folder,
            count_total,
            &tags,
        )
    })
    .await
    .map_err(|e| e.to_string())
}

/// 秩可寻址分页有序索引上限：超过则前端必须切 keyset 档（禁止静默截断）。
const MAX_ORDER_INDEX_ROWS: i64 = 300_000;

/// 「缺封面 PDF」命令的调用计数：只用于启动日志的限流打点（诊断 O(N²) 形态）。
static PDF_MISSING_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 封面写入计数（同样只服务于限流打点）。
static COVER_IMG_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 容器格式封面提取计数：直出命中率的采样打点（每 20 张记一行）。
static COVER_DIRECT_SEEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// range 请求计数：前 20 条逐条记区间，之后只记大块（见 pdf_read_range）。
static PDF_RANGE_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
    tags: Option<Vec<String>>,
) -> Result<tauri::ipc::Response, String> {
    memlog::watch("cmd:list_order_index");
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
        let ids = db::order_index_ids_tags(&conn, &sort, &query, MAX_ORDER_INDEX_ROWS as usize, &formats, &states, desc, &folder, &tags)?;
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

/// R12-2：格式列表缓存命中判断（纯函数，供命令与单测共用）。
fn formats_cache_get(
    cache: &Mutex<Option<(u64, Vec<String>)>>,
    epoch: u64,
) -> Option<Vec<String>> {
    let c = cache.lock().ok()?;
    c.as_ref()
        .filter(|(e, _)| *e == epoch)
        .map(|(_, v)| v.clone())
}

/// R12-2：查询完成后写缓存；若查询期间 epoch 已变则丢弃（避免把旧结果写回）。
/// 返回是否真的写入。
fn formats_cache_put(
    cache: &Mutex<Option<(u64, Vec<String>)>>,
    epoch_at_query: u64,
    current_epoch: u64,
    formats: &[String],
) -> bool {
    if epoch_at_query != current_epoch {
        return false;
    }
    if let Ok(mut c) = cache.lock() {
        *c = Some((current_epoch, formats.to_vec()));
        return true;
    }
    false
}

/// R12-2：books 实际变更后递增格式缓存 epoch（仅真正变化路径调用）。
pub fn bump_formats_epoch(state: &AppState) {
    state.formats_epoch.fetch_add(1, Ordering::SeqCst);
}

/// Task B：书库当前实际存在的格式列表（去重、固定排序）。供前端「分类」面板动态生成格式标签。
/// R12-2：带进程内缓存（epoch 匹配直接返回 clone；命令级 warm 命中 <1ms）。
#[tauri::command]
pub async fn list_formats(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    let st = state.inner().clone();
    let epoch = st.formats_epoch.load(Ordering::SeqCst);
    if let Some(cached) = formats_cache_get(&st.formats_cache, epoch) {
        return Ok(cached);
    }
    let st2 = st.clone();
    let formats = tauri::async_runtime::spawn_blocking(move || {
        let conn = st2.db.0.lock().unwrap();
        db::list_formats(&conn)
    })
    .await
    .map_err(|e| e.to_string())?;
    formats_cache_put(
        &st.formats_cache,
        epoch,
        st.formats_epoch.load(Ordering::SeqCst),
        &formats,
    );
    Ok(formats)
}

/// R12-3：切分书跨虚拟章锚点定位命令。返回目标虚拟章号；None=未切分/文件不匹配/找不到锚点。
#[tauri::command]
pub async fn locate_epub_anchor(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    file_base: String,
    fragment: String,
) -> Result<Option<i64>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = {
            let conn = st.db.0.lock().unwrap();
            db::get_book(&conn, book_id).map(|b| b.path).ok_or("书籍不存在")?
        };
        let cached = load_epub(&st, &path, book_id)?;
        let data = cached.read_all().ok_or("读取 EPUB 失败")?;
        Ok(crate::epub::locate_split_anchor_chapter(
            &cached.doc,
            &data,
            &cached.ci,
            &file_base,
            &fragment,
        )
        .map(|i| i as i64))
    })
    .await
    .map_err(|e| e.to_string())?
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
        // 书被并发删除时 UPDATE 会影响 0 行：先确认行存在，避免静默成功。
        match conn.query_row(
            "SELECT 1 FROM books WHERE id=?1",
            rusqlite::params![book_id],
            |_| Ok(()),
        ) {
            Ok(()) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err("书籍不存在".into());
            }
            Err(e) => return Err(e.to_string()),
        }
        db::set_read_state(&conn, book_id, v)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// F3-H2：右键标记已读/未读的最小后端入口：只写 read_state/read_pct/progress，
/// 不打开书、不解析整本、不触发 book-opened、不改 last_read。state 只接受 "read"/"unread"。
#[tauri::command]
pub async fn mark_book_read_state(
    app: State<'_, Arc<AppState>>,
    book_id: i64,
    state: String,
) -> Result<(), String> {
    let read = match state.as_str() {
        "read" => true,
        "unread" => false,
        _ => return Err("未知阅读状态".into()),
    };
    let st = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::mark_book_read_state(&conn, book_id, read)
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
    memlog::watch("cmd:list_books_by_ids");
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
        // rescan 命令：转换完成后由前端调用，属用户动作触发的扫描。
        scan_folders(&st, None, ScanOrigin::User).await;
    });
    Ok(())
}

#[derive(serde::Serialize)]
pub struct RefreshResult {
    pub removed: usize,
    pub added: usize,
    /// 删除阶段失败的行数（>0 时前端提示「部分未能删除」）。
    pub failed: usize,
    /// 因"比例分级"被挂起、等待用户二次确认的待删条数（0 = 已直接处理）。
    pub pending: usize,
    /// 因父目录不可列举而跳过的条数（0 = 没有不可达目录）。
    pub skipped: usize,
    /// true = 本次只做了判定，**一条都没删**，需要前端确认后带 confirm=true 再调一次。
    pub needs_confirm: bool,
    /// 待删行的父目录聚合（降序，最多 5 条）：让确认框能说出"主要在哪"。
    pub dirs: Vec<DirCount>,
    /// 快照总行数（确认框与日志的口径）。
    pub total: usize,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct DirCount {
    pub dir: String,
    pub count: usize,
}

/// 单条路径的探测结果（纯数据，不依赖文件系统；便于纯函数决策与单测）。
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    /// 文件存在。
    Exists,
    /// 文件不存在，且**父目录可以 read_dir**（目录在、只是文件不在）⇒ 允许删除。
    Missing,
    /// 文件不存在，且父目录 read_dir 失败（权限/脱机/共享冲突）⇒ 一律跳过、只记数。
    ParentUnreachable(String),
}

/// 待删行在某个父目录下的聚合：`missing` = 判为不存在、`present` = 仍存在。
/// 比例分级的分母用 `missing + present`（= 该父目录在库行数）。
#[derive(Clone, Debug, PartialEq)]
pub struct DirStat {
    pub dir: String,
    pub missing: usize,
    pub present: usize,
}

/// 对账计划（纯决策结果，不含任何 IO）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReconcilePlan {
    /// 将被删除的行（needs_confirm=true 时**为空**）。
    pub to_delete: Vec<(i64, String)>,
    /// 父目录不可列举而被跳过的行。
    pub skipped: Vec<(i64, String)>,
    /// 不可列举的父目录：(目录, 错误文本, 跳过条数)。
    pub unreachable: Vec<(String, String, usize)>,
    /// 父目录聚合（按 missing 降序）。
    pub dirs: Vec<DirStat>,
    pub needs_confirm: bool,
    pub total: usize,
    pub missing: usize,
}

/// 比例分级的绝对下限：待删 > 200 行就需要确认（普通刷新通常删个位数）。
pub const RECONCILE_MIN_ROWS: usize = 200;
/// 比例分级的总量阈值：待删 > 总行数 × 1% 需要确认（17 万本约 1,738 行）。
pub const RECONCILE_TOTAL_PCT: usize = 1;
/// 比例分级的单目录阈值：某父目录待删 ≥ 该目录在库行数 × 30% 需要确认。
/// 依据：整目录被移走/改名时，该目录的缺失比例接近 100%，与"零星删几本"能清晰区分。
pub const RECONCILE_DIR_PCT: usize = 30;

/// 纯决策层：给定快照与探测结果，决定"删哪些 / 跳过哪些 / 是否需要用户确认"。
///
/// 规则（两条独立，**不是"都要满足"**）：
/// 1. **必要条件**：父目录可列举才允许删；不可列举 ⇒ 该目录下全部缺失项跳过（只记数）。
/// 2. **比例分级**：待删 > max(200, 总数×1%) 或 某父目录待删 ≥ 该目录×30% ⇒ `needs_confirm`，
///    此时 `to_delete` 为空（绝不删），由调用方拿摘要去问用户；用户确认后以 `confirmed=true`
///    再调一次即可正常删除。**分级只要求确认，不是拒绝**。
pub fn plan_reconcile(
    records: &[(i64, String)],
    mut probe: impl FnMut(&str) -> Probe,
    confirmed: bool,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan {
        total: records.len(),
        ..Default::default()
    };
    let mut stats: std::collections::HashMap<String, DirStat> = std::collections::HashMap::new();
    let mut unreachable: std::collections::HashMap<String, (String, usize)> =
        std::collections::HashMap::new();
    let mut candidates: Vec<(i64, String)> = Vec::new();
    for (id, path) in records {
        match probe(path) {
            Probe::Exists => {
                let dir = parent_dir(path);
                stats.entry(dir.clone()).or_insert(DirStat { dir, missing: 0, present: 0 }).present += 1;
            }
            Probe::Missing => {
                plan.missing += 1;
                let dir = parent_dir(path);
                stats.entry(dir.clone()).or_insert(DirStat { dir, missing: 0, present: 0 }).missing += 1;
                candidates.push((*id, path.clone()));
            }
            Probe::ParentUnreachable(err) => {
                plan.missing += 1;
                let dir = parent_dir(path);
                let e = unreachable.entry(dir).or_insert((err, 0));
                e.1 += 1;
                plan.skipped.push((*id, path.clone()));
            }
        }
    }
    let mut dirs: Vec<DirStat> = stats.into_values().filter(|d| d.missing > 0).collect();
    dirs.sort_by(|a, b| b.missing.cmp(&a.missing).then(a.dir.cmp(&b.dir)));
    // 分级判定
    let threshold_rows = RECONCILE_MIN_ROWS.max(plan.total * RECONCILE_TOTAL_PCT / 100);
    let over_total = plan.missing > threshold_rows;
    let over_dir = dirs.iter().any(|d| {
        let lib = d.missing + d.present;
        lib > 0 && d.missing * 100 >= lib * RECONCILE_DIR_PCT
    });
    plan.dirs = dirs;
    for (dir, (err, n)) in unreachable {
        plan.unreachable.push((dir, err, n));
    }
    plan.unreachable.sort_by(|a, b| b.2.cmp(&a.2));
    // 没有任何可删行时不需要确认（全部被跳过 = 只是暂时不可达，问用户没有意义）。
    plan.needs_confirm = !confirmed && !candidates.is_empty() && (over_total || over_dir);
    if !plan.needs_confirm {
        plan.to_delete = candidates;
    }
    plan
}

/// 父目录（与 scan_dirs 同口径：`D:\Pdf\a\b.epub` → `D:\Pdf\a`）；无父目录时用空串。
fn parent_dir(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 任务2 核心：一次性对账（纯目录遍历，不新增任何文件系统监听）。
/// 1) 记录路径在磁盘上不存在的 → 从索引移除并计数；
/// 2) 扫描集 = 存活记录所在目录 ∪ 已登记书库根目录及其**一级子目录**；
///    逐目录只扫一层，磁盘存在但索引里没有的书籍新增入库（不做全递归）；
/// 3) 返回 {removed, added}。不触碰正在阅读的窗口/reader 状态（不强关）。
// staged: 对账算法的唯一有测试的实现（命令 refresh_library 目前内联重写同逻辑且无测试；最干净的做法是让命令委托至此以复用其 5 项测试，属行为相邻重构，另行评审）。
#[allow(dead_code)]
pub fn refresh_library_core(conn: &rusqlite::Connection) -> RefreshResult {
    let t0 = std::time::Instant::now();
    let records = db::all_book_paths(conn);
    let mut cache: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    // 测试用的这条路：confirmed=true（沿用旧语义：直接删），但同样受"父目录可列举"约束。
    let plan = plan_reconcile(&records, |p| probe_path_fs(p, &mut cache), true);
    log_reconcile_plan("refresh_core", &plan, t0.elapsed().as_millis(), true);
    let (ok, failed, first_err) = delete_rows(conn, &plan.to_delete);
    log_delete_result("refresh_core", ok, failed, first_err.as_deref(), t0.elapsed().as_millis());
    let gone: std::collections::HashSet<&str> = plan
        .to_delete
        .iter()
        .chain(plan.skipped.iter())
        .map(|(_, p)| p.as_str())
        .collect();
    let survivors: Vec<String> = records
        .iter()
        .filter(|(_, p)| !gone.contains(p.as_str()))
        .map(|(_, p)| p.clone())
        .collect();

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
    RefreshResult {
        removed: ok,
        added,
        failed,
        pending: 0,
        skipped: plan.skipped.len(),
        needs_confirm: false,
        dirs: plan
            .dirs
            .iter()
            .take(5)
            .map(|d| DirCount { dir: d.dir.clone(), count: d.missing })
            .collect(),
        total: plan.total,
    }
}

/// 真实文件系统探测：文件存在 ⇒ Exists；不存在则看**父目录能否 read_dir**
/// （目录在、只是文件不在 ⇒ Missing；列举失败 ⇒ ParentUnreachable）。
/// 父目录结果按目录缓存：17 万本下同一目录会被探测上万次，重复 read_dir 不可接受。
/// 注意 `Path::exists()` ≈ `fs::metadata(..).is_ok()`：权限/共享冲突/占用都会是 false，
/// 所以"不存在"必须再由父目录可列举性做一次区分 —— 这是唯一的直接证据。
fn probe_path_fs(
    path: &str,
    cache: &mut std::collections::HashMap<String, Option<String>>,
) -> Probe {
    if Path::new(path).exists() {
        return Probe::Exists;
    }
    let dir = parent_dir(path);
    let entry = cache.entry(dir.clone()).or_insert_with(|| {
        match std::fs::read_dir(&dir) {
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        }
    });
    match entry {
        None => Probe::Missing,
        Some(err) => Probe::ParentUnreachable(err.clone()),
    }
}

/// 取证日志（删除**之前**）：一条主行 + 目录聚合 + 不可达父目录 + 前 5 条样例路径。
/// 之所以这么细：上一轮少了 9,402 行却没有任何删除日志，事后无法判断
/// "文件真没了"还是"目录当时不可达"，只能靠这四类读数区分。
fn log_reconcile_plan(site: &str, plan: &ReconcilePlan, elapsed_ms: u128, confirmed: bool) {
    let snap_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!(
        "site={site} snapshot_at={snap_at} total={} missing={} delete={} confirm_required={} confirmed={} skipped={} unreachable_dirs={} elapsed_ms={elapsed_ms}",
        plan.total,
        plan.missing,
        plan.to_delete.len(),
        u8::from(plan.needs_confirm),
        u8::from(confirmed),
        plan.skipped.len(),
        plan.unreachable.len(),
    );
    eprintln!("[refresh] plan {line}");
    memlog::stage_note("refresh:plan", &line);
    if !plan.dirs.is_empty() {
        let dirs = plan
            .dirs
            .iter()
            .take(5)
            .map(|d| format!("{}={}/{}", d.dir, d.missing, d.missing + d.present))
            .collect::<Vec<_>>()
            .join(" ; ");
        eprintln!("[refresh] dirs(missing/lib) {dirs}");
        memlog::stage_note("refresh:dirs", &dirs);
    }
    for (dir, err, n) in &plan.unreachable {
        let s = format!("dir={dir} skipped={n} err={err}");
        eprintln!("[refresh] parent_unreachable {s}");
        memlog::stage_note("refresh:parent_err", &s);
    }
    if !plan.to_delete.is_empty() {
        let sample = plan
            .to_delete
            .iter()
            .take(5)
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        eprintln!("[refresh] sample {sample}");
        memlog::stage_note("refresh:sample", &sample);
    }
}

/// 删除（**错误必须被记录**，但仍不因失败中断）：返回 (成功行数, 失败行数, 首个错误文本)。
/// 每 500 行一个事务块，避免 17 万行级别的一次性大事务。
fn delete_rows(
    conn: &rusqlite::Connection,
    rows: &[(i64, String)],
) -> (usize, usize, Option<String>) {
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut first_err: Option<String> = None;
    for chunk in rows.chunks(500) {
        let paths: Vec<String> = chunk.iter().map(|(_, p)| p.clone()).collect();
        match db::delete_books(conn, &paths) {
            Ok(()) => ok += chunk.len(),
            Err(e) => {
                failed += chunk.len();
                if first_err.is_none() {
                    first_err = Some(e.to_string());
                }
                eprintln!("[refresh] delete chunk failed n={} err={e}", chunk.len());
            }
        }
    }
    (ok, failed, first_err)
}

/// 删除链路的统一结构化日志（不依赖 `TIDEPAGE_OP_DIAG`：那条链路历史上完全没有日志，
/// 事后无法判断"用户点过什么、删到哪一步失败"）。
fn log_op(site: &str, msg: &str) {
    eprintln!("[{site}] {msg}");
    memlog::stage_note(&format!("{site}"), msg);
}

fn log_delete_result(site: &str, ok: usize, failed: usize, first_err: Option<&str>, elapsed_ms: u128) {
    let line = format!(
        "site={site} deleted_ok={ok} failed={failed} first_err={} elapsed_ms={elapsed_ms}",
        first_err.unwrap_or("-")
    );
    eprintln!("[refresh] delete {line}");
    memlog::stage_note("refresh:delete", &line);
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
pub async fn refresh_library(
    state: State<'_, Arc<AppState>>,
    confirm: Option<bool>,
) -> Result<RefreshResult, String> {
    let confirm = confirm.unwrap_or(false);
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
    let (records, folders, ignored) = {
        let conn = st.db.0.lock().unwrap();
        (
            db::all_book_paths(&conn),
            db::list_folders(&conn),
            db::ignored_paths_all(&conn),
        )
    };
    emit("check", 0, records.len());
    let t0 = std::time::Instant::now();
    let mut cache: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    let mut probed = 0usize;
    let plan = plan_reconcile(
        &records,
        |p| {
            probed += 1;
            if probed % 200 == 0 {
                emit("check", probed, records.len());
            }
            probe_path_fs(p, &mut cache)
        },
        confirm,
    );
    emit("check", records.len(), records.len());
    // 取证：删除之前把"缺多少、缺在哪、哪些父目录不可达、样例路径、耗时"全部落日志。
    log_reconcile_plan("refresh_cmd", &plan, t0.elapsed().as_millis(), confirm);
    if plan.needs_confirm {
        // 分级：只回摘要、一条都不删（前端拿摘要问用户，确认后带 confirm=true 再来一次）。
        // 只报「确认后真会删」的数量：不可达目录下被跳过的不算。
        let pending = plan.missing.saturating_sub(plan.skipped.len());
        eprintln!(
            "[refresh] pending_confirm missing={pending} dirs={} unreachable={}",
            plan.dirs.len(),
            plan.unreachable.len()
        );
        return Ok(RefreshResult {
            removed: 0,
            added: 0,
            failed: 0,
            pending,
            skipped: plan.skipped.len(),
            needs_confirm: true,
            dirs: plan
                .dirs
                .iter()
                .take(5)
                .map(|d| DirCount { dir: d.dir.clone(), count: d.missing })
                .collect(),
            total: plan.total,
        });
    }
    // 存活判定改 O(n)：先把待删/跳过路径收成集合，避免对每条记录线性扫两个 Vec。
    let deleted_set: std::collections::HashSet<&str> =
        plan.to_delete.iter().map(|(_, p)| p.as_str()).collect();
    let skipped_set: std::collections::HashSet<&str> =
        plan.skipped.iter().map(|(_, p)| p.as_str()).collect();
    let survivors: Vec<String> = records
        .iter()
        .filter(|(_, p)| !deleted_set.contains(p.as_str()) && !skipped_set.contains(p.as_str()))
        .map(|(_, p)| p.clone())
        .collect();
    // 删除分批：每 2000 条一个独立事务；提交成功后清理这批书的封面与 res/<id> 资源。
    // 批间释放锁并 yield，避免大规模删除期间长持锁阻塞其它 IPC。
    let mut removed_ok = 0usize;
    let mut removed_failed = 0usize;
    let mut first_err: Option<String> = None;
    for chunk in plan.to_delete.chunks(2000) {
        let chunk_paths: Vec<String> = chunk.iter().map(|(_, p)| p.clone()).collect();
        // 同一把锁内：删除前先收集可再生资源清单（行删掉后就查不到了），再开本批事务。
        let r: Result<(usize, usize, Option<String>, Vec<(i64, Option<String>)>), String> = (|| {
            let mut guard = st.db.0.lock().unwrap();
            let artifacts = db::book_artifacts_by_paths(&guard, &chunk_paths);
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            let (ok, failed, err) = delete_rows(&tx, chunk);
            tx.commit().map_err(|e| e.to_string())?;
            Ok((ok, failed, err, artifacts))
        })();
        match r {
            Ok((ok, failed, err, artifacts)) => {
                removed_ok += ok;
                removed_failed += failed;
                if first_err.is_none() {
                    first_err = err;
                }
                // 整批全部删除成功才清理资源：部分失败的行还在库里，删封面会让它变成破图。
                if failed == 0 && ok == chunk.len() {
                    cleanup_book_artifacts(&st, &artifacts);
                }
            }
            Err(e) => {
                removed_failed += chunk.len();
                if first_err.is_none() {
                    first_err = Some(e.clone());
                }
                eprintln!("[refresh] delete chunk failed n={} err={e}", chunk.len());
            }
        }
        tokio::task::yield_now().await;
    }
    log_delete_result(
        "refresh_cmd",
        removed_ok,
        removed_failed,
        first_err.as_deref(),
        t0.elapsed().as_millis(),
    );

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
    // 新书去重改集合（原实现 O(n²)）：先判 known/ignored，避免把已存在路径塞进 seen_new。
    let mut seen_new: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                if known.contains(&path)
                    || ignored.contains(&path)
                    || !seen_new.insert(path.clone())
                {
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

    // 新书入库分批：每 2000 条一个独立事务，批间释放锁并 yield。
    // 单批失败只记日志、继续下一批（不让整次刷新失败）。
    let mut added = 0usize;
    let mut new_ids: Vec<i64> = Vec::new();
    for chunk in new_books.chunks(2000) {
        let r: Result<Vec<i64>, String> = (|| {
            let mut guard = st.db.0.lock().unwrap();
            let tx = guard.transaction().map_err(|e| e.to_string())?;
            let mut ids = Vec::with_capacity(chunk.len());
            for (path, title, ext, size, mtime) in chunk {
                if let Ok(id) = db::upsert_book(&tx, path, title, ext, *size, *mtime) {
                    ids.push(id);
                }
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(ids)
        })();
        match r {
            Ok(ids) => {
                added += ids.len();
                new_ids.extend(ids);
            }
            Err(e) => eprintln!("[refresh] upsert tx failed n={} err={e}", chunk.len()),
        }
        tokio::task::yield_now().await;
    }
    if removed_ok > 0 || added > 0 {
        bump_formats_epoch(&st);
        st.app.emit("library-changed", ()).ok();
    }
    emit("done", 1, 1);
    // 导入目录落库后：新书 meta_done=0，拉起增量扫描（若空闲）。
    if !new_ids.is_empty() {
        ensure_scan_worker(&st.app);
    }
    // 刷新可能删除/重导书籍：元信息缓存整体失效。
    st.book_meta.lock().unwrap().clear();
    Ok(RefreshResult {
        removed: removed_ok,
        added,
        failed: removed_failed,
        pending: 0,
        skipped: plan.skipped.len(),
        needs_confirm: false,
        dirs: plan
            .dirs
            .iter()
            .take(5)
            .map(|d| DirCount { dir: d.dir.clone(), count: d.missing })
            .collect(),
        total: plan.total,
    })
}

/// 读取 `last_scan_stale`（书库页首屏提示条的唯一来源）。
/// 返回原始 JSON 字符串：前端只需判 `state == "stale"` 与展示计数；
/// 多窗口读同一份（DB），重启后仍在（若最近一次确实异常）。
#[tauri::command]
pub fn get_last_scan_stale(state: State<'_, Arc<AppState>>) -> Option<String> {
    let conn = state.db.0.lock().unwrap();
    db::get_setting(&conn, LAST_SCAN_STALE_KEY)
}

/// 任务3：打开入口先查文件是否存在（不存在则由前端弹询问，绝不进入空白阅读）。
/// **三态**（必修 3）：`ok` / `missing` / `unreachable`。
/// 为什么必须分开：`Path::exists()` 对权限、共享冲突、脱机盘一律返回 false，
/// 若把它当"文件缺失"就会引导用户去刷新目录 —— 而刷新会删行（连带进度/书签/笔记）。
/// `unreachable` 时前端只提示"暂时无法访问"，绝不进删除确认。
#[tauri::command]
pub fn file_exists(path: String) -> String {
    if path.is_empty() {
        return "missing".into();
    }
    if Path::new(&path).exists() {
        return "ok".into();
    }
    match std::fs::read_dir(parent_dir(&path)) {
        Ok(_) => "missing".into(),
        Err(_) => "unreachable".into(),
    }
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

/// 图片页书判定（第五轮）：图片数门槛与「每张图平均最多配多少非空白文字」。
/// 目标书 3,659 图 / 约 33,059 字 ≈ 9.0 字/图 → 命中；普通插图小说正文比例远高于此。
const IMAGE_PAGES_MIN_IMAGES: usize = 100;
const IMAGE_PAGES_TEXT_PER_IMG_MAX: i64 = 20;

/// 图片页书判定（纯函数，独立于 is_manga）：图片够多、文本按比例足够少、扫描完整
/// （被预算截断一律不判，保守兜底），且图片数不少于 spine 文档数（每个内容文档至少一张图）。
/// 不修改 is_manga_like 与既有漫画判定。
fn is_image_page_like(images: usize, text_chars: i64, complete: bool, spine_n: usize) -> bool {
    complete
        && images >= IMAGE_PAGES_MIN_IMAGES
        && text_chars <= images as i64 * IMAGE_PAGES_TEXT_PER_IMG_MAX
        && images >= spine_n.max(1)
}


/// R6-1 门控（纯函数）：P11 锚点切分绝对优先（doc_split=true 直接 false）；只有「目录折叠
/// （legacy 目录 <=2 项）+ 单篇 spine 文档图片数 > AUTO_BLOCK_IMGS_PER_CHAPTER + 图片主导度
/// 达标」的候选书才自动块级切章；有划线且未迁移时保持旧行为（划线按旧章号可见）。
fn should_auto_block_split(
    doc_split: bool,
    legacy_chapters: usize,
    max_doc_imgs: usize,
    image_dominant: bool,
    has_highlights: bool,
    migrated: bool,
) -> bool {
    !doc_split
        && legacy_chapters <= 2
        && max_doc_imgs > crate::epub::AUTO_BLOCK_IMGS_PER_CHAPTER
        && image_dominant
        && (!has_highlights || migrated)
}


/// R6-B 迁移标记动作（纯函数，便于单测）：标记值格式 {size}-{mtime}|{layout_sig}，
/// layout_sig：P11 锚点原布局 v1 / 块级切章或超大章细分后的布局 v2。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum MarkerAction {
    /// 标记已是最新（stamp|sig 完全一致）：no-op。
    Noop,
    /// 旧格式同 stamp 且当前布局仍为 v1：只重写标记，不改 progress/bookmarks。
    Rewrite,
    /// 同 stamp 但布局签名不同（v1→v2）：按 base_chapters 做布局升级映射。
    LayoutUpgrade,
    /// 无标记或文件已替换：沿用 legacy 迁移（map_legacy_position）。
    LegacyMigrate,
}

fn split_marker_action(marker: Option<&str>, stamp: &str, sig: &str) -> MarkerAction {
    let Some(m) = marker else {
        return MarkerAction::LegacyMigrate;
    };
    if let Some((ms, msig)) = m.split_once('|') {
        if ms != stamp {
            return MarkerAction::LegacyMigrate;
        }
        return if msig == sig {
            MarkerAction::Noop
        } else {
            MarkerAction::LayoutUpgrade
        };
    }
    // 旧格式：只有文件指纹，没有布局签名。
    if m != stamp {
        return MarkerAction::LegacyMigrate;
    }
    if sig == "v1" {
        MarkerAction::Rewrite
    } else {
        MarkerAction::LayoutUpgrade
    }
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


/// R7-1：读取迁移标记里的布局签名。旧格式（无 |）视为 v1；无 marker 返回 None。
fn marker_layout_sig(marker: Option<&str>) -> Option<&str> {
    marker.map(|m| m.split_once('|').map(|(_, s)| s).unwrap_or("v1"))
}

/// R7-1 布局目标：KeepP11=不 patch、保持 P11 锚点原布局；Restricted=第六轮受限细分（v2）；
/// General=第七轮泛化细分（v3）。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SplitLayoutTarget {
    KeepP11,
    Restricted,
    General,
}

/// R7-1 布局目标选择（纯函数）：
/// - 有划线：不改变既有布局——v3 用 General、v2 用 Restricted 重建，v1/无 marker 保持 P11；
/// - 无划线：有 General 可用就升 v3；否则现有 v2 用 Restricted 重建（避免降级）；其余保持 P11。
fn choose_split_layout(
    has_highlights: bool,
    current_sig: Option<&str>,
    general_available: bool,
    restricted_available: bool,
) -> SplitLayoutTarget {
    if has_highlights {
        match current_sig {
            Some("v3") if general_available => SplitLayoutTarget::General,
            Some("v2") if restricted_available => SplitLayoutTarget::Restricted,
            _ => SplitLayoutTarget::KeepP11,
        }
    } else if general_available {
        SplitLayoutTarget::General
    } else if current_sig == Some("v2") && restricted_available {
        SplitLayoutTarget::Restricted
    } else {
        SplitLayoutTarget::KeepP11
    }
}

/// 按新 chapters 的 legacy 字段迁移 progress/bookmarks（marker 缺失、文件替换、或布局升级
/// 但 base 缺失时的退化路径）。pct 按章平均重算，updated_at 由 set_progress 保持。
fn legacy_migrate_positions(
    tx: &rusqlite::Transaction<'_>,
    book_id: i64,
    chapters: &[crate::epub::EpubChapter],
) {
    if let Some(mut p) = db::get_progress(tx, book_id) {
        let (new_idx, new_ratio) =
            crate::epub::map_legacy_position(chapters, p.chapter_index, p.chapter_ratio);
        let n = chapters.len().max(1) as f64;
        p.chapter_index = new_idx as i64;
        p.chapter_ratio = new_ratio;
        p.pct = (((new_idx as f64 + new_ratio) / n) * 100.0).clamp(0.0, 100.0);
        let _ = db::set_progress(tx, book_id, &p);
    }
    let mut first_by_legacy: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for (i, c) in chapters.iter().enumerate() {
        first_by_legacy.entry(c.legacy as i64).or_insert(i as i64);
    }
    for (legacy, new_idx) in &first_by_legacy {
        tx.execute(
            "UPDATE bookmarks SET chapter_index=?1 WHERE book_id=?2 AND chapter_index=?3",
            rusqlite::params![new_idx, book_id, legacy],
        )
        .ok();
    }
}

/// P11/R6-B：图片页书切章/细分后的一次性位置迁移。标记键 epub.split_migrated.<book_id>，
/// 值 = "{size}-{mtime}|{layout_sig}"（layout_sig：P11 锚点原布局 v1 / 块级切章或超大章细分后 v2）。
/// 同一事务内先重查标记：多窗口并发打开时只有第一个迁移，其余直接提交返回。
/// 四类动作（split_marker_action）：Noop（完全匹配）/ Rewrite（旧格式同 stamp 且仍 v1）/
/// LayoutUpgrade（同 stamp 但 v1→v2，用 base_chapters 按全书图片累计位置映射）/
/// LegacyMigrate（无标记或文件替换，按新 chapters 的 legacy 字段映射）。
/// updated_at 保持原值，避免迁移被当成一次新阅读影响「按最近阅读」或同步排序。
#[allow(clippy::too_many_arguments)]
fn migrate_epub_split_positions(
    state: &Arc<AppState>,
    book_id: i64,
    size: i64,
    mtime: i64,
    chapters: &[crate::epub::EpubChapter],
    base_chapters: Option<&[crate::epub::EpubChapter]>,
    layout_sig: &str,
) {
    if chapters.is_empty() {
        return;
    }
    let key = format!("epub.split_migrated.{book_id}");
    let stamp = format!("{size}-{mtime}");
    let marker = format!("{stamp}|{layout_sig}");
    let mut conn = state.db.0.lock().unwrap();
    let action = split_marker_action(db::get_setting(&conn, &key).as_deref(), &stamp, layout_sig);
    if action == MarkerAction::Noop {
        return;
    }
    let Ok(tx) = conn.transaction() else {
        return;
    };
    let action = split_marker_action(db::get_setting(&tx, &key).as_deref(), &stamp, layout_sig);
    if action == MarkerAction::Noop {
        let _ = tx.commit();
        return;
    }
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[split-migrate] book={book_id} action={action:?} sig={layout_sig} chapters={} base={}",
            chapters.len(),
            base_chapters.map(|b| b.len()).unwrap_or(0)
        );
    }
    let base = base_chapters.unwrap_or(&[]);
    if action == MarkerAction::LayoutUpgrade {
        if !base.is_empty() {
            if crate::epub::same_chapter_layout(base, chapters) {
                // R7-1：布局没变（如 776MB 样本 v2→v3 只是签名变化）→ 只重写 marker，
                // progress/bookmarks 坐标原样保留，不做任何数据迁移。
                if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
                    eprintln!("[split-migrate] book={book_id} same_layout => marker only");
                }
            } else {
                // 旧布局 → 新布局：按全书图片累计位置映射。
                if let Some(mut p) = db::get_progress(&tx, book_id) {
                    let (new_idx, new_ratio) = crate::epub::map_split_layout_position(
                        base,
                        chapters,
                        p.chapter_index,
                        p.chapter_ratio,
                    );
                    let n = chapters.len().max(1) as f64;
                    p.chapter_index = new_idx as i64;
                    p.chapter_ratio = new_ratio;
                    p.pct = (((new_idx as f64 + new_ratio) / n) * 100.0).clamp(0.0, 100.0);
                    let _ = db::set_progress(&tx, book_id, &p);
                }
                // 书签：旧章号（ratio=0）→ 新章号（书签无章内偏移）。
                let mut rows: Vec<(i64, i64)> = Vec::new();
                if let Ok(mut stmt) =
                    tx.prepare("SELECT id, chapter_index FROM bookmarks WHERE book_id=?1")
                {
                    if let Ok(it) = stmt.query_map([book_id], |r| Ok((r.get(0)?, r.get(1)?))) {
                        for row in it.flatten() {
                            rows.push(row);
                        }
                    }
                }
                for (bid, old_idx) in rows {
                    let (new_idx, _) =
                        crate::epub::map_split_layout_position(base, chapters, old_idx, 0.0);
                    tx.execute(
                        "UPDATE bookmarks SET chapter_index=?1 WHERE id=?2",
                        rusqlite::params![new_idx as i64, bid],
                    )
                    .ok();
                }
            }
        } else {
            // R7-1：base 缺失（无法重建旧布局）时退化为 legacy 迁移，不得静默重写旧坐标。
            legacy_migrate_positions(&tx, book_id, chapters);
        }
    } else if action == MarkerAction::LegacyMigrate {
        legacy_migrate_positions(&tx, book_id, chapters);
    }
    // M31：marker 写入失败直接丢弃事务（下次仍会迁移，不写假标记）。
    if let Err(e) = db::set_setting_checked(&tx, &key, &marker) {
        eprintln!("[split-migrate] book={book_id} marker write failed: {e}");
        return;
    }
    // M31：commit 失败必须留痕；事务回滚后 marker 不生效，下次打开会重试迁移。
    if let Err(e) = tx.commit() {
        eprintln!("[split-migrate] book={book_id} action={action:?} commit failed: {e}");
    }
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
    // 任何窗口打开书都会更新“最后阅读时间”：广播给所有窗口，主窗书库据此即时刷新
    // 「最近阅读」顺序（独立书窗打开时不经过主窗，没有这个事件要重启后才正确）。
    state.app.emit("book-opened", ()).ok();
    let ext = ext_of(&book.path).unwrap_or_default();
    let st = state.inner().clone();
    // 开书是最权威的元信息来源：顺手写入元信息缓存，后续 get_chapter 不必每章抢 DB 锁。
    push_cache(&st.book_meta, id, (book.path.clone(), ext.clone()), 1024);
    let t_open = std::time::Instant::now();
    // 同书重复打开（弹出目录/面板窗会再调 openBook）不失效已渲染章节；只有真换书才整体清空
    // （换书仍整体清空：内存有界；章节缓存 key 已含 book_id，不会串书）。
    if st.last_open_book.swap(id, Ordering::Relaxed) != id {
        st.chapter_cache.lock().unwrap().clear();
    }
    // 阶段 1：按格式加载解析结果（各格式现有路径与耗时不变；EPUB 只持有 Arc，不复制正文）。
    // 解析 7000+ 章的大书可达数秒：全部放阻塞线程池，避免占用 async 运行时。
    let mut spine_n: usize = 0;
    let mut epub_cached: Option<Arc<CachedEpub>> = None;
    let mut txt_cached: Option<Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>> = None;
    let mut mobi_cached: Option<Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>> = None;
    let mut pdf_toc: Option<Vec<TocEntry>> = None;
    let mut warn: Option<String> = None;
    if ext == "epub" {
        let st2 = st.clone();
        let bpath = book.path.clone();
        let cached = tauri::async_runtime::spawn_blocking(move || load_epub(&st2, &bpath, id))
            .await
            .map_err(|e| e.to_string())??;
        spine_n = cached.doc.spine.len();
        epub_cached = Some(cached);
    } else if ext == "txt" {
        let st2 = st.clone();
        let bpath = book.path.clone();
        let cached = tauri::async_runtime::spawn_blocking(move || load_txt(&st2, &bpath, id))
            .await
            .map_err(|e| e.to_string())??;
        txt_cached = Some(cached);
    } else if ext == "pdf" {
        // PDF：后端不解析内容，正文由前端 pdf.js 逐页渲染（见 Reader.svelte）。
        // 目录 = 页码列表，页数来自前端上次上报的设置项；缺失时先给空目录。
        pdf_toc = Some(pdf_page_toc(&st, id));
    } else {
        let st2 = st.clone();
        let bpath = book.path.clone();
        let (cached, warning) = tauri::async_runtime::spawn_blocking(move || load_mobi(&st2, &bpath, id))
            .await
            .map_err(|e| e.to_string())??;
        mobi_cached = Some(cached);
        warn = warning;
    }
    // 目录条数（漫画判定分母；EPUB 用 legacy toc —— 切章不改变漫画/图片页判定语义）。
    let toc_n = if let Some(c) = &epub_cached {
        c.doc.toc.len()
    } else if let Some(c) = &txt_cached {
        c.2.len()
    } else if let Some(c) = &mobi_cached {
        c.2.len()
    } else {
        pdf_toc.as_ref().map(|t| t.len()).unwrap_or(0)
    }
    .max(1);

    // 阶段 2：图片列表 + 漫画判定（顺序与各格式原有逻辑一致，只读必要正文）。
    let images = collect_book_images(&st, id).unwrap_or_default();
    let images = image_cache_put(&st, id, file_stamp(&book.path), images.clone(), images);
    let mut body_chars = 0i64;
    let mut scanned_bytes = 0usize;
    let mut is_manga = false;
    if manga_image_gate(images.len(), spine_n, toc_n, &ext) {
        if let Some(c) = &epub_cached {
            // C13：大文件跳过整本探针（预算边界 32MB），避免 990MB 图片型书瞬时读入整本。
            let small = std::fs::metadata(&c.path).map(|m| m.len() <= 32 * 1024 * 1024).unwrap_or(false);
            if small {
                if let Some(data) = c.read_all() {
                    let (b, used) = manga_body_chars_epub(&data, &c.doc.spine, &c.doc.toc);
                    body_chars = b;
                    scanned_bytes = used;
                }
            }
        } else if let Some(c) = &mobi_cached {
            body_chars = manga_body_chars_mobi(&c.0, &c.1, &c.2);
        }
        is_manga = is_manga_like(images.len(), spine_n, toc_n, &ext, body_chars);
    }

    // 阶段 3：图片页判定 + R7-1 泛化/受限细分 + R6-1 块级切章 + R6-2 划线门控（仅 EPUB）。
    let mut image_pages = false;
    // R7-1：patch 前保留旧布局（base_chapters）供迁移映射；desired 签名 v1/v2/v3。
    let mut split_base_chapters: Option<Vec<crate::epub::EpubChapter>> = None;
    let mut split_layout_sig = "v1";
    if let Some(cached) = epub_cached.clone() {
        // 折叠目录候选（R6-1）：P11 锚点切分绝对优先（split=true 不进入）；普通多章书 toc>2 不进入。
        let fold_candidate = !cached.doc.split && cached.doc.toc.len() <= 2;
        // C13：打开路径只允许小文件临时整本视图（≤32MB）；大文件探针/切章计划一律跳过，
        // 保持文本模式（P11 已切分的 split 布局不受影响）。
        let probe_data: Option<Vec<u8>> = if std::fs::metadata(&cached.path).map(|m| m.len() <= 32 * 1024 * 1024).unwrap_or(false) {
            cached.read_all()
        } else {
            None
        };
        let mut max_doc_imgs = 0usize;
        if fold_candidate && images.len() >= IMAGE_PAGES_MIN_IMAGES {
            if let Some(data) = &probe_data {
                max_doc_imgs =
                    crate::epub::spine_doc_max_img_count(data, &cached.ci, &cached.doc.spine);
            }
        }
        let block_candidate =
            fold_candidate && max_doc_imgs > crate::epub::AUTO_BLOCK_IMGS_PER_CHAPTER;
        // R7-1 候选：P11 已切且存在单章实际图片数 >200（不再限制章数；春秋 17 章进入）。
        let mut max_split_imgs = 0usize;
        if cached.doc.split && images.len() >= IMAGE_PAGES_MIN_IMAGES {
            if let Some(data) = &probe_data {
                max_split_imgs =
                    crate::epub::max_split_chapter_imgs(&cached.doc, data, &cached.ci);
            }
        }
        let subdiv_candidate = cached.doc.split
            && max_split_imgs > crate::epub::AUTO_BLOCK_IMGS_PER_CHAPTER;
        // R6-2：有划线且未迁移 → 不自动图片页排版、不自动切章（保持文字模式让划线可见）。
        let (has_highlights, migrated, cur_marker) = if images.len() >= IMAGE_PAGES_MIN_IMAGES {
            let conn = st.db.0.lock().unwrap();
            let hl = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM highlights WHERE book_id=?1)",
                    [id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            let marker = db::get_setting(&conn, &format!("epub.split_migrated.{id}"));
            let migrated = marker.is_some();
            (hl, migrated, marker)
        } else {
            (false, false, None)
        };
        let current_sig: Option<&str> = marker_layout_sig(cur_marker.as_deref());
        // 探测：普通书（图片<100）零成本；漫画书只在块级切章/细分候选上探测
        // （主样本 is_manga=true 也要拿到数据）；image_pages 排版标记仍要求 !is_manga，复用同一次结果。
        if images.len() >= IMAGE_PAGES_MIN_IMAGES
            && (!is_manga || block_candidate || subdiv_candidate)
        {
            let t_probe = std::time::Instant::now();
            // 大文件无 probe_data：用空探测结果（complete=false → dominant=false），
            // 因 max_doc_imgs/max_split_imgs 也为 0，block/subdiv gate 同样不触发。
            let probe = match probe_data.as_ref() {
                Some(data) => crate::epub::image_page_text_probe(
                    data,
                    &cached.doc.spine,
                    images.len(),
                    MANGA_SCAN_BUDGET,
                ),
                None => crate::epub::ImagePageProbe { text_chars: 0, scanned_bytes: 0, complete: false },
            };
            let dominant = is_image_page_like(
                images.len(),
                probe.text_chars,
                probe.complete,
                cached.doc.spine.len(),
            );
            image_pages = !is_manga && dominant && !has_highlights;
            if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
                let ratio = if images.is_empty() {
                    0.0
                } else {
                    probe.text_chars as f64 / images.len() as f64
                };
                eprintln!(
                    "[open] image_pages id={id} images={} text={} ratio={:.1} complete={} scanned={} probe_ms={} dominant={} has_highlights={} => {image_pages}",
                    images.len(),
                    probe.text_chars,
                    ratio,
                    probe.complete,
                    probe.scanned_bytes,
                    t_probe.elapsed().as_millis(),
                    dominant,
                    has_highlights
                );
            }
            let block_gate = should_auto_block_split(
                cached.doc.split,
                cached.doc.toc.len(),
                max_doc_imgs,
                dominant,
                has_highlights,
                migrated,
            );
            // R7-1 门控：存在单章实际图片 >200 + 图片主导度 + 划线门控（章数不再限制）。
            let subdiv_gate = subdiv_candidate && dominant && (!has_highlights || migrated);
            if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
                eprintln!(
                    "[open] split_candidates id={id} block={block_candidate} subdiv={subdiv_candidate} max_doc_imgs={max_doc_imgs} max_split_imgs={max_split_imgs} cur_sig={current_sig:?} dominant={dominant} has_highlights={has_highlights} migrated={migrated} => block_gate={block_gate} subdiv_gate={subdiv_gate}"
                );
            }
            if block_gate {
                let data = probe_data.clone().ok_or("读取 EPUB 失败")?;
                let ci = cached.ci.clone();
                let doc = cached.doc.clone();
                let t_plan = std::time::Instant::now();
                let plan = crate::epub::plan_block_split(
                    &doc,
                    &data,
                    &ci,
                    crate::epub::AUTO_BLOCK_IMGS_PER_CHAPTER,
                    crate::epub::AUTO_BLOCK_MAX_CHAPTERS,
                );
                if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
                    eprintln!(
                        "[open] auto_split kind=block id={id} max_doc_imgs={max_doc_imgs} cur_sig={current_sig:?} plan_ms={} => chapters={}",
                        t_plan.elapsed().as_millis(),
                        plan.as_ref().map(|p| p.1.len()).unwrap_or(0)
                    );
                }
                if let Some((split_toc, chapters)) = plan {
                    // 旧布局：v2 就是同一次块级切章结果（确定性重算）；v1/无 marker 是未切分布局。
                    let old_layout = if current_sig == Some("v2") {
                        Some(chapters.clone())
                    } else {
                        Some(doc.chapters.clone())
                    };
                    let mut patched_doc = doc.clone();
                    patched_doc.split = true;
                    patched_doc.split_toc = split_toc;
                    patched_doc.chapters = chapters;
                    patched_doc.auto_block_split = true;
                    let patched = Arc::new(CachedEpub {
                        doc: patched_doc,
                        path: cached.path.clone(),
                        ci: ci.clone(),
                        stamp: cached.stamp,
                    });
                    push_epub_cache(&st, id, patched.clone());
                    epub_cached = Some(patched);
                    split_base_chapters = old_layout;
                    // R7-1：块级切章布局统一签名为 v3（与第六轮 v2 布局相同 → 迁移只重写 marker）。
                    split_layout_sig = "v3";
                }
            } else if subdiv_gate {
                let data = probe_data.clone().ok_or("读取 EPUB 失败")?;
                let ci = cached.ci.clone();
                let doc = cached.doc.clone();
                let t_plan = std::time::Instant::now();
                // 无划线：优先泛化细分（v3）；有划线：按现有签名恢复既有布局。
                let general_plan = if !has_highlights || current_sig == Some("v3") {
                    crate::epub::plan_subdivide_oversized_chapters_mode(
                        &doc,
                        &data,
                        &ci,
                        crate::epub::AUTO_BLOCK_IMGS_PER_CHAPTER,
                        crate::epub::AUTO_BLOCK_MAX_CHAPTERS,
                        crate::epub::SubdivideMode::General,
                    )
                } else {
                    None
                };
                let restricted_plan = if current_sig == Some("v2") {
                    crate::epub::plan_subdivide_oversized_chapters(
                        &doc,
                        &data,
                        &ci,
                        crate::epub::AUTO_BLOCK_IMGS_PER_CHAPTER,
                        crate::epub::AUTO_BLOCK_MAX_CHAPTERS,
                    )
                } else {
                    None
                };
                let target = choose_split_layout(
                    has_highlights,
                    current_sig,
                    general_plan.is_some(),
                    restricted_plan.is_some(),
                );
                // 旧布局 base：v2→受限计划、v3→泛化计划、v1/无 marker→P11 原布局；
                // 计划无法重建时置 None，迁移退化为 legacy 映射（不静默重写旧坐标）。
                let base: Option<Vec<crate::epub::EpubChapter>> = match current_sig {
                    Some("v2") => restricted_plan.as_ref().map(|p| p.1.clone()),
                    Some("v3") => general_plan.as_ref().map(|p| p.1.clone()),
                    _ => Some(doc.chapters.clone()),
                };
                let general_len = general_plan.as_ref().map(|p| p.1.len()).unwrap_or(0);
                let restricted_len = restricted_plan.as_ref().map(|p| p.1.len()).unwrap_or(0);
                if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
                    eprintln!(
                        "[open] layout_target id={id} cur_sig={current_sig:?} target={target:?} general_chapters={general_len} restricted_chapters={restricted_len} plan_ms={}",
                        t_plan.elapsed().as_millis()
                    );
                }
                let chosen = match target {
                    SplitLayoutTarget::General => general_plan,
                    SplitLayoutTarget::Restricted => restricted_plan,
                    SplitLayoutTarget::KeepP11 => None,
                };
                if let Some((split_toc, chapters)) = chosen {
                    let mut patched_doc = doc.clone();
                    patched_doc.split = true;
                    patched_doc.split_toc = split_toc;
                    patched_doc.chapters = chapters;
                    patched_doc.auto_block_split = true;
                    let patched = Arc::new(CachedEpub {
                        doc: patched_doc,
                        path: cached.path.clone(),
                        ci: ci.clone(),
                        stamp: cached.stamp,
                    });
                    push_epub_cache(&st, id, patched.clone());
                    epub_cached = Some(patched);
                    split_base_chapters = base;
                    split_layout_sig = if target == SplitLayoutTarget::General {
                        "v3"
                    } else {
                        "v2"
                    };
                } else {
                    // 保持 P11 原布局；若 marker 是 v2/v3，迁移会把旧坐标映射回 v1。
                    split_base_chapters = base;
                    split_layout_sig = "v1";
                }
            }
        }
    }

    // 阶段 4：构建 toc/docs（EPUB 切分书走 split 分支：迁移 + 搜索缓存按书懒清理）。
    let (toc, docs) = if let Some(cached) = &epub_cached {
        if cached.doc.split {
            // P11/R6-1/R6-B/R7-1：切分书按虚拟章给目录与文档清单（docs 与 toc 等长，spine 文档可重复）。
            // 迁移标记带布局签名：P11 锚点原布局 v1 / 第六轮受限细分 v2 / 第七轮泛化细分或块级切章 v3。
            // 顺手做一次进度/书签迁移（已迁移过则为空操作；布局变化时用 base_chapters 映射）。
            let layout_sig = split_layout_sig;
            migrate_epub_split_positions(
                &st,
                id,
                book.size,
                book.mtime,
                &cached.doc.chapters,
                split_base_chapters.as_deref(),
                layout_sig,
            );
            // 切章/细分后旧搜索缓存按书懒清理：只删该书 chapter_text 并给 fts_meta 写负哨兵，
            // 不做 DELETE FROM fts（book_id 是 UNINDEXED，全表扫描会卡住全局 DB 锁）。
            // marker 带布局签名：v1→v2 后会重新清理一次，避免旧布局缓存残留。
            {
                let conn = st.db.0.lock().unwrap();
                db::invalidate_split_book_search_cache(
                    &conn,
                    id,
                    cached.doc.chapters.len() as i64,
                    &format!("{}-{}|{}", book.size, book.mtime, layout_sig),
                );
            }
            let docs: Vec<String> = cached
                .doc
                .chapters
                .iter()
                .map(|c| {
                    cached
                        .doc
                        .spine
                        .get(c.spine)
                        .map(|s| s.rsplit('/').next().unwrap_or(s).to_string())
                        .unwrap_or_default()
                })
                .collect();
            (cached.doc.split_toc.clone(), docs)
        } else {
            let docs: Vec<String> = cached
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
            (cached.doc.toc.clone(), docs)
        }
    } else if let Some(c) = &txt_cached {
        (c.2.clone(), Vec::new())
    } else if let Some(t) = &pdf_toc {
        (t.clone(), Vec::new())
    } else if let Some(c) = &mobi_cached {
        (c.2.clone(), Vec::new())
    } else {
        (Vec::new(), Vec::new())
    };
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[open] open_book id={id} ext={ext} total={:?} toc={}",
            t_open.elapsed(),
            toc.len()
        );
        eprintln!(
            "[open] is_manga id={id} images={} spine={} toc={} body_chars={} scanned={} => {is_manga}",
            images.len(),
            spine_n,
            toc_n,
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
        image_pages,
    })
}

#[tauri::command]
pub async fn get_chapter(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    chapter_index: i64,
    keep_styles: bool,
) -> Result<ChapterData, String> {
    // 元信息走进程内缓存（P3）：命中路径完全不碰 state.db.0，翻章不再与后台建索引抢全局锁；
    // 未命中回退 DB（书籍不存在时的错误行为不变）。
    let (path, ext) = cached_book_meta(state.inner(), book_id).ok_or("书籍不存在")?;
    let idx = chapter_index.max(0) as usize;
    let st = state.inner().clone();

    // S1：缓存命中必须校验文件指纹（同路径文件被替换后不能返回旧正文）。
    // PDF 走 chapter_text/页位图，文件在本函数不参与解析；取指纹是廉价的 metadata 调用。
    let cache_stamp = file_stamp(&path);
    if let Some(hit) = st
        .chapter_cache
        .lock()
        .unwrap()
        .get((book_id, idx as i64, keep_styles), cache_stamp)
    {
        return Ok(hit.as_ref().clone());
    }

    if ext == "epub" {
        // 解析/渲染放到阻塞线程池，避免占用 async 运行时拖死其它 IPC。
        let st2 = st.clone();
        let bpath = path.clone();
        let t_render = std::time::Instant::now();
        let data = tauri::async_runtime::spawn_blocking(move || -> Result<ChapterData, String> {
            let cached = load_epub(&st2, &bpath, book_id)?;
            let doc = &cached.doc;
            // P11：切分书按虚拟章渲染：切片章只渲染自身 body 区间（图片 URL/样式与整篇一致），
            // 整篇 fallback 章仍走多文件渲染；切片章不参与字数统计（chars=0）。
            if doc.split {
                let (title, html, chars) = match doc.chapters.get(idx) {
                    Some(c) => {
                        if let Some(start) = c.start {
                            let dp = doc.spine.get(c.spine).cloned().unwrap_or_default();
                            let raw = crate::epub::read_entry_file(std::path::Path::new(&cached.path), &cached.ci, &dp)
                                .map(|b| String::from_utf8_lossy(&b).into_owned())
                                .unwrap_or_default();
                            let html = crate::epub::chapter_html_fragment_file(
                                std::path::Path::new(&cached.path),
                                &cached.ci,
                                &dp,
                                &raw,
                                start,
                                c.end,
                                Some(book_id),
                                keep_styles,
                            )?;
                            (c.title.clone(), html, 0)
                        } else {
                            let end = c.spine_end.min(doc.spine.len());
                            let docs: Vec<String> = (c.spine..end)
                                .filter_map(|si| doc.spine.get(si).cloned())
                                .collect();
                            let html = crate::epub::chapter_html_multi_file(
                                std::path::Path::new(&cached.path),
                                &cached.ci,
                                &docs,
                                Some(book_id),
                                keep_styles,
                            )
                            .unwrap_or_default();
                            let chars: i64 = (c.spine..end)
                                .map(|si| doc.spine_chars.get(si).copied().unwrap_or(0))
                                .sum();
                            (c.title.clone(), html, chars)
                        }
                    }
                    None => (format!("第 {} 节", idx + 1), String::new(), 0),
                };
                return Ok(ChapterData { title, html, chars });
            }
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
            let mut html = crate::epub::chapter_html_multi_file(
                std::path::Path::new(&cached.path),
                &cached.ci,
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
                    html = crate::epub::chapter_html_ci_file(
                        std::path::Path::new(&cached.path),
                        &cached.ci,
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
        st.chapter_cache.lock().unwrap().put(
            (book_id, idx as i64, keep_styles),
            cache_stamp,
            Arc::new(data.clone()),
        );
        return Ok(data);
    }

    if ext == "pdf" {
        // PDF 正文由阅读器前端用 pdf.js 渲染成位图，后端没有「按章取文」这条路。
        return Err("PDF 由阅读器前端渲染，不支持按章取文".into());
    }

    let is_mobi = is_mobi_ext(&ext);
    // MOBI：解析统一走 load_mobi（自己带缓存判据），不再在本函数里内联第二套逻辑。
    // TXT：保留原来的内联缓存快路径（命中不额外 spawn_blocking），未命中走 load_txt。
    let entry: Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)> = if is_mobi {
        let st2 = st.clone();
        let bpath = path.clone();
        tauri::async_runtime::spawn_blocking(move || load_mobi(&st2, &bpath, book_id).map(|r| r.0))
            .await
            .map_err(|e| e.to_string())??
    } else {
        let ver = st.rules_ver.load(Ordering::Relaxed);
        // 与 load_txt 同一套判据：规则版本 + 文件指纹都对上才复用。
        let stamp = file_stamp(&path);
        let cached_hit = {
            let m = st.txt_cache.lock().unwrap();
            m.get(&book_id)
                .filter(|c| cache_entry_valid(c.ver, ver, c.stamp, stamp))
                .map(|c| c.doc.clone())
        };
        match cached_hit {
            Some(c) => c,
            None => {
                let st2 = st.clone();
                let bpath = path.clone();
                let entry = tauri::async_runtime::spawn_blocking(move || load_txt(&st2, &bpath, book_id))
                    .await
                    .map_err(|e| e.to_string())??;
                // 指纹取不到就不入缓存：无法校验的条目会一直命中。
                if stamp.is_some() {
                    push_cache(
                        &st.txt_cache,
                        book_id,
                        CachedText { ver, stamp, doc: entry.clone(), warn: None },
                        3,
                    );
                }
                entry
            }
        }
    };
    let (content, ranges, toc) = entry.as_ref();
    let (start, end) = ranges.get(idx).copied().ok_or("章节越界")?;
    let body = safe_slice(content, start, end);
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
    st.chapter_cache.lock().unwrap().put(
        (book_id, idx as i64, keep_styles),
        cache_stamp,
        Arc::new(data.clone()),
    );
    Ok(data)
}

#[tauri::command]
pub async fn save_progress(state: State<'_, Arc<AppState>>, book_id: i64, p: Progress) -> Result<(), String> {
    let mut p = p;
    // 时间戳在命令入口（spawn 之前）取：异步化后闭包里再取可能与新请求反序，导致旧进度反超。
    p.updated_at = db::now_ms();
    p.device = "tidepage-pc".into();
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::set_progress_monotonic(&conn, book_id, &p)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn load_progress(state: State<'_, Arc<AppState>>, book_id: i64) -> Option<Progress> {
    let conn = state.db.0.lock().unwrap();
    db::get_progress(&conn, book_id)
}

/// 清理书籍的可再生资源：封面文件（covers 目录内）与 res/<book_id>/ 解包目录。
/// 只处理这两类可再生资源；笔记/划线由 delete_books_policy 负责。失败只记日志，不影响删除结果。
fn cleanup_book_artifacts(state: &Arc<AppState>, items: &[(i64, Option<String>)]) {
    cleanup_book_artifacts_for(&state.app, items);
}

/// 同上，按 AppHandle 执行：scan_folders 只持有 &AppState，无法构造 Arc。
fn cleanup_book_artifacts_for(app: &AppHandle, items: &[(i64, Option<String>)]) {
    if items.is_empty() {
        return;
    }
    let covers_dir = covers::covers_dir(app);
    let res_dir = crate::data_dir(app).join("res");
    for (id, cover) in items {
        if let Some(name) = cover.as_deref().filter(|s| !s.trim().is_empty()) {
            // 只删 covers 目录内的单一文件名（与协议端点同一套校验），防越界删除。
            if let Some(p) = crate::resolve_under(&covers_dir, name) {
                if let Err(e) = std::fs::remove_file(&p) {
                    if p.exists() {
                        eprintln!(
                            "[cleanup] cover remove failed id={id} path={} err={e}",
                            p.display()
                        );
                    }
                }
            }
        }
        let dir = res_dir.join(id.to_string());
        if dir.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                eprintln!("[cleanup] res remove failed id={id} dir={} err={e}", dir.display());
            }
        }
    }
    // S1：解析缓存改为本批一次扫描删除（旧实现每本书各 read_dir 一次，O(书数×缓存文件数)）。
    let ids: Vec<i64> = items.iter().map(|(id, _)| *id).collect();
    crate::parsecache::cleanup_books(&crate::data_dir(app).join("parse_cache"), &ids);
}

/// 删除书籍。`fromDisk`=删除本地文件；`keepNotes`=保留带笔记的划线（默认 true）。
/// 前端需先调用 `book_note_info` 判断是否有笔记并弹窗让用户选择。
///
/// 顺序约定：
/// - fromDisk=true：先逐条移入回收站，只有成功的路径才删库；失败的保留行与文件，可重试。
/// - fromDisk=false（移出书库）：不碰文件，删库的同一事务写入 ignored_paths，
///   使该路径长期忽略；直到所在目录被移除后重新添加才会重新导入。
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
    let outcome = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        // 与 remove_folder 对齐：分批(2000/批) + 每批独立事务，消除「每本 7~8 条 DELETE
        // 各自自动提交 = 8N 次 fsync」的大批量删除慢。批间释放 db 锁，避免长持锁冻结其它读。
        let t0 = std::time::Instant::now();
        // fromDisk=true：先物理删除。只有真的进回收站的路径才进 DB 删除；
        // 失败的路径保留书库行与文件，用户可重试（旧实现先删库，文件删不掉也返回成功）。
        let mut to_delete = paths;
        let mut trash_failed = 0usize;
        if from_disk {
            let attempted = to_delete.len();
            let mut ok_paths: Vec<String> = Vec::with_capacity(attempted);
            for (i, p) in to_delete.iter().enumerate() {
                match trash::delete(p) {
                    Ok(()) => {
                        log_op(
                            "delete_books",
                            &format!("trash {}/{} path={p} ok", i + 1, attempted),
                        );
                        ok_paths.push(p.clone());
                    }
                    Err(e) => {
                        trash_failed += 1;
                        log_op(
                            "delete_books",
                            &format!("trash {}/{} path={p} err={e:?}", i + 1, attempted),
                        );
                    }
                }
            }
            log_op(
                "delete_books",
                &format!(
                    "trash done ok={} failed={} attempted={attempted} elapsed_ms={}",
                    ok_paths.len(),
                    trash_failed,
                    t0.elapsed().as_millis()
                ),
            );
            to_delete = ok_paths;
        }

        let total_batches = to_delete.len().div_ceil(2000).max(1);
        let mut deleted_ok = 0usize;
        for (i, chunk) in to_delete.chunks(2000).enumerate() {
            log_op(
                "delete_books",
                &format!(
                    "batch={}/{} rows={} from_disk={} keep_notes={} start",
                    i + 1,
                    total_batches,
                    chunk.len(),
                    from_disk,
                    keep
                ),
            );
            // 删除前先收集可再生资源清单（封面/res）：行删掉后就查不到了。
            let artifacts = {
                let conn = st.db.0.lock().unwrap();
                db::book_artifacts_by_paths(&conn, chunk)
            };
            let r: Result<(), String> = (|| {
                let mut guard = st.db.0.lock().unwrap();
                let tx = guard.transaction().map_err(|e| e.to_string())?;
                db::delete_books_policy(&tx, chunk, keep)?;
                if !from_disk {
                    // 移出书库：忽略记录与删行同一事务，避免「行已删、忽略未写」被扫描立刻导回。
                    db::add_ignored_paths(&tx, chunk)?;
                }
                tx.commit().map_err(|e| e.to_string())?;
                Ok(())
            })();
            if let Err(e) = r {
                // 失败语义保持不变：任一批失败立即中止返回（后面的批次不再执行）。
                log_op(
                    "delete_books",
                    &format!(
                        "failed_at_batch={}/{} rows_in_failed_batch={} committed_batches={} err={e} elapsed_ms={}",
                        i + 1,
                        total_batches,
                        chunk.len(),
                        i,
                        t0.elapsed().as_millis()
                    ),
                );
                return Err(e);
            }
            deleted_ok += chunk.len();
            // 事务已提交：清理这批书的封面与 res 资源（失败只记日志）。
            cleanup_book_artifacts(&st, &artifacts);
            log_op(
                "delete_books",
                &format!(
                    "batch={}/{} ok deleted_ok={} elapsed_ms={}",
                    i + 1,
                    total_batches,
                    deleted_ok,
                    t0.elapsed().as_millis()
                ),
            );
        }
        log_op(
            "delete_books",
            &format!(
                "done deleted_ok={} trash_failed={} from_disk={} keep_notes={} elapsed_ms={}",
                deleted_ok,
                trash_failed,
                from_disk,
                keep,
                t0.elapsed().as_millis()
            ),
        );
        if from_disk && trash_failed > 0 {
            return Err(format!(
                "已删除 {deleted_ok} 本；{trash_failed} 本文件删除失败，仍保留在书库，可重试"
            ));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    // 部分失败时也有书被删除/移出：仍要通知前端刷新书库与笔记视图（格式集可能变化）。
    bump_formats_epoch(state.inner());
    app.emit("library-changed", ()).ok();
    app.emit("notes-changed", ()).ok();
    // 删过的书不再需要元信息缓存（st 已被 move 进闭包，这里用 state 访问同一份表）。
    state.book_meta.lock().unwrap().clear();
    outcome
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
/// H11：改 async + 阻塞线程池；Tauri async 命令带引用入参须返回 Result，
/// 前端 invoke 仍拿到数组（join 失败返回空数组，与旧实现一致）。
#[tauri::command]
pub async fn list_note_books(state: State<'_, Arc<AppState>>) -> Result<Vec<serde_json::Value>, String> {
    let st = state.inner().clone();
    let rows = tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
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
    })
    .await
    .unwrap_or_default();
    Ok(rows)
}

/// 批量判断选中书里是否有任何一本带笔记（批量删除时决定是否显示「笔记处理方式」）。
/// 只读一行 SQL 的 EXISTS 汇总，不逐本读笔记内容。
#[tauri::command]
pub fn books_have_notes(state: State<'_, Arc<AppState>>, book_ids: Vec<i64>) -> bool {
    let conn = state.db.0.lock().unwrap();
    db::any_book_has_notes(&conn, &book_ids)
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
        // S1：只允许预览 txt/mobi/azw/azw3 且必须存在（前端只会传 txt）。
        validate_source_path(&path, &["txt", "mobi", "azw", "azw3"])?;
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
    // M32：设置写入失败必须上抛；只有写入成功才递增版本并清缓存，
    // 否则规则没落库但缓存被清、版本已增，下次启动仍按旧规则解析。
    {
        let conn = state.db.0.lock().unwrap();
        db::set_setting_checked(&conn, "txt_rules", &pattern)?;
    }
    state.rules_ver.fetch_add(1, Ordering::Relaxed);
    state.txt_cache.lock().unwrap().clear();
    state.chapter_cache.lock().unwrap().clear();
    Ok(())
}

/// R8-1：按需读取 MOBI/AZW3 正文图片。请求级快路径：进程内清单缓存 → 直接文件读取 →
/// 最后才查 DB/目录三态（避免每张图都读整份清单或扫 res 目录）。
pub fn read_mobi_res_image(state: &AppState, book_id: i64, name: &str) -> Option<(String, Vec<u8>)> {
    if book_id <= 0 {
        return None;
    }
    let mime = crate::mobi::res_mime_for_name(name)?;
    let out_dir = crate::data_dir(&state.app).join("res").join(book_id.to_string());
    // a. 清单缓存命中（按 book_id，不查 DB、不读盘）：Lazy 模式下绝大多数图片走这里。
    let cached = {
        let c = state.mobi_manifest_cache.lock().unwrap();
        c.iter()
            .find(|(id, _, _, _)| *id == book_id)
            .map(|(_, _, _, m)| m.clone())
    };
    if let Some(m) = cached {
        if let Some(v) = crate::mobi::read_res_image_manifest(&m, name, None) {
            return Some(v);
        }
    }
    // b. 直接文件回退（旧 Files 目录/已落盘图片）：不扫目录、不查 DB。
    if let Some(v) = crate::mobi::read_res_image_file(&out_dir, name) {
        return Some(v);
    }
    // c. 仍未命中：取 DB 当前路径/指纹后按目录三态处理（Lazy 首次请求加载清单）。
    let book_path = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).map(|b| b.path)
    };
    let stamp = book_path.as_deref().and_then(crate::mobi::src_file_stamp);
    match crate::mobi::res_dir_state(&out_dir, stamp) {
        crate::mobi::ResDirState::Lazy => {
            let m = cached_mobi_manifest(state, book_id, &out_dir, stamp)?;
            crate::mobi::read_res_image_manifest(&m, name, book_path.as_deref())
        }
        _ => None,
    }
    .map(|(_, b)| (mime.to_string(), b))
}

/// R8-1：进程内缓存图片清单（容量 4，带源文件指纹；指纹变化/清单不符即视为未命中）。
fn cached_mobi_manifest(
    state: &AppState,
    book_id: i64,
    out_dir: &std::path::Path,
    stamp: Option<(i64, i64)>,
) -> Option<Arc<crate::mobi::ResManifest>> {
    let (size, mtime) = stamp?;
    {
        let mut c = state.mobi_manifest_cache.lock().unwrap();
        if let Some(pos) = c.iter().position(|(id, _, _, _)| *id == book_id) {
            let e = c.remove(pos);
            if e.1 == size && e.2 == mtime {
                c.push(e.clone());
                return Some(e.3);
            }
        }
    }
    let m = Arc::new(crate::mobi::read_res_manifest(out_dir)?);
    if m.size != size || m.mtime != mtime {
        return None;
    }
    let mut c = state.mobi_manifest_cache.lock().unwrap();
    c.retain(|(id, _, _, _)| *id != book_id);
    c.push((book_id, size, mtime, m.clone()));
    if c.len() > 4 {
        c.remove(0);
    }
    Some(m)
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
    let doc = crate::mobi::parse_full_mode(
        path,
        force.as_deref(),
        Some((&dir, &prefix)),
        crate::mobi::ResMode::Lazy,
    )?;
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

/// P0-2：把设置里的正文字符编码（与阅读共用 mobi.encoding）归一化为 decode 可用的 label。
/// 空/auto/未知编码名一律按 None（自动探测），未知非空值记一次日志但不让转换失败。
fn normalize_text_encoding(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        return None;
    }
    match encoding_rs::Encoding::for_label(s.as_bytes()) {
        Some(enc) => Some(enc.name().to_string()),
        None => {
            let msg = format!("[convert] 未知正文字符编码 {s:?}，回退自动探测");
            eprintln!("{msg}");
            crate::memlog::stage_note("convert:encoding", &msg);
            None
        }
    }
}

#[tauri::command]
pub async fn convert_book(
    state: tauri::State<'_, Arc<AppState>>,
    path: String,
    target: String,
    pattern: String,
    out_path: String,
    author: Option<String>,
    cover_path: Option<String>,
    css: Option<String>,
    title: Option<String>,
) -> Result<String, String> {
    // 前端 invoke 参数不变：State 由 Tauri 自动注入（不进入 JS 参数表）。
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let forced = {
            let conn = st.db.0.lock().unwrap();
            db::get_setting(&conn, "mobi.encoding")
        };
        let forced = normalize_text_encoding(forced.as_deref());
        let ext = ext_of(&path).unwrap_or_default();
        let is_mobi_like = is_mobi_ext(&ext);
        if (ext == "txt" && target == "epub")
            || (is_mobi_like && (target == "txt" || target == "epub"))
            || (ext == "epub" && target == "txt")
        {
            // S1：输出路径白名单按 target 决定（txt→txt，epub→epub）。
            let out_exts: &[&str] = if target == "txt" { &["txt"] } else { &["epub"] };
            validate_output_path(&out_path, out_exts, true)?;
            let mut book = if ext == "txt" {
                crate::convert::txt_to_epub_data(&path, &pattern, forced.as_deref())?
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

/// S1：Windows 保留设备名（不区分大小写；带扩展名同样不能用，如 CON.txt）。
const WINDOWS_RESERVED_STEMS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// S1：不得写入的系统目录环境变量（取不到该变量就跳过，不因此拒绝合法路径）。
const WINDOWS_SYSTEM_ENV_DIRS: &[&str] = &["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"];

/// 路径归一化：按组件消解 ".." 与 "."、统一分隔符为 / 并转小写，只用于系统目录前缀比较。
/// 必须消解父目录：否则 C:\Windows\..\Windows\x.txt 这类路径能绕开前缀判断。
fn normalize_path_key(path: &Path) -> String {
    use std::path::Component;
    let mut prefix = String::new();
    let mut parts: Vec<String> = Vec::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => {
                prefix = p.as_os_str().to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")
            }
            Component::RootDir => {
                if !prefix.ends_with('/') {
                    prefix.push('/');
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
        }
    }
    format!("{}{}", prefix, parts.join("/")).to_lowercase()
}

/// S1：写入类命令的纵深防御校验：
/// 非空、绝对路径、文件名合法（非空/不以空格或点结尾/非 Windows 保留设备名）、
/// 扩展名在白名单、目标不是已存在目录、父目录存在且为目录、不落在系统目录。
/// 说明：这是 IPC 层的兜底，正常流程的路径来自 pickSave/pickFile 保存对话框。
fn validate_output_path(path: &str, allowed_exts: &[&str], needs_existing_parent: bool) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("必须使用绝对路径：{path}"));
    }
    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
        return Err("路径缺少文件名".into());
    };
    if name.is_empty() {
        return Err("文件名为空".into());
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err("文件名不能以空格或点结尾".into());
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if WINDOWS_RESERVED_STEMS
        .iter()
        .any(|r| stem.eq_ignore_ascii_case(r))
    {
        return Err(format!("文件名 {stem} 是 Windows 保留设备名"));
    }
    let ext = p
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !allowed_exts.iter().any(|e| *e == ext) {
        return Err(format!("不允许的文件类型：.{ext}"));
    }
    if p.exists() && p.is_dir() {
        return Err("目标是目录，不能写入".into());
    }
    if needs_existing_parent {
        let Some(parent) = p.parent() else {
            return Err("路径缺少父目录".into());
        };
        if !parent.exists() {
            return Err("父目录不存在".into());
        }
        if !parent.is_dir() {
            return Err("父路径不是目录".into());
        }
    }
    let key = normalize_path_key(p);
    for env in WINDOWS_SYSTEM_ENV_DIRS {
        let Ok(dir) = std::env::var(env) else { continue };
        if dir.trim().is_empty() {
            continue;
        }
        let base = normalize_path_key(Path::new(&dir));
        if !base.is_empty() && (key == base || key.starts_with(&format!("{base}/"))) {
            return Err(format!("不允许写入系统目录：{dir}"));
        }
    }
    Ok(())
}

/// S1：读取类命令（preview_split）的校验：绝对路径、存在且是普通文件、扩展名白名单。
fn validate_source_path(path: &str, allowed_exts: &[&str]) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("必须使用绝对路径：{path}"));
    }
    if !p.is_file() {
        return Err("文件不存在或不是普通文件".into());
    }
    let ext = p
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !allowed_exts.iter().any(|e| *e == ext) {
        return Err(format!("不允许的文件类型：.{ext}"));
    }
    Ok(())
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

/// D1：导出划线/笔记等文本文件（仅写目标文件，不碰 DB、不做迁移）。
/// 内容可能较大（全库导出），放到阻塞线程池写，避免占用 Tauri 命令线程。
#[tauri::command]
pub async fn write_text_file(path: String, content: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // S1：文本导出只允许 txt/md/csv，且父目录必须已存在；拒绝保留设备名与系统目录。
        validate_output_path(&path, &["txt", "md", "csv"], true)?;
        std::fs::write(&path, content).map_err(|e| format!("写入文件失败：{e}"))
    })
    .await
    .map_err(|e| format!("写入任务失败：{e}"))?
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
        // S1：AI 导出只允许 txt/epub，且父目录必须已存在。
        let out_exts: &[&str] = if target == "epub" { &["epub"] } else { &["txt"] };
        validate_output_path(&path, out_exts, true)?;
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

/// H11：图片类命令的源文件上限。比 read_book_bytes 的 256MiB 更保守——这里要
/// 整图解码（内存 ≈ 宽×高×4），64MiB 压缩数据足以覆盖正常封面/背景图，再大只会放大 OOM 风险。
const IMAGE_SOURCE_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// P0-3：默认封面底图解码上限（image::load_from_memory 会按宽×高×4 分配）。
/// 100MP/边长 20000 足够覆盖常见封面/背景图；超限在解码前拒绝，避免整图解码 OOM。
const IMAGE_DIM_MAX_PIXELS: u64 = 100_000_000;
const IMAGE_DIM_MAX_EDGE: u32 = 20_000;

/// P0-3：尺寸上限判定（纯函数，便于测试边界）。返回 Some(可读错误) 表示超限/非法。
fn image_dim_limit_error(w: u32, h: u32) -> Option<String> {
    if w == 0 || h == 0 {
        return Some("图片尺寸无效（宽或高为 0）".to_string());
    }
    let pixels = w as u64 * h as u64;
    if pixels > IMAGE_DIM_MAX_PIXELS || w.max(h) > IMAGE_DIM_MAX_EDGE {
        return Some(format!(
            "图片尺寸过大（{w}x{h}，上限 100MP/边长 20000）"
        ));
    }
    None
}

/// P0-3：只读图片 header 获取尺寸（不整图解码）；非图片/损坏文件给可读错误。
fn check_image_dimensions(image_path: &str) -> Result<(u32, u32), String> {
    image::image_dimensions(image_path)
        .map_err(|e| format!("读取图片尺寸失败（不是有效图片或文件损坏）：{e}"))
}

fn image_source_too_large(len: u64) -> bool {
    len > IMAGE_SOURCE_MAX_BYTES
}

/// 读取图片命令的公共前置：先看 metadata 卡上限，避免先整读再拒绝。
fn check_image_source(image_path: &str) -> Result<(), String> {
    let meta = std::fs::metadata(image_path).map_err(|e| format!("读取图片信息失败: {e}"))?;
    if image_source_too_large(meta.len()) {
        return Err(format!(
            "图片过大（{}MB），上限 {}MB",
            meta.len() / 1048576,
            IMAGE_SOURCE_MAX_BYTES / 1048576
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn set_custom_cover(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    image_path: String,
) -> Result<(), String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        check_image_source(&image_path)?;
        let data = std::fs::read(&image_path).map_err(|e| e.to_string())?;
        let dir = covers::covers_dir(&st.app);
        let name = format!("custom-{book_id}.png");
        let out = dir.join(&name);
        covers::make_thumbnail(&data, &out).map_err(|e| format!("图片处理失败: {e}"))?;
        // 同 stem 清理：手动换封面时旧扩展名（如 PDF 首页生成的 .webp）必须删掉，
        // 否则同一本书会留下多份不同扩展名的封面（白占磁盘、孤儿永不回收）。
        // 保留刚写入的这一份（精确文件名，不按 mtime 猜）。
        covers::clean_same_stem(&dir, &format!("custom-{book_id}"), &name);
        {
            let conn = st.db.0.lock().unwrap();
            conn.execute(
                "UPDATE books SET cover_path=?2, cover_ver=cover_ver+1 WHERE id=?1",
                rusqlite::params![book_id, name],
            )
            .map_err(|e| e.to_string())?;
        }
        // 该本封面已变：通知前端刷新，per-book URL 因 cover_ver+1 而失效重取。
        st.app.emit("book-updated", ()).ok();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读取整本 PDF 的原始字节，供前端 pdf.js 解析。
/// 走 `tauri::ipc::Response` 原始字节通道（与 `list_order_index` 同一套）：前端
/// `invoke<ArrayBuffer>` 直接拿到 ArrayBuffer，不做 base64 往返（大 PDF 可省一倍内存）。
/// 只允许 pdf：否则这里就成了任意文件的读取口。
#[tauri::command]
pub async fn read_book_bytes(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<tauri::ipc::Response, String> {
    let path = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?.path
    };
    if ext_of(&path).as_deref() != Some("pdf") {
        return Err("仅 PDF 支持按原始字节读取".into());
    }
    // 单次整读的硬上限（内存上限保护）：这条命令按「整份读入」设计，只能服务中小文件。
    // 真实库里存在 3.75GB 的扫描 PDF；一旦前端走到「range 打开失败 → 回退整读」，
    // std::fs::read 会一次性要出同样大的 Vec（实测把工作集推到 6.8GB 后崩溃）。
    // 与其 OOM，不如在这里如实报错：range 路径对超大文件才是正解。
    const READ_BYTES_MAX: u64 = 256 * 1024 * 1024;
    let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if sz > READ_BYTES_MAX {
        memlog::stage_note(
            "cmd:read_book_bytes:reject",
            &format!("book={book_id} size={}MB limit={}MB", sz / 1048576, READ_BYTES_MAX / 1048576),
        );
        return Err(format!(
            "PDF 文件过大（{}MB），不支持整份读入；请改用区间读取",
            sz / 1048576
        ));
    }
    if sz > 8 * 1024 * 1024 {
        memlog::stage_note("cmd:read_book_bytes", &format!("book={book_id} size={}MB", sz / 1048576));
    }
    memlog::watch("cmd:read_book_bytes");
    // 几十 MB 的整读放阻塞线程池，避免占用 async 运行时拖住其它 IPC。
    let bytes = tauri::async_runtime::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    memlog::watch("cmd:read_book_bytes:after");
    Ok(tauri::ipc::Response::new(bytes))
}

/// 解析书籍路径并确认是 PDF（两个 range 命令共用前置校验）。
/// 非 pdf 一律拒绝：否则这两个命令就成了任意文件读取口。
pub(crate) fn pdf_book_path(state: &State<'_, Arc<AppState>>, book_id: i64) -> Result<std::path::PathBuf, String> {
    let path = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?.path
    };
    if ext_of(&path).as_deref() != Some("pdf") {
        return Err("仅 PDF 支持按区间读取".into());
    }
    Ok(std::path::PathBuf::from(path))
}

/// 单次 range 上限 32MB：扫描页的单个嵌入图片流常见十几 MB，卡到 8MB 会让该页直接渲染失败；
/// 32MB 仍然是一道「单次内存上限」，只是不会误伤正常的按需读取。超限直接报错，**不截断返回**
/// （截断一个对象流会把它损坏）。
const PDF_RANGE_MAX: u64 = 32 * 1024 * 1024;
/// 在途 range **总字节**上限 64MB：只限「请求个数」挡不住极端组合（4 × 32MB = 128MB 在途缓冲）。
/// 实测正常路径单次只有几十 KB~2.6MB（97MB 扫描件打开时最后一次只取 2.6MB），
/// 所以 64MB 对真实使用没有任何影响，却把最坏情况钉死在可接受范围。
const PDF_RANGE_INFLIGHT_MAX: u64 = 64 * 1024 * 1024;
static PDF_RANGE_INFLIGHT_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// PDF 文件字节总数：前端据此决定「整份读入」还是「range 分块拉取」。
#[tauri::command]
pub async fn pdf_file_size(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<u64, String> {
    memlog::watch("cmd:pdf_file_size");
    let path = pdf_book_path(&state, book_id)?;
    tauri::async_runtime::spawn_blocking(move || std::fs::metadata(&path))
        .await
        .map_err(|e| e.to_string())?
        .map(|m| m.len())
        .map_err(|e| format!("读取文件失败：{e}"))
}

/// 读取 PDF 的 [begin, end) 原始字节（供 pdf.js 的 PDFDataRangeTransport 按需拉取）。
/// 只返回原始字节，不做 base64 往返：大 PDF 能省一倍内存。
#[tauri::command]
pub async fn pdf_read_range(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    begin: u64,
    end: u64,
) -> Result<tauri::ipc::Response, String> {
    let path = pdf_book_path(&state, book_id)?;
    if end <= begin {
        return Err("range 非法：end 必须大于 begin".into());
    }
    let want = end - begin;
    // 请求区间打点：前 20 条逐条记（验收要看「按需小块」的分布），之后只记 >4MB 的大块。
    let n = PDF_RANGE_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 20 || want > 4 * 1024 * 1024 {
        memlog::stage_note(
            "cmd:pdf_read_range",
            &format!(
                "n={n} book={book_id} begin={begin} end={end} want={}KB",
                want / 1024
            ),
        );
    }
    if want > PDF_RANGE_MAX {
        // 错误里带上区间与文件大小：日志里能直接看出是「谁要了多大一块」，便于定位。
        let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        return Err(format!(
            "单次 range 过大：book={book_id} begin={begin} end={end} want={}MB 上限={}MB 文件={}MB",
            want / 1048576,
            PDF_RANGE_MAX / 1048576,
            total / 1048576
        ));
    }
    // RAII 守卫：带着本次字节数，任何提前返回都会把额度还回去
    struct Guard(u64);
    impl Drop for Guard {
        fn drop(&mut self) {
            PDF_RANGE_INFLIGHT_BYTES.fetch_sub(self.0, std::sync::atomic::Ordering::SeqCst);
        }
    }
    if PDF_RANGE_INFLIGHT_BYTES.fetch_add(want, std::sync::atomic::Ordering::SeqCst) + want
        > PDF_RANGE_INFLIGHT_MAX
    {
        PDF_RANGE_INFLIGHT_BYTES.fetch_sub(want, std::sync::atomic::Ordering::SeqCst);
        return Err("在途 range 总字节超限（上限 64MB）".into());
    }
    memlog::watch("cmd:pdf_read_range");
    let _guard = Guard(want);
    let bytes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&path).map_err(|e| format!("打开文件失败：{e}"))?;
        let total = f.metadata().map_err(|e| e.to_string())?.len();
        if begin >= total {
            return Err(format!("range 越界：begin {begin} ≥ 文件大小 {total}"));
        }
        // 末尾不足按实际长度返回（pdf.js 能接受短读，不能读失败）
        let len = std::cmp::min(want, total - begin);
        f.seek(SeekFrom::Start(begin)).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; len as usize];
        f.read_exact(&mut buf).map_err(|e| format!("读取失败：{e}"))?;
        Ok(buf)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// PDF 首页封面：前端**按目标缩略图尺寸**渲染好 WebP（base64，去掉 data URL 前缀）传上来，
/// 后端只落盘、不再解码缩放（前端已是最终像素；原实现让后端把 800px 整幅解码再缩到 320×480，
/// 白白多吃一次整幅位图的内存）。落库流程与 `set_custom_cover` 一致：cover_path/cover_ver
/// + book-updated，保证书库封面即时刷新且 URL 失效重取。
#[tauri::command]
pub fn set_book_cover_image(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    webp_base64: String,
) -> Result<String, String> {
    // 诊断打点：单次封面写入的 base64 有多大（PDF 首页整幅 PNG 可能数 MB，是内存峰值主要来源之一）。
    // 限流：>4MB 一律记（需要人工看一眼的异常值），其余每 200 次记一行 ——
    // 否则全库补封面会给 startup.log 写几万行。
    let ci = COVER_IMG_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if webp_base64.len() > 4 * 1024 * 1024 || ci == 1 || ci % 200 == 0 {
        memlog::stage_note(
            "cmd:setBookCoverImage",
            &format!("calls={ci} book={book_id} b64={}KB", webp_base64.len() / 1024),
        );
    }
    memlog::watch("cmd:set_book_cover_image");
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD
        .decode(webp_base64.trim())
        .map_err(|e| format!("封面数据解码失败: {e}"))?;
    // 只认图片魔数：既决定扩展名，也挡住把任意字节写进封面目录。
    let ext = covers::sniff_ext(&data).ok_or("封面数据格式不支持")?;
    let dir = covers::covers_dir(&state.app);
    let name = format!("custom-{book_id}.{ext}");
    let out = dir.join(&name);
    std::fs::write(&out, &data).map_err(|e| format!("封面写入失败: {e}"))?;
    // 同 stem 清理：PDF 首页封面按魔数落 .webp/.png 时，把同一本书的其它扩展名旧文件删掉
    // （例如先前手动选图生成的 custom-{id}.png）。保留刚写入的这一份。
    covers::clean_same_stem(&dir, &format!("custom-{book_id}"), &name);
    {
        let conn = state.db.0.lock().unwrap();
        conn.execute(
            "UPDATE books SET cover_path=?2, cover_ver=cover_ver+1 WHERE id=?1",
            rusqlite::params![book_id, name],
        )
        .map_err(|e| e.to_string())?;
    }
    state.app.emit("book-updated", ()).ok();
    // 返回落盘文件名：调用方（重新提取封面）要用它更新 cover_path。
    Ok(name)
}

/// 前端回传的「某页纯文本」：写入既有 chapter_text（划词全文搜索 / FTS 的数据源）。
/// 后端不解析 PDF —— 文字只能由前端 pdf.js 抽取后回传；乱码页前端不会回传（见 pdfTextUsable）。
#[tauri::command]
pub async fn set_pdf_page_text(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    page_index: i64,
    title: String,
    text: String,
) -> Result<(), String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let conn = st.db.0.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO chapter_text(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
            rusqlite::params![book_id, page_index, title, text],
        )
        .map_err(|e| e.to_string())?;
        // 全库搜索（search_all_inner）查的是 fts 表，而 index_book_fts 只在打开后的 idle 跑一次
        // 且以「fts 里该书 0 行」为建索引条件 —— 之后回传的页永远进不了 fts，全库搜索就搜不到 PDF。
        // 所以这里同步补一条 FTS 行（先删同页再插，重复回传保持幂等）。
        if db::has_fts(&conn) {
            db::ensure_fts(&conn).ok();
            conn.execute(
                "DELETE FROM fts WHERE book_id=?1 AND chapter_index=?2",
                rusqlite::params![book_id, page_index],
            )
            .ok();
            conn.execute(
                "INSERT INTO fts(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
                rusqlite::params![book_id, page_index, title, text],
            )
            .ok();
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
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
pub async fn set_default_cover(state: State<'_, Arc<AppState>>, image_path: String) -> Result<(), String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        check_image_source(&image_path)?;
        // P0-3：整读/解码前先只读 header 校验像素与边长，避免超大图直接把进程内存打爆。
        let (w, h) = check_image_dimensions(&image_path)?;
        if let Some(msg) = image_dim_limit_error(w, h) {
            return Err(msg);
        }
        let data = std::fs::read(&image_path).map_err(|e| format!("读取图片失败: {e}"))?;
        let img = image::load_from_memory(&data).map_err(|e| format!("不是有效图片: {e}"))?;
        // 下采样到封面比例（2:3 居中裁切填满），再编码为 PNG。
        let cover = img.resize_to_fill(320, 480, image::imageops::FilterType::Lanczos3);
        let rgba = cover.to_rgba8();
        let mut png: Vec<u8> = Vec::new();
        cover
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| format!("编码封面失败: {e}"))?;

        let out = covers::default_cover_path(&st.app);
        write_default_cover_atomic(&out, &png)?;

        // 分析书名区亮度 → 自适应字色（偏亮配柔和深色、偏暗配柔和浅色；非纯黑纯白、护眼）。
        let region_light = covers::region_is_light(&rgba);
        let (text, is_light) = if region_light {
            ("#2A2723", false)
        } else {
            ("#EFEBE4", true)
        };
        {
            let conn = st.db.0.lock().unwrap();
            db::set_setting(&conn, "dc_text", text);
            db::set_setting(&conn, "dc_text_light", if is_light { "1" } else { "0" });
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    // 收尾按原顺序放在 await 之后：版本号失效 + 通知前端刷新。
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
    c.ensure_dir("TidePage").await?;
    c.ensure_dir("TidePage/settings").await?;
    let path = format!("TidePage/settings/{name}");
    c.put(&path, Arc::new(data.into_bytes())).await?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
        c.put(&remote_key, Arc::new(data)).await?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
    let remote_key = format!("TidePage/progress/{}.json", db::path_id(&book_path));
    // get()：404 → Ok(None)，传输错误 → Err；两种失败都按「云端无进度」处理。
    Ok(match c.get(&remote_key).await {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    })
}

/// WebDAV 单文件大小硬上限（8 GiB）：下载/上传读取共用，超限立即报错。
pub(crate) const WEBDAV_FILE_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// 下载缓冲的预分配上限（4 MiB）：远端 Content-Length 不可信，按它全额预分配会 OOM/abort。
const WEBDAV_PREALLOC_MAX: u64 = 4 * 1024 * 1024;

/// WebDAV 远端条目名的校验与重建：只接受单一文件名。
/// PROPFIND 名字不可信（percent_decode_path 会把 %5C 解成反斜杠）；
/// 含 `/`、`\\`、`..`、盘符冒号，或 file_name() 与原名不一致的一律拒绝。
fn safe_file_name(name: &str) -> Option<String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains(':')
    {
        return None;
    }
    let file_name = Path::new(name).file_name()?.to_str()?;
    if file_name != name {
        return None;
    }
    Some(file_name.to_string())
}

/// 把已校验的文件名拼到目标目录下，并确认 join 结果没有跳出目标目录。
/// 不用 canonicalize（目标文件可能还不存在）：逐组件校验 + 前缀比较即可。
fn safe_join_under(base: &Path, name: &str) -> Option<PathBuf> {
    let out = base.join(name);
    if !out.starts_with(base) {
        return None;
    }
    if out
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(out)
}


#[cfg(test)]
mod webdav_name_tests {
    use super::{safe_file_name, safe_join_under};
    use std::path::Path;

    /// 远端条目名不可信：路径分隔、反斜杠、盘符冒号与上跳目录一律拒绝。
    #[test]
    fn safe_file_name_rejects_traversal_and_absolute_names() {
        for bad in [
            "",
            ".",
            "..",
            "../x.epub",
            "..\\x.epub",
            "a/b.epub",
            "a\\b.epub",
            "C:\\x.epub",
            "C:/x.epub",
            "d:x.epub",
            "a..b.epub",
        ] {
            assert!(safe_file_name(bad).is_none(), "{bad:?} 必须拒绝");
        }
        assert_eq!(safe_file_name("x.epub").as_deref(), Some("x.epub"));
        assert_eq!(safe_file_name("书名.epub").as_deref(), Some("书名.epub"));
        // join 结果必须留在目标目录内（.. 会被拒绝）。
        let base = Path::new("C:\\tidepage\\books");
        assert!(safe_join_under(base, "x.epub").is_some());
        assert!(safe_join_under(base, "..\\x.epub").is_none());
    }
}

#[tauri::command]
pub async fn webdav_upload_book(
    base: String,
    user: String,
    pass: String,
    book_path: String,
) -> Result<String, String> {
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
    c.ensure_dir("TidePage").await?;
    c.ensure_dir("TidePage/books").await?;
    let name = Path::new(&book_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or("路径无效")?;
    // S2：流式上传，内存与文件大小无关。先 metadata 预检，再打开后重新取 metadata 校验
    //（防 metadata 与打开之间文件被替换/变大），同时覆盖 32 位 >2GiB 的硬上限。
    let f = std::fs::File::open(&book_path).map_err(|e| format!("打开文件失败：{e}"))?;
    let meta = f.metadata().map_err(|e| format!("读取文件信息失败：{e}"))?;
    if let Some(msg) = crate::dav::too_large_for_arch(meta.len(), usize::BITS) {
        return Err(msg.to_string());
    }
    if meta.len() > WEBDAV_FILE_MAX_BYTES {
        return Err(format!(
            "文件超过上限 {} 字节，已取消上传",
            WEBDAV_FILE_MAX_BYTES
        ));
    }
    c.put_file(&format!("TidePage/books/{name}"), f).await?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
        // 远端名字不可信：只接受单一文件名，并确认 join 后没有跳出目标目录。
        let safe_name = match safe_file_name(&name) {
            Some(n) => n,
            None => continue,
        };
        let out = match safe_join_under(&target, &safe_name) {
            Some(p) => p,
            None => continue,
        };
        if out.exists() {
            continue;
        }
        // S2：流式下载到同目录 tmp 再 rename；内存与文件大小无关，失败自动清理 tmp。
        let remote = format!("TidePage/books/{safe_name}");
        if c.download_to_file(&remote, &out, WEBDAV_FILE_MAX_BYTES).await? {
            imported.push(out.to_string_lossy().into_owned());
        }
    }
    if !imported.is_empty() {
        // 只扫描导入文件所在的已登记目录；映射不到就不扫描（不做全量遍历）。
        let folders = {
            let conn = state.db.0.lock().unwrap();
            db::list_folders(&conn)
        };
        let dirs =
            registered_dirs_for_paths(imported.iter().map(|p| Path::new(p.as_str())), &folders);
        if !dirs.is_empty() {
            scan_folders(&state, Some(dirs), ScanOrigin::Timer).await;
        }
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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
    // 先校验远端文件名再下载：只接受单一文件名，join 结果不得跳出目标目录。
    let name = match remote_path.rsplit('/').next().and_then(safe_file_name) {
        Some(n) => n,
        None => return Err("无效文件名".into()),
    };
    let target = PathBuf::from(&target_dir);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let out = safe_join_under(&target, &name).ok_or("无效文件名")?;
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
    let data = c
        .get_limited(&remote_path, WEBDAV_FILE_MAX_BYTES)
        .await?
        .ok_or("远程文件不存在")?;
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
        bump_formats_epoch(state.inner());
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
    // 先校验文件名再开始下载：非法名字不浪费带宽与内存（原实现下载完才校验）。
    let name = match remote_path.rsplit('/').next().and_then(safe_file_name) {
        Some(n) => n,
        None => return Err("无效文件名".into()),
    };
    let c = crate::dav::DavClient::new(&base, &user, &pass)?;
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

    // 远端 Content-Length 不可信：只为它预分配一个上限内的起步缓冲，绝不按声明值全额分配。
    let mut buf: Vec<u8> = Vec::with_capacity(total.min(WEBDAV_PREALLOC_MAX) as usize);
    let mut stream = resp.bytes_stream();
    let mut received: u64 = 0;
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                // 先累计并卡硬上限，再写入内存：超限立即中止，不继续读也不继续涨。
                received = received.saturating_add(chunk.len() as u64);
                if received > WEBDAV_FILE_MAX_BYTES {
                    let msg = format!(
                        "文件超过上限 {} 字节，已中止下载",
                        WEBDAV_FILE_MAX_BYTES
                    );
                    emit(received, "red", true, Some(msg.clone()));
                    return Err(msg);
                }
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

    let target = PathBuf::from(&target_dir);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    // 文件名入口已校验；这里再确认 join 结果没有跳出目标目录。
    let out = safe_join_under(&target, &name).ok_or("无效文件名")?;
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
        bump_formats_epoch(state.inner());
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
    db::set_setting_checked(&conn, &key, &value)
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
    /// B3：解析缓存目录（data_dir/parse_cache）占用；可单独清理，不计入 cache_bytes。
    pub parse_cache_bytes: u64,
    /// B10：freelist 可回收字节（page_size * freelist_count）。
    pub freelist_bytes: u64,
    /// B10：数据库所在盘可用字节（取不到为 None）。
    pub db_dir_free_bytes: Option<u64>,
    /// B10：临时目录路径与可用字节。
    pub temp_dir: String,
    pub temp_free_bytes: Option<u64>,
    /// B10：临时目录与数据库是否同一卷（同卷只计一次临时空间）。
    pub temp_same_disk: bool,
    /// B10：最近一次 WAL checkpoint 记录。
    pub last_checkpoint_at: i64,
    pub last_checkpoint_busy: bool,
    pub checkpoint_wal_before: u64,
    pub checkpoint_wal_after: u64,
}


#[tauri::command]
pub async fn cover_audit(state: State<'_, Arc<AppState>>) -> Result<crate::covers::CoverAudit, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = crate::covers::covers_dir(&st.app);
        let refs = {
            let conn = st.db.0.lock().unwrap();
            let mut set = std::collections::HashSet::new();
            if let Ok(mut query) = conn.prepare(
                "SELECT DISTINCT lower(cover_path) FROM books WHERE cover_path IS NOT NULL AND cover_path<>''",
            ) {
                if let Ok(rows) = query.query_map([], |r| r.get::<_, String>(0)) {
                    for r in rows.flatten() { set.insert(r); }
                }
            }
            set
        };
        let mut audit = crate::covers::audit_covers(&dir, &refs);
        {
            let conn = st.db.0.lock().unwrap();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            // 首次扫描也返回本次时间；写入 now，后续扫描自然更新。
            audit.last_scan_at = now;
            let _ = db::set_setting(&conn, "maintenance.cover_audit_at", &now.to_string());
        }
        Ok(audit)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clean_cover_orphans(
    state: State<'_, Arc<AppState>>,
    confirm: bool,
) -> Result<crate::covers::CoverCleanResult, String> {
    if !confirm {
        return Err("需要二次确认后才能清理孤儿封面".into());
    }
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = crate::covers::covers_dir(&st.app);
        let refs = {
            let conn = st.db.0.lock().unwrap();
            let mut set = std::collections::HashSet::new();
            if let Ok(mut query) = conn.prepare(
                "SELECT DISTINCT lower(cover_path) FROM books WHERE cover_path IS NOT NULL AND cover_path<>''",
            ) {
                if let Ok(rows) = query.query_map([], |r| r.get::<_, String>(0)) {
                    for r in rows.flatten() { set.insert(r); }
                }
            }
            set
        };
        let result = crate::covers::clean_cover_orphans(&dir, &refs);
        {
            let conn = st.db.0.lock().unwrap();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            let _ = db::set_setting(&conn, "maintenance.cover_audit_at", &now.to_string());
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn storage_stats(state: State<'_, Arc<AppState>>) -> Result<StorageStats, String> {
    let dir = crate::data_dir(&state.app);
    let data_dir = dir.to_string_lossy().into_owned();
    let fallback_dir = data_dir.clone();
    // 大目录遍历是阻塞 IO：整体放阻塞线程池，避免同步命令卡主线程。
    let stats = tauri::async_runtime::spawn_blocking(move || {
        let mut s = StorageStats {
            data_dir,
            total_bytes: 0,
            covers_bytes: 0,
            res_bytes: 0,
            fonts_bytes: 0,
            db_bytes: 0,
            cache_bytes: 0,
            parse_cache_bytes: 0,
            freelist_bytes: 0,
            db_dir_free_bytes: None,
            temp_dir: String::new(),
            temp_free_bytes: None,
            temp_same_disk: false,
            last_checkpoint_at: 0,
            last_checkpoint_busy: false,
            checkpoint_wal_before: 0,
            checkpoint_wal_after: 0,
        };
        // 只遍历一次 data_dir：所有文件计入 total，按第一段分类到 covers/res/fonts/db。
        collect_storage(&dir, &dir, &mut s);
        // 解析缓存单独统计（parsecache 只认合法缓存扩展名，忽略临时文件）。
        s.parse_cache_bytes = crate::parsecache::cache_bytes(&dir.join("parse_cache"));
        s.cache_bytes = s.covers_bytes + s.res_bytes;
        // B10：freelist 可回收 + 数据库盘/临时目录空间（不读取大表，只查 PRAGMA）。
        s.freelist_bytes = rusqlite::Connection::open(dir.join("tidepage.db"))
            .ok()
            .and_then(|c| {
                let page: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0)).ok()?;
                let free: i64 = c.query_row("PRAGMA freelist_count", [], |r| r.get(0)).ok()?;
                Some((page.max(0) * free.max(0)) as u64)
            })
            .unwrap_or(0);
        s.db_dir_free_bytes = disk_free_bytes(&dir);
        if let Some(cp) = rusqlite::Connection::open(dir.join("tidepage.db")).ok().and_then(|c| {
            db::get_setting(&c, "maintenance.last_checkpoint")
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        }) {
            s.last_checkpoint_at = cp.get("at").and_then(|v| v.as_i64()).unwrap_or(0);
            s.last_checkpoint_busy = cp.get("busy").and_then(|v| v.as_bool()).unwrap_or(false);
            s.checkpoint_wal_before = cp.get("walBefore").and_then(|v| v.as_u64()).unwrap_or(0);
            s.checkpoint_wal_after = cp.get("walAfter").and_then(|v| v.as_u64()).unwrap_or(0);
        }
        let temp = std::env::temp_dir();
        s.temp_same_disk = same_disk(&dir, &temp);
        s.temp_free_bytes = disk_free_bytes(&temp);
        s.temp_dir = temp.to_string_lossy().into_owned();
        s
    })
    .await;
    match stats {
        Ok(s) => Ok(s),
        Err(e) => {
            // 统计失败不阻塞设置页：返回空统计并记日志（前端照常渲染 0）。
            eprintln!("[storage] stats task failed: {e}");
            Ok(StorageStats {
                data_dir: fallback_dir,
                total_bytes: 0,
                covers_bytes: 0,
                res_bytes: 0,
                fonts_bytes: 0,
                db_bytes: 0,
                cache_bytes: 0,
                parse_cache_bytes: 0,
                freelist_bytes: 0,
                db_dir_free_bytes: None,
                temp_dir: String::new(),
                temp_free_bytes: None,
                temp_same_disk: false,
                last_checkpoint_at: 0,
                last_checkpoint_busy: false,
                checkpoint_wal_before: 0,
                checkpoint_wal_after: 0,
            })
        }
    }
}

#[cfg(windows)]
pub(crate) fn disk_free_bytes(path: &Path) -> Option<u64> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut wide: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    wide.push(0);
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None).ok()?;
    }
    Some(free)
}

#[cfg(not(windows))]
pub(crate) fn disk_free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// B3：VACUUM 前磁盘余量判定（保守：至少 DB 大小 + 64MB；取不到余量时拒绝执行）。
#[cfg_attr(not(test), allow(dead_code))]
pub fn vacuum_space_ok(free: Option<u64>, db_bytes: u64) -> bool {
    match free {
        Some(f) => f > db_bytes.saturating_add(64 * 1024 * 1024),
        None => false,
    }
}

#[cfg(windows)]
fn same_disk(a: &Path, b: &Path) -> bool {
    let ra: String = a.to_string_lossy().chars().take(2).collect::<String>().to_ascii_lowercase();
    let rb: String = b.to_string_lossy().chars().take(2).collect::<String>().to_ascii_lowercase();
    !ra.is_empty() && ra == rb && a.exists() && b.exists()
}

#[cfg(not(windows))]
fn same_disk(_a: &Path, _b: &Path) -> bool {
    false
}

fn maintenance_now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// B10：维护历史记录（保留最近 5 次，settings JSON 幂等）。
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRecord {
    pub at: i64,
    pub kind: String,
    pub result: String,
    pub error: String,
    pub before_db: u64,
    pub before_wal: u64,
    pub after_db: u64,
    pub after_wal: u64,
    pub freed: u64,
    pub duration_ms: u64,
    pub checkpoint_busy: bool,
    pub backup_path: String,
}

fn push_maintenance_record(conn: &rusqlite::Connection, rec: MaintenanceRecord) {
    let mut list: Vec<MaintenanceRecord> = db::get_setting(conn, "maintenance.history.v1")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    list.insert(0, rec);
    list.truncate(5);
    let _ = db::set_setting(conn, "maintenance.history.v1", &serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()));
}

/// B10：维护历史（最近 5 条）。
/// B10：维护/扫描运行状态（自动整理调度用）。
#[tauri::command]
pub fn maintenance_status(state: State<'_, Arc<AppState>>) -> serde_json::Value {
    serde_json::json!({
        "maintenanceRunning": state.maintenance_running.load(Ordering::SeqCst),
        "scanRunning": state.scan_running.load(Ordering::SeqCst),
    })
}

#[tauri::command]
pub fn maintenance_history(state: State<'_, Arc<AppState>>) -> Result<Vec<MaintenanceRecord>, String> {
    let conn = state.db.0.lock().unwrap();
    Ok(db::get_setting(&conn, "maintenance.history.v1")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

/// B10：VACUUM 空间预检（同卷只要求一次；异卷要求 DB 盘与临时目录各自满足）。
pub fn vacuum_precheck(
    db_free: Option<u64>,
    temp_free: Option<u64>,
    same_disk: bool,
    db_bytes: u64,
) -> Result<(), String> {
    let required = db_bytes.saturating_add(64 * 1024 * 1024);
    let need = required as f64 / 1024.0 / 1024.0;
    let check = |name: &str, free: Option<u64>| -> Result<(), String> {
        match free {
            Some(f) if f > required => Ok(()),
            Some(f) => Err(format!("{name}空间不足：整理约需 {need:.0}MB，可用 {:.0}MB。请清理后重试", f as f64 / 1024.0 / 1024.0)),
            None => Err(format!("无法读取{name}可用空间；为避免损坏数据库，已取消整理")),
        }
    };
    check("数据库所在盘", db_free)?;
    if !same_disk {
        check("临时目录", temp_free)?;
    }
    Ok(())
}

/// B3：清理解析缓存（可再生），返回释放字节数。
#[tauri::command]
pub async fn clear_parse_cache(state: State<'_, Arc<AppState>>) -> Result<u64, String> {
    let app = state.app.clone();
    let dir = crate::parsecache::cache_dir(&state.app);
    tauri::async_runtime::spawn_blocking(move || {
        let mut last_emit = std::time::Instant::now();
        crate::parsecache::clear_cache_with_progress(&dir, |p| {
            if p.done == 0 || p.done == p.total || last_emit.elapsed().as_millis() >= 200 {
                last_emit = std::time::Instant::now();
                let _ = app.emit(
                    "maintenance-progress",
                    serde_json::json!({
                        "phase": "parse_cache",
                        "state": if p.done == 0 && p.total > 0 { "start" } else if p.done >= p.total { "done" } else { "progress" },
                        "done": p.done,
                        "total": p.total,
                        "bytes": p.bytes,
                    }),
                );
            }
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// B3：数据库主体 + WAL 文件大小（释放量口径：VACUUM 期间 WAL 会临时增大，必须合计）。
fn db_file_sizes(db: &Path) -> (u64, u64) {
    let db_len = std::fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    let wal_path = PathBuf::from(format!("{}-wal", db.to_string_lossy()));
    let wal_len = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
    (db_len, wal_len)
}

/// B3：执行 WAL checkpoint(TRUNCATE)，最多重试 attempts 次；返回是否仍 busy。
/// 非 WAL 模式返回 (0,0,0) 视为成功；busy 时由调用方退化为 freelist 估算。
fn checkpoint_truncate(conn: &rusqlite::Connection, attempts: u32) -> bool {
    for i in 0..attempts {
        let busy = match conn.query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
        ) {
            Ok((busy, _log, _ckpt)) => busy != 0,
            Err(e) => {
                memlog::stage_note("maintenance:checkpoint", &format!("err={e}"));
                true
            }
        };
        if !busy {
            return false;
        }
        if i + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    true
}

/// B3：数据库整理（VACUUM）。仅用户显式触发；与 FTS 重建/其它维护互斥；先检查磁盘余量。
/// VACUUM 期间打开/搜索会等待数据库锁，前端应提示「可能耗时」。
#[tauri::command]
pub async fn vacuum_db(state: State<'_, Arc<AppState>>, auto: Option<bool>) -> Result<u64, String> {
    let auto = auto.unwrap_or(false);
    if state.fts_index_running.load(Ordering::SeqCst)
        || state.fts_optimize_running.load(Ordering::SeqCst)
    {
        return Err("正在优化搜索索引，请稍后再试".into());
    }
    if state.maintenance_running.swap(true, Ordering::SeqCst) {
        return Err("已有维护任务在进行中".into());
    }
    // 双检：拿到维护标志后再确认没有 FTS 任务刚好启动，避免两边同时写库。
    if state.fts_index_running.load(Ordering::SeqCst)
        || state.fts_optimize_running.load(Ordering::SeqCst)
    {
        state.maintenance_running.store(false, Ordering::SeqCst);
        return Err("正在优化搜索索引，请稍后再试".into());
    }
    // B10：自动整理不在书库扫描期间执行；手动维护保持原行为。
    if auto && state.scan_running.load(Ordering::SeqCst) {
        state.maintenance_running.store(false, Ordering::SeqCst);
        return Err("正在扫描书库，已跳过自动整理".into());
    }
    let st = state.inner().clone();
    let app = st.app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<u64, String> {
        let dir = crate::data_dir(&app);
        let db = dir.join("tidepage.db");
        let (db_len, _) = db_file_sizes(&db);
        if db_len == 0 {
            return Err("数据库文件不存在".into());
        }
        let conn = rusqlite::Connection::open(&db).map_err(|e| format!("打开数据库失败：{e}"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
        // VACUUM 期间 WAL 会临时增大、结束后 checkpoint 回落：前后都先 checkpoint 再按
        // (db + db-wal) 总大小计量，避免刚结束时 db 文件未缩小而显示释放 0。
        let initial_busy = checkpoint_truncate(&conn, 3);
        let (before_db, before_wal) = db_file_sizes(&db);
        let before_total = before_db + before_wal;
        let page_size: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap_or(4096);
        let freelist: i64 = conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap_or(0);
        let reclaim = (page_size * freelist).max(0) as u64;
        // B10：同时检查 DB 盘与临时目录；同卷只计一次，取不到空间信息则拒绝。
        let temp = std::env::temp_dir();
        let same = same_disk(&dir, &temp);
        if let Err(e) = vacuum_precheck(disk_free_bytes(&dir), disk_free_bytes(&temp), same, before_db) {
            if auto {
                push_maintenance_record(&conn, MaintenanceRecord {
                    at: maintenance_now_ms(), kind: "vacuum-auto".into(), result: "skipped".into(), error: e.clone(),
                    before_db, before_wal, after_db: before_db, after_wal: before_wal, freed: 0,
                    duration_ms: 0, checkpoint_busy: initial_busy, backup_path: String::new(),
                });
            }
            return Err(e);
        }
        // B10 自动整理：先做用户数据逻辑备份（非整库镜像），保留最近 1 份；失败则跳过。
        let mut backup_path = String::new();
        if auto {
            let backups = dir.join("backups");
            if let Err(e) = std::fs::create_dir_all(&backups) {
                let msg = format!("创建备份目录失败：{e}");
                push_maintenance_record(&conn, MaintenanceRecord {
                    at: maintenance_now_ms(), kind: "vacuum-auto".into(), result: "skipped".into(), error: msg.clone(),
                    before_db, before_wal, after_db: before_db, after_wal: before_wal, freed: 0,
                    duration_ms: 0, checkpoint_busy: initial_busy, backup_path: String::new(),
                });
                return Err(msg);
            }
            let path = backups.join(format!("auto-vacuum-{}.json", maintenance_now_ms()));
            match crate::databackup::export_backup_to_file(&conn, &path.to_string_lossy(), None) {
                Ok(_) => {
                    backup_path = path.to_string_lossy().into_owned();
                    // 保留最近 1 份自动备份。
                    if let Ok(rd) = std::fs::read_dir(&backups) {
                        let mut olds: Vec<_> = rd.flatten().filter(|e| {
                            let n = e.file_name().to_string_lossy().into_owned();
                            n.starts_with("auto-vacuum-") && n.ends_with(".json")
                        }).collect();
                        olds.sort_by_key(|e| e.file_name());
                        while olds.len() > 1 {
                            let e = olds.remove(0);
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("自动整理前备份失败：{e}");
                    push_maintenance_record(&conn, MaintenanceRecord {
                        at: maintenance_now_ms(), kind: "vacuum-auto".into(), result: "skipped".into(), error: msg.clone(),
                        before_db, before_wal, after_db: before_db, after_wal: before_wal, freed: 0,
                        duration_ms: 0, checkpoint_busy: initial_busy, backup_path: String::new(),
                    });
                    return Err(msg);
                }
            }
        }
        let _ = app.emit(
            "maintenance-progress",
            serde_json::json!({"phase":"vacuum","state":"start","reclaim_bytes":reclaim}),
        );
        let started = std::time::Instant::now();
        if let Err(e) = conn.execute_batch("VACUUM") {
            push_maintenance_record(&conn, MaintenanceRecord {
                at: maintenance_now_ms(), kind: "vacuum".into(), result: "error".into(), error: format!("数据库整理失败：{e}"),
                before_db, before_wal, after_db: before_db, after_wal: before_wal, freed: 0,
                duration_ms: started.elapsed().as_millis() as u64, checkpoint_busy: initial_busy, backup_path: backup_path.clone(),
            });
            return Err(format!("数据库整理失败：{e}"));
        }
        let (vac_db, vac_wal) = db_file_sizes(&db);
        let checkpoint_busy = checkpoint_truncate(&conn, 5);
        let (after_db, after_wal) = db_file_sizes(&db);
        let after_total = after_db + after_wal;
        let actual = before_total.saturating_sub(after_total);
        // checkpoint 始终 busy 或实际差值为 0 时，退化为 freelist 估算，避免界面显示 0。
        let freed = if (checkpoint_busy || actual == 0) && reclaim > 0 {
            reclaim
        } else {
            actual
        };
        memlog::stage_note(
            "maintenance:vacuum",
            &format!(
                "before_db={before_db} before_wal={before_wal} init_busy={initial_busy} vac_db={vac_db} vac_wal={vac_wal} after_db={after_db} after_wal={after_wal} checkpoint_busy={checkpoint_busy} reclaim={reclaim} actual={actual} freed={freed}"
            ),
        );
        push_maintenance_record(&conn, MaintenanceRecord {
            at: maintenance_now_ms(), kind: "vacuum".into(), result: "success".into(), error: String::new(),
            before_db, before_wal, after_db, after_wal, freed,
            duration_ms: started.elapsed().as_millis() as u64, checkpoint_busy, backup_path: backup_path.clone(),
        });
        let _ = db::set_setting(&conn, "maintenance.last_checkpoint", &serde_json::json!({
            "at": maintenance_now_ms(), "busy": checkpoint_busy, "walBefore": before_wal, "walAfter": after_wal
        }).to_string());
        let _ = app.emit(
            "maintenance-progress",
            serde_json::json!({"phase":"vacuum","state":"done","freed_bytes":freed}),
        );
        Ok(freed)
    })
    .await;
    // 无论 join 成功/失败都必须复位维护标志，否则后续维护会被永久挡住。
    state.maintenance_running.store(false, Ordering::SeqCst);
    match result {
        Ok(r) => r,
        Err(e) => Err(format!("维护任务异常终止：{e}")),
    }
}

/// 递归累加存储分类：只遍历 data_dir 一次。
/// - 相对 data_dir 的第一段 covers / res / fonts → 对应字段；
/// - tidepage.db(-wal/-shm) → db_bytes；
/// - 所有文件字节数都计入 total_bytes（含 db 与其它），分类字段互不重复。
fn collect_storage(data_dir: &Path, dir: &Path, s: &mut StorageStats) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            collect_storage(data_dir, &p, s);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let bytes = meta.len();
        s.total_bytes += bytes;
        let rel = match p.strip_prefix(data_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        match rel.components().next().and_then(|c| c.as_os_str().to_str()) {
            Some("covers") => s.covers_bytes += bytes,
            Some("res") => s.res_bytes += bytes,
            Some("fonts") => s.fonts_bytes += bytes,
            Some("parse_cache") => s.parse_cache_bytes += bytes,
            Some("tidepage.db") | Some("tidepage.db-wal") | Some("tidepage.db-shm") => {
                s.db_bytes += bytes
            }
            _ => {}
        }
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

/// 优先封面只接受真正未扫描的书：md=0。md=1（已完成）/2（pathological/超时失败）/3（处理中）
/// 一律跳过，避免失败书被前端可见区 effect 反复入队形成自转。
pub fn prioritize_eligible(meta_done: i64) -> bool {
    meta_done == 0
}

/// 方案 A：把「列表可见的未扫描书籍」提升为高优先级扫描目标（按用户视野顺序生成封面）。
/// - 单事务批量入队，复用 `enqueue_priority`（ON CONFLICT 刷新 queued_at → 幂等，重复滚动不膨胀）；
/// - 逐本校验 `meta_done != 1` 才入队（已完成的不入队）；
/// - 一次最多处理 1000 个 id；
/// - 结束后 `ensure_scan_worker`（worker 空闲则拉起，`next_priority` 按 queued_at DESC 取最近关注项）。
#[tauri::command]
pub fn prioritize_covers(state: State<'_, Arc<AppState>>, ids: Vec<i64>) -> Result<u64, String> {
    // 连续启动失败保护：本轮跳过封面重建时不再入队（否则 worker 又会把全库封面重扫一遍）。
    if memlog::skip_cover_rebuild() {
        return Ok(0);
    }
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
            // 仅对真正未扫描的书入队：md=1/2/3 一律跳过（失败/处理中不得被可见区反复入队）。
            if matches!(db::meta_done_of(&tx, *id), Some(md) if prioritize_eligible(md)) {
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
/// 含封面时先停 worker（最多 10 秒）：**确认停住才删目录**，停不住直接报错让用户重试。
#[tauri::command]
pub async fn clear_cache(state: State<'_, Arc<AppState>>, what: String) -> Result<u64, String> {
    let st = state.inner().clone();
    let targets: Vec<&'static str> = match what.as_str() {
        "covers" => vec!["covers"],
        "res" => vec!["res"],
        "all" => vec!["covers", "res"],
        other => return Err(format!("未知缓存类型: {other}")),
    };
    let clear_covers = targets.contains(&"covers");
    let clear_res = targets.contains(&"res");
    // 停止等待最长 10 秒且全是阻塞 IO：整体放阻塞线程池，不占 async 运行时。
    tauri::async_runtime::spawn_blocking(move || -> Result<u64, String> {
        if clear_covers && !stop_cover_worker_and_wait(&st) {
            // 未确认停住：一个目录都不删，避免与仍在写封面的 worker 并发。
            return Err("封面扫描仍在收尾，请稍后重试".into());
        }
        let dir = crate::data_dir(&st.app);
        let mut freed: u64 = 0;
        for name in &targets {
            let p = dir.join(name);
            if *name == "covers" {
                // 封面目录不整体删除：里面还有用户自选封面（custom-*）、阅读背景图（bg-*、
                // background.img）与默认底图；只按命名精确清理自动生成的封面文件。
                let (b, _) = crate::covers::clean_auto_covers(&p);
                freed += b;
                std::fs::create_dir_all(&p).ok();
                continue;
            }
            let (b, _) = dir_size(&p);
            if p.exists() {
                std::fs::remove_dir_all(&p).map_err(|e| format!("清理 {name} 失败: {e}"))?;
            }
            std::fs::create_dir_all(&p).ok();
            freed += b;
        }
        // 清理封面后，把元数据标记复位，下次扫描/打开会重建封面。
        if clear_covers {
            // 换底图/清缓存让旧封面缓存整体失效（F5）：规则版本 +1 参与缓存 key。
            st.cover_style_ver.fetch_add(1, Ordering::SeqCst);
            {
                let conn = st.db.0.lock().unwrap();
                // 只复位自动封面行；custom-* 用户自选封面保持原值（文件也未被删除）。
                conn.execute(
                    "UPDATE books SET cover_path=NULL, meta_done=0 \
                     WHERE cover_path IS NULL OR lower(cover_path) NOT LIKE 'custom-%'",
                    [],
                )
                .map_err(|e| e.to_string())?;
            }
            // worker 已确认停止：重新拉起增量扫描（全局互斥，若已在跑则 no-op）。
            ensure_scan_worker(&st.app);
        }
        if clear_res {
            // 提取资源清空后，已缓存的章节/文本/图片都需要失效
            // （MOBI 章节 HTML 里的 /res/<id>/... URL 会指向已删文件）。
            st.chapter_cache.lock().unwrap().clear();
            st.txt_cache.lock().unwrap().clear();
            st.image_cache.lock().unwrap().clear();
        }
        st.app.emit("library-changed", ()).ok();
        Ok(freed)
    })
    .await
    .map_err(|e| e.to_string())?
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


/// D2：阅读统计汇总（只读；区间由前端按本地日历传入）。
/// B9：前端删除书架等非 DB 同步项时记录墓碑（幂等；不推断缺失删除）。
#[tauri::command]
pub fn record_sync_tombstone(
    state: State<'_, Arc<AppState>>,
    kind: String,
    key: String,
    deleted_at: Option<i64>,
) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    let at = deleted_at.unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    });
    db::record_tombstone(&conn, &kind, &key, at)
}

#[tauri::command]
pub async fn get_reading_stats(
    state: State<'_, Arc<AppState>>,
    start_day: String,
    end_day: String,
    prev_start_day: String,
    prev_end_day: String,
    heat_start_day: String,
    heat_end_day: String,
) -> Result<db::ReadingStats, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        Ok(db::get_reading_stats(
            &conn, &start_day, &end_day, &prev_start_day, &prev_end_day, &heat_start_day, &heat_end_day,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
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
        .map_err(|e| e.to_string())?
}

/// 非空白字符数：短查询分流的判据（空白不计入长度）。
fn non_whitespace_chars(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// H12：2 字符查询走正文回退时的库大小阈值（page_count * page_size 估算值）。
/// trigram FTS 对 <3 字符不产生 token；全库 LIKE 在超大库上不可接受，超过阈值只搜书名/作者。
const SEARCH_SHORT_BODY_DB_MAX_BYTES: i64 = 256 * 1024 * 1024;

/// 规模守卫（纯函数）：取不到库大小一律保守跳过正文扫描；恰好等于阈值允许。
fn short_body_allowed(db_bytes: Option<i64>) -> bool {
    match db_bytes {
        Some(b) => b <= SEARCH_SHORT_BODY_DB_MAX_BYTES,
        None => false,
    }
}

/// 估算库大小（毫秒级 pragma，不扫页）。取不到返回 None。
fn estimate_db_bytes(conn: &rusqlite::Connection) -> Option<i64> {
    let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0)).ok()?;
    let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).ok()?;
    Some(pages.saturating_mul(size))
}

/// 原串上的 ASCII 大小写不敏感查找（返回原串字节偏移；禁止用 to_lowercase 偏移切串）。
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| {
        h[i..i + n.len()]
            .iter()
            .zip(n)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// 取命中前后约 40 字符的片段，并用 \u{1}/\u{2} 哨兵标出命中，
/// 交给 snippet_highlight 统一转义与还原 <mark>（转义在哨兵还原之前）。
fn short_snippet(raw: &str, needle: &str) -> String {
    let q = needle.trim();
    let Some(hit) = find_ci(raw, q) else {
        return raw.chars().take(80).collect();
    };
    let before: Vec<usize> = raw[..hit].char_indices().map(|(i, _)| i).collect();
    let begin = if before.len() > 40 {
        before[before.len() - 40]
    } else {
        0
    };
    let hit_end = hit + q.len();
    let after_end = raw[hit_end..]
        .char_indices()
        .nth(40)
        .map(|(i, _)| hit_end + i)
        .unwrap_or(raw.len());
    format!(
        "{}\u{1}{}\u{2}{}",
        &raw[begin..hit],
        &raw[hit..hit_end],
        &raw[hit_end..after_end]
    )
}

/// I23：把用户原始输入转成 FTS5 安全查询（整体短语 + 双引号加倍）。
/// FTS5 的 MATCH 语法把 AND/OR/NOT/NEAR、冒号、连字符、括号、星号都当运算符；
/// 直接拼进 MATCH 会语法错误（旧实现被 filter_map/ unwrap_or_default 吞成空结果）。
/// 用整体短语引号后，所有特殊字符都按普通文本处理，查询不再报错。
fn fts_user_query(raw: &str) -> String {
    let trimmed = raw.trim();
    format!("\"{}\"", trimmed.replace('"', "\"\""))
}

/// 旧 FTS 路径（≥3 字符）：LIMIT 80 与 snippet 规则保持原样，错误改为可读返回。
fn search_fts(conn: &rusqlite::Connection, query: &str) -> Result<Vec<SearchHit>, String> {
    // 直接用 FTS5 snippet() 生成片段：snippet 第二参 -1 = 首个命中列（title 或 body）。
    // snippet() 不做 HTML 转义，故用正文不会出现的哨兵字符包裹命中，取出后先在 Rust 侧
    // 按 highlight_snippet 同款规则转义（& < >），再把哨兵还原为 <mark>/</mark>。
    let q = fts_user_query(query);
    let sql = "SELECT fts.book_id, fts.chapter_index, fts.title, COALESCE(b.title,''), \
               snippet(fts, -1, ?2, ?3, '…', 12) \
               FROM fts LEFT JOIN books b ON b.id=fts.book_id \
               WHERE fts MATCH ?1 LIMIT 80";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("搜索失败：{e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![&q, "\u{1}", "\u{2}"], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| format!("搜索失败：{e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("搜索失败：{e}"))?;
    let mut hits: Vec<SearchHit> = Vec::with_capacity(rows.len() + 8);
    let mut seen: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    for (book_id, chapter_index, chapter_title, book_title, raw) in rows {
        seen.insert((book_id, chapter_index));
        hits.push(SearchHit {
            book_id,
            book_title,
            chapter_index,
            chapter_title,
            snippet: snippet_highlight(&raw),
        });
    }
    // P1-9：书名参与搜索（≥3 字路径与 2 字路径一致）。补充查询保持与 FTS 相同的可见性：
    // 只返回 books 中现存、且确实存在 FTS 行（已索引、未被排除）的书；LIMIT 80。
    let pat = format!("%{}%", db::escape_like(query.trim()));
    let mut tstmt = conn
        .prepare(
            "SELECT b.id, COALESCE(b.title,'') FROM books b \
             WHERE b.title LIKE ?1 ESCAPE '\\' \
               AND EXISTS(SELECT 1 FROM fts f WHERE f.book_id=b.id) \
             ORDER BY b.added_at DESC, b.id DESC LIMIT 80",
        )
        .map_err(|e| format!("搜索失败：{e}"))?;
    let trows = tstmt
        .query_map(rusqlite::params![&pat], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| format!("搜索失败：{e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("搜索失败：{e}"))?;
    for (book_id, title) in trows {
        if hits.len() >= 80 {
            break;
        }
        if !seen.insert((book_id, 0)) {
            continue;
        }
        let escaped = title
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        hits.push(SearchHit {
            book_id,
            book_title: title.clone(),
            chapter_index: 0,
            chapter_title: String::new(),
            snippet: format!("<mark>{escaped}</mark>"),
        });
    }
    Ok(hits)
}

/// H12：2 字符查询的替代路径（trigram FTS 对 <3 字符恒 0 结果，旧实现静默返回空）。
/// 顺序硬约定：先书名/作者 LIKE；有命中直接返回（即使正文被规模守卫跳过，也不返回 Err）；
/// 只有书名/作者 0 命中且正文被守卫跳过时才返回 Err，消息说明需要至少 3 个字符。
fn search_short(conn: &rusqlite::Connection, query: &str) -> Result<Vec<SearchHit>, String> {
    let allow_body = short_body_allowed(estimate_db_bytes(conn));
    search_short_with_body(conn, query, allow_body)
}

/// 可注入「是否允许正文回退」的实现（便于单测守卫分支；生产入口见 search_short）。
fn search_short_with_body(
    conn: &rusqlite::Connection,
    query: &str,
    allow_body: bool,
) -> Result<Vec<SearchHit>, String> {
    let pat = format!("%{}%", db::escape_like(query.trim()));
    let mut hits: Vec<SearchHit> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, title, COALESCE(author,'') FROM books \
                 WHERE title LIKE ?1 ESCAPE '\\' OR COALESCE(author,'') LIKE ?1 ESCAPE '\\' \
                 ORDER BY added_at DESC, id DESC LIMIT 80",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![&pat], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, title, author) = row.map_err(|e| e.to_string())?;
            let raw = if author.trim().is_empty() {
                title.clone()
            } else {
                format!("{title} · {author}")
            };
            hits.push(SearchHit {
                book_id: id,
                book_title: title,
                chapter_index: 0,
                chapter_title: String::new(),
                snippet: snippet_highlight(&short_snippet(&raw, query)),
            });
        }
    }
    if !hits.is_empty() {
        return Ok(hits);
    }
    // 正文回退：先过规模守卫。
    if !allow_body {
        return Err("2 个字符的查询只匹配书名与作者；正文搜索需要至少 3 个字符".to_string());
    }
    let mut stmt = conn
        .prepare(
            "SELECT ct.book_id, ct.chapter_index, ct.title, COALESCE(b.title,''), ct.body \
             FROM chapter_text ct LEFT JOIN books b ON b.id=ct.book_id \
             WHERE ct.body LIKE ?1 ESCAPE '\\' LIMIT 80",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![&pat], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (book_id, chapter_index, chapter_title, book_title, body) = row.map_err(|e| e.to_string())?;
        hits.push(SearchHit {
            book_id,
            book_title,
            chapter_index,
            chapter_title,
            snippet: snippet_highlight(&short_snippet(&body, query)),
        });
    }
    Ok(hits)
}

/// 分流：<2 非空白字符维持现状（返回空）；≥3 走 FTS；2 走 search_short。
fn search_all_inner(state: &Arc<AppState>, query: String) -> Result<Vec<SearchHit>, String> {
    let n = non_whitespace_chars(&query);
    if n < 2 {
        return Ok(vec![]);
    }
    let conn = state.db.0.lock().unwrap();
    if n >= 3 {
        if !db::has_fts(&conn) {
            return Ok(vec![]);
        }
        return search_fts(&conn, &query);
    }
    search_short(&conn, &query)
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
        if doc.split {
            // P11：切分书按虚拟章收集纯文本；每个 spine 文档只读一次。
            let mut raws: HashMap<usize, String> = HashMap::new();
            for c in &doc.chapters {
                for si in c.spine..c.spine_end.min(doc.spine.len()) {
                    if !raws.contains_key(&si) {
                        if let Some(b) = doc
                            .spine
                            .get(si)
                            .and_then(|dp| crate::epub::read_entry_file(std::path::Path::new(&cached.path), &cached.ci, dp))
                        {
                            raws.insert(si, String::from_utf8_lossy(&b).into_owned());
                        }
                    }
                }
            }
            doc.chapters
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let text = if let Some(start) = c.start {
                        raws.get(&c.spine)
                            .map(|raw| {
                                crate::epub::html_to_text(crate::epub::slice_body(
                                    raw, start, c.end,
                                ))
                            })
                            .unwrap_or_default()
                    } else {
                        (c.spine..c.spine_end.min(doc.spine.len()))
                            .filter_map(|si| raws.get(&si))
                            .map(|raw| crate::epub::html_to_text(raw))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    (i as i64, c.title.clone(), text)
                })
                .collect()
        } else {
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
            let data = cached.read_all().ok_or("读取 EPUB 失败")?;
            let texts = crate::epub::collect_texts(&data, &doc.spine);
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
        }
    } else if ext == "pdf" {
        // PDF 的文字**只能由前端 pdf.js 抽取后回传**（后端不解析 PDF）：直接用回传结果。
        // 没回传过（扫描版 / 乱码页 / 尚未访问的页）就是空 —— 搜索与朗读自然跳过，不报错。
        let conn = state.db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT chapter_index, title, body FROM chapter_text WHERE book_id=?1 ORDER BY chapter_index")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([book_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        rows
    } else {
        // 复用 MOBI 解析缓存（load_mobi）：建索引不再重复解析整本（旧实现 open 一次 + 索引一次）。
        let (cached, _warn) = load_mobi(state, &book.path, book_id)?;
        let (html, parts, toc) = cached.as_ref();
        toc.iter()
            .zip(parts)
            .map(|(t, (s, e))| {
                let raw = safe_slice(html, *s, *e);
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
    // 分批写（P2）：与 FTS 同款上限，批间释放 state.db.0，避免首次书内搜索建缓存时长时间卡住翻章/进度保存。
    let batches = plan_fts_batches(&chapters);
    let total = batches.len();
    for (n, (start, end)) in batches.iter().enumerate() {
        {
            let mut conn = state.db.0.lock().unwrap();
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            for (idx, title, body) in &chapters[*start..*end] {
                tx.execute(
                    "INSERT OR REPLACE INTO chapter_text(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
                    rusqlite::params![book_id, idx, title, body],
                )
                .ok();
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        // R7-3：每批提交后发一次进度（最后一次 done=total），供搜索弹窗显示百分比。
        state
            .app
            .emit(
                "book-text-progress",
                crate::models::BookTextProgress { book_id, done: n + 1, total },
            )
            .ok();
        std::thread::sleep(std::time::Duration::from_millis(FTS_BATCH_PAUSE_MS));
    }
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
        .map_err(|e| e.to_string())?
}

fn search_in_book_inner(
    state: &Arc<AppState>,
    book_id: i64,
    query: String,
) -> Result<Vec<BookSearchHit>, String> {
    let compact_q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if compact_q.is_empty() {
        return Ok(vec![]);
    }
    // R7-3：首次搜索建立缓存失败时把错误上抛（前端提示重试），不再静默返回空结果。
    ensure_book_text(state, book_id)?;
    let rows: Vec<(i64, String, String)> = {
        let conn = state.db.0.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT chapter_index, title, body FROM chapter_text WHERE book_id=?1 ORDER BY chapter_index",
            )
            .map_err(|e| e.to_string())?;
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
    Ok(out)
}

/// S2：划线创建用的书指纹（大小+首尾 64KB sha256）。抽成 helper 以便单测；
/// 文件不可读时返回 None（哈希为空串），调用方跳过 upsert 而不是写一条空指纹。
fn highlight_fingerprint(path: &str) -> Option<(i64, i64, String)> {
    let (size, mtime, hash) = db::file_fingerprint(path);
    if hash.is_empty() {
        None
    } else {
        Some((size, mtime, hash))
    }
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
    // S2：读文件+sha256 是重 IO，绝不能持全局 DB 锁做（旧实现会让所有 DB 访问排队等哈希）。
    // 第一段短锁只取 path/书名/作者快照；书不存在返回可读错误（旧实现静默跳过指纹）。
    let (path, title_snap, author_snap) = {
        let conn = state.db.0.lock().unwrap();
        let b = db::get_book(&conn, book_id).ok_or("书籍不存在")?;
        let (t, a) = if b.title.trim().is_empty() {
            parse_filename(&b.path)
        } else {
            (b.title.clone(), b.author.clone())
        };
        (b.path, t, a)
    };
    // 锁外算指纹。
    let fp = highlight_fingerprint(&path);
    let conn = state.db.0.lock().unwrap();
    // 再确认书仍在：读盘期间可能被并发删除，避免给已删书写指纹孤儿行。
    if db::get_book(&conn, book_id).is_none() {
        return Err("书籍不存在".into());
    }
    if let Some((size, mtime, hash)) = fp {
        // 指纹写失败必须上报（旧实现 .ok() 静默吞错）。
        db::upsert_book_fingerprint(&conn, book_id, size, mtime, &hash, &title_snap, &author_snap)?;
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

/// 把书名/作者写回原始电子书文件（当前支持 EPUB），同时更新数据库显示。
///
/// 写文件在 spawn_blocking 里执行：重写整本 zip 是秒级磁盘 IO，不能占 Tauri 主线程；
/// 失败时原文件字节不变（见 epub::write_meta 的同卷临时文件 + 原子替换）。
#[tauri::command]
pub async fn write_book_meta(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    title: String,
    author: String,
) -> Result<(), String> {
    // 只取路径后立即释放 DB 锁：重写期间不阻塞其它 IPC。
    let path = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?.path
    };
    let ext = ext_of(&path).unwrap_or_default();
    if ext != "epub" {
        return Err("写回原文件目前仅支持 EPUB；其他格式请先用「转换格式」转为 EPUB".into());
    }
    let st = state.inner().clone();
    let (p, t, a) = (path, title.clone(), author.clone());
    tauri::async_runtime::spawn_blocking(move || crate::epub::write_meta(&p, &t, &a))
        .await
        .map_err(|e| format!("写回任务失败：{e}"))??;
    {
        let conn = st.db.0.lock().unwrap();
        db::update_book_title_author(&conn, book_id, &title, &author)?;
    }
    st.app.emit("library-changed", ()).ok();
    Ok(())
}

/// ── FTS 索引（P2）──
///
/// 旧实现把整本正文放进**一个事务**写 fts（47MB TXT 实测持锁 18.6s、MOBI 建索引 26.1s），
/// 期间任何 DB IPC（翻章/进度保存）都排在 state.db.0 后面 → 用户感知「点什么都没反应」。
/// 现在：一致性判据决定是否重建；重建时按小批提交，批间释放锁并短暂 sleep，让其它 IPC 拿到锁。

/// 每批最多章数。
const FTS_BATCH_MAX_CHAPTERS: usize = 32;
/// 每批正文累计字节上限（title + body）。32 章 / 256KB 的单批提交是几十毫秒级，
/// 把整本 18.6~26.1s 的持锁压到约 100~150ms。
const FTS_BATCH_MAX_BYTES: usize = 256 * 1024;
/// 批间让出 state.db.0 的停顿：给翻章/存进度等短 IPC 拿锁窗口，整本索引只多花毫秒级。
const FTS_BATCH_PAUSE_MS: u64 = 2;
/// S2：手动全库重建等待索引闸门的上限；持锁方卡死时不能永久挂起命令。
const FTS_GUARD_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// 闸门等待是否超时（纯函数便于测试边界）。
fn fts_guard_wait_exceeded(
    start: std::time::Instant,
    now: std::time::Instant,
    limit: std::time::Duration,
) -> bool {
    now.saturating_duration_since(start) > limit
}

/// 自动索引的体积上限（P9）：超过只允许用户在搜索时显式建立（大书建索引实测 8~40s，
/// 阅读中/关书时自动跑就是用户感知的「更卡」）。显式搜索仍可建（auto=false 不受限）。
const AUTO_INDEX_MAX_BYTES: i64 = 20 * 1024 * 1024;
/// 自动索引的章数上限（同上：章数极多的书建索引同样贵）。
const AUTO_INDEX_MAX_CHAPTERS: i64 = 3000;

/// 自动索引是否允许这个体积（等于上限通过）。
fn auto_index_size_allowed(size: i64) -> bool {
    size <= AUTO_INDEX_MAX_BYTES
}

/// 自动索引是否允许这个章数（等于上限通过）。
fn auto_index_chapters_allowed(n: i64) -> bool {
    n <= AUTO_INDEX_MAX_CHAPTERS
}

/// FTS 是否需要重建：行数与期望章数不一致就重建。
/// 相等 = 已索引完成；0/N、部分行、历史重复多出行都会走到重建（先 DELETE 再写，重复行被修正）。
fn fts_needs_rebuild(fts_rows: i64, expected: i64) -> bool {
    fts_rows != expected
}

/// FTS 一致性检查结论（P12）：meta 优先；meta 缺失时用一次旧实例 count 做迁移。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FtsMetaPlan {
    /// 已索引（meta 行数 == 期望章数），无需动作。
    Skip,
    /// 旧库迁移：实例行数 == 期望章数，补写 meta 后视为已索引（不重建）。
    Adopt(i64),
    /// 需要重建（meta 与期望不一致，或没有 meta 且实例也不一致）。
    Rebuild,
}

/// 纯函数：根据 meta 行数 / 旧实例计数 / 期望章数给出动作。
/// - meta 存在：与 expected 一致 → Skip，否则 Rebuild；
/// - meta 缺失：旧实例 count 与 expected 一致 → Adopt(count)（升级迁移，避免全库重建），否则 Rebuild。
fn fts_meta_plan(meta_rows: Option<i64>, legacy_count: Option<i64>, expected: i64) -> FtsMetaPlan {
    if let Some(rows) = meta_rows {
        return if fts_needs_rebuild(rows, expected) {
            FtsMetaPlan::Rebuild
        } else {
            FtsMetaPlan::Skip
        };
    }
    match legacy_count {
        Some(n) if !fts_needs_rebuild(n, expected) => FtsMetaPlan::Adopt(n),
        _ => FtsMetaPlan::Rebuild,
    }
}

/// 按「章数 + 正文字节」上限把整本书切成若干批，返回每批的 [start, end)。
/// 单章超过字节上限时独占一批（fts 行是一章一条，不能拆章）。
fn plan_fts_batches(chapters: &[(i64, String, String)]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0usize;
    for (i, (_, title, body)) in chapters.iter().enumerate() {
        let n = title.len() + body.len();
        let count = i - start;
        if count > 0 && (count >= FTS_BATCH_MAX_CHAPTERS || bytes + n > FTS_BATCH_MAX_BYTES) {
            out.push((start, i));
            start = i;
            bytes = 0;
        }
        bytes += n;
    }
    if start < chapters.len() {
        out.push((start, chapters.len()));
    }
    out
}

/// 该书写入 FTS 后应有的行数（一致性判据的基准）。
/// PDF：文字只能由前端回传，直接数 chapter_text 行；其它格式：解析缓存里的章数
/// （load_* 在「刚打开/刚索引」这类 warm 路径上是 Arc 命中，成本可忽略）。
/// 注意 EPUB 按 spine 出章（一章可拆成多个文件），所以用 spine.len() 而不是 toc.len()。
fn expected_fts_rows(state: &Arc<AppState>, book_id: i64, path: &str, ext: &str) -> Result<i64, String> {
    if ext == "pdf" {
        let conn = state.db.0.lock().unwrap();
        let n = conn
            .query_row(
                "SELECT count(*) FROM chapter_text WHERE book_id=?1",
                [book_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        return Ok(n);
    }
    let n = if ext == "epub" {
        let cached = load_epub(state, path, book_id)?;
        // P11：切分书的索引行数是虚拟章数，不是 spine 文档数。
        if cached.doc.split {
            cached.doc.chapters.len()
        } else {
            cached.doc.spine.len()
        }
    } else if ext == "txt" {
        load_txt(state, path, book_id)?.2.len()
    } else {
        load_mobi(state, path, book_id)?.0.2.len()
    };
    Ok(n as i64)
}

/// S2：单事务替换某书的全部 fts 行并写 meta。成功才动旧行：
/// DELETE → 逐行 INSERT → meta=实际行数；任一步失败整体回滚，旧行与旧 meta 原样保留。
fn replace_book_fts(
    conn: &mut rusqlite::Connection,
    book_id: i64,
    chapters: &[(i64, String, String)],
) -> Result<i64, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM fts WHERE book_id=?1", [book_id])
        .map_err(|e| e.to_string())?;
    for (idx, title, body) in chapters {
        tx.execute(
            "INSERT INTO fts(book_id, chapter_index, title, body) VALUES(?1,?2,?3,?4)",
            rusqlite::params![book_id, idx, title, body],
        )
        .map_err(|e| format!("写入全文索引失败：{e}"))?;
    }
    // 直接写 fts_meta（不调 db::set_fts_meta）：meta 写入失败也必须回滚，不能静默吞错。
    tx.execute(
        "INSERT INTO fts_meta(book_id, rows) VALUES(?1,?2)
         ON CONFLICT(book_id) DO UPDATE SET rows=?2",
        rusqlite::params![book_id, chapters.len() as i64],
    )
    .map_err(|e| format!("写入全文索引标记失败：{e}"))?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(chapters.len() as i64)
}

/// S2：解析失败时保留旧 fts 行，把 meta 写成负哨兵 -1。
/// 依据（已 grep 确认）：
///  - 搜索路径只读 fts 表，不读 fts_meta（get_fts_meta 仅索引一致性检查使用）；
///  - fts_meta_plan(Some(-1), _, expected) 中 -1 != expected 必为 Rebuild，因此下次索引会重建；
///  - 同样的 -1 哨兵语义已在 db::invalidate_split_book_search_cache 使用（P11）。
fn mark_book_fts_stale(conn: &rusqlite::Connection, book_id: i64) {
    conn.execute(
        "INSERT INTO fts_meta(book_id, rows) VALUES(?1,-1)
         ON CONFLICT(book_id) DO UPDATE SET rows=-1",
        [book_id],
    )
    .ok();
}

/// S2：解析结果 + 连接级重建动作。解析失败时保留旧 fts 行、只写 -1 哨兵并返回原错误；
/// 解析成功才在单事务里替换（任一步失败回滚，旧行与旧 meta 不动）。
/// 抽出连接级版本是为了在无 AppState 的单测里覆盖「失败保留旧行」语义。
fn apply_book_fts_rebuild(
    conn: &mut rusqlite::Connection,
    book_id: i64,
    chapters: Result<Vec<(i64, String, String)>, String>,
) -> Result<i64, String> {
    match chapters {
        Ok(ch) => replace_book_fts(conn, book_id, &ch),
        Err(e) => {
            mark_book_fts_stale(conn, book_id);
            Err(e)
        }
    }
}

/// S2：逐本重建的循环（单本失败跳过并记录，不中断其余书）；抽出来便于测试「多书一坏」语义。
fn rebuild_fts_books_with<F>(ids: &[i64], mut one: F) -> i64
where
    F: FnMut(i64) -> Result<i64, String>,
{
    let mut total = 0i64;
    for id in ids {
        match one(*id) {
            Ok(n) => total += n,
            Err(e) => eprintln!("[fts] rebuild skip book={id}: {e}"),
        }
    }
    total
}

#[tauri::command]
pub async fn rebuild_fts(state: State<'_, Arc<AppState>>) -> Result<i64, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 与自动索引共用闸门；手动重建是用户显式操作，等后台索引让出后继续（不会静默返回 0）。
        // S2：等待有超时——持锁方卡死时命令不能永久挂起，超时返回可读错误。
        let start = std::time::Instant::now();
        let guard = loop {
            if let Some(g) = FtsIndexGuard::acquire(&st.fts_index_running) {
                break g;
            }
            if fts_guard_wait_exceeded(start, std::time::Instant::now(), FTS_GUARD_WAIT_TIMEOUT) {
                return Err("索引任务仍在运行，请稍后重试".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let out = rebuild_fts_inner(&st);
        drop(guard);
        out
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 全库重建（调用方必须已持有索引闸门）：逐本重建，返回成功总章数。
/// S2：不再开头 DELETE 全表 + 清 meta —— 那样一旦某本书解析失败，它的索引会永久丢失。
/// 改为逐本走同一套「成功才替换、失败保留旧行 + 需重建标记」的流程。
fn rebuild_fts_inner(state: &Arc<AppState>) -> Result<i64, String> {
    {
        let conn = state.db.0.lock().unwrap();
        if !db::has_fts(&conn) {
            return Err("SQLite 编译未包含 FTS5".into());
        }
        db::ensure_fts(&conn)?;
    }
    let ids: Vec<i64> = {
        let conn = state.db.0.lock().unwrap();
        db::all_book_paths(&conn).into_iter().map(|(id, _)| id).collect()
    };
    // 单本失败（文件缺失/解析失败等）跳过，不影响整体重建；全库重建是用户显式操作，传 auto=false。
    let total = rebuild_fts_books_with(&ids, |id| index_book_fts_inner(state, id, false));
    // S2：重建结束后清理孤儿（已删书籍遗留的 fts 行与 meta）。
    {
        let conn = state.db.0.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM fts WHERE book_id NOT IN (SELECT id FROM books)",
            [],
        );
        let _ = conn.execute(
            "DELETE FROM fts_meta WHERE book_id NOT IN (SELECT id FROM books)",
            [],
        );
    }
    // R11-4：重建完成后整理 FTS5 索引（只压缩索引、不删数据）。副本库实测 14.6s
    //（远低于 60s 阈值，故放在重建命令内同步执行；启动期绝不执行）。
    {
        let t = std::time::Instant::now();
        let conn = state.db.0.lock().unwrap();
        match db::optimize_fts(&conn) {
            Ok(()) => eprintln!("[fts] optimize done ms={}", t.elapsed().as_millis()),
            Err(e) => eprintln!("[fts] optimize failed: {e}"),
        }
    }
    Ok(total)
}

#[tauri::command]
pub async fn index_book_fts(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    auto: Option<bool>,
) -> Result<i64, String> {
    // 前端显式操作传 false/缺省，自动调度传 true：后者受体积/章数上限约束（P9）。
    let auto = auto.unwrap_or(false);
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 单实例闸门：已有索引任务在跑就直接跳过（前端是延迟调度，下一轮还会触发）。
        let Some(_guard) = FtsIndexGuard::acquire(&st.fts_index_running) else {
            return Ok(0);
        };
        index_book_fts_inner(&st, book_id, auto)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn index_book_fts_inner(state: &Arc<AppState>, book_id: i64, auto: bool) -> Result<i64, String> {
    // 1) 能力检查 + 书元信息（短锁）。
    let (path, ext, size) = {
        let conn = state.db.0.lock().unwrap();
        if !db::has_fts(&conn) {
            return Ok(0);
        }
        db::ensure_fts(&conn)?;
        let book = db::get_book(&conn, book_id).ok_or("书籍不存在")?;
        let ext = ext_of(&book.path).unwrap_or_default();
        (book.path, ext, book.size)
    };
    // 自动索引只服务小书（P9）：体积判断放在解析之前，避免为超大文件白解析一遍。
    if auto && !auto_index_size_allowed(size) {
        return Ok(0);
    }
    // 2) 一致性判据：行数与期望章数相等视为已索引（含空书），不做整本解析/写库；
    //    不一致（0/N、部分、历史重复）才重建 —— 中断/崩溃留下的半成品会被自动修正。
    let expected = expected_fts_rows(state, book_id, &path, &ext)?;
    // 章数上限同样只约束自动索引：超大书留给用户搜索时显式建立（P9）。
    if auto && !auto_index_chapters_allowed(expected) {
        return Ok(0);
    }
    // P12：一致性检查优先用 fts_meta（fts5 的 book_id 是 UNINDEXED，count(*) 会全表扫描）；
    // meta 缺失时读一次旧实例计数做升级迁移（避免升级后全库重建）。
    let (meta_rows, legacy_count) = {
        let conn = state.db.0.lock().unwrap();
        let meta = db::get_fts_meta(&conn, book_id);
        let legacy = if meta.is_none() {
            Some(
                conn.query_row(
                    "SELECT count(*) FROM fts WHERE book_id=?1",
                    [book_id],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            )
        } else {
            None
        };
        (meta, legacy)
    };
    match fts_meta_plan(meta_rows, legacy_count, expected) {
        FtsMetaPlan::Skip => return Ok(0),
        FtsMetaPlan::Adopt(rows) => {
            let conn = state.db.0.lock().unwrap();
            db::set_fts_meta(&conn, book_id, rows);
            return Ok(0);
        }
        FtsMetaPlan::Rebuild => {}
    }
    // 3) 先解析（warm 路径命中解析缓存）再做任何写入：解析失败时保留旧 fts 行，
    //    只把 meta 写成 -1 哨兵，下次索引必然重建（旧实现先 DELETE 再解析，失败即永久丢索引）。
    //    不持 DB 锁解析（collect 内部会短锁/读盘），完成后才进连接级替换。
    let chapters = collect_book_chapters(state, book_id);
    let mut conn = state.db.0.lock().unwrap();
    apply_book_fts_rebuild(&mut conn, book_id, chapters)
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

/// 缺封面的 PDF（仅 id，升序）：给书架「后台补封面」用。
/// 为什么不复用 list_books：那是整表全量行（含 path/title 字符串），16 万本时每次书库刷新
/// 拉一遍太重；这里只取 id，代价是一次列扫描。format 落库时已是小写（ext_of 归一）。
#[tauri::command]
pub fn pdf_books_missing_cover(state: State<'_, Arc<AppState>>) -> Result<Vec<i64>, String> {
    // 连续启动失败保护：本轮跳过封面重建时返回空清单 —— 前端「补 PDF 封面」循环据此立即收工，
    // 不再让有崩溃风险的封面链路在启动期继续跑。下一轮成功启动会自动恢复。
    if memlog::skip_cover_rebuild() {
        memlog::stage_note("cmd:pdfBooksMissingCover", "skipped (previous start crashed)");
        return Ok(Vec::new());
    }
    let conn = state.db.0.lock().unwrap();
    let ids = db::list_pdf_ids_missing_cover(&conn);
    drop(conn);
    memlog::watch("cmd:pdf_books_missing_cover");
    // 调用次数本身是诊断信息（O(N²) 的形态就长这样），但逐调用落盘会刷屏：每 200 次记一行。
    let calls = PDF_MISSING_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if calls == 1 || calls % 200 == 0 {
        memlog::stage_note("cmd:pdfBooksMissingCover", &format!("calls={calls} ids={}", ids.len()));
    }
    Ok(ids)
}

/// 设置封面扫描开关（首选项「封面扫描」）。语义见 AppState::cover_scan_enabled 的说明。
#[tauri::command]
pub fn set_cover_scan(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), String> {
    // 「连续启动失败」保护优先：本轮跳过封面重建时，前端启动期的 setCoverScan(true) 不得把它打开。
    if memlog::skip_cover_rebuild() {
        state.cover_scan_enabled.store(false, Ordering::SeqCst);
        return Ok(());
    }
    state.cover_scan_enabled.store(enabled, Ordering::SeqCst);
    // 持久化模式：启动自愈在 worker 起来之前就要判断 auto/manual（atomic 的默认值恒为 true）。
    {
        let conn = state.db.0.lock().unwrap();
        db::set_setting(&conn, COVER_SCAN_MODE_KEY, if enabled { "auto" } else { "manual" });
    }
    // R11-3：首屏就绪前只持久化模式/开关，不触发自愈/扫描；ready 时由 library_ready 补做。
    if !state.library_ready.load(Ordering::SeqCst) {
        return Ok(());
    }
    // 模式键此前不存在（升级后第一次启动）时，启动路径没敢复位；这里在 auto 下补做一次。
    // 每进程最多一次，且整段带 worker 停/起，避免与在途认领交叉。
    if enabled && !COVER_HEAL_DONE.swap(true, Ordering::SeqCst) {
        heal_covers_now(state.inner());
    }
    // R11-3：无论 auto/manual 都要确保 worker 在跑——manual 只跳过封面提取，
    // 仍要处理 meta_done=0 的元数据/字数（cover_scan_enabled=false 在 worker 内部生效）。
    ensure_scan_worker(&state.app);
    Ok(())
}

/// 只取 id 的分页入口（每批上限 500）：给「按 id 遍历全库」的操作（如重建 FTS）用。
/// 为什么不让前端用 list_books：那是整表物化 + 整表序列化，17 万本一次 IPC 就是几十 MB 的
/// 行对象与 JSON 文本；这里每批只有 limit 个 i64，内存与书数无关。
#[tauri::command]
pub async fn list_book_ids_page(
    state: State<'_, Arc<AppState>>,
    offset: i64,
    limit: i64,
) -> Result<Vec<i64>, String> {
    memlog::watch("cmd:list_book_ids_page");
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::list_book_ids_page(&conn, offset.max(0), limit.clamp(1, 500))
    })
    .await
    .map_err(|e| e.to_string())
}

/// 前端书库挂载并取到首屏行后调用：把本轮启动标记为「已走到书库」。
/// 下一次启动据此判断上一轮是否崩溃 —— 崩溃则自动跳过封面重建并记一条日志。
/// B2：空闲 FTS optimize 的进程内调度闸门（同一时刻最多一个后台检查线程，避免风暴）。
/// 暂停/索引/维护中不是放弃，而是延后：线程内每 3 分钟重试一轮，最多 10 轮；
/// set_scan_paused(false) 也会再触发一次（若闸门空闲）。
static FTS_IDLE_OPT_ACTIVE: AtomicBool = AtomicBool::new(false);

fn schedule_fts_idle_optimize(st: &Arc<AppState>, initial_delay_secs: u64) {
    if FTS_IDLE_OPT_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    let st = st.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(initial_delay_secs));
        for attempt in 0..10u32 {
            match try_idle_optimize_fts(&st) {
                FtsOptimizeOutcome::Done => break,
                FtsOptimizeOutcome::Retry => {
                    memlog::stage_note(
                        "fts:optimize",
                        &format!("retry attempt={attempt} wait=180s"),
                    );
                    std::thread::sleep(std::time::Duration::from_secs(180));
                }
            }
        }
        FTS_IDLE_OPT_ACTIVE.store(false, Ordering::SeqCst);
    });
}

enum FtsOptimizeOutcome {
    Done,
    Retry,
}

/// B2：后台检查并执行 FTS5 optimize（独立连接，不持有全局 db Mutex）。
/// 返回 Retry 表示当前被暂停/索引/维护/临时打开失败挡住，稍后再试；Done 表示本轮无需或已完成。
fn try_idle_optimize_fts(st: &Arc<AppState>) -> FtsOptimizeOutcome {
    if st.scan_paused.load(Ordering::SeqCst)
        || st.fts_index_running.load(Ordering::SeqCst)
        || st.maintenance_running.load(Ordering::SeqCst)
    {
        memlog::stage_note("fts:optimize", "skip paused_indexing_or_maintenance");
        return FtsOptimizeOutcome::Retry;
    }
    let path = crate::data_dir(&st.app).join("tidepage.db");
    let conn = match rusqlite::Connection::open(&path) {
        Ok(c) => c,
        Err(e) => {
            memlog::stage_note("fts:optimize", &format!("skip open_err={e}"));
            return FtsOptimizeOutcome::Retry;
        }
    };
    // H6：独立写连接同样要有等锁上限（与 VACUUM 的 5s 口径一致），否则并发写时
    // optimize 直接报 BUSY 被当成普通失败。设置失败只记日志，不改变原有重试语义。
    if let Err(e) = conn.busy_timeout(std::time::Duration::from_secs(5)) {
        memlog::stage_note("fts:optimize", &format!("busy_timeout_set_err={e}"));
    }
    if !db::has_fts(&conn) {
        return FtsOptimizeOutcome::Done;
    }
    let Some((bytes, rows)) = db::fts_stats(&conn) else {
        memlog::stage_note("fts:optimize", "skip no_dbstat");
        return FtsOptimizeOutcome::Done;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let marker = db::get_setting(&conn, "fts.optimize.marker").unwrap_or_default();
    let mut parts = marker.split('|');
    let last_at = parts.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let last_bytes = parts.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let last_rows = parts.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    // 失败退避：6 小时内不重复尝试，避免每次启动风暴。
    let fail_at = db::get_setting(&conn, "fts.optimize.fail_at")
        .and_then(|v| v.split('|').next().and_then(|s| s.parse::<i64>().ok()))
        .unwrap_or(0);
    if fail_at > 0 && now.saturating_sub(fail_at) < 6 * 3600 {
        memlog::stage_note("fts:optimize", "skip backoff");
        return FtsOptimizeOutcome::Done;
    }
    if !db::should_optimize_fts(
        now,
        last_at,
        last_bytes,
        last_rows,
        bytes,
        rows,
        st.fts_index_running.load(Ordering::SeqCst),
        st.scan_paused.load(Ordering::SeqCst),
    ) {
        memlog::stage_note(
            "fts:optimize",
            &format!("skip bytes={bytes} rows={rows} marker_at={last_at}"),
        );
        return FtsOptimizeOutcome::Done;
    }
    // 长耗时操作前再与维护/VACUUM 做一次互斥握手：拿到 optimize 标志后再确认维护没启动。
    if st.maintenance_running.load(Ordering::SeqCst) {
        memlog::stage_note("fts:optimize", "skip maintenance_before_exec");
        return FtsOptimizeOutcome::Retry;
    }
    if st.fts_optimize_running.swap(true, Ordering::SeqCst) {
        return FtsOptimizeOutcome::Retry;
    }
    if st.maintenance_running.load(Ordering::SeqCst) {
        st.fts_optimize_running.store(false, Ordering::SeqCst);
        return FtsOptimizeOutcome::Retry;
    }
    let t = std::time::Instant::now();
    let exec_result = conn.execute("INSERT INTO fts(fts) VALUES('optimize')", []);
    st.fts_optimize_running.store(false, Ordering::SeqCst);
    match exec_result {
        Ok(_) => {
            let new_marker = format!("{now}|{bytes}|{rows}");
            let _ = db::set_setting(&conn, "fts.optimize.marker", &new_marker);
            memlog::stage_note(
                "fts:optimize",
                &format!(
                    "done elapsed_ms={} bytes={bytes} rows={rows}",
                    t.elapsed().as_millis()
                ),
            );
        }
        Err(e) => {
            let _ = db::set_setting(&conn, "fts.optimize.fail_at", &format!("{now}|{e}"));
            memlog::stage_note("fts:optimize", &format!("failed err={e}"));
        }
    }
    FtsOptimizeOutcome::Done
}

/// B1：前端启动阶段打点（写入 startup.log，供首屏分解；不参与业务逻辑）。
#[tauri::command]
pub fn boot_stage(stage: String) {
    memlog::stage_note("frontend:boot", &stage);
}

#[tauri::command]
pub fn library_ready(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    memlog::stage("frontend:library_ready");
    memlog::ready();
    let st = state.inner().clone();
    // R11-3：首屏就绪后才允许启动封面自愈/扫描；重复调用只置位一次。
    if st.library_ready.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    std::thread::spawn(move || {
        if memlog::skip_cover_rebuild() {
            return;
        }
        // 模式键可能由前端 set_cover_scan 稍后才写入：短暂轮询（最多 2s）再决定，
        // 避免「library_ready 早于 set_cover_scan」时自愈被永久推迟。
        let mut mode = None;
        for _ in 0..20 {
            mode = {
                let conn = st.db.0.lock().unwrap();
                db::get_setting(&conn, COVER_SCAN_MODE_KEY)
            };
            if mode.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        match mode.as_deref() {
            Some("manual") => {
                // manual 只跳过封面提取（heal 也不做），但仍要处理 meta_done=0 的元数据/字数，
                // 所以不能在这里 return：由 match 之后统一 ensure_scan_worker。
                memlog::stage("setup:cover_heal_skipped_manual");
            }
            Some(_) => {
                // 每进程最多一次（与启动路径同一标记）；自愈完再由下面统一拉起 worker。
                if !COVER_HEAL_DONE.swap(true, Ordering::SeqCst) {
                    heal_covers_now(&st);
                    mark_cover_heal_done();
                }
            }
            None => {
                // 模式未知：不猜自愈；worker 仍启动（前端随后 set_cover_scan 会写入模式）。
                memlog::stage("setup:cover_heal_deferred");
            }
        }
        ensure_scan_worker(&st.app);
        // B2：首屏就绪后 25s 由独立线程检查 FTS 空闲 optimize（不占启动关键路径，可暂停跳过）。
        schedule_fts_idle_optimize(&st, 25);
    });
    Ok(())
}

/// R11-3：暂停/恢复后台封面扫描（进程内状态；resume 后有 pending/优先项时确保 worker 在跑）。
#[tauri::command]
pub fn set_scan_paused(state: State<'_, Arc<AppState>>, paused: bool) -> Result<(), String> {
    state.scan_paused.store(paused, Ordering::SeqCst);
    if !paused && state.library_ready.load(Ordering::SeqCst) {
        let has_work = {
            let conn = state.db.0.lock().unwrap();
            db::count_meta_pending(&conn) > 0 || db::next_priority(&conn).is_some()
        };
        if has_work {
            ensure_scan_worker(&state.app);
        }
        // B2：恢复（离开阅读/窗口可见）后再给空闲 FTS optimize 一次机会；闸门保证不并发风暴。
        schedule_fts_idle_optimize(&state.inner().clone(), 2);
    }
    Ok(())
}

#[tauri::command]
pub async fn reextract_cover(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<String, String> {
    // 前置只做短锁读路径；「读整本 + 解析/解包 + 生成缩略图」整体放阻塞线程池，避免卡主线程。
    let book = {
        let conn = state.db.0.lock().unwrap();
        db::get_book(&conn, book_id).ok_or("书籍不存在")?
    };
    let covers_dir = covers::covers_dir(&state.app);
    let ext = ext_of(&book.path).unwrap_or_default();
    let name = format!("{}.png", db::path_id(&book.path));
    let out = covers_dir.join(&name);
    let path = book.path.clone();
    let ok = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        if ext == "epub" {
            let data = std::fs::read(&path).map_err(|e| e.to_string())?;
            let doc = crate::epub::parse(&data).ok();
            let bytes = doc
                .as_ref()
                .and_then(|d| d.cover_href.as_ref().and_then(|h| crate::epub::read_cover(&data, h)))
                .or_else(|| doc.as_ref().map(|d| crate::epub::find_cover_fallback(&data, &d.spine)).flatten());
            Ok(match bytes {
                Some(b) => covers::make_thumbnail(&b, &out).is_ok(),
                None => false,
            })
        } else if is_mobi_ext(&ext) {
            // MOBI/AZW3：EXTH201/202/129 或 first_image_index 起首张图
            Ok(crate::mobi::extract_cover_path(&path)
                .map(|b| covers::make_thumbnail(&b, &out).is_ok())
                .unwrap_or(false))
        } else {
            Ok(false)
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    if ok {
        {
            let conn = state.db.0.lock().unwrap();
            conn.execute(
                "UPDATE books SET cover_path=?2, cover_ver=cover_ver+1 WHERE id=?1",
                rusqlite::params![book_id, &name],
            )
            .map_err(|e| e.to_string())?;
        }
        state.app.emit("book-updated", ()).ok();
        Ok(name)
    } else {
        Err("该书未能自动提取封面，请右键「设置封面」手动指定图片".into())
    }
}

/// 保存阅读器背景图。
///
/// 文件名必须**每次都不一样**：背景图 URL 只由文件名决定，同名文件会让 WebView 命中
/// 同一 URL 的缓存（`bg/` 是短缓存）、也不会重新解码，用户看到的一直是上一张图。
/// 因此按 `<时间戳>-<序号>` 唯一命名并保留原扩展名（`bg/` 端点读文件时按内容嗅探 MIME）。
///
/// `keep`：调用方仍然引用的背景图文件名（全局 `bgImage` 与各书籍方案的 `schemeBgImage`）。
/// 不在此列、且写入时间超过 `RECLAIM_AGE` 的旧图才回收 —— 留出这段窗口，
/// 避免删掉别的窗口或在途请求正在加载的文件。
#[tauri::command]
pub fn set_background_image(
    state: State<'_, Arc<AppState>>,
    image_path: String,
    keep: Option<Vec<String>>,
) -> Result<String, String> {
    let data = std::fs::read(&image_path).map_err(|e| e.to_string())?;
    let dir = covers::covers_dir(&state.app);
    let ext = bg_ext_of(&image_path);
    let name = format!("bg-{}-{}.{}", now_millis(), BG_SEQ.fetch_add(1, Ordering::Relaxed), ext);
    std::fs::write(dir.join(&name), &data).map_err(|e| e.to_string())?;
    reclaim_bg_images(&dir, &name, keep.as_deref().unwrap_or(&[]));
    Ok(name)
}

#[tauri::command]
pub fn clear_background_image(
    state: State<'_, Arc<AppState>>,
    keep: Option<Vec<String>>,
) -> Result<(), String> {
    let dir = covers::covers_dir(&state.app);
    // 当前这张随后就不再被引用，交给回收逻辑统一处理（同样受 keep 与时间窗保护）。
    reclaim_bg_images(&dir, "", keep.as_deref().unwrap_or(&[]));
    Ok(())
}

/// 背景图序号：同一毫秒内连续选图也不会撞名。
static BG_SEQ: AtomicU64 = AtomicU64::new(0);
/// 回收窗口：比这更新的文件可能仍在别的窗口或在途请求里，先留着。
const RECLAIM_AGE: std::time::Duration = std::time::Duration::from_secs(30);

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 源图扩展名（白名单；未知一律 png）。`bg/` 端点按内容嗅探真实 MIME，名字只用于区分 URL。
fn bg_ext_of(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "jpg",
        Some("webp") => "webp",
        Some("bmp") => "bmp",
        Some("gif") => "gif",
        _ => "png",
    }
}

/// 回收 `covers/bg-*` 与历史遗留的 `background.img`：跳过 `keep` 与刚写入的那张，
/// 回收本命令自己写出的 `covers/bg-*`：跳过 `keep` 与刚写入的那张，
/// 且只删写入时间超过 RECLAIM_AGE 的文件。
/// 历史遗留的 `background.img` **不回收**：它可能是用户选过、但配置里没落上的那张图，
/// 删掉就是丢用户数据；它不占 URL 空间（放着不影响换图生效）。
fn reclaim_bg_images(dir: &Path, current: &str, keep: &[String]) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_bg = name.starts_with("bg-");
        if !is_bg || name == current || keep.iter().any(|k| k == &name) {
            continue;
        }
        let fresh = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|d| d < RECLAIM_AGE).unwrap_or(false))
            .unwrap_or(true);
        if fresh {
            continue;
        }
        std::fs::remove_file(entry.path()).ok();
    }
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


/* ---------------- D3：标签命令 ---------------- */

#[derive(serde::Serialize)]
pub struct TagInfo {
    pub id: i64,
    pub name: String,
    pub book_count: i64,
}

#[tauri::command]
pub fn list_tags(state: State<'_, Arc<AppState>>) -> Result<Vec<TagInfo>, String> {
    let conn = state.db.0.lock().unwrap();
    Ok(db::list_tags(&conn)
        .into_iter()
        .map(|(id, name, book_count)| TagInfo { id, name, book_count })
        .collect())
}

#[tauri::command]
pub fn list_book_tags(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<Vec<TagInfo>, String> {
    let conn = state.db.0.lock().unwrap();
    Ok(db::list_book_tags(&conn, book_id)
        .into_iter()
        .map(|(id, name)| TagInfo { id, name, book_count: 0 })
        .collect())
}

#[tauri::command]
pub fn create_tag(state: State<'_, Arc<AppState>>, name: String) -> Result<TagInfo, String> {
    let conn = state.db.0.lock().unwrap();
    let (id, name) = db::create_tag(&conn, &name)?;
    state.app.emit("library-tags-changed", ()).ok();
    Ok(TagInfo { id, name, book_count: 0 })
}

#[tauri::command]
pub fn rename_tag(state: State<'_, Arc<AppState>>, id: i64, name: String) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::rename_tag(&conn, id, &name)?;
    state.app.emit("library-tags-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub fn delete_tag(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let conn = state.db.0.lock().unwrap();
    db::delete_tag(&conn, id)?;
    state.app.emit("library-tags-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub fn set_book_tags(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    tag_ids: Vec<i64>,
) -> Result<(), String> {
    let mut conn = state.db.0.lock().unwrap();
    db::set_book_tags(&mut conn, book_id, &tag_ids)?;
    state.app.emit("library-tags-changed", ()).ok();
    Ok(())
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
    tags: Option<Vec<String>>,
) -> Result<Option<BookRow>, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        db::random_book_filtered_tags(&conn, &query, &sort, &formats, &states, &folder, &tags)
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
    bump_formats_epoch(state.inner());
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
        if ext == "pdf" {
            // PDF 不参与字数统计：不读盘、不回填（回填 0 会让 DB 短路判定失效）。
            return Ok(WordCount { total: 0, pure: 0 });
        }
        let is_mobi = is_mobi_ext(&ext);
        if is_mobi {
            // 复用 MOBI 解析缓存（load_mobi）：书详情/本书统计不再整本重解析（同进程内刚读过也不重读）。
            let (cached, _warn) = load_mobi(&st, &path, book_id)?;
            // 借用缓存里的正文生成纯文本，不额外复制整本 HTML。
            let text = crate::epub::html_to_text(&cached.0);
            let total = text.chars().count() as i64;
            let pure = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
            backfill_book_chars(&st, book_id, total, pure);
            return Ok(WordCount { total, pure });
        }
        // TXT：load_txt 返回 Arc 元组，借用其中正文计数，不 clone 整本 String。
        let cached = load_txt(&st, &path, book_id)?;
        let text: &str = &cached.0;
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
        // P11：切分书的章节是虚拟章（目录项数 != 逐章数组长度），「全书总页数」直接给空数组，
        // 前端按长度不符忽略（该类书不显示总页数），避免整本解析后返回错位的逐章字数。
        let cached = load_epub(&st, &path, book_id)?;
        if cached.doc.split {
            return Ok(Vec::new());
        }
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
        let d = std::env::temp_dir().join(format!("tp_dcover_{}_{}", tag, std::process::id()));
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
mod stale_tests {
    use super::*;

    fn set(v: &[&str]) -> std::collections::HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 首次判缺失只登记不删除() {
        let plan = plan_stale(vec!["D:/a/1.pdf".into()], &set(&[]), &[], 10);
        assert!(plan.delete.is_empty(), "首次不得删除");
        assert_eq!(plan.pending, vec!["D:/a/1.pdf".to_string()]);
        assert!(!plan.refused);
    }

    #[test]
    fn 连续两次判缺失才删除() {
        let plan = plan_stale(vec!["D:/a/1.pdf".into()], &set(&["D:/a/1.pdf"]), &[], 10);
        assert_eq!(plan.delete, vec!["D:/a/1.pdf".to_string()]);
        assert!(plan.pending.is_empty());
    }

    #[test]
    fn 目录未走完时整目录跳过() {
        let plan = plan_stale(
            vec!["D:/a/1.pdf".into(), "D:/b/2.pdf".into()],
            &set(&["D:/a/1.pdf"]),
            &["D:/a".to_string()],
            10,
        );
        assert!(plan.delete.is_empty(), "跳过的目录不得删除");
        assert!(!plan.pending.contains(&"D:/a/1.pdf".to_string()), "跳过的目录不得进待确认");
        assert_eq!(plan.pending, vec!["D:/b/2.pdf".to_string()], "未跳过的目录照常登记");
        assert_eq!(plan.skipped, vec!["D:/a/1.pdf".to_string()]);
        assert_eq!(plan.delete.len() + plan.pending.len() + plan.skipped.len(), 2);
    }

    #[test]
    fn 规模超阈值一律不删只登记() {
        let many: Vec<String> = (0..300).map(|i| format!("D:/a/{i}.pdf")).collect();
        let pending: std::collections::HashSet<String> = many.iter().cloned().collect();
        let plan = plan_stale(many, &pending, &[], 100);
        assert!(plan.refused, "300 > max(200, 5) 必须拒绝");
        assert!(plan.delete.is_empty(), "拒绝时不得删除");
        assert_eq!(plan.pending.len(), 300, "全部转为待确认");
        assert_eq!(plan.limit, STALE_AUTO_DELETE_MIN);
    }

    #[test]
    fn 阈值随登记数增长() {
        // 登记 20000 条 → 阈值取 5% = 1000
        let plan = plan_stale((0..900).map(|i| format!("D:/a/{i}.pdf")).collect(), &set(&[]), &[], 20_000);
        assert_eq!(plan.limit, 1000);
        assert!(!plan.refused);
    }

    #[test]
    fn 文件回来时清掉待确认() {
        let plan = plan_stale(vec![], &set(&["D:/a/1.pdf"]), &[], 10);
        assert_eq!(plan.revived, vec!["D:/a/1.pdf".to_string()]);
        assert!(plan.pending.is_empty());
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_root(tag: &str) -> PathBuf {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let d = std::env::temp_dir().join(format!("tp_refresh_{}_{}_{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ---------- 纯决策层 plan_reconcile（必修 4.2）----------
    fn recs(rows: &[(i64, &str)]) -> Vec<(i64, String)> {
        rows.iter().map(|(i, p)| (*i, p.to_string())).collect()
    }

    /// 父目录可列举 ⇒ 正常列入删除（同目录存活行多，比例远低于阈值）。
    #[test]
    fn plan_deletes_when_parent_listable() {
        let mut rows: Vec<(i64, String)> = (0..100).map(|i| (i, format!("D:/a/keep{i}.txt"))).collect();
        rows.push((900, "D:/a/gone1.txt".into()));
        rows.push((901, "D:/a/gone2.txt".into()));
        let plan = plan_reconcile(&rows, |p| if p.contains("gone") { Probe::Missing } else { Probe::Exists }, false);
        assert_eq!(plan.to_delete.len(), 2);
        assert!(plan.skipped.is_empty());
        assert!(!plan.needs_confirm, "2/102 ≈ 2% 低于阈值，不该要求确认");
        assert_eq!(plan.dirs[0].dir, "D:/a");
        assert_eq!(plan.dirs[0].missing, 2);
    }

    /// 父目录不可列举 ⇒ 该目录全部跳过（必要条件）；confirmed=true 也不会删它们。
    #[test]
    fn plan_skips_whole_dir_when_parent_unreachable() {
        let r = recs(&[(1, "D:/pdf/1.pdf"), (2, "D:/pdf/2.pdf"), (3, "D:/ok/3.txt")]);
        let plan = plan_reconcile(
            &r,
            |p| {
                if p.starts_with("D:/pdf/") {
                    Probe::ParentUnreachable("拒绝访问".into())
                } else {
                    Probe::Missing
                }
            },
            true,
        );
        assert_eq!(plan.skipped.len(), 2, "不可达目录下全部跳过");
        assert_eq!(plan.to_delete.len(), 1, "其它目录照常（确认后）");
        assert_eq!(plan.missing, 3);
        assert_eq!(plan.unreachable, vec![("D:/pdf".to_string(), "拒绝访问".to_string(), 2)]);
        assert!(plan.to_delete.iter().all(|(_, p)| !p.starts_with("D:/pdf/")), "不可达目录的行绝不能进删除列表");
    }

    /// 比例超阈值 ⇒ 返回待确认且**不删除**；confirmed=true ⇒ 正常列入删除。
    #[test]
    fn plan_needs_confirm_over_threshold() {
        // 1,000 行里删 300 行 = 30% > 1%（且 300 > 200 绝对值下限）
        let mut rows: Vec<(i64, String)> = (0..700).map(|i| (i, format!("D:/a/{i}.txt"))).collect();
        rows.extend((700..1000).map(|i| (i, format!("D:/a/gone{i}.txt"))));
        let plan = plan_reconcile(&rows, |p| if p.contains("gone") { Probe::Missing } else { Probe::Exists }, false);
        assert!(plan.needs_confirm);
        assert!(plan.to_delete.is_empty(), "需要确认时一条都不能删");
        assert_eq!(plan.missing, 300);
        let plan2 = plan_reconcile(&rows, |p| if p.contains("gone") { Probe::Missing } else { Probe::Exists }, true);
        assert!(!plan2.needs_confirm);
        assert_eq!(plan2.to_delete.len(), 300);
    }

    /// 单目录 30% 阈值：总量很小但某目录整仓缺失 ⇒ 也要确认。
    #[test]
    fn plan_needs_confirm_by_dir_ratio() {
        let mut rows: Vec<(i64, String)> = (0..3).map(|i| (i, format!("D:/pdf/keep{i}.pdf"))).collect();
        rows.push((10, "D:/pdf/gone1.pdf".into()));
        rows.push((11, "D:/pdf/gone2.pdf".into()));
        // D:/pdf: missing=2, present=3 ⇒ 40% ≥ 30%
        let plan = plan_reconcile(&rows, |p| if p.contains("gone") { Probe::Missing } else { Probe::Exists }, false);
        assert!(plan.needs_confirm);
        assert!(plan.to_delete.is_empty());
        // 同规模但缺失比例低于 30% ⇒ 不要求确认
        let mut rows2: Vec<(i64, String)> = (0..10).map(|i| (i, format!("D:/pdf/keep{i}.pdf"))).collect();
        rows2.push((10, "D:/pdf/gone1.pdf".into()));
        let plan2 = plan_reconcile(&rows2, |p| if p.contains("gone") { Probe::Missing } else { Probe::Exists }, false);
        assert!(!plan2.needs_confirm, "1/11 ≈ 9% < 30%");
        assert_eq!(plan2.to_delete.len(), 1);
    }

    /// 删除失败必须被计数并回传错误文本（不中断）。
    #[test]
    fn delete_rows_reports_failure() {
        let root = tmp_root("del_fail");
        let p = root.join("t.db");
        let conn = db::init(&p).unwrap();
        db::upsert_book(&conn, "D:/x/1.txt", "1", "txt", 1, 0).unwrap();
        // 只读连接：DELETE 必然失败
        let ro = rusqlite::Connection::open_with_flags(
            &p,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let (ok, failed, err) = delete_rows(&ro, &[(1, "D:/x/1.txt".to_string())]);
        assert_eq!((ok, failed), (0, 1));
        assert!(err.is_some(), "失败必须带上错误文本");
        // 正常连接：成功
        let (ok2, failed2, _) = delete_rows(&conn, &[(1, "D:/x/1.txt".to_string())]);
        assert_eq!((ok2, failed2), (1, 0));
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

        // R11-2：include_total=false 跳过 COUNT（total=-1 哨兵），行数据与完整版逐行一致。
        let full = db::list_books_page(&conn, "", "recent", 0, 3, &None, &None, true, &None);
        let no_count =
            db::list_books_page_opts(&conn, "", "recent", 0, 3, &None, &None, true, &None, false);
        assert_eq!(no_count.total, -1, "未计数时 total 必须为哨兵 -1");
        assert_eq!(
            no_count.rows.iter().map(|b| b.id).collect::<Vec<_>>(),
            full.rows.iter().map(|b| b.id).collect::<Vec<_>>(),
            "行数据与完整版一致"
        );
        // 过滤 + 搜索场景同样成立。
        let q0 = db::list_books_page_opts(
            &conn, "epsilon", "title", 0, 10, &None, &None, false, &None, false,
        );
        assert_eq!((q0.total, q0.rows.len()), (-1, 1));

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
    ///   $env:TP_MANGA_FALSE_FIXTURE="<path.epub>"; cargo test manga_probe_against_python_fixture -- --nocapture
    #[test]
    fn manga_probe_against_python_fixture() {
        let Ok(path) = std::env::var("TP_MANGA_FALSE_FIXTURE") else {
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

    /// 图片页书判定（第五轮）：按「平均每图文字数」识别图片页型书，独立于漫画判定。
    #[test]
    fn image_page_like_thresholds() {
        // 目标书：3,659 图 / 33,059 字 ≈ 9.0 字/图 → true。
        assert!(is_image_page_like(3659, 33059, true, 3), "图片页书应命中");
        // 插图小说：正文远高于 20 字/图 → false。
        assert!(!is_image_page_like(120, 300_000, true, 3), "插图小说不得判图片页书");
        // 图片数门槛 100。
        assert!(!is_image_page_like(99, 100, true, 1), "图片数低于 100 不判");
        // 图片数不得少于 spine 文档数。
        assert!(!is_image_page_like(100, 100, true, 101), "图片数少于 spine 数不判");
        // 边界：恰好 images*20 命中，多 1 即否。
        assert!(is_image_page_like(100, 2000, true, 1), "text == images*20 应命中");
        assert!(!is_image_page_like(100, 2001, true, 1), "text 超 images*20 不命中");
        // 扫描不完整（预算截断/提前退出）一律不判，保守兜底。
        assert!(!is_image_page_like(3659, 33059, false, 3), "扫描不完整不得判");
        // 空 spine 时 max(1) 兜底：100 图 >= 1。
        assert!(is_image_page_like(100, 0, true, 0));
    }

    /// R6-1 门控：P11 锚点切分绝对优先；只有折叠目录 + 单文档巨量图片 + 图片主导度达标才
    /// 自动块级切章；有划线且未迁移时保持旧行为。
    #[test]
    fn auto_block_split_gate() {
        // 目标无锚点样本结构：1 个 legacy 目录项、单文档 2042 图、图片主导 → true。
        assert!(should_auto_block_split(false, 1, 2042, true, false, false));
        // P11 已切分（doc_split=true）绝对优先 → false。
        assert!(!should_auto_block_split(true, 1, 2042, true, false, false));
        // 多章漫画/普通书（legacy 目录多）→ false。
        assert!(!should_auto_block_split(false, 20, 5000, true, false, false));
        assert!(!should_auto_block_split(false, 3, 2042, true, false, false));
        // 单文档图片不够多 → false。
        assert!(!should_auto_block_split(false, 1, 200, true, false, false));
        // 图片主导度不达标（带大量正文）→ false。
        assert!(!should_auto_block_split(false, 1, 2042, false, false, false));
        // 有划线且未迁移 → false；已迁移（之前已切过章）→ true（保持章号一致）。
        assert!(!should_auto_block_split(false, 1, 2042, true, true, false));
        assert!(should_auto_block_split(false, 1, 2042, true, true, true));
    }

    /// R6-B 迁移标记动作：完全匹配 no-op；旧格式同 stamp v1 只重写；同 stamp 布局变化
    /// 做布局升级；无标记/文件替换走 legacy 迁移。
    #[test]
    fn split_marker_action_branches() {
        assert_eq!(split_marker_action(None, "100-200", "v1"), MarkerAction::LegacyMigrate);
        // 旧格式（只有指纹）+ 同 stamp + 当前 v1：只重写标记。
        assert_eq!(
            split_marker_action(Some("100-200"), "100-200", "v1"),
            MarkerAction::Rewrite
        );
        // 旧格式 + 同 stamp + 当前 v2（R6-B 细分）：布局升级。
        assert_eq!(
            split_marker_action(Some("100-200"), "100-200", "v2"),
            MarkerAction::LayoutUpgrade
        );
        // 新格式完全匹配：no-op（进程重启后重复打开）。
        assert_eq!(
            split_marker_action(Some("100-200|v1"), "100-200", "v1"),
            MarkerAction::Noop
        );
        assert_eq!(
            split_marker_action(Some("100-200|v2"), "100-200", "v2"),
            MarkerAction::Noop
        );
        // 同 stamp 但布局签名不同（v1→v2）：布局升级。
        assert_eq!(
            split_marker_action(Some("100-200|v1"), "100-200", "v2"),
            MarkerAction::LayoutUpgrade
        );
        // 文件替换（stamp 不同）：legacy 迁移。
        assert_eq!(
            split_marker_action(Some("100-200"), "101-201", "v1"),
            MarkerAction::LegacyMigrate
        );
        assert_eq!(
            split_marker_action(Some("100-200|v1"), "101-201", "v2"),
            MarkerAction::LegacyMigrate
        );
        // R7-1：v3（泛化细分/新块级布局）分支。
        assert_eq!(
            split_marker_action(Some("100-200|v3"), "100-200", "v3"),
            MarkerAction::Noop
        );
        assert_eq!(
            split_marker_action(Some("100-200"), "100-200", "v3"),
            MarkerAction::LayoutUpgrade,
            "旧格式同 stamp + 当前 v3 → 布局升级"
        );
        assert_eq!(
            split_marker_action(Some("100-200|v2"), "100-200", "v3"),
            MarkerAction::LayoutUpgrade,
            "v2→v3 → 布局升级"
        );
        assert_eq!(
            split_marker_action(Some("100-200|v1"), "100-200", "v3"),
            MarkerAction::LayoutUpgrade,
            "v1→v3 → 布局升级"
        );
        assert_eq!(
            split_marker_action(Some("100-200|v3"), "101-201", "v3"),
            MarkerAction::LegacyMigrate,
            "文件替换 → legacy 迁移"
        );
    }

    /// R7-1 布局目标选择：无划线优先 v3；有划线按现有签名恢复既有布局；无法重建保持 P11。
    #[test]
    fn choose_split_layout_targets() {
        // 无划线：General 可用就升 v3。
        assert_eq!(
            choose_split_layout(false, Some("v2"), true, true),
            SplitLayoutTarget::General
        );
        assert_eq!(
            choose_split_layout(false, Some("v1"), true, false),
            SplitLayoutTarget::General
        );
        // 无划线：General 不可用、现有 v2 → 用受限细分保持 v2，避免降级。
        assert_eq!(
            choose_split_layout(false, Some("v2"), false, true),
            SplitLayoutTarget::Restricted
        );
        // 无划线：General 不可用、现有 v1/无 marker → 保持 P11。
        assert_eq!(
            choose_split_layout(false, Some("v1"), false, false),
            SplitLayoutTarget::KeepP11
        );
        assert_eq!(choose_split_layout(false, None, false, true), SplitLayoutTarget::KeepP11);
        // 有划线：不改变既有布局。
        assert_eq!(
            choose_split_layout(true, Some("v3"), true, false),
            SplitLayoutTarget::General
        );
        assert_eq!(
            choose_split_layout(true, Some("v3"), false, false),
            SplitLayoutTarget::KeepP11,
            "v3 恢复失败保持 P11"
        );
        assert_eq!(
            choose_split_layout(true, Some("v2"), true, true),
            SplitLayoutTarget::Restricted,
            "有划线 v2 只按 v2 恢复，不升 v3"
        );
        assert_eq!(
            choose_split_layout(true, Some("v2"), false, false),
            SplitLayoutTarget::KeepP11
        );
        assert_eq!(
            choose_split_layout(true, Some("v1"), true, true),
            SplitLayoutTarget::KeepP11
        );
        assert_eq!(choose_split_layout(true, None, true, true), SplitLayoutTarget::KeepP11);
    }

    /// R12-2：格式缓存 miss→hit、epoch 变化重算、查询期间 epoch 变化不写旧值。
    #[test]
    fn formats_cache_miss_hit_and_epoch_guard() {
        let cache: std::sync::Mutex<Option<(u64, Vec<String>)>> = std::sync::Mutex::new(None);
        assert_eq!(formats_cache_get(&cache, 1), None, "冷缓存 miss");
        assert!(formats_cache_put(&cache, 1, 1, &["epub".into(), "pdf".into()]));
        assert_eq!(
            formats_cache_get(&cache, 1),
            Some(vec!["epub".to_string(), "pdf".to_string()]),
            "同 epoch 命中"
        );
        assert_eq!(formats_cache_get(&cache, 2), None, "epoch 变化必须重算");
        // 查询期间 epoch 已变：不得写回旧值。
        assert!(!formats_cache_put(&cache, 3, 4, &["txt".into()]));
        assert_eq!(formats_cache_get(&cache, 4), None);
        assert!(formats_cache_put(&cache, 4, 4, &["txt".into()]));
        assert_eq!(formats_cache_get(&cache, 4), Some(vec!["txt".to_string()]));
    }

    /// B2：FTS 空闲 optimize 触发条件（无 marker/超期/体积/行数变化/暂停/索引中）。
    #[test]
    fn fts_idle_optimize_trigger_conditions() {
        let big = 300 * 1024 * 1024i64;
        let small = 64 * 1024 * 1024i64;
        let now = 1_700_000_000i64;
        let day = 24 * 3600i64;
        // 无 marker：体积超阈值才执行一次；体积不足不执行。
        assert!(db::should_optimize_fts(now, 0, 0, 0, big, 100, false, false));
        assert!(!db::should_optimize_fts(now, 0, 0, 0, small, 100, false, false));
        // 暂停/运行中一律不执行。
        assert!(!db::should_optimize_fts(now, 0, 0, 0, big, 100, true, false));
        assert!(!db::should_optimize_fts(now, 0, 0, 0, big, 100, false, true));
        // 未超期、体积/行数变化都很小 → 不执行。
        assert!(!db::should_optimize_fts(
            now,
            now - day,
            big,
            1000,
            big + 1024,
            1000,
            false,
            false
        ));
        // 超过 7 天 → 执行。
        assert!(db::should_optimize_fts(
            now,
            now - 8 * day,
            big,
            1000,
            big,
            1000,
            false,
            false
        ));
        // 行数变化 >2%（体积不变）→ 执行；2% 以内不执行。
        assert!(db::should_optimize_fts(
            now,
            now - day,
            big,
            1000,
            big,
            1030,
            false,
            false
        ));
        assert!(!db::should_optimize_fts(
            now,
            now - day,
            big,
            1000,
            big,
            1010,
            false,
            false
        ));
        // 体积变化 >10% → 执行。
        assert!(db::should_optimize_fts(
            now,
            now - day,
            big,
            1000,
            big + big / 5,
            1000,
            false,
            false
        ));
    }

    /// B3：VACUUM 磁盘余量判定（保守：DB 大小 + 64MB 以上才允许；取不到余量拒绝）。
    #[test]
    fn vacuum_space_guard() {
        let db = 2_000_000_000u64;
        let margin = 64 * 1024 * 1024u64;
        assert!(!vacuum_space_ok(None, db));
        assert!(!vacuum_space_ok(Some(db + margin), db));
        assert!(vacuum_space_ok(Some(db + margin + 1), db));
        assert!(vacuum_space_ok(Some(5_000_000_000), db));
        assert!(!vacuum_space_ok(Some(100), 0));
    }


    /// B10：VACUUM 预检（同盘只查一次；异盘两处都要够；取不到拒绝；边界严格）。
    #[test]
    fn vacuum_precheck_same_and_diff_disk() {
        let db = 1024 * 1024 * 1024u64;
        let req = db + 64 * 1024 * 1024;
        assert!(vacuum_precheck(Some(req + 1), None, true, db).is_ok());
        assert!(vacuum_precheck(Some(req + 1), Some(req + 1), false, db).is_ok());
        assert!(vacuum_precheck(Some(req + 1), None, false, db).is_err(), "异盘临时空间未知应拒绝");
        assert!(vacuum_precheck(Some(req + 1), Some(10), false, db).is_err(), "异盘临时不足应拒绝");
        assert!(vacuum_precheck(None, Some(req + 1), true, db).is_err(), "取不到拒绝");
        assert!(vacuum_precheck(Some(req), Some(req + 1), true, db).is_err(), "边界必须严格大于");
    }

    /// B10：维护历史写入/倒序/最多 5 条。
    #[test]
    fn maintenance_history_truncates_to_five() {
        let root = tmp_root("b10-history");
        let conn = db::init(&root.join("t.db")).unwrap();
        for i in 0..7 {
            push_maintenance_record(&conn, MaintenanceRecord {
                at: i, kind: "vacuum".into(), result: "success".into(), error: String::new(),
                before_db: 10, before_wal: 1, after_db: 5, after_wal: 1, freed: 5,
                duration_ms: 1, checkpoint_busy: false, backup_path: String::new(),
            });
        }
        let raw = db::get_setting(&conn, "maintenance.history.v1").unwrap();
        let list: Vec<MaintenanceRecord> = serde_json::from_str(&raw).unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].at, 6, "最新在前");
        // 自动整理跳过/成功（含备份路径）可持久化。
        push_maintenance_record(&conn, MaintenanceRecord {
            at: 10, kind: "vacuum-auto".into(), result: "skipped".into(), error: "备份失败".into(),
            before_db: 10, before_wal: 1, after_db: 10, after_wal: 1, freed: 0,
            duration_ms: 0, checkpoint_busy: true, backup_path: String::new(),
        });
        push_maintenance_record(&conn, MaintenanceRecord {
            at: 11, kind: "vacuum-auto".into(), result: "success".into(), error: String::new(),
            before_db: 10, before_wal: 1, after_db: 5, after_wal: 1, freed: 5,
            duration_ms: 2, checkpoint_busy: false, backup_path: "backups/auto-vacuum-1.json".into(),
        });
        let raw2 = db::get_setting(&conn, "maintenance.history.v1").unwrap();
        let list2: Vec<MaintenanceRecord> = serde_json::from_str(&raw2).unwrap();
        assert_eq!(list2[0].result, "success");
        assert_eq!(list2[0].backup_path, "backups/auto-vacuum-1.json");
        assert_eq!(list2[1].result, "skipped");
        assert_eq!(list2[1].error, "备份失败");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// D3：标签迁移幂等、CRUD/归一、过滤同源、删书清理、刷新保留。

    /// D2：统计区间/日/小时/上期/完成/引用与空数据。


    /// A2：folder 生效时强制排序索引，结果顺序与无索引查询逐条一致；EXPLAIN 命中排序索引。
    #[test]
    fn folder_sort_index_equivalence_and_plan() {
        let root = tmp_root("a2-folder");
        let conn = db::init(&root.join("t.db")).unwrap();
        std::fs::create_dir_all(root.join("dir").join("sub")).unwrap();
        for i in 0..40 {
            let dir = root.join("dir");
            let p = if i % 2 == 0 { dir.join(format!("b{i}.txt")) } else { dir.join("sub").join(format!("b{i}.txt")) };
            std::fs::write(&p, b"x").unwrap();
            db::upsert_book(&conn, &p.to_string_lossy(), &format!("t{:02}", (i * 7) % 40), "txt", (i * 13) as i64, i as i64).unwrap();
        }
        let folder = Some(root.join("dir").to_string_lossy().into_owned());
        // A2：count 不得强制排序索引（否则退化为 SCAN 排序索引）。
        let base = root.join("dir").to_string_lossy().replace('/', "\\") + "\\";
        let hi = format!("{base}\u{10FFFF}");
        let count_plan: Vec<String> = conn
            .prepare("EXPLAIN QUERY PLAN SELECT COUNT(*) FROM books b WHERE b.path>=?1 AND b.path<?2")
            .unwrap()
            .query_map([&base, &hi], |r| r.get::<_, String>(3))
            .unwrap()
            .flatten()
            .collect();
        assert!(!count_plan.join("|").contains("idx_books_added_id"), "count 不应强制排序索引: {count_plan:?}");
        for sort in ["added", "lastread", "title", "recent", "size"] {
            for desc in [true, false] {
                let order = db::order_by_clause(sort, desc);
                let base = root.join("dir").to_string_lossy().replace('/', "\\");
                let hi = format!("{base}􏿿");
                let expect: Vec<i64> = conn
                    .prepare(&format!("SELECT b.id FROM books b WHERE b.path>=?1 AND b.path<?2 ORDER BY {order}"))
                    .unwrap()
                    .query_map([&base, &hi], |r| r.get(0))
                    .unwrap()
                    .flatten()
                    .collect();
                let p = db::list_books_page_opts_tags(&conn, "", sort, 0, 500, &None, &None, desc, &folder, true, &None);
                let got: Vec<i64> = p.rows.iter().map(|b| b.id).collect();
                let (frag, _) = db::build_book_filter(&None, &None, &Some(folder.clone().unwrap_or_default()), &None);
                assert_eq!(got, expect, "sort={sort} desc={desc} got={got:?} expect={expect:?} base={base:?} frag={frag} total={}", p.total);
                let plan: Vec<String> = conn
                    .prepare(&format!("EXPLAIN QUERY PLAN SELECT b.id FROM books b INDEXED BY {} WHERE b.path>=?1 AND b.path<?2 ORDER BY {order}", db::sort_index_name(sort)))
                    .unwrap()
                    .query_map([&base, &hi], |r| r.get::<_, String>(3))
                    .unwrap()
                    .flatten()
                    .collect();
                assert!(plan.iter().any(|x| x.contains("USING INDEX") || x.contains("INDEX")), "plan: {plan:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a1_indexes_exist_and_day_plan() {
        let root = tmp_root("idx-a1");
        let conn = db::init(&root.join("t.db")).unwrap();
        let _ = db::init(&root.join("t.db")).unwrap();
        for name in ["idx_sessions_day", "idx_sessions_started"] {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1", [name], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "{name} 应存在");
        }
        let plan: Vec<String> = conn
            .prepare("EXPLAIN QUERY PLAN SELECT day, SUM(seconds) FROM reading_sessions WHERE day>=?1 AND day<=?2 GROUP BY day")
            .unwrap()
            .query_map(["2026-01-01", "2026-01-31"], |r| r.get::<_, String>(3))
            .unwrap()
            .flatten()
            .collect();
        let joined = plan.join(" | ");
        assert!(joined.contains("idx_sessions_day") || joined.contains("USING INDEX"), "day 查询应走索引: {joined}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reading_stats_intervals_and_empty() {
        let root = tmp_root("stats2");
        let conn = db::init(&root.join("t.db")).unwrap();
        let p = root.join("a.epub");
        std::fs::write(&p, b"x").unwrap();
        let id = db::upsert_book(&conn, &p.to_string_lossy(), "书", "epub", 10, 1).unwrap();
        conn.execute("UPDATE books SET read_pct=100, last_read=5 WHERE id=?1", [id]).unwrap();
        // 2026-01-05 10:00 本地时间的毫秒（用 SQLite 反推，避免时区假设）。
        let ts: i64 = conn.query_row("SELECT CAST(strftime('%s','2026-01-05 10:00:00','utc') AS INTEGER)*1000", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO reading_sessions(book_id,day,started_at,seconds,chars) VALUES(?1,'2026-01-05',?2,1800,900)", rusqlite::params![id, ts]).unwrap();
        conn.execute("INSERT INTO reading_sessions(book_id,day,started_at,seconds,chars) VALUES(?1,'2026-01-06',0,600,300)", [id]).unwrap();
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,0,'c',1,2,'q','underline','','n',1)", [id]).unwrap();
        let s = db::get_reading_stats(&conn, "2026-01-05", "2026-01-06", "2026-01-01", "2026-01-04", "2026-01-01", "2026-01-06");
        assert_eq!((s.total_seconds, s.total_chars, s.session_count, s.active_days), (2400, 1200, 2, 2));
        assert_eq!(s.prev_seconds, 0);
        assert_eq!(s.daily.len(), 2);
        assert_eq!(s.heatmap.len(), 2);
        assert_eq!(s.active_dates, vec!["2026-01-05".to_string(), "2026-01-06".to_string()]);
        assert_eq!(s.hourly.iter().map(|h| h.seconds).sum::<i64>(), 2400);
        assert_eq!((s.finished_books, s.reading_books, s.highlights_total, s.notes_total), (1, 0, 1, 1));
        assert!(s.finished_books == 1 && s.quote.is_some() && s.composition.len() == 1);
        assert_eq!(s.completed_books.len(), 1);
        assert!(s.current_book.is_none(), "已读完/无 progress 不应算当前在读");
        db::set_progress(&conn, id, &crate::models::Progress {
            chapter_index: 0, chapter_ratio: 0.0, pct: 40.0, updated_at: 9, device: String::new(),
        }).unwrap();
        let cur = db::get_reading_stats(&conn, "2026-01-01", "2026-01-31", "2025-12-01", "2025-12-31", "2025-10-01", "2026-01-31");
        assert_eq!(cur.current_book.as_ref().map(|b| b.id), Some(id));
        assert!((cur.current_book.as_ref().unwrap().pct - 40.0).abs() < 1e-6);
        let empty = db::get_reading_stats(&conn, "2030-01-01", "2030-01-02", "2030-01-01", "2030-01-01", "2030-01-01", "2030-01-02");
        assert_eq!((empty.total_seconds, empty.daily.len(), empty.hourly.len()), (0, 0, 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tags_crud_filter_and_cleanup() {
        let root = tmp_root("tags");
        let db_path = root.join("t.db");
        let mut conn = db::init(&db_path).unwrap();
        let _ = db::init(&db_path).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('tags','book_tags','idx_book_tags_tag')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 3);

        let sp = |p: &Path| p.to_string_lossy().into_owned();
        let p1 = root.join("a.txt");
        let p2 = root.join("b.txt");
        std::fs::write(&p1, b"x").unwrap();
        std::fs::write(&p2, b"y").unwrap();
        let b1 = db::upsert_book(&conn, &sp(&p1), "A", "txt", 1, 1).unwrap();
        let b2 = db::upsert_book(&conn, &sp(&p2), "B", "txt", 1, 2).unwrap();

        let (t1, n1) = db::create_tag(&conn, "  Work   Tag ").unwrap();
        assert_eq!(n1, "Work Tag");
        let (t1b, _) = db::create_tag(&conn, "work tag").unwrap();
        assert_eq!(t1, t1b);
        assert!(db::create_tag(&conn, "   ").is_err());
        assert!(db::create_tag(&conn, &"字".repeat(33)).is_err());
        let (t2, _) = db::create_tag(&conn, "Shelf").unwrap();
        assert!(db::rename_tag(&conn, t2, "WORK TAG").is_err());
        db::rename_tag(&conn, t2, "Shelf 2").unwrap();

        db::set_book_tags(&mut conn, b1, &[t1, t1, t2]).unwrap();
        db::set_book_tags(&mut conn, b2, &[t1]).unwrap();
        assert_eq!(db::list_book_tags(&conn, b1).len(), 2);
        let all = db::list_tags(&conn);
        assert_eq!(all.iter().find(|(id, _, _)| *id == t1).unwrap().2, 2);

        let both = db::list_books_page_opts_tags(
            &conn, "", "title", 0, 10, &None, &None, false, &None, true,
            &Some(vec!["work tag".into(), "shelf 2".into()]),
        );
        assert_eq!((both.total, both.rows.len()), (1, 1));
        let one = db::list_books_page_opts_tags(
            &conn, "", "title", 0, 10, &None, &None, false, &None, true,
            &Some(vec!["work tag".into()]),
        );
        assert_eq!(one.total, 2);
        let ids = db::order_index_ids_tags(
            &conn, "title", "", 10, &None, &None, false, &None,
            &Some(vec!["shelf 2".into()]),
        )
        .unwrap();
        assert_eq!(ids, vec![b1]);
        let rnd = db::random_book_filtered_tags(
            &conn, "", "title", &None, &None, &None, &Some(vec!["shelf 2".into()]),
        );
        assert_eq!(rnd.map(|b| b.id), Some(b1));

        // 不存在标签：多选任一缺失 → 空集合，total=0/order 空/random 无。
        let missing = db::list_books_page_opts_tags(
            &conn, "", "title", 0, 10, &None, &None, false, &None, true,
            &Some(vec!["work tag".into(), "no-such-tag".into()]),
        );
        assert_eq!((missing.total, missing.rows.len()), (0, 0));
        assert!(db::order_index_ids_tags(
            &conn, "title", "", 10, &None, &None, false, &None,
            &Some(vec!["no-such-tag".into()]),
        )
        .unwrap()
        .is_empty());
        assert!(db::random_book_filtered_tags(
            &conn, "", "title", &None, &None, &None,
            &Some(vec!["no-such-tag".into()]),
        )
        .is_none());

        // 粗略性能对比（debug 口径，非基准）：集合式单标签/多标签/不存在标签。
        let tx = conn.transaction().unwrap();
        for i in 0..2000 {
            let p = root.join(format!("bulk-{i}.txt"));
            db::upsert_book(&tx, &sp(&p), "Bulk", "txt", 10, i as i64).unwrap();
        }
        tx.commit().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO book_tags(book_id, tag_id) SELECT id, ?1 FROM books WHERE title='Bulk'",
            [t1],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO book_tags(book_id, tag_id) SELECT id, ?1 FROM books WHERE title='Bulk' AND id % 2 = 0",
            [t2],
        )
        .unwrap();
        let time = |label: &str, f: &dyn Fn() -> (i64, usize)| {
            let t0 = std::time::Instant::now();
            let (total, rows) = f();
            eprintln!("[tags {label}] {}ms total={total} rows={rows}", t0.elapsed().as_millis());
        };
        time("single-count-page", &|| {
            let p = db::list_books_page_opts_tags(
                &conn, "", "title", 0, 50, &None, &None, false, &None, true,
                &Some(vec!["work tag".into()]),
            );
            (p.total, p.rows.len())
        });
        time("multi-and-count-page", &|| {
            let p = db::list_books_page_opts_tags(
                &conn, "", "title", 0, 50, &None, &None, false, &None, true,
                &Some(vec!["work tag".into(), "shelf 2".into()]),
            );
            (p.total, p.rows.len())
        });
        time("multi-missing-count-page", &|| {
            let p = db::list_books_page_opts_tags(
                &conn, "", "title", 0, 50, &None, &None, false, &None, true,
                &Some(vec!["work tag".into(), "no-such-tag".into()]),
            );
            (p.total, p.rows.len())
        });
        time("order-index-multi", &|| {
            let ids = db::order_index_ids_tags(
                &conn, "title", "", 10_000, &None, &None, false, &None,
                &Some(vec!["work tag".into(), "shelf 2".into()]),
            )
            .unwrap();
            (ids.len() as i64, ids.len())
        });
        // EXPLAIN QUERY PLAN：仅打印并宽松断言命中标签表（不绑定具体索引名）。
        let (frag, fp) = db::build_book_filter(&None, &None, &None, &Some(vec!["work tag".into(), "shelf 2".into()]));
        let sql = format!("EXPLAIN QUERY PLAN SELECT b.id FROM books b WHERE {frag}");
        let mut stmt = conn.prepare(&sql).unwrap();
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(fp.iter()), |r| r.get::<_, String>(3))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        eprintln!("[tags plan] {}", plan.join(" | "));
        assert!(plan.iter().any(|x| x.contains("book_tags") || x.contains("tags")), "plan 应使用标签表: {plan:?}");

        db::upsert_book(&conn, &sp(&p1), "A2", "txt", 3, 3).unwrap();
        assert_eq!(db::list_book_tags(&conn, b1).len(), 2);
        db::delete_books(&conn, &[sp(&p1)]).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM book_tags WHERE book_id=?1", [b1], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        let _ = std::fs::remove_dir_all(&root);
    }


    /// C1：EPUB 缓存按字节权重/条目数淘汰，当前书不被淘汰。
    #[test]
    fn epub_cache_weight_eviction() {
        let mb = 1024 * 1024usize;
        let sizes = [(1, 600 * mb), (2, 600 * mb), (3, 600 * mb)];
        let order = [1, 2, 3];
        assert_eq!(plan_epub_evictions(&sizes, &order, 3, 1536 * mb, 4), vec![1]);
        let ev = plan_epub_evictions(&sizes, &order, 1, 1000 * mb, 4);
        assert!(ev.contains(&2) && ev.contains(&3) && !ev.contains(&1));
        let sizes4 = [(1, 10), (2, 10), (3, 10), (4, 10), (9, 10)];
        let ev4 = plan_epub_evictions(&sizes4, &[1, 2, 3, 4, 9], 9, 10 * mb, 4);
        assert_eq!(ev4, vec![1]);
        assert!(plan_epub_evictions(&[(7, 5000 * mb)], &[7], 7, 1536 * mb, 4).is_empty());
        assert_eq!(
            plan_epub_evictions(&sizes, &[8, 1, 2, 3], 3, 1000 * mb, 4),
            vec![1, 2]
        );
        // 命中缓存后 order 更新：1 变成最新，后续淘汰不会先淘汰它。
        let mut order_hit = vec![1, 2, 3];
        touch_epub_cache_order(&mut order_hit, 1);
        assert_eq!(order_hit, vec![2, 3, 1]);
        assert_eq!(
            plan_epub_evictions(&sizes, &order_hit, 3, 1200 * mb, 4),
            vec![2],
            "命中过的 1 不应被当作最旧淘汰"
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

    /// 目录里是否有本次写回残留的临时文件（epub::write_meta 用 .tidepage-tmp-{pid}）。
    fn tmp_leftovers(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tidepage-tmp"))
            .collect()
    }

    /// 缺 dc:title 的 EPUB：write_meta 必须失败且原文件字节不变。
    fn write_min_epub_no_title(path: &std::path::Path) {
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
        zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator id="c">Old Author</dc:creator></metadata><manifest/><spine/></package>"#).unwrap();
        zw.finish().unwrap();
    }

    #[test]
    fn write_meta_overwrites_title_and_author() {
        let dir = std::env::temp_dir().join(format!("tp_epub_meta_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("b.epub");
        write_min_epub(&p);
        let ps = p.to_string_lossy().into_owned();
        crate::epub::write_meta(&ps, "新标题 & <tag>", "新作者").unwrap();
        let data = std::fs::read(&p).unwrap();
        let doc = crate::epub::parse(&data).unwrap();
        assert_eq!(doc.title, "新标题 & <tag>");
        assert_eq!(doc.author, "新作者");
        // C1①：成功路径不得残留临时文件（同目录只应剩原 EPUB）。
        assert!(tmp_leftovers(&dir).is_empty(), "成功写回后不得残留临时文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// C1②：失败路径（OPF 缺 dc:title）返回 Err、原文件字节完全一致、无临时文件残留。
    #[test]
    fn write_meta_failure_keeps_original_bytes_and_cleans_tmp() {
        let dir = std::env::temp_dir().join(format!("tp_epub_meta_fail_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.epub");
        write_min_epub_no_title(&p);
        let before = std::fs::read(&p).unwrap();
        let ps = p.to_string_lossy().into_owned();
        let err = crate::epub::write_meta(&ps, "新标题", "新作者").unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(
            std::fs::read(&p).unwrap(),
            before,
            "失败时原文件字节必须完全不变"
        );
        assert!(tmp_leftovers(&dir).is_empty(), "失败路径必须清理临时文件");
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
        let dir = std::env::temp_dir().join(format!("tp_stamp_{}", std::process::id()));
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

#[cfg(test)]
mod cache_and_batch_tests {
    //! 第一批性能优化引入的三个纯函数判据：
    //! 1) 解析缓存命中（规则版本 + 文件指纹）—— MOBI/TXT 共用；
    //! 2) FTS 一致性（行数 vs 期望章数）—— 决定要不要重建；
    //! 3) 建索引分批切分（章数 / 正文字节上限）。
    use super::*;

    #[test]
    fn cache_entry_valid_requires_same_ver_and_stamps() {
        let a = Some((10, 20));
        let b = Some((10, 20));
        assert!(cache_entry_valid(3, 3, a, b), "同版本同指纹应命中");
        assert!(!cache_entry_valid(2, 3, a, b), "规则版本不同必须失效");
        assert!(!cache_entry_valid(3, 3, None, b), "缓存侧没指纹不得复用");
        assert!(!cache_entry_valid(3, 3, a, None), "当前取不到指纹不得复用");
        assert!(!cache_entry_valid(3, 3, Some((11, 20)), b), "大小不同必须失效");
        assert!(!cache_entry_valid(3, 3, Some((10, 21)), b), "mtime 不同必须失效");
    }

    #[test]
    fn fts_needs_rebuild_compares_rows_with_expected() {
        assert!(!fts_needs_rebuild(0, 0), "空书行数一致，不重建");
        assert!(!fts_needs_rebuild(120, 120), "已索引完成，不重建");
        assert!(fts_needs_rebuild(0, 120), "从未建过要重建");
        assert!(fts_needs_rebuild(60, 120), "中断/崩溃留下的部分行要重建");
        assert!(fts_needs_rebuild(121, 120), "历史重复多出的行要重建");
    }

    fn ch(i: i64, body_bytes: usize) -> (i64, String, String) {
        (i, format!("第{i}章"), "x".repeat(body_bytes))
    }

    #[test]
    fn plan_fts_batches_handles_empty_single_and_oversized() {
        // 空书：没有批次（不写任何行）。
        assert!(plan_fts_batches(&[]).is_empty());

        // 少量章节：单批。
        let small: Vec<_> = (0..3).map(|i| ch(i, 10)).collect();
        assert_eq!(plan_fts_batches(&small), vec![(0usize, 3usize)]);

        // 超长章：独占一批（fts 行是一章一条，不能拆章）。
        let mut mixed = vec![ch(0, 10)];
        mixed.push(ch(1, FTS_BATCH_MAX_BYTES + 1));
        mixed.push(ch(2, 10));
        assert_eq!(
            plan_fts_batches(&mixed),
            vec![(0usize, 1usize), (1, 2), (2, 3)]
        );

        // 章数上限：41 章且每章很小 → 32 + 9 两批。
        let many: Vec<_> = (0..41).map(|i| ch(i, 10)).collect();
        assert_eq!(plan_fts_batches(&many), vec![(0usize, 32usize), (32, 41)]);

        // 字节上限：每章约 100KB，三章累计超 256KB → 每批两章。
        let per = 100 * 1024;
        let bytes_limited: Vec<_> = (0..5).map(|i| ch(i, per)).collect();
        assert_eq!(
            plan_fts_batches(&bytes_limited),
            vec![(0usize, 2usize), (2, 4), (4, 5)]
        );
    }

    #[test]
    fn auto_index_thresholds_are_inclusive() {
        // P9：自动索引的体积/章数上限，等于上限通过、超过拒绝。
        assert!(auto_index_size_allowed(0));
        assert!(auto_index_size_allowed(AUTO_INDEX_MAX_BYTES), "等于体积上限应通过");
        assert!(!auto_index_size_allowed(AUTO_INDEX_MAX_BYTES + 1), "超过体积上限应拒绝");
        assert!(auto_index_chapters_allowed(AUTO_INDEX_MAX_CHAPTERS), "等于章数上限应通过");
        assert!(!auto_index_chapters_allowed(AUTO_INDEX_MAX_CHAPTERS + 1), "超过章数上限应拒绝");
    }

    #[test]
    fn db_cache_stamp_usable_requires_equal_fingerprint() {
        // P7/P8 共用的 DB 缓存判据：文件指纹一致才复用。
        assert!(db_cache_stamp_usable(10, 20, Some((10, 20))), "指纹一致应复用");
        assert!(!db_cache_stamp_usable(10, 20, Some((11, 20))), "size 变了必须失效");
        assert!(!db_cache_stamp_usable(10, 20, Some((10, 21))), "mtime 变了必须失效");
        assert!(!db_cache_stamp_usable(10, 20, None), "取不到指纹不得复用");
    }

    #[test]
    fn epub_per_chapter_json_roundtrip() {
        // P8：逐章字数数组以 JSON 存表，反序列化后长度与内容必须完全一致。
        let per: Vec<i64> = vec![0, 1, 123, 4567, -1];
        let json = serde_json::to_string(&per).unwrap();
        let back: Vec<i64> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, per, "逐章字数数组 JSON 往返应完全一致");
        assert_eq!(back.len(), per.len());
    }

    #[test]
    fn fts_meta_plan_covers_meta_and_legacy_paths() {
        // P12：meta 存在时以 meta 为准；meta 缺失时用旧实例计数做一次性迁移。
        assert_eq!(fts_meta_plan(Some(5), None, 5), FtsMetaPlan::Skip);
        assert_eq!(fts_meta_plan(Some(5), None, 4), FtsMetaPlan::Rebuild);
        assert_eq!(fts_meta_plan(Some(0), None, 0), FtsMetaPlan::Skip, "空书 meta=0 视为已索引");
        assert_eq!(fts_meta_plan(None, Some(5), 5), FtsMetaPlan::Adopt(5));
        assert_eq!(fts_meta_plan(None, Some(0), 0), FtsMetaPlan::Adopt(0));
        assert_eq!(fts_meta_plan(None, Some(5), 6), FtsMetaPlan::Rebuild);
        assert_eq!(fts_meta_plan(None, None, 5), FtsMetaPlan::Rebuild);
    }

    #[test]
    fn single_flight_same_key_computes_once() {
        use std::sync::atomic::{AtomicUsize, Ordering as AOrd};
        let map: Mutex<HashMap<i64, Arc<Flight<i32>>>> = Mutex::new(HashMap::new());
        let calls = AtomicUsize::new(0);
        std::thread::scope(|s| {
            let a = s.spawn(|| {
                single_flight(&map, 7, || {
                    calls.fetch_add(1, AOrd::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    42
                })
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
            let b = s.spawn(|| {
                single_flight(&map, 7, || {
                    calls.fetch_add(1, AOrd::SeqCst);
                    99
                })
            });
            assert_eq!(a.join().unwrap(), 42);
            assert_eq!(b.join().unwrap(), 42, "follower 应复用 leader 的结果");
        });
        assert_eq!(calls.load(AOrd::SeqCst), 1, "同 key 只应计算一次");
    }

    #[test]
    fn single_flight_different_keys_run_in_parallel() {
        use std::sync::atomic::{AtomicBool, Ordering as AOrd};
        let map: Mutex<HashMap<i64, Arc<Flight<i32>>>> = Mutex::new(HashMap::new());
        let started = AtomicBool::new(false);
        std::thread::scope(|s| {
            let a = s.spawn(|| {
                single_flight(&map, 1, || {
                    started.store(true, AOrd::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    1
                })
            });
            while !started.load(AOrd::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let t0 = std::time::Instant::now();
            let v = single_flight(&map, 2, || 2i32);
            assert_eq!(v, 2);
            assert!(
                t0.elapsed() < std::time::Duration::from_millis(200),
                "不同 key 的调用不应互相阻塞"
            );
            assert_eq!(a.join().unwrap(), 1);
        });
    }

    #[test]
    fn single_flight_propagates_leader_error() {
        use std::sync::atomic::{AtomicUsize, Ordering as AOrd};
        let map: Mutex<HashMap<i64, Arc<Flight<Result<i32, String>>>>> = Mutex::new(HashMap::new());
        let calls = AtomicUsize::new(0);
        std::thread::scope(|s| {
            let a = s.spawn(|| {
                single_flight(&map, 3, || {
                    calls.fetch_add(1, AOrd::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    Err::<i32, String>("boom".into())
                })
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
            let b = s.spawn(|| {
                single_flight(&map, 3, || {
                    calls.fetch_add(1, AOrd::SeqCst);
                    Ok::<i32, String>(1)
                })
            });
            assert_eq!(a.join().unwrap().unwrap_err(), "boom");
            assert_eq!(
                b.join().unwrap().unwrap_err(),
                "boom",
                "follower 应拿到同一错误而不是挂起"
            );
        });
        assert_eq!(calls.load(AOrd::SeqCst), 1, "出错时也不应重复计算");
    }

    #[test]
    fn single_flight_follower_recovers_after_leader_panic() {
        use std::sync::atomic::{AtomicUsize, Ordering as AOrd};
        let map: Mutex<HashMap<i64, Arc<Flight<i32>>>> = Mutex::new(HashMap::new());
        let calls = AtomicUsize::new(0);
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|s| {
            let leader = s.spawn(|| {
                // leader 在 compute 中 panic（resume_unwind 不触发全局 panic hook，避免日志噪音）。
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    single_flight(&map, 9, || {
                        calls.fetch_add(1, AOrd::SeqCst);
                        let _ = started_tx.send(());
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        std::panic::resume_unwind(Box::new("leader panic"))
                    })
                }));
            });
            started_rx.recv().unwrap();
            // leader 正在 compute 中睡眠：follower 此时必然看到旧 Flight 并进入等待。
            let follower = s.spawn(|| {
                single_flight(&map, 9, || {
                    calls.fetch_add(1, AOrd::SeqCst);
                    7i32
                })
            });
            assert_eq!(
                follower.join().unwrap(),
                7,
                "leader panic 后 follower 应重新竞争并返回结果，而不是挂死"
            );
            leader.join().unwrap();
        });
        assert_eq!(calls.load(AOrd::SeqCst), 2, "leader 一次 + follower 重算一次");
    }
}

#[cfg(test)]
mod storage_tests {
    use super::{collect_storage, StorageStats};
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp_storage_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 单次遍历的分类口径：covers/res/fonts/db 各自累计，所有文件都计入 total。
    #[test]
    fn collect_storage_classifies_in_single_pass() {
        let data = tmp_dir("stats");
        for (rel, size) in [
            ("covers/a.png", 10u64),
            ("res/1/b.jpg", 20),
            ("fonts/f.ttf", 30),
            ("tidepage.db", 40),
            ("tidepage.db-wal", 5),
            ("tidepage.db-shm", 1),
            ("other.txt", 7),
        ] {
            let p = data.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, vec![0u8; size as usize]).unwrap();
        }
        let mut s = StorageStats {
            data_dir: data.to_string_lossy().into_owned(),
            total_bytes: 0,
            covers_bytes: 0,
            res_bytes: 0,
            fonts_bytes: 0,
            db_bytes: 0,
            cache_bytes: 0,
            parse_cache_bytes: 0,
            freelist_bytes: 0,
            db_dir_free_bytes: None,
            temp_dir: String::new(),
            temp_free_bytes: None,
            temp_same_disk: false,
            last_checkpoint_at: 0,
            last_checkpoint_busy: false,
            checkpoint_wal_before: 0,
            checkpoint_wal_after: 0,
        };
        collect_storage(&data, &data, &mut s);
        assert_eq!(s.covers_bytes, 10);
        assert_eq!(s.res_bytes, 20);
        assert_eq!(s.fonts_bytes, 30);
        assert_eq!(s.db_bytes, 46);
        // total = 10+20+30+40+5+1+7 = 113，分类字段互不重复但都包含在 total 内。
        assert_eq!(s.total_bytes, 113);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn prioritize_covers_only_md0() {
        assert!(super::prioritize_eligible(0));
        assert!(!super::prioritize_eligible(1));
        assert!(!super::prioritize_eligible(2), "pathological/超时失败 md=2 不得再入队");
        assert!(!super::prioritize_eligible(3), "处理中 md=3 不得再入队");
    }
}

#[cfg(test)]
mod round2_search_tests {
    //! H11/H12：图片上限、短查询规模守卫、2 字查询回退路径与 3 字 FTS 路径。
    use super::*;
    use rusqlite::Connection;

    fn tmpdb(tag: &str) -> Connection {
        let p = std::env::temp_dir().join(format!("tp_lib_r2_{}_{}.db", tag, std::process::id()));
        let _ = std::fs::remove_file(&p);
        db::init(&p).unwrap()
    }

    fn add_book(conn: &Connection, path: &str, title: &str, author: &str) -> i64 {
        db::upsert_book(conn, path, title, "txt", 10, 1).unwrap();
        conn.execute("UPDATE books SET author=?2 WHERE path=?1", rusqlite::params![path, author])
            .unwrap();
        conn.query_row("SELECT id FROM books WHERE path=?1", [path], |r| r.get(0))
            .unwrap()
    }

    /// H11：图片源文件上限边界（恰好等于上限允许，超过 1 字节拒绝）。
    #[test]
    fn image_source_limit_boundaries() {
        assert!(!image_source_too_large(0));
        assert!(!image_source_too_large(IMAGE_SOURCE_MAX_BYTES));
        assert!(image_source_too_large(IMAGE_SOURCE_MAX_BYTES + 1));
        assert_eq!(IMAGE_SOURCE_MAX_BYTES, 64 * 1024 * 1024, "图片命令上限应为 64MiB");
    }

    /// H12：规模守卫三分支（恰好等于阈值允许 / 超过 / 取不到）。
    #[test]
    fn short_body_guard_three_branches() {
        assert!(short_body_allowed(Some(SEARCH_SHORT_BODY_DB_MAX_BYTES)));
        assert!(!short_body_allowed(Some(SEARCH_SHORT_BODY_DB_MAX_BYTES + 1)));
        assert!(!short_body_allowed(None), "取不到库大小必须保守跳过正文");
        assert_eq!(non_whitespace_chars("  "), 0);
        assert_eq!(non_whitespace_chars(" a b "), 2);
    }

    /// H12：2 字查询命中书名/作者时返回 <mark> 片段，即使正文回退被守卫跳过也不返回 Err。
    #[test]
    fn short_query_title_hit_wins_even_when_body_skipped() {
        let conn = tmpdb("short_title");
        let id = add_book(&conn, "C:/x/santi.txt", "三体", "刘慈欣");
        conn.execute(
            "INSERT INTO chapter_text(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','人类文明与三体文明的碰撞')",
            [id],
        )
        .unwrap();
        let hits = search_short_with_body(&conn, "三体", false).expect("书名命中时不得因为正文被跳过而报错");
        assert_eq!(hits.len(), 1, "书名命中应直接返回: {hits:?}");
        assert_eq!(hits[0].book_id, id);
        assert!(hits[0].snippet.contains("<mark>三体</mark>"), "snippet={}", hits[0].snippet);
    }

    /// H12：书名/作者 0 命中且正文被守卫跳过 → Err 且消息说明需要 3 个字符。
    #[test]
    fn short_query_no_title_hit_and_body_skipped_is_error() {
        let conn = tmpdb("short_err");
        let id = add_book(&conn, "C:/x/a.txt", "普通书名", "作者");
        conn.execute(
            "INSERT INTO chapter_text(book_id,chapter_index,title,body) VALUES(?1,0,'章','正文里有碰撞二字')",
            [id],
        )
        .unwrap();
        let err = search_short_with_body(&conn, "碰撞", false).unwrap_err();
        assert!(err.contains("3 个字符"), "错误消息应说明需要 3 个字符: {err}");
    }

    /// H12：允许正文时，2 字查询命中正文并生成 <mark>；正文检查也走规模守卫。
    #[test]
    fn short_query_body_fallback_marks_hit() {
        let conn = tmpdb("short_body");
        let id = add_book(&conn, "C:/x/b.txt", "普通书名", "作者");
        conn.execute(
            "INSERT INTO chapter_text(book_id,chapter_index,title,body) VALUES(?1,2,'第二章','这是一段包含碰撞的正文')",
            [id],
        )
        .unwrap();
        assert!(short_body_allowed(estimate_db_bytes(&conn)), "临时库很小，应允许正文回退");
        let hits = search_short(&conn, "碰撞").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].chapter_index, 2);
        assert_eq!(hits[0].chapter_title, "第二章");
        assert!(hits[0].snippet.contains("<mark>碰撞</mark>"), "snippet={}", hits[0].snippet);
        // 无命中时返回空（不是错误）
        assert!(search_short(&conn, "不存在").unwrap().is_empty());
    }

    /// H12：≥3 字符仍走 FTS（trigram），snippet 由 FTS 生成并高亮。
    #[test]
    fn three_char_query_still_uses_fts() {
        let conn = tmpdb("fts3");
        let id = add_book(&conn, "C:/x/c.txt", "书", "作者");
        db::ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','人类文明与三体文明的碰撞')",
            [id],
        )
        .unwrap();
        let hits = search_fts(&conn, "人类文明").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].book_id, id);
        assert!(hits[0].snippet.contains("<mark>"), "snippet={}", hits[0].snippet);
    }

    /// I23：用户输入中的 FTS5 运算符/符号被整体短语转义，MATCH 可执行且不报错。
    #[test]
    fn fts_user_query_escapes_operators_and_symbols() {
        // 纯函数：双引号加倍、整体加引号。
        assert_eq!(fts_user_query("3.14"), "\"3.14\"");
        assert_eq!(fts_user_query("a-b"), "\"a-b\"");
        assert_eq!(fts_user_query("C:"), "\"C:\"");
        assert_eq!(fts_user_query("AND"), "\"AND\"");
        assert_eq!(fts_user_query("a\"b"), "\"a\"\"b\"");
        assert_eq!(fts_user_query("  "), "\"\"");

        let conn = tmpdb("fts_escape");
        let id = add_book(&conn, "C:/x/d.txt", "书", "作者");
        db::ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','版本 3.14 与 C: 盘符，纯符号 *** 不算命中')",
            [id],
        )
        .unwrap();
        // 特殊字符/运算符查询不得报错；能命中的仍有结果。
        for q in ["3.14", "a-b", "C:", "AND", "***", "()", "a\"b"] {
            let hits = search_fts(&conn, q).unwrap_or_else(|e| panic!("查询 {q:?} 不应报错：{e}"));
            let _ = hits;
        }
        let hits = search_fts(&conn, "3.14").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
    }
    /// P1-9：书名参与搜索（≥3 字；FTS 正文无命中时由书名补充查询返回并带 <mark>）。
    #[test]
    fn long_query_matches_book_title_with_mark() {
        let conn = tmpdb("title_hit");
        let id = add_book(&conn, "C:/x/title.txt", "三体文明史诗", "作者");
        db::ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','无关正文')",
            [id],
        )
        .unwrap();
        let hits = search_fts(&conn, "三体文明史诗").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].book_id, id);
        assert_eq!(hits[0].chapter_index, 0);
        assert!(hits[0].snippet.contains("<mark>三体文明史诗</mark>"), "snippet={}", hits[0].snippet);
    }

    /// P1-9：书名补充查询不得带出没有 FTS 行（未索引/已排除）的书。
    #[test]
    fn long_query_title_supplement_respects_fts_scope() {
        let conn = tmpdb("title_scope");
        let indexed = add_book(&conn, "C:/x/idx.txt", "索引书", "作者");
        let _missing = add_book(&conn, "C:/x/miss.txt", "索引书二", "作者");
        db::ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','无关正文')",
            [indexed],
        )
        .unwrap();
        let hits = search_fts(&conn, "索引书二").unwrap();
        assert!(hits.is_empty(), "无 FTS 行的书不得被书名补充带出: {hits:?}");
        let hits = search_fts(&conn, "索引书").unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].book_id, indexed);
    }

    /// P1-9：正文与书名同时命中时按 (book_id, chapter_index) 去重，不重复返回。
    #[test]
    fn long_query_dedupes_title_and_body_hits() {
        let conn = tmpdb("title_dedupe");
        let id = add_book(&conn, "C:/x/dup.txt", "碰撞文明", "作者");
        db::ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','碰撞文明在这里')",
            [id],
        )
        .unwrap();
        let hits = search_fts(&conn, "碰撞文明").unwrap();
        let same: Vec<_> = hits.iter().filter(|h| h.book_id == id && h.chapter_index == 0).collect();
        assert_eq!(same.len(), 1, "同书同章不得重复: {hits:?}");
        assert!(same[0].snippet.contains("<mark>"), "snippet={}", same[0].snippet);
    }

    /// P1-9：书名含特殊字符不报错；FTS 运算符转义行为不受影响。
    #[test]
    fn long_query_title_special_chars_no_error() {
        let conn = tmpdb("title_special");
        let id = add_book(&conn, "C:/x/special.txt", "a-b 100%", "作者");
        db::ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id,chapter_index,title,body) VALUES(?1,0,'第一章','无关正文')",
            [id],
        )
        .unwrap();
        for q in ["a-b 100%", "a-b", "100%", "C:", "AND", "\"quoted\""] {
            assert!(search_fts(&conn, q).is_ok(), "查询 {q:?} 不应报错");
        }
    }

}

#[cfg(test)]
mod path_guard_tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!(
            "tp_pathguard_{}_{}_{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    /// S1：写入路径白名单：合法路径通过；相对/错误扩展名/保留名/目录目标/父目录缺失/空格结尾拒绝。
    #[test]
    fn output_path_accepts_legal_and_rejects_invalid() {
        let d = tmp("out");
        assert!(validate_output_path(&s(&d.join("a.txt")), &["txt", "md", "csv"], true).is_ok());
        assert!(validate_output_path("a.txt", &["txt"], true).is_err(), "相对路径必须拒绝");
        assert!(validate_output_path(&s(&d.join("a.exe")), &["txt"], true).is_err(), "扩展名必须白名单");
        assert!(validate_output_path(&s(&d.join("CON.txt")), &["txt"], true).is_err(), "保留设备名必须拒绝");
        let dir_as_target = d.join("dir.txt");
        std::fs::create_dir_all(&dir_as_target).unwrap();
        assert!(validate_output_path(&s(&dir_as_target), &["txt"], true).is_err(), "已存在目录不能当目标");
        assert!(validate_output_path(&s(&d.join("none").join("a.txt")), &["txt"], true).is_err(), "父目录必须存在");
        assert!(validate_output_path(&s(&d.join("a.txt ")), &["txt"], true).is_err(), "空格结尾必须拒绝");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// S1：系统目录拒绝（环境变量存在时断言；取不到则跳过该项，不误伤其它机器）。
    #[test]
    fn output_path_rejects_system_dirs_when_env_available() {
        let Ok(root) = std::env::var("SystemRoot") else { return; };
        if root.trim().is_empty() {
            return;
        }
        let p = Path::new(&root).join("tp_path_guard_should_not_exist.txt");
        assert!(
            validate_output_path(&s(&p), &["txt"], false).is_err(),
            "系统目录必须拒绝: {}",
            p.display()
        );
        // 用 ".." 绕回系统目录也必须被拒绝（组件归一化后再比较前缀）。
        let sneaky = Path::new(&root)
            .join("..")
            .join(root.rsplit(['\\', '/']).next().unwrap_or(""))
            .join("tp_path_guard_sneaky.txt");
        assert!(
            validate_output_path(&s(&sneaky), &["txt"], false).is_err(),
            "经 .. 回到系统目录必须拒绝: {}",
            sneaky.display()
        );
    }

    /// S1：读取路径必须存在、是普通文件、扩展名白名单。
    #[test]
    fn source_path_requires_existing_file_and_ext() {
        let d = tmp("src");
        let txt = d.join("book.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert!(validate_source_path(&s(&txt), &["txt", "mobi"]).is_ok());
        assert!(validate_source_path(&s(&d.join("book.png")), &["txt"]).is_err());
        assert!(validate_source_path(&s(&d.join("missing.txt")), &["txt"]).is_err());
        assert!(validate_source_path("book.txt", &["txt"]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod r6_fts_tests {
    use super::*;
    use rusqlite::Connection;

    /// 连接级单测不需要真实 AppState/FTS5：用普通表模拟 fts 与 fts_meta 的读写语义。
    fn plain_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE fts(book_id INTEGER NOT NULL, chapter_index INTEGER NOT NULL, title TEXT, body TEXT);
             CREATE TABLE fts_meta(book_id INTEGER PRIMARY KEY, rows INTEGER NOT NULL);",
        )
        .unwrap();
        conn
    }

    fn ch(i: i64) -> (i64, String, String) {
        (i, format!("第 {i} 章"), format!("正文 {i}"))
    }

    fn count_fts(conn: &Connection, book_id: i64) -> i64 {
        conn.query_row("SELECT count(*) FROM fts WHERE book_id=?1", [book_id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn replace_book_fts_replaces_only_target_book() {
        let mut conn = plain_conn();
        conn.execute("INSERT INTO fts VALUES(2,0,'t','b')", []).unwrap();
        conn.execute("INSERT INTO fts VALUES(1,0,'old','old')", []).unwrap();
        conn.execute("INSERT INTO fts_meta VALUES(1,1)", []).unwrap();
        let n = replace_book_fts(&mut conn, 1, &[ch(0), ch(1), ch(2)]).unwrap();
        assert_eq!(n, 3);
        assert_eq!(count_fts(&conn, 1), 3, "目标书旧行应被整体替换");
        assert_eq!(count_fts(&conn, 2), 1, "其它书的行不得被删除");
        let meta: i64 = conn.query_row("SELECT rows FROM fts_meta WHERE book_id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(meta, 3);
    }

    #[test]
    fn replace_book_fts_rolls_back_on_insert_failure() {
        let mut conn = plain_conn();
        conn.execute("INSERT INTO fts VALUES(1,0,'old','old')", []).unwrap();
        conn.execute("INSERT INTO fts_meta VALUES(1,1)", []).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER r6_fts_fail BEFORE INSERT ON fts BEGIN SELECT RAISE(ABORT,'boom'); END;",
        )
        .unwrap();
        let err = replace_book_fts(&mut conn, 1, &[ch(0), ch(1)]).unwrap_err();
        assert!(err.contains("写入全文索引失败"), "{err}");
        assert_eq!(count_fts(&conn, 1), 1, "失败必须回滚，旧行仍在");
        let meta: i64 = conn.query_row("SELECT rows FROM fts_meta WHERE book_id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(meta, 1, "失败不得写 meta");
    }

    #[test]
    fn parse_failure_keeps_old_rows_and_marks_rebuild() {
        let mut conn = plain_conn();
        conn.execute("INSERT INTO fts VALUES(1,0,'old','old')", []).unwrap();
        conn.execute("INSERT INTO fts VALUES(1,1,'old2','old2')", []).unwrap();
        conn.execute("INSERT INTO fts_meta VALUES(1,2)", []).unwrap();
        let err = apply_book_fts_rebuild(&mut conn, 1, Err("解析失败".into())).unwrap_err();
        assert_eq!(err, "解析失败");
        assert_eq!(count_fts(&conn, 1), 2, "解析失败不得删除旧索引行");
        let meta: i64 = conn.query_row("SELECT rows FROM fts_meta WHERE book_id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(meta, -1, "失败后写 -1 哨兵");
        assert_eq!(
            fts_meta_plan(Some(meta), None, 2),
            FtsMetaPlan::Rebuild,
            "-1 哨兵必须让 fts_meta_plan 返回 Rebuild"
        );
        // 修复解析后重跑：旧行被替换、meta 等于实际行数。
        let n = apply_book_fts_rebuild(&mut conn, 1, Ok(vec![ch(0)])) .unwrap();
        assert_eq!(n, 1);
        assert_eq!(count_fts(&conn, 1), 1);
        let meta2: i64 = conn.query_row("SELECT rows FROM fts_meta WHERE book_id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(meta2, 1);
    }

    #[test]
    fn stale_sentinel_is_rebuild_for_any_expected() {
        let conn = plain_conn();
        mark_book_fts_stale(&conn, 7);
        let meta: i64 = conn.query_row("SELECT rows FROM fts_meta WHERE book_id=7", [], |r| r.get(0)).unwrap();
        assert_eq!(meta, -1);
        for expected in [0i64, 1, 5, 5000] {
            assert_eq!(fts_meta_plan(Some(-1), None, expected), FtsMetaPlan::Rebuild);
        }
    }

    #[test]
    fn rebuild_books_continue_after_single_failure() {
        let ids = [1i64, 2, 3];
        let total = rebuild_fts_books_with(&ids, |id| match id {
            2 => Err("坏书".into()),
            _ => Ok(5),
        });
        assert_eq!(total, 10, "失败书跳过、其余书仍计入总章数");
    }

    #[test]
    fn guard_wait_timeout_boundaries() {
        let start = std::time::Instant::now();
        let limit = std::time::Duration::from_secs(60);
        assert!(!fts_guard_wait_exceeded(start, start, limit), "未超时");
        assert!(!fts_guard_wait_exceeded(start, start + limit, limit), "恰好等于上限不算超时");
        assert!(fts_guard_wait_exceeded(start, start + limit + std::time::Duration::from_millis(1), limit));
        assert!(!fts_guard_wait_exceeded(start, start + std::time::Duration::from_millis(1), std::time::Duration::from_secs(1)));
    }
}

#[cfg(test)]
mod r6_misc_tests {
    use super::*;

    fn cd(tag: &str) -> crate::models::ChapterData {
        crate::models::ChapterData { title: tag.into(), html: format!("<p>{tag}</p>"), chars: 1 }
    }

    /// S1：章节缓存命中必须同时校验文件指纹；stamp 变化即失效并移除。
    #[test]
    fn chapter_cache_requires_matching_stamp() {
        let mut c = ChapterCache::new(8);
        let key = (1i64, 0i64, false);
        c.put(key, Some((10, 20)), Arc::new(cd("旧")));
        assert!(c.get(key, Some((10, 20))).is_some(), "同指纹应命中");
        assert!(c.get(key, Some((11, 20))).is_none(), "size 变化不得命中");
        assert!(c.get(key, Some((10, 21))).is_none(), "mtime 变化不得命中");
        assert!(c.get(key, None).is_none(), "当前指纹取不到时不得复用");
        // 失效条目应被移除：再次用旧指纹也不命中。
        assert!(c.get(key, Some((10, 20))).is_none(), "失效条目必须被移除");
        // 无指纹的条目同样不复用。
        c.put(key, None, Arc::new(cd("无指纹")));
        assert!(c.get(key, Some((10, 20))).is_none());
    }

    /// S2：指纹 helper：存在文件返回大小/哈希；不存在返回 None；内容变化后 size 跟着变。
    #[test]
    fn highlight_fingerprint_tracks_file() {
        let dir = std::env::temp_dir().join(format!("tp_r6_hl_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("book.txt");
        std::fs::write(&p, b"hello").unwrap();
        let (size, _, hash) = highlight_fingerprint(&p.to_string_lossy()).expect("存在的文件应有指纹");
        assert_eq!(size, 5);
        assert!(!hash.is_empty());
        assert!(highlight_fingerprint(&dir.join("missing.txt").to_string_lossy()).is_none());
        std::fs::write(&p, b"hello world").unwrap();
        let (size2, _, _) = highlight_fingerprint(&p.to_string_lossy()).unwrap();
        assert_eq!(size2, 11);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod r9_cover_tests {
    use super::*;

    /// P0-3：像素/边长上限边界（恰好上限通过，+1 拒绝，0 拒绝）。
    #[test]
    fn image_dim_limit_boundaries() {
        assert!(image_dim_limit_error(10_000, 10_000).is_none(), "恰好 100MP 允许");
        assert!(image_dim_limit_error(10_000, 10_001).is_some(), "超 100MP 拒绝");
        assert!(image_dim_limit_error(20_000, 1).is_none(), "恰好边长上限允许");
        assert!(image_dim_limit_error(20_001, 1).is_some(), "边长 +1 拒绝");
        assert!(image_dim_limit_error(0, 10).is_some(), "0 宽拒绝");
        assert!(image_dim_limit_error(10, 0).is_some(), "0 高拒绝");
        assert!(image_dim_limit_error(1, 1).is_none());
        // 大像素但乘积合法（100000 x 1000 = 100MP）允许；多 1 像素边 → 拒绝。
        assert!(image_dim_limit_error(100_000, 1_000).is_some(), "边长超限优先拒绝");
        assert!(image_dim_limit_error(20_000, 5_000).is_none(), "20k x 5k = 100MP 允许");
    }

    /// P0-3：非图片/损坏文件在 header 读取阶段给可读错误，不做整图解码。
    #[test]
    fn image_dimensions_rejects_non_image() {
        let dir = std::env::temp_dir().join(format!("tp_r9_img_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("not-image.bin");
        std::fs::write(&p, b"this is not an image").unwrap();
        let err = check_image_dimensions(&p.to_string_lossy()).unwrap_err();
        assert!(err.contains("读取图片尺寸失败"), "{err}");
        let missing = dir.join("missing.png");
        assert!(check_image_dimensions(&missing.to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod r10_encoding_tests {
    use super::*;

    /// P0-2：编码设置归一化：空/auto/未知 → None，合法 label → 规范化名称。
    #[test]
    fn normalize_text_encoding_rules() {
        assert_eq!(normalize_text_encoding(None), None);
        assert_eq!(normalize_text_encoding(Some("")), None);
        assert_eq!(normalize_text_encoding(Some("  auto ")), None);
        assert_eq!(normalize_text_encoding(Some("AUTO")), None);
        assert_eq!(normalize_text_encoding(Some("gb18030")).as_deref(), Some("gb18030"));
        assert_eq!(normalize_text_encoding(Some("UTF-8")).as_deref(), Some("UTF-8"));
        // 未知编码名：回退 None（自动探测），不让调用方失败。
        assert_eq!(normalize_text_encoding(Some("not-a-real-encoding")), None);
        assert_eq!(normalize_text_encoding(Some("  ")), None);
    }
}
