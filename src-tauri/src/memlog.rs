//! 启动期打点与内存观测（startup.log）。
//!
//! 为什么需要它：真实库（17 万本）启动期出现过程序工作集涨到数 GB 后崩溃（0xE0000008），
//! 而崩溃现场没有留下任何「死在哪一步」的信息 —— 只有一份 1.9MB 的 minidump。
//! 这里把「阶段名 / 耗时 / 工作集 / 私有内存」写成追加日志，并提供一个内存观察点
//! （watch）：只在工作集比上次记录涨了 ≥64MB 时才落一行，既能精确定位「哪一次调用吃掉了 GB」，
//! 又不会把日志写成逐调用刷屏。
//!
//! 崩溃可诊断：
//!   - init() 先写一次 begin（ok=0）标记；ready() 写 ok=1；
//!   - 下一次启动看到 ok=0 即说明上次没走到书库就死了 → 累加失败次数，
//!     并从下一次启动起自动跳过封面重建（skip_cover_rebuild()）；
//!   - 日志与状态文件都在数据目录（默认 %APPDATA%\com.a.tidepage）下。
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// 内存观察点的触发阈值：工作集比上次记录涨这么多才落一行。
const WATCH_STEP_MB: u64 = 64;

static T0: OnceLock<Instant> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static STATE_PATH: OnceLock<PathBuf> = OnceLock::new();
static LAST_LOGGED_MB: AtomicU64 = AtomicU64::new(0);
static PEAK_MB: AtomicU64 = AtomicU64::new(0);
static SKIP_COVERS: AtomicBool = AtomicBool::new(false);
static FAILS: AtomicU64 = AtomicU64::new(0);
static LOCK: Mutex<()> = Mutex::new(());

#[cfg(windows)]
fn mem_bytes() -> (u64, u64) {
    #[repr(C)]
    struct Pmc {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut core::ffi::c_void,
            counters: *mut Pmc,
            cb: u32,
        ) -> i32;
    }
    unsafe {
        let mut c: Pmc = std::mem::zeroed();
        c.cb = std::mem::size_of::<Pmc>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            (c.working_set_size as u64, c.private_usage as u64)
        } else {
            (0, 0)
        }
    }
}

#[cfg(not(windows))]
fn mem_bytes() -> (u64, u64) {
    (0, 0)
}

/// 当前工作集（MB）。0 表示取不到。
pub fn ws_mb() -> u64 {
    mem_bytes().0 / (1024 * 1024)
}

/// 本次启动记录到的最大工作集（MB）。
pub fn peak_mb() -> u64 {
    PEAK_MB.load(Ordering::Relaxed)
}

fn elapsed_ms() -> u128 {
    T0.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}

fn write_line(line: &str) {
    let Some(p) = LOG_PATH.get() else { return };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 日志自限：超过上限就整体换新（保留上一份为 .1），绝不无界增长。
    if let Ok(md) = std::fs::metadata(p) {
        if md.len() > LOG_MAX_BYTES {
            let _ = std::fs::rename(p, p.with_extension("log.1"));
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{line}");
    }
}

fn write_state(ok: bool, stage_name: &str) {
    let Some(p) = STATE_PATH.get() else { return };
    let fails = FAILS.load(Ordering::Relaxed);
    let body = format!(
        "ok={}\nfails={}\nstage={}\nattempt_ms={}\n",
        if ok { 1 } else { 0 },
        fails,
        stage_name,
        elapsed_ms()
    );
    let _ = std::fs::write(p, body);
}

/// 初始化：设置日志/状态路径，读上次启动是否成功，写 begin 标记。
/// 返回「是否应跳过本轮封面重建」。
pub fn init(data_dir: &Path) -> bool {
    let _ = T0.set(Instant::now());
    let _ = LOG_PATH.set(data_dir.join("startup.log"));
    let _ = STATE_PATH.set(data_dir.join("startup.state"));
    let state = std::fs::read_to_string(data_dir.join("startup.state")).unwrap_or_default();
    let prev_ok = state.contains("ok=1");
    let had_state = !state.is_empty();
    let prev_fails = state
        .lines()
        .find_map(|l| l.strip_prefix("fails="))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let fails = if !had_state || prev_ok { 0 } else { prev_fails + 1 };
    FAILS.store(fails, Ordering::Relaxed);
    // 「连续启动失败」判定：上次没走到书库 → 本轮跳过封面重建（记一条日志）。
    let skip = fails >= 1;
    SKIP_COVERS.store(skip, Ordering::Relaxed);
    let (ws, private) = mem_bytes();
    let wsm = ws / (1024 * 1024);
    PEAK_MB.store(wsm, Ordering::Relaxed);
    LAST_LOGGED_MB.store(wsm, Ordering::Relaxed);
    write_line(&format!(
        "=== run start pid={} ws={}MB private={}MB prev_ok={} prev_fails={} skip_covers={}",
        std::process::id(),
        wsm,
        private / (1024 * 1024),
        prev_ok,
        prev_fails,
        skip
    ));
    if skip {
        write_line(&format!(
            "SKIP cover rebuild: previous start did not reach library (fails={fails})"
        ));
    }
    write_state(false, "begin");
    // panic 钩子：崩溃前把 panic 位置与消息落盘（默认 handler 在 windows_subsystem 下无处可见）。
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        write_line(&format!("[PANIC] {loc} {msg}"));
        write_state(false, "panic");
        prev(info);
    }));
    // 后台采样线程：每 250ms 看一次工作集，只在「比上次落盘涨了 ≥128MB」时记一行。
    // 为什么需要它：阶段打点只在被显式插桩的代码路径上落盘，一旦暴涨发生在没打点的路径
    // （本地协议处理器 / 第三方库内部），日志就会「什么都没写就崩了」。采样线程保证任何形态的
    // 内存暴涨都有时间戳，从而能反推它发生在哪两个阶段之间。只跑前 60s，之后自动退出。
    std::thread::spawn(|| {
        let mut last = ws_mb();
        for _ in 0..240 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let wsm = ws_mb();
            PEAK_MB.fetch_max(wsm, Ordering::Relaxed);
            if wsm >= last + 128 {
                last = wsm;
                let private = mem_bytes().1 / (1024 * 1024);
                write_line(&format!(
                    "[{:>7}ms] +MEM-SAMPLE                  ws={}MB private={}MB",
                    elapsed_ms(),
                    wsm,
                    private
                ));
            }
        }
    });
    skip
}

