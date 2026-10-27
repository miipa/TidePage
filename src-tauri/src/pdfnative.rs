//! 超大 PDF 的 Windows 原生渲染通道。
//!
//! 32 位构建不编译 WinRT 实现（见文件末尾 imp 门控），通道/常量/任务类型只剩声明。
#![cfg_attr(all(windows, target_arch = "x86"), allow(dead_code))]
//!
//! 为什么需要：pdf.js 的 range 路径用 ChunkedStream，构造时就是 new Uint8Array(length)
//! （按整份文件长度要一块连续缓冲），而 WebView2 渲染进程单次 ArrayBuffer 上限是 2³¹−1 字节
//! （实测 1.75GiB 成功、2.00GiB 失败）。所以超过约 2GiB 的 PDF 在 pdf.js 里无解；
//! 系统自带的 Windows.Data.Pdf 能按页渲染，不需要把整份文件物化。
//!
//! 线程模型：WinRT 对象不能跨线程搬运，所以所有调用都落在一条专用线程上（MTA）；
//! 命令把「路径 + 页码 + 目标尺寸」投进通道、等这条线程回一个结果。
//! 这条线程只保留「当前这一本书」的文档句柄（不含任何页面位图），换书即关闭旧文档；
//! 每次渲染完立刻 Close 掉页面对象。
//!
//! **废弃与重起**：`Windows.Data.Pdf` 有永久不返回的坏文件（实测 3.50GiB 那册停在 Started）。
//! 超时后这条线程可能永远卡在 WinRT 调用里，投递给它等于排队等死 —— 所以超时即把槽位标为废弃、
//! 下一次请求另起新线程（旧线程不 join、不回收，句柄随线程一起丢弃）。废弃线程数有上限，
//! 达到上限直接返回稳定错误码，避免无限起线程。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use crate::memlog;

/// 单页位图像素上限：与前端 PDF_BITMAP_MAX_PIXELS 对齐（同为 1200 万像素）。
/// 为什么对齐：原生通道返回的是 PNG 字节，位图要先在宿主里编好再经 IPC 出去，
/// 峰值比 pdf.js（画在 webview 画布上）更高；两侧上限不一致时前端按 1200 万算出来的
/// 目标尺寸会被后端以 4000 万放行，等于上限形同虚设。
const MAX_PIXELS: u64 = 12_000_000;
/// 单边像素上限：WinRT 目标尺寸是 u32，给个更保守的上界。
const MAX_SIDE: u32 = 20_000;
/// WinRT PdfPage.Size 的单位是 DIP（1/96 英寸），pdf.js 的 pt 是 1/72 英寸。
/// 判定依据（实测同一页两通道）：Letter 页 pdf.js getViewport({ scale: 1 }).width = 612.0，
/// WinRT Size.Width = 816.0 → 816/612 = 1.3333 = 96/72，确认是 DIP。
/// 官方文档未写明单位（见 MicrosoftDocs/winrt-api#2122），所以这里显式换算成 pt 返回，
/// 与 pdf.js 路径的 ptW/ptH 同语义（前端再统一乘 PDF_PX_PER_PT = 4/3 得到像素）。
const DIP_TO_PT: f32 = 72.0 / 96.0;
/// 加载等待期间回收工作集的间隔（tick × 2ms = 约 64ms）。
/// 取舍：系统 PDF 引擎把整份文件映射进进程，不回收时打开 3.31GiB 那册工作集峰值 3475MB；
/// 主动回收把峰值压到 200MB 量级，代价是映射页被让出后再次访问要重新缺页（I/O 换内存），
/// 不是免费优化。峰值 ≈ 间隔 × 引擎读取速度（实测 1.8s 读完 3.5GB ≈ 2GB/s）。
const TRIM_INTERVAL_TICKS: u32 = 32;
/// 渲染调用计数：只服务于启动日志限流（前 20 条逐条记，之后只记大块）。
static NATIVE_RENDER_CALLS: AtomicU64 = AtomicU64::new(0);
/// Windows.Foundation.AsyncStatus 的 ABI 取值（Started/Completed/Canceled/Error）。
const AS_STARTED: i32 = 0;
const AS_COMPLETED: i32 = 1;
const AS_CANCELED: i32 = 2;

/// 一页的尺寸（与 pdf.js 路径的 ptW/ptH 同语义：只用于比例与切片换算）。
#[derive(Clone, Copy)]
pub struct PageSize {
    pub w: f32,
    pub h: f32,
}

