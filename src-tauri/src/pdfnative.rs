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
    Close {
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

/// 当前活跃的渲染线程槽位。`None` = 还没起过，或上一条已被判废弃。
#[cfg(all(windows, not(target_arch = "x86")))]
struct Slot {
    tx: Sender<Job>,
    id: u64,
}
#[cfg(all(windows, not(target_arch = "x86")))]
static SLOT: Mutex<Option<Slot>> = Mutex::new(None);
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
/// 超时错误的内部标记：命令层据此判定「这条线程必须废弃」，返回前会把标记去掉。
const TIMEOUT_PREFIX: &str = "native_timeout:";

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
fn slot() -> Result<(Sender<Job>, u64), String> {
    Err(UNSUPPORTED.to_string())
}

#[cfg(not(all(windows, not(target_arch = "x86"))))]
fn slot_if_live() -> Option<(Sender<Job>, u64)> {
    None
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
fn slot() -> Result<(Sender<Job>, u64), String> {
    let mut guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = guard.as_ref() {
        return Ok((s.tx.clone(), s.id));
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
    *guard = Some(Slot { tx: tx.clone(), id });
    Ok((tx, id))
}

/// 只取现存槽位，不起新线程（Close 用：没有活线程就该立刻成功）。
#[cfg(all(windows, not(target_arch = "x86")))]
fn slot_if_live() -> Option<(Sender<Job>, u64)> {
    let guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(|s| (s.tx.clone(), s.id))
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
        let mut guard = SLOT.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().map(|s| s.id) == Some(id) {
            *guard = None;
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

    fn open_doc(
        cur: &mut Option<(i64, PdfDocument)>,
        book_id: i64,
        path: &Path,
    ) -> Result<u32, String> {
        if cur.as_ref().map(|(b, _)| *b) != Some(book_id) {
            // PdfDocument 没有 IClosable::Close 投影：丢掉句柄即释放文档（页面位图从来不在这层缓存）
            cur.take();
            let doc = load(path)?;
            let n = doc.PageCount().map_err(|e| format!("读取页数失败：{e}"))?;
            *cur = Some((book_id, doc));
            return Ok(n);
        }
        cur.as_ref()
            .unwrap()
            .1
            .PageCount()
            .map_err(|e| format!("读取页数失败：{e}"))
    }

    fn current<'a>(
        cur: &'a Option<(i64, PdfDocument)>,
        book_id: i64,
    ) -> Result<&'a PdfDocument, String> {
        match cur {
            Some((b, d)) if *b == book_id => Ok(d),
            _ => Err("原生文档未打开".to_string()),
        }
    }

    fn geometry(
        cur: &mut Option<(i64, PdfDocument)>,
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

    #[allow(clippy::too_many_arguments)]
    fn render(
        cur: &mut Option<(i64, PdfDocument)>,
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
            let (x, y, w, h) = src;
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

    fn close_doc(cur: &mut Option<(i64, PdfDocument)>) {
        cur.take();
        trim_ws();
    }

    pub(super) fn run(rx: Receiver<Job>) {
        ensure_apartment();
        let mut cur: Option<(i64, PdfDocument)> = None;
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
                Job::Close { reply } => {
                    close_doc(&mut cur);
                    let _ = reply.send(Ok(()));
                }
            }
        }
        close_doc(&mut cur);
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

async fn call<T, F>(build: F) -> Result<T, String>
where
    F: FnOnce(tokio::sync::oneshot::Sender<Result<T, String>>) -> Job,
{
    let (tx, id) = slot()?;
    let (rtx, rrx) = tokio::sync::oneshot::channel();
    if tx.send(build(rtx)).is_err() {
        // 投递失败 = 线程已不在收件：当作废弃，下一次请求自会另起新线程。
        abandon(id, "send-failed");
        return Err("原生渲染线程已退出".to_string());
    }
    match rrx.await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => {
            if let Some(rest) = e.strip_prefix(TIMEOUT_PREFIX) {
                memlog::stage_note("native:timeout", &format!("id={id} err={rest}"));
                abandon(id, "timeout");
                Err(rest.to_string())
            } else {
                Err(e)
            }
        }
        Err(_) => {
            abandon(id, "reply-dropped");
            Err("原生渲染线程已退出".to_string())
        }
    }
}

/// 打开（或复用）原生文档，返回页数。
#[tauri::command]
pub async fn pdf_native_open(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<u32, String> {
    let path = library::pdf_book_path(&state, book_id)?;
    let n = call(|reply| Job::Open {
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
    let s = call(|reply| Job::Geometry {
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
    let bytes = call(|reply| Job::Render {
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

/// 关闭当前原生文档（关书/换书时调用）。
/// 没有活线程时直接成功：从未打开过，或上一条线程已被判废弃 —— 此时文档句柄随那条挂住的
/// 线程一起丢弃，调用方不该等任何人。等待也带上限：线程卡住时关闭必须立刻返回。
#[tauri::command]
pub async fn pdf_native_close() -> Result<(), String> {
    let (tx, id) = match slot_if_live() {
        Some(v) => v,
        None => {
            memlog::stage_note("cmd:pdf_native_close", "skipped: no live worker");
            return Ok(());
        }
    };
    let (rtx, rrx) = tokio::sync::oneshot::channel();
    if tx.send(Job::Close { reply: rtx }).is_err() {
        abandon(id, "send-failed");
        return Ok(());
    }
    match tokio::time::timeout(std::time::Duration::from_millis(CLOSE_TIMEOUT_MS), rrx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(e))) => Err(e),
        _ => {
            memlog::stage_note("native:timeout", &format!("id={id} err=close"));
            abandon(id, "close-timeout");
            Ok(())
        }
    }
}