/// 阶段打点（总是写）。批次边界、启动步骤用它。
pub fn stage(name: &str) {
    let (ws, private) = mem_bytes();
    let wsm = ws / (1024 * 1024);
    PEAK_MB.fetch_max(wsm, Ordering::Relaxed);
    LAST_LOGGED_MB.store(wsm, Ordering::Relaxed);
    write_line(&format!(
        "[{:>7}ms] {:<28} ws={}MB private={}MB",
        elapsed_ms(),
        name,
        wsm,
        private / (1024 * 1024)
    ));
}

/// 内存观察点：只在工作集比上次记录涨了 ≥64MB 时落一行。热路径（每个 IPC 命令）都可调用。
pub fn watch(name: &str) {
    let wsm = ws_mb();
    PEAK_MB.fetch_max(wsm, Ordering::Relaxed);
    let last = LAST_LOGGED_MB.load(Ordering::Relaxed);
    if wsm < last + WATCH_STEP_MB {
        return;
    }
    if LAST_LOGGED_MB
        .compare_exchange(last, wsm, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let private = mem_bytes().1 / (1024 * 1024);
    write_line(&format!(
        "[{:>7}ms] +MEM {:<24} ws={}MB private={}MB",
        elapsed_ms(),
        name,
        wsm,
        private
    ));
}

/// 带补充说明的阶段行（如「批 N 本」）。
pub fn stage_note(name: &str, note: &str) {
    let wsm = ws_mb();
    PEAK_MB.fetch_max(wsm, Ordering::Relaxed);
    LAST_LOGGED_MB.store(wsm, Ordering::Relaxed);
    write_line(&format!("[{:>7}ms] {:<28} {note}", elapsed_ms(), name));
}

/// 致命错误出口：写日志 + 落 ok=0 + 弹一个阻塞对话框。
/// 为什么必须弹窗：release 构建是 windows_subsystem="windows"，没有控制台，
/// 任何 Stage 失败（数据库损坏/建窗失败）都会「双击没反应就没了」——这正是要消灭的静默退出。
pub fn fatal(where_: &str, msg: &str) {
    stage_note("FATAL", &format!("{where_}: {msg}"));
    write_state(false, where_);
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("TidePage 启动失败")
        .set_description(format!("{where_} 失败：\n{msg}\n\n详见数据目录下的 startup.log。"))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// 走到书库（或本轮启动完成的判定点）：写 ok=1，清零失败计数。
pub fn ready() {
    FAILS.store(0, Ordering::Relaxed);
    write_state(true, "ready");
    stage_note("ready", &format!("peak_ws={}MB", peak_mb()));
}

/// 是否应跳过封面重建（上一轮没走到书库）。
pub fn skip_cover_rebuild() -> bool {
    SKIP_COVERS.load(Ordering::Relaxed)
}