enum Job {
    Open {
        book_id: i64,
        path: PathBuf,
        reply: tokio::sync::oneshot::Sender<Result<u32, String>>,
    },
    Geometry {
        book_id: i64,
        path: PathBuf,
        page: u32,
        reply: tokio::sync::oneshot::Sender<Result<PageSize, String>>,
    },
    Render {
        book_id: i64,
        path: PathBuf,
        page: u32,
        /// 源矩形（页面坐标系，左上原点）：x, y, w, h
        src: (f32, f32, f32, f32),
        /// 目标像素尺寸：w, h
        dest: (u32, u32),
        reply: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
    },
    /// L30：按书关闭。worker 一进程一本，book_id 仅用于日志/语义（None=全部由命令层展开）。
    Close {
        book_id: Option<i64>,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

/// L30：每本书一条渲染线程（一进程一本 PdfDocument），最近 MAX_BOOK_SLOTS 本保留，
/// 超限/超 TTL 淘汰时主动发 Close 并丢弃投递端，让旧线程关闭文档后自行退出。
/// 为什么不共享单线程：两窗口交替看书会在每页渲染时重新加载整份 PDF（数百 MB 映射）。
#[cfg(all(windows, not(target_arch = "x86")))]
struct Slot {
    tx: Sender<Job>,
    id: u64,
    last_used: std::time::Instant,
}
#[cfg(all(windows, not(target_arch = "x86")))]
#[derive(Default)]
struct BookSlots {
    map: std::collections::HashMap<i64, Slot>,
    /// 最近使用顺序（旧→新）；仅作淘汰候选，实际是否存活以 map 为准。
    order: std::collections::VecDeque<i64>,
}
#[cfg(all(windows, not(target_arch = "x86")))]
static SLOTS: Mutex<Option<BookSlots>> = Mutex::new(None);
/// 同时保留的文档数上限：2 本（活跃书 + 上一本），再淘汰最久未用的。
#[cfg(all(windows, not(target_arch = "x86")))]
const MAX_BOOK_SLOTS: usize = 2;
/// 空闲 TTL：10 分钟没碰过的书回收，避免多窗口长期占内存。
#[cfg(all(windows, not(target_arch = "x86")))]
const SLOT_TTL_SECS: u64 = 600;

/// L30：Close 的纯目标决策（None=全部存活槽位；Some(id)=仅该书，未打开则空）。
fn close_target_ids(slot_ids: &[i64], target: Option<i64>) -> Vec<i64> {
    match target {
        Some(id) => {
            if slot_ids.contains(&id) {
                vec![id]
            } else {
                Vec::new()
            }
        }
        None => slot_ids.to_vec(),
    }
}

/// L30：淘汰候选（order 中最旧且仍存活的），未超上限返回 None。
fn eviction_candidate(order: &[i64], alive: &[i64], current: usize, max: usize) -> Option<i64> {
    if current < max {
        return None;
    }
    order.iter().copied().find(|id| alive.contains(id))
}
/// 已判定废弃、任其挂住的线程数（进程内累计，不回收）。
#[cfg(all(windows, not(target_arch = "x86")))]
static ABANDONED: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, not(target_arch = "x86")))]
static SLOT_SEQ: AtomicU64 = AtomicU64::new(1);
#[cfg(all(windows, not(target_arch = "x86")))]
static SPAWNED: AtomicUsize = AtomicUsize::new(0);
/// 同时存在的废弃线程上限：到顶就不再起新线程（否则一个坏文件能无限起线程）。
#[cfg(all(windows, not(target_arch = "x86")))]
const ABANDON_LIMIT: usize = 3;

/// Close 的等待上限：线程卡在别的 WinRT 调用里时，关闭也必须立刻返回。
const CLOSE_TIMEOUT_MS: u64 = 2_000;
/// P0-5：命令层按 Job 类型分档看门狗。同步 WinRT 调用（PageCount/GetPage/Size/ReadBytes）
/// 没有内层超时，worker 一旦卡死 rrx.await 就会永久挂起。
/// - Geometry（同步查询）20s；
/// - Open 45s（内部 load 有 30s LOAD 超时 + 15s 余量）；
/// - Render 90s（内部 RENDER 60s + 30s 余量）。
const CALL_TIMEOUT_QUERY_MS: u64 = 20_000;
const CALL_TIMEOUT_OPEN_MS: u64 = 45_000;
const CALL_TIMEOUT_RENDER_MS: u64 = 90_000;

/// P0-5：Job 类型 → 命令层超时（纯函数便于单测）。
fn job_timeout_ms(kind: &str) -> u64 {
    match kind {
        "open" => CALL_TIMEOUT_OPEN_MS,
        "render" => CALL_TIMEOUT_RENDER_MS,
        _ => CALL_TIMEOUT_QUERY_MS,
    }
}

impl Job {
    fn call_timeout_ms(&self) -> u64 {
        match self {
            Job::Open { .. } => job_timeout_ms("open"),
            Job::Render { .. } => job_timeout_ms("render"),
            Job::Geometry { .. } => job_timeout_ms("geometry"),
            // Close 不走 call()（由 close 命令直接发并自有 CLOSE_TIMEOUT_MS）。
            Job::Close { .. } => CLOSE_TIMEOUT_MS,
        }
    }
}
/// 超时错误的内部标记：命令层据此判定「这条线程必须废弃」，返回前会把标记去掉。
const TIMEOUT_PREFIX: &str = "native_timeout:";

/// 本次运行内"原生已判死"的书（book_id）。一旦某本书原生超时，就不再为它起新线程：
/// 废弃线程会永久占住该文件与地址空间，重复尝试只是叠加代价，且很可能再次超时。
/// 与 `ABANDON_LIMIT`（全局上限）互补：那是"一共允许废几条线程"，这是"这本书别再试了"。
static NATIVE_DEAD: Mutex<Option<std::collections::HashMap<i64, &'static str>>> = Mutex::new(None);

/// 记录一本书"原生已判死"（重复调用幂等），返回当前累计本数（用于日志）。
fn mark_native_dead(book_id: i64, reason: &'static str) -> usize {
    let mut g = NATIVE_DEAD.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(std::collections::HashMap::new);
    m.insert(book_id, reason);
    m.len()
}

/// 这本书本次运行内是否已被判死。
fn native_dead(book_id: i64) -> bool {
    let g = NATIVE_DEAD.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().map(|m| m.contains_key(&book_id)).unwrap_or(false)
}

/// 平台不支持时的稳定错误码：文案由前端 i18n 的错误码表给出（errText）。
/// 只在 32 位 / 非 Windows 构建里被引用，64 位构建下允许未使用。
#[allow(dead_code)]
const UNSUPPORTED: &str = "pdf_native_unsupported";
/// 废弃线程到上限时的稳定错误码（与「平台不支持」区分开）。
#[allow(dead_code)]
const ABANDONED_LIMIT: &str = "pdf_native_abandoned";

/// 取（必要时起）渲染线程的投递端。
/// 32 位 / 非 Windows：直接拒绝，不起线程（见 imp 门控）。
#[cfg(not(all(windows, not(target_arch = "x86"))))]
fn slot(_book_id: i64) -> Result<(Sender<Job>, u64), String> {
    Err(UNSUPPORTED.to_string())
}

#[cfg(not(all(windows, not(target_arch = "x86"))))]
fn live_targets(_book_id: Option<i64>) -> Vec<(Sender<Job>, u64)> {
    Vec::new()
}


#[cfg(not(all(windows, not(target_arch = "x86"))))]
fn abandon(_id: u64, _reason: &str) {}

/// 主动回收进程工作集（废弃线程还占着数 GB 映射页；EmptyWorkingSet(-1) 作用于整个进程，
/// 由任意线程调用即可）。
#[cfg(not(all(windows, not(target_arch = "x86"))))]
fn trim_process_ws() {}

#[cfg(not(all(windows, not(target_arch = "x86"))))]
fn start_trim_bg() {}

#[cfg(all(windows, not(target_arch = "x86")))]
fn slot(book_id: i64) -> Result<(Sender<Job>, u64), String> {
    let mut guard = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let slots = guard.get_or_insert_with(BookSlots::default);
    // TTL 回收：过期书主动 Close 并移除（发送失败说明线程已退，直接丢槽位）。
    let now = std::time::Instant::now();
    let expired: Vec<i64> = slots
        .map
        .iter()
        .filter(|(_, s)| now.duration_since(s.last_used).as_secs() > SLOT_TTL_SECS)
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        if let Some(s) = slots.map.remove(&id) {
            let (rtx, _rrx) = tokio::sync::oneshot::channel();
            let _ = s.tx.send(Job::Close { book_id: Some(id), reply: rtx });
            slots.order.retain(|x| *x != id);
            memlog::stage_note("native:slot_ttl", &format!("book={id} ttl={SLOT_TTL_SECS}s"));
        }
    }
    if let Some(s) = slots.map.get_mut(&book_id) {
        s.last_used = now;
        slots.order.retain(|x| *x != book_id);
        slots.order.push_back(book_id);
        return Ok((s.tx.clone(), s.id));
    }
    // 容量淘汰：按 order 找最旧存活项，主动发 Close 并丢弃槽位（worker 收 Close 后关闭文档，
    // 投递端全丢弃后线程 recv 结束自行退出）。
    let alive: Vec<i64> = slots.map.keys().copied().collect();
    while slots.map.len() >= MAX_BOOK_SLOTS {
        let Some(old) = eviction_candidate(
            &slots.order.iter().copied().collect::<Vec<_>>(),
            &alive,
            slots.map.len(),
            MAX_BOOK_SLOTS,
        ) else {
            break;
        };
        if let Some(s) = slots.map.remove(&old) {
            let (rtx, _rrx) = tokio::sync::oneshot::channel();
            let _ = s.tx.send(Job::Close { book_id: Some(old), reply: rtx });
            memlog::stage_note(
                "native:slot_evict",
                &format!("evict={old} for={book_id} max={MAX_BOOK_SLOTS}"),
            );
        }
        slots.order.retain(|x| *x != old);
        let alive: Vec<i64> = slots.map.keys().copied().collect();
        if slots.map.len() < MAX_BOOK_SLOTS {
            break;
        }
        let _ = alive;
    }
    let n_abandoned = ABANDONED.load(Ordering::Relaxed);
    if n_abandoned >= ABANDON_LIMIT {
        memlog::stage_note(
            "native:abandon_limit",
            &format!("abandoned={n_abandoned} limit={ABANDON_LIMIT} ws={}MB", memlog::ws_mb()),
        );
        trim_process_ws();
        return Err(ABANDONED_LIMIT.to_string());
    }
    let (tx, rx) = channel::<Job>();
    let id = SLOT_SEQ.fetch_add(1, Ordering::Relaxed);
    std::thread::Builder::new()
        .name(format!("pdf-native-{id}"))
        .spawn(move || run(rx))
        .map_err(|e| format!("启动原生渲染线程失败：{e}"))?;
    if SPAWNED.fetch_add(1, Ordering::Relaxed) == 0 {
        memlog::stage_note("native:worker_start", &format!("id={id}"));
    } else {
        memlog::stage_note(
            "native:worker_respawn",
            &format!("id={id} abandoned={n_abandoned}"),
        );
    }
    slots.map.insert(
        book_id,
        Slot {
            tx: tx.clone(),
            id,
            last_used: now,
        },
    );
    slots.order.retain(|x| *x != book_id);
    slots.order.push_back(book_id);
    Ok((tx, id))
}

/// 只取现存槽位，不起新线程（Close 用：没有活线程就该立刻成功）。
/// None=全部书；Some(id)=仅该书。
#[cfg(all(windows, not(target_arch = "x86")))]
fn live_targets(book_id: Option<i64>) -> Vec<(Sender<Job>, u64)> {
    let guard = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(slots) = guard.as_ref() else {
        return Vec::new();
    };
    let ids = close_target_ids(&slots.map.keys().copied().collect::<Vec<_>>(), book_id);
    ids.into_iter()
        .filter_map(|id| slots.map.get(&id).map(|s| (id, s.tx.clone(), s.id)))
        .map(|(_, tx, id)| (tx, id))
        .collect()
}

/// 判定某条线程废弃：清掉槽位（下次请求另起新线程），并记账。
/// 旧线程不 join、不回收 —— 它可能永远卡在 WinRT 调用里，回收就等于再次阻塞调用方。
///
/// **已知代价**：被放弃的线程会一直持有该 PDF 的文件句柄与地址空间映射，直到进程退出；
/// 每条废弃线程都会占住这份文件（实测 3.50GiB 那册），所以 `ABANDON_LIMIT` 一到，本次运行
/// 就不再尝试系统渲染（返回 `pdf_native_abandoned`），用户需要重启应用才能继续。
/// 不做回收是刻意的：WinRT 的加载操作没有可用的强制中止手段，唯一安全的做法是把这条线程连同
/// 它占的资源一起放着，换一条干净线程服务后续请求。
#[cfg(all(windows, not(target_arch = "x86")))]
fn trim_process_ws() {
    imp::trim_ws();
}

/// 有没有正在跑的后台回收线程（同一时间只允许一条）。
#[cfg(all(windows, not(target_arch = "x86")))]
static TRIM_BG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 废弃线程活着的时候，它们占的映射页会一直挂在进程上；从这里起一条轻量后台线程定期回收。
/// 有界：最多跑 5 分钟，且一旦没有废弃线程就退出（不做常驻空转）。
#[cfg(all(windows, not(target_arch = "x86")))]
fn start_trim_bg() {
    if TRIM_BG.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("pdf-native-trim".into())
        .spawn(|| {
            let mut idle = 0u32;
            for _ in 0..1500 {
                if ABANDONED.load(Ordering::Relaxed) == 0 {
                    idle += 1;
                    if idle > 5 {
                        break;
                    }
                } else {
                    idle = 0;
                    trim_process_ws();
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            TRIM_BG.store(false, Ordering::Relaxed);
        });
    if spawned.is_err() {
        TRIM_BG.store(false, Ordering::Relaxed);
    }
}

#[cfg(all(windows, not(target_arch = "x86")))]
fn abandon(id: u64, reason: &str) {
    {
        let mut guard = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slots) = guard.as_mut() {
            let book = slots
                .map
                .iter()
                .find(|(_, s)| s.id == id)
                .map(|(b, _)| *b);
            if let Some(book) = book {
                slots.map.remove(&book);
                slots.order.retain(|x| *x != book);
            }
        }
    }
    let n = ABANDONED.fetch_add(1, Ordering::Relaxed) + 1;
    // 废弃线程之前按间隔回收的映射页从此没人管了：这里补一次，并起一条有界后台回收。
    trim_process_ws();
    start_trim_bg();
    memlog::stage_note(
        "native:worker_abandoned",
        &format!(
            "id={id} reason={reason} abandoned={n} limit={ABANDON_LIMIT} ws={}MB",
            memlog::ws_mb()
        ),
    );
}

// ============================ 平台实现（Windows） ============================

#[cfg(all(windows, not(target_arch = "x86")))]
mod imp {
    use super::*;
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Foundation::{Rect, Size};
    use windows::Graphics::Imaging::BitmapEncoder;
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    /// WinRT 需要线程已初始化套间。MTA 足够（渲染在后台线程等，不需要消息泵）；
    /// 已初始化（含 STA）时 RoInitialize 返回 S_FALSE / RPC_E_CHANGED_MODE，都按可用处理。
    fn ensure_apartment() {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
    }

    /// 加载/渲染的超时（毫秒）。实测 3.75GB 的《康熙字典》在 LoadFromFileAsync 上永久停在 Started：
    /// 不报错、不完成、内存也不再增长；没有超时就会把这条线程和用户界面一起挂住。
    const LOAD_TIMEOUT_MS: u64 = 30_000;
    const RENDER_TIMEOUT_MS: u64 = 60_000;
    const STREAM_TIMEOUT_MS: u64 = 20_000;

    /// 阻塞等一个 WinRT 异步操作：轮询 Status，完成取结果，超时则取消并报错。
    /// 为什么自己写：windows-future 0.3 没有阻塞 get()，而为一行轮询把 windows-future
    /// 提成直接依赖不值得（它本来就在依赖树里，只是没有被我们直接引用）。
    macro_rules! wait_op {
        ($op:expr, $ms:expr, $trim:expr) => {{
            let op = $op;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis($ms);
            let mut tick: u32 = 0;
            loop {
                tick = tick.wrapping_add(1);
                // WinRT 的 AsyncStatus 由 windows-future 定义（不是 windows 的 Foundation 模块），
                // 这里按 ABI 取值判断，避免为一行状态比较把它提成直接依赖。
                let st = op.Status().map_err(|e| format!("等待异步操作失败：{e}"))?.0;
                match st {
                    AS_COMPLETED => {
                        break op.GetResults().map_err(|e| format!("异步操作失败：{e}"))?;
                    }
                    AS_STARTED => {
                        if std::time::Instant::now() > deadline {
                            // 取消不一定生效（实测坏文件会停在 Started）：带内部标记，
                            // 命令层据此把这条线程判为废弃、另起新线程。
                            let _ = op.Cancel();
                            return Err(format!("{TIMEOUT_PREFIX}打开或渲染超时"));
                        }
                        // 加载期间按固定间隔回收映射页：引擎读文件是一遍过，不需要常驻。
                        if $trim && tick % TRIM_INTERVAL_TICKS == 0 {
                            trim_ws();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    AS_CANCELED => {
                        return Err("原生渲染被取消".to_string());
                    }
                    _ => {
                        let e = op
                            .GetResults()
                            .err()
                            .map(|e| e.message().to_string())
                            .unwrap_or_else(|| "原生渲染失败".to_string());
                        return Err(e);
                    }
                }
            }
        }};
    }

    fn load(path: &Path) -> Result<PdfDocument, String> {
        let file = wait_op!(
            StorageFile::GetFileFromPathAsync(&HSTRING::from(path.to_string_lossy().as_ref()))
                .map_err(|e| format!("打开文件失败：{e}"))?,
            STREAM_TIMEOUT_MS,
            false
        );
        Ok(wait_op!(
            PdfDocument::LoadFromFileAsync(&file).map_err(|e| format!("解析 PDF 失败：{e}"))?,
            LOAD_TIMEOUT_MS,
            true
        ))
    }

    /// 当前打开的原生文档 + 建立句柄时的文件指纹。
    /// 同路径 PDF 被替换/重导/WebDAV 下载后 book_id 不变，只按 id 复用会一直显示旧内容。
    struct CurDoc {
        book_id: i64,
        path: String,
        stamp: Option<(i64, i64)>,
        doc: PdfDocument,
    }

    /// 复用判定的纯逻辑：book_id、路径、文件指纹三者全同才复用。
    /// 抽成不依赖 CurDoc 的形态，便于单测（PdfDocument 无法在单测里构造）。
    fn same_doc_key(
        cached: Option<(i64, &str, Option<(i64, i64)>)>,
        book_id: i64,
        path: &str,
        stamp: Option<(i64, i64)>,
    ) -> bool {
        matches!(cached, Some((b, p, s)) if b == book_id && p == path && s == stamp)
    }

    fn open_doc(
        cur: &mut Option<CurDoc>,
        book_id: i64,
        path: &Path,
    ) -> Result<u32, String> {
        let path_s = path.to_string_lossy().into_owned();
        // 每次调用都现取指纹：文件被就地替换时必须丢弃旧句柄（旧内容、旧页树）。
        let stamp = crate::library::file_stamp(&path_s);
        let cached = cur
            .as_ref()
            .map(|c| (c.book_id, c.path.as_str(), c.stamp));
        if !same_doc_key(cached, book_id, &path_s, stamp) {
            // PdfDocument 没有 IClosable::Close 投影：丢掉句柄即释放文档（页面位图从来不在这层缓存）
            cur.take();
            let doc = load(path)?;
            let n = doc.PageCount().map_err(|e| format!("读取页数失败：{e}"))?;
            *cur = Some(CurDoc {
                book_id,
                path: path_s,
                stamp,
                doc,
            });
            return Ok(n);
        }
        cur.as_ref()
            .unwrap()
            .doc
            .PageCount()
            .map_err(|e| format!("读取页数失败：{e}"))
    }

    fn current<'a>(
        cur: &'a Option<CurDoc>,
        book_id: i64,
    ) -> Result<&'a PdfDocument, String> {
        match cur {
            Some(c) if c.book_id == book_id => Ok(&c.doc),
            _ => Err("原生文档未打开".to_string()),
        }
    }

    fn geometry(
        cur: &mut Option<CurDoc>,
        book_id: i64,
        path: &Path,
        page: u32,
    ) -> Result<PageSize, String> {
        open_doc(cur, book_id, path)?;
        let doc = current(cur, book_id)?;
        let p = doc
            .GetPage(page)
            .map_err(|e| format!("读取第 {} 页失败：{e}", page + 1))?;
        let r = (|| -> Result<PageSize, String> {
            let size: Size = p.Size().map_err(|e| format!("读取页尺寸失败：{e}"))?;
            // DIP → pt：不换算会让前端把 ptW 当 DIP 再乘 4/3，页面直接放大 33%。
            Ok(PageSize {
                w: (size.Width * DIP_TO_PT).max(1.0),
                h: (size.Height * DIP_TO_PT).max(1.0),
            })
        })();
        let _ = p.Close();
        r
    }

    /// P1-1：前端裁切矩形（页面 pt）→ WinRT SourceRect 的页面 DIP。
/// 负 x/y 钳到 0（页面外区域不渲染）；宽/高非正或非有限值返回可读错误。
pub(super) fn pt_rect_to_dip(x: f32, y: f32, w: f32, h: f32) -> Result<(f32, f32, f32, f32), String> {
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
        return Err("裁切区域包含非法数值".into());
    }
    if w <= 0.0 || h <= 0.0 {
        return Err(format!("裁切区域尺寸无效：{w}x{h}"));
    }
    const PT_TO_DIP: f32 = 96.0 / 72.0;
    Ok((
        (x * PT_TO_DIP).max(0.0),
        (y * PT_TO_DIP).max(0.0),
        w * PT_TO_DIP,
        h * PT_TO_DIP,
    ))
}

#[allow(clippy::too_many_arguments)]
    fn render(
        cur: &mut Option<CurDoc>,
        book_id: i64,
        path: &Path,
        page: u32,
        src: (f32, f32, f32, f32),
        dest: (u32, u32),
    ) -> Result<Vec<u8>, String> {
        let (dw, dh) = dest;
        if dw == 0 || dh == 0 || dw > MAX_SIDE || dh > MAX_SIDE || (dw as u64) * (dh as u64) > MAX_PIXELS {
            return Err(format!("渲染尺寸过大：{dw}x{dh}"));
        }
        open_doc(cur, book_id, path)?;
        let doc = current(cur, book_id)?;
        let p = doc
            .GetPage(page)
            .map_err(|e| format!("读取第 {} 页失败：{e}", page + 1))?;
        let out = (|| -> Result<Vec<u8>, String> {
            // P1-1：frontend 传的是 pt（geometry 已 DIP→pt）；WinRT SetSourceRect 的坐标系是
            // 页面 DIP。不转回 DIP 会把裁剪区域缩小 3/4、内容放大 33%。真机截图对比未验证。
            let (x, y, w, h) = pt_rect_to_dip(src.0, src.1, src.2, src.3)?;
            let stream =
                InMemoryRandomAccessStream::new().map_err(|e| format!("创建内存流失败：{e}"))?;
            let opts = PdfPageRenderOptions::new().map_err(|e| format!("创建渲染参数失败：{e}"))?;
            opts.SetBitmapEncoderId(
                BitmapEncoder::PngEncoderId().map_err(|e| format!("取 PNG 编码器失败：{e}"))?,
            )
            .map_err(|e| format!("设置编码器失败：{e}"))?;
            opts.SetSourceRect(Rect {
                X: x,
                Y: y,
                Width: w,
                Height: h,
            })
            .map_err(|e| format!("设置渲染区域失败：{e}"))?;
            opts.SetDestinationWidth(dw)
                .map_err(|e| format!("设置目标宽失败：{e}"))?;
            opts.SetDestinationHeight(dh)
                .map_err(|e| format!("设置目标高失败：{e}"))?;
            wait_op!(
                p.RenderWithOptionsToStreamAsync(&stream, &opts)
                    .map_err(|e| format!("渲染失败：{e}"))?,
                RENDER_TIMEOUT_MS,
                false
            );
            let len = stream.Size().map_err(|e| format!("读取位图长度失败：{e}"))?;
            if len == 0 {
                return Err("渲染结果为空".to_string());
            }
            let input = stream
                .GetInputStreamAt(0)
                .map_err(|e| format!("取输入流失败：{e}"))?;
            let reader =
                DataReader::CreateDataReader(&input).map_err(|e| format!("创建读取器失败：{e}"))?;
            wait_op!(
                reader.LoadAsync(len as u32).map_err(|e| format!("读取位图失败：{e}"))?,
                STREAM_TIMEOUT_MS,
                false
            );
            let mut buf = vec![0u8; len as usize];
            reader
                .ReadBytes(&mut buf)
                .map_err(|e| format!("拷贝位图失败：{e}"))?;
            Ok(buf)
        })();
        let _ = p.Close();
        out
    }

    /// 回收本进程工作集：系统 PDF 引擎是把文件映射进来的（实测 3.31GiB 那册打开后工作集
    /// 3.5GB，而私有内存只有 156MB —— 全是可回收的映射页）。不主动回收，映射页会一直挂在
    /// 进程工作集上，一本本打开就一层层叠加。
    pub(super) fn trim_ws() {
        unsafe {
            #[link(name = "kernel32")]
            extern "system" {
                fn EmptyWorkingSet(hprocess: isize) -> i32;
            }
            // -1 = 当前进程（GetCurrentProcess 的伪句柄）
            let _ = EmptyWorkingSet(-1);
        }
    }

    fn close_doc(cur: &mut Option<CurDoc>) {
        cur.take();
        trim_ws();
    }

    pub(super) fn run(rx: Receiver<Job>) {
        ensure_apartment();
        let mut cur: Option<CurDoc> = None;
        while let Ok(job) = rx.recv() {
            match job {
                Job::Open { book_id, path, reply } => {
                    let _ = reply.send(open_doc(&mut cur, book_id, &path));
                }
                Job::Geometry {
                    book_id,
                    path,
                    page,
                    reply,
                } => {
                    let _ = reply.send(geometry(&mut cur, book_id, &path, page));
                }
                Job::Render {
                    book_id,
                    path,
                    page,
                    src,
                    dest,
                    reply,
                } => {
                    let _ = reply.send(render(&mut cur, book_id, &path, page, src, dest));
                }
                Job::Close { book_id, reply } => {
                    // L30：本 worker 只服务一本书；book_id 仅用于诊断日志。
                    memlog::stage_note(
                        "native:close_doc",
                        &format!("worker book={book_id:?}"),
                    );
                    close_doc(&mut cur);
                    let _ = reply.send(Ok(()));
                }
            }
        }
        close_doc(&mut cur);
    }

    #[cfg(test)]
    mod tests {
        use super::same_doc_key;

        /// book_id、路径、指纹任一不同（或旧句柄缺失/指纹取不到）都必须重新加载。
        #[test]
        fn same_doc_key_requires_identity_path_and_stamp() {
            let cached = Some((7i64, "C:\\books\\a.pdf", Some((1024i64, 1_700_000_000_000i64))));
            assert!(same_doc_key(cached, 7, "C:\\books\\a.pdf", Some((1024, 1_700_000_000_000))));
            // 换书
            assert!(!same_doc_key(cached, 8, "C:\\books\\a.pdf", Some((1024, 1_700_000_000_000))));
            // 换路径
            assert!(!same_doc_key(cached, 7, "C:\\books\\b.pdf", Some((1024, 1_700_000_000_000))));
            // 文件被替换：size 或 mtime 变化
            assert!(!same_doc_key(cached, 7, "C:\\books\\a.pdf", Some((1025, 1_700_000_000_000))));
            assert!(!same_doc_key(cached, 7, "C:\\books\\a.pdf", Some((1024, 1_700_000_000_001))));
            // 当前文件取不到指纹 / 尚无句柄
            assert!(!same_doc_key(cached, 7, "C:\\books\\a.pdf", None));
            assert!(!same_doc_key(None, 7, "C:\\books\\a.pdf", Some((1024, 1_700_000_000_000))));
        }

        use crate::pdfnative::{close_target_ids, eviction_candidate, job_timeout_ms};

        /// L30：Close 目标决策与淘汰候选（纯函数）。
        #[test]
        fn close_and_eviction_targets() {
            assert_eq!(close_target_ids(&[1, 2], Some(1)), vec![1]);
            assert!(close_target_ids(&[1, 2], Some(3)).is_empty());
            assert_eq!(close_target_ids(&[1, 2], None), vec![1, 2]);
            assert_eq!(eviction_candidate(&[1], &[1], 1, 2), None);
            assert_eq!(eviction_candidate(&[5, 6], &[5, 6], 2, 2), Some(5));
            assert_eq!(eviction_candidate(&[3, 4], &[4], 1, 1), Some(4));
        }

        /// P0-5：Job→超时映射与内层超时的关系（纯函数；真实挂死无法单测）。
        #[test]
        fn job_timeout_mapping() {
            assert_eq!(job_timeout_ms("open"), 45_000);
            assert_eq!(job_timeout_ms("render"), 90_000);
            assert_eq!(job_timeout_ms("geometry"), 20_000);
            assert_eq!(job_timeout_ms("other"), 20_000, "未知类型按查询档保守处理");
        }
    }
}

/// 32 位构建：WinRT 的 PDF 引擎要把数 GB 的文件映射进进程，32 位地址空间放不下，
/// 直接在入口拒绝（前端按错误码给出「此设备无法显示该文件」）。
#[cfg(all(windows, target_arch = "x86"))]
mod imp {
    use super::*;
    pub(super) fn run(_rx: Receiver<Job>) {}
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub(super) fn run(_rx: Receiver<Job>) {}
}

fn run(rx: Receiver<Job>) {
    imp::run(rx)
}

// ================================ 命令层 ================================

use crate::library::{self, AppState};
use std::sync::Arc;
use tauri::State;

/// 线程废弃时把"这本书"记进本次运行的判死表：同一本不再重复起线程（`pdf_native_open` 会拦住）。
async fn call_for_book<T, F>(book_id: i64, build: F) -> Result<T, String>
where
    F: FnOnce(tokio::sync::oneshot::Sender<Result<T, String>>) -> Job,
{
    let r = call(book_id, build).await;
    if let Err(e) = &r {
        // 只有"超时/线程已退出"这类**引擎级**失败才判死；文件损坏之类的普通错误不判死
        // （那是确定性失败，前端会照常展示错误，不需要记忆）。
        if e.contains("超时") || e.contains("原生渲染线程已退出") || e == ABANDONED_LIMIT {
            let n = mark_native_dead(book_id, "engine");
            memlog::stage_note(
                "native:book_dead",
                &format!("book={book_id} dead_total={n} err={e} ws={}MB", memlog::ws_mb()),
            );
        }
    }
    r
}

async fn call<T, F>(book_id: i64, build: F) -> Result<T, String>
where
    F: FnOnce(tokio::sync::oneshot::Sender<Result<T, String>>) -> Job,
{
    let (tx, id) = slot(book_id)?;
    let (rtx, rrx) = tokio::sync::oneshot::channel();
    let job = build(rtx);
    // P0-5：Job 类型决定看门狗时长；与 wait_op 的内层超时是两层保护（内层先到按普通
    // 错误返回；内层被同步 COM 调用卡住时，这里到点放弃线程，保证命令层不永久挂起）。
    let timeout_ms = job.call_timeout_ms();
    if tx.send(job).is_err() {
        // 投递失败 = 线程已不在收件：当作废弃，下一次请求自会另起新线程。
        abandon(id, "send-failed");
        return Err("原生渲染线程已退出".to_string());
    }
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rrx).await {
        Ok(Ok(Ok(v))) => Ok(v),
        Ok(Ok(Err(e))) => {
            if let Some(rest) = e.strip_prefix(TIMEOUT_PREFIX) {
                memlog::stage_note("native:timeout", &format!("id={id} err={rest}"));
                abandon(id, "timeout");
                Err(rest.to_string())
            } else {
                Err(e)
            }
        }
        Ok(Err(_)) => {
            abandon(id, "reply-dropped");
            Err("原生渲染线程已退出".to_string())
        }
        Err(_) => {
            memlog::stage_note(
                "native:call_timeout",
                &format!(
                    "id={id} book={book_id} timeout={timeout_ms}ms ws={}MB",
                    memlog::ws_mb()
                ),
            );
            // best-effort Close：线程大概率卡在同步 COM 调用里，Close 不会立即生效；
            // 这里只保证「槽位被放弃、后续请求快速失败/另起线程」，不等待回复。
            let (ctx, _crx) = tokio::sync::oneshot::channel();
            let _ = tx.send(Job::Close {
                book_id: Some(book_id),
                reply: ctx,
            });
            abandon(id, "call-timeout");
            Err(format!(
                "原生渲染超时（{timeout_ms}ms），已放弃该文档，请重试"
            ))
        }
    }
}

/// 打开（或复用）原生文档，返回页数。
#[tauri::command]
pub async fn pdf_native_open(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<u32, String> {
    // 本次运行内已经判定"这本书原生超时"：**不再起新线程**。
    // 每条废弃线程都会一直占住该文件与地址空间（见 trim_process_ws 的说明），
    // 重复打开同一本只会叠加代价；直接返回废弃码，前端据此转向"用系统程序打开"。
    if native_dead(book_id) {
        memlog::stage_note(
            "native:already_dead",
            &format!("book={book_id} hit=1 ws={}MB", memlog::ws_mb()),
        );
        return Err(ABANDONED_LIMIT.to_string());
    }
    let path = library::pdf_book_path(&state, book_id)?;
    let n = call_for_book(book_id, |reply| Job::Open {
        book_id,
        path,
        reply,
    })
    .await?;
    memlog::stage_note(
        "cmd:pdf_native_open",
        &format!("book={book_id} pages={n} ws={}MB", memlog::ws_mb()),
    );
    Ok(n)
}

/// 单页尺寸（页面坐标系，左上原点）。
#[tauri::command]
pub async fn pdf_native_page_geometry(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    page: u32,
) -> Result<(f32, f32), String> {
    let path = library::pdf_book_path(&state, book_id)?;
    let s = call_for_book(book_id, |reply| Job::Geometry {
        book_id,
        path,
        page,
        reply,
    })
    .await?;
    Ok((s.w, s.h))
}

/// 渲染页面的一块区域为 PNG 字节（沿用现有位图通道：前端拿到字节自己造图 URL）。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn pdf_native_render_page(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    page: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    dest_w: u32,
    dest_h: u32,
) -> Result<tauri::ipc::Response, String> {
    let path = library::pdf_book_path(&state, book_id)?;
    // 渲染块打点：前 20 条逐条记（验收看「按需出图」的分布），之后只记大块（≥1200 万像素）。
    let n = NATIVE_RENDER_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 20 || (dest_w as u64) * (dest_h as u64) >= 12_000_000 {
        memlog::stage_note(
            "cmd:pdf_native_render_page",
            &format!(
                "n={n} book={book_id} page={page} src={x},{y},{w},{h} dest={dest_w}x{dest_h} ws={}MB",
                memlog::ws_mb()
            ),
        );
    }
    let bytes = call_for_book(book_id, |reply| Job::Render {
        book_id,
        path,
        page,
        src: (x, y, w, h),
        dest: (dest_w, dest_h),
        reply,
    })
    .await?;
    memlog::watch("cmd:pdf_native_render_page");
    Ok(tauri::ipc::Response::new(bytes))
}

/// L30：关闭原生文档（关书/换书时调用）。
/// book_id=None 保持旧行为：关闭全部已打开文档（兼容旧前端无参 invoke）；
/// Some(id) 只关对应书，双窗口互不影响。没有对应活线程时直接成功。
#[tauri::command]
pub async fn pdf_native_close(book_id: Option<i64>) -> Result<(), String> {
    let targets = live_targets(book_id);
    if targets.is_empty() {
        memlog::stage_note("cmd:pdf_native_close", "skipped: no live worker");
        return Ok(());
    }
    for (tx, id) in targets {
        let (rtx, rrx) = tokio::sync::oneshot::channel();
        if tx.send(Job::Close { book_id, reply: rtx }).is_err() {
            abandon(id, "send-failed");
            continue;
        }
        match tokio::time::timeout(std::time::Duration::from_millis(CLOSE_TIMEOUT_MS), rrx).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => eprintln!("[pdf-native] close id={id} book={book_id:?} failed: {e}"),
            _ => {
                memlog::stage_note("native:timeout", &format!("id={id} err=close"));
                abandon(id, "close-timeout");
            }
        }
    }
    Ok(())

}

#[cfg(test)]
mod r11_rect_tests {
    #[cfg(windows)]
    use super::imp::pt_rect_to_dip;

    /// P1-1：pt→DIP 换算与非法值防护。
    #[cfg(windows)]
    #[test]
    fn pt_rect_to_dip_boundaries() {
        let (x, y, w, h) = pt_rect_to_dip(0.0, 0.0, 612.0, 792.0).unwrap();
        assert!((x - 0.0).abs() < 1e-3 && (y - 0.0).abs() < 1e-3);
        assert!((w - 816.0).abs() < 1e-2, "w={w}");
        assert!((h - 1056.0).abs() < 1e-2, "h={h}");
        // 负 x/y 钳到 0；极薄条保留正值。
        let (x, y, w, h) = pt_rect_to_dip(-5.0, -8.0, 1.0, 792.0).unwrap();
        assert_eq!((x, y), (0.0, 0.0));
        assert!(w > 0.0 && h > 0.0);
        // 宽高为 0/负、NaN/Inf 必须 Err（不 panic）。
        for (x, y, w, h) in [
            (0.0, 0.0, 0.0, 10.0),
            (0.0, 0.0, 10.0, -1.0),
            (f32::NAN, 0.0, 10.0, 10.0),
            (0.0, f32::INFINITY, 10.0, 10.0),
        ] {
            assert!(pt_rect_to_dip(x, y, w, h).is_err(), "{x},{y},{w},{h}");
        }
    }
}
