mod ai;
mod backup;
mod bookmeta;
mod convert;
mod covers;
mod dav;
mod db;
mod epub;
mod fonts;
mod library;
mod memlog;
mod pdfnative;
mod pdfscan;
mod mobi;
mod models;
mod sanitize;
mod secret;
mod txt;
mod update;

use library::AppState;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri_plugin_window_state::{StateFlags, WindowExt};

/// window-state 统一标志：排除 DECORATIONS（历史保存值 decorated=true 会把原生标题栏带回来）。
/// 仅去掉 DECORATIONS，位置/大小/最大化等其余状态仍正常持久化。
/// 本地协议请求计数（启动打点用，限流输出）。
static PROTO_REQUESTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const WINDOW_STATE_FLAGS: StateFlags = StateFlags::all().difference(StateFlags::DECORATIONS);
/// 主窗口手动恢复用：去掉 VISIBLE，保持既有「建窗后 140ms 再 show」的启动观感。
const MAIN_RESTORE_FLAGS: StateFlags = WINDOW_STATE_FLAGS.difference(StateFlags::VISIBLE);

/// 应用数据目录（数据库/封面/字体/提取资源）。
/// 默认使用系统的 app_data_dir；当设置了 `TIDEPAGE_DATA_DIR` 时改为该目录，
/// 用于「便携模式」以及测试时的数据隔离（默认不设置 → 行为完全不变）。
pub fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("TIDEPAGE_DATA_DIR") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn default_pattern() -> &'static str {
    r"第\s*[0-9〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,10}\s*[章节卷回部篇]"
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // 逐字节解析 %XX：旧实现用 `&s[i + 1..i + 3]` 切片，
        // `%中` 这类输入会落在 UTF-8 字符中间 panic（URL 查询串不可信）。
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 从 cover URL 查询串解析外壳主题配色（g1/g2/g3/tc），供默认封面按主题上色。
/// 颜色值来自 URL（不可信）：非法值一律回落到内置默认观感，绝不原样拼进 SVG。
fn parse_dc_theme(query: Option<&str>) -> library::DcTheme {
    let mut t = library::DcTheme::default();
    let Some(q) = query else { return t };
    // 与 render_default_cover 的回落色同源（CoverStyle::builtin），避免两处硬编码漂移。
    let builtin = covers::CoverStyle::builtin(None);
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let val = percent_decode(v);
        let valid = covers::parse_hex(&val).is_some();
        match k {
            "g1" => t.g1 = if valid { val } else { builtin.g1.clone() },
            "g2" => t.g2 = if valid { val } else { builtin.g2.clone() },
            "g3" => t.g3 = if valid { val } else { builtin.g3.clone() },
            "tc" => t.tc = if valid { val } else { builtin.text.clone() },
            _ => {}
        }
    }
    t
}

/// 协议端点单文件读取上限（32 MB）：超过直接按不存在处理，避免超大文件整份读入内存。
const MAX_PROTOCOL_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// 把已解码的相对资源名解析到 `base` 之下：只接受单一文件名，拒绝路径穿越与绝对路径。
///
/// - 非空、不以 `/` 开头、不含 `\\` / `:` / `..`；
/// - `Path::new(rel).file_name()` 必须与 rel 完全一致（排除任何分隔符/上层目录）；
/// - join 后追加部分只允许 Normal 组件（Windows 下盘符/根路径会让 join 整体替换）；
/// - 只认已存在的普通文件；不依赖文件是否存在做 canonicalize。
fn resolve_under(base: &std::path::Path, rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.contains(':')
        || rel.contains("..")
    {
        return None;
    }
    let rel_path = std::path::Path::new(rel);
    let name = rel_path.file_name()?.to_str()?;
    if name != rel {
        return None;
    }
    if rel_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let file = base.join(rel_path);
    // 追加部分必须仍是 Normal（父目录/新前缀都不允许）；base 自身的盘符前缀属于可信配置。
    let rest = file.strip_prefix(base).ok()?;
    if rest
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    if !file.is_file() {
        return None;
    }
    Some(file)
}

/// 带上限读取协议端点文件：超过 `MAX_PROTOCOL_FILE_BYTES` 视为不存在。
fn read_protocol_file(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    // take 在读取层设硬上限：即使文件在 metadata 之后变大也不会无界分配。
    let mut buf = Vec::new();
    f.take(MAX_PROTOCOL_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_PROTOCOL_FILE_BYTES {
        return None;
    }
    Some(buf)
}

fn sniff_mime(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.starts_with(b"BM") {
        "image/bmp"
    } else {
        "image/png"
    }
}

/// `tidepage://` 各资源前缀的 HTTP 缓存策略（`None` = 不设，走启发式缓存）。
///
/// - `epubimg/`、`res/`、`covers/`、`book-cover/`、`cover-default/`：URL 由内容本身决定
///   （book_id + zip 内路径 / 提取文件名 / 封面文件名，封面端点还带 `v=cover_ver`、`db=`），
///   换图必换 URL，故可长期 immutable。漫画窗口重建 `.manga-block` 时能直接命中缓存，
///   不必反复重新解压 zip 条目、重新下载与重新解码同一张几百 KB 的图。
/// - `bg/`：保存时按时间戳+序号唯一命名（每次选图都是新 URL），
///   但历史遗留的 `background.img` 仍可能在使用中，故整体继续给短缓存。
/// - 其余（`fonts/` 等）不设：字体/未知资源保持原行为。
fn cache_control_for(path: &str) -> Option<&'static str> {
    const IMMUTABLE: &str = "public, max-age=31536000, immutable";
    if path.starts_with("epubimg/")
        || path.starts_with("res/")
        || path.starts_with("covers/")
        || path.starts_with("book-cover/")
        || path.starts_with("cover-default/")
    {
        return Some(IMMUTABLE);
    }
    if path.starts_with("bg/") {
        return Some("public, max-age=60");
    }
    None
}

/// 主窗口几何的手动恢复（仅 Windows）。
///
/// 背景：主窗口以 `.visible(false)` 创建，窗口一直隐藏时 tao 收不到 `WM_NCCALCSIZE`，
/// 客户区仍是建窗时按原生边框样式算出的旧值；此时 window-state 若自动 `set_size(PhysicalSize)`，
/// tao 的 Windows `set_inner_size` 会按当前 `window_rect - client_rect` 差值做补偿，把这份
/// 差值固化进真实客户区 → 每启动一次窗口就向下（向右）长高一点。
/// 修法与上游 readest PR #6546 一致：先 `SWP_FRAMECHANGED` 强制重算非客户区，再手动恢复几何。
#[cfg(windows)]
fn restore_main_window_state(window: &tauri::WebviewWindow) {
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SET_WINDOW_POS_FLAGS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER,
    };

    // ① 强制重算非客户区。tauri 的 HWND 可能来自另一个 windows 小版本，
    //    取裸句柄 `hwnd.0` 重新用本 crate 的 HWND 包一层最稳。
    if let Ok(raw) = window.hwnd() {
        let hwnd = HWND(raw.0);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SET_WINDOW_POS_FLAGS(
                    SWP_FRAMECHANGED.0
                        | SWP_NOMOVE.0
                        | SWP_NOSIZE.0
                        | SWP_NOZORDER.0
                        | SWP_NOACTIVATE.0,
                ),
            );
        }
    }

    // ② 再恢复几何（不含 VISIBLE，仍由既有「建窗后 140ms show」的线程负责显示）。
    //    失败时 show 兜底，绝不留下永远不可见的主窗口。
    if window.restore_state(MAIN_RESTORE_FLAGS).is_err() {
        let _ = window.show();
    }
}

/// 把调用方窗口抬到指定基准窗之上（不移动、不改尺寸、不激活）。
/// 用于吸附成组时让卫星窗紧贴其基准窗显示，且不置顶到其它程序之上。
#[tauri::command]
fn raise_above(window: tauri::WebviewWindow, base_label: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        use ::windows::Win32::Foundation::HWND;
        use ::windows::Win32::UI::WindowsAndMessaging::{
            GetWindow, SetWindowPos, GW_HWNDPREV, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SET_WINDOW_POS_FLAGS,
        };
        let app = window.app_handle();
        let base = app.get_webview_window(&base_label).ok_or("base window not found")?;
        let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0);
        let base_hwnd = HWND(base.hwnd().map_err(|e| e.to_string())?.0);
        // SetWindowPos 的 hWndInsertAfter 是「插到这扇窗的下面」：直接把基准窗传进来会把
        // 卫星压到基准窗之下。要落在基准窗之上，应插到基准窗当前上方邻窗的下面；基准窗
        // 已在最前（上方无窗）时用 HWND_TOP。
        let insert_after = match unsafe { GetWindow(base_hwnd, GW_HWNDPREV) }.ok() {
            // 已经紧贴在基准窗之上，不必再动。
            Some(h) if h.0 == hwnd.0 => return Ok(()),
            Some(h) if !h.0.is_null() => h,
            _ => HWND_TOP,
        };
        unsafe {
            SetWindowPos(
                hwnd,
                Some(insert_after),
                0,
                0,
                0,
                0,
                SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (&window, &base_label);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 排除 DECORATIONS 标志：window-state 插件默认会持久化并恢复窗口装饰，
        // 历史保存值为 true 会覆盖 builder 的 .decorations(false) 导致原生标题栏复现。
        // 主窗口跳过自动恢复（隐藏创建会触发 tao 的尺寸补偿漂移），改由
        // restore_main_window_state 在建窗后手动恢复；插件仍会跟踪 main 并在退出时保存。
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(WINDOW_STATE_FLAGS)
                // 独立书窗的 label 是 reader-<id>（每本一个），插件按 label 各存各的，
                // 新书没有历史尺寸就会用建窗时的固定初值。归并成同一个 reader 键后，
                // 所有书窗共享一份尺寸与位置，新书沿用最近一次调整的结果。
                .map_label(|label: &str| {
                    if label.starts_with("reader-") {
                        "reader"
                    } else {
                        label
                    }
                })
                .skip_initial_state("main")
                .build(),
        )
        // 主窗关闭时连带关闭所有子窗（书窗 / 面板窗 / AI 窗）：每个子窗走自己的关闭流程，
        // 窗口状态插件据此落盘；不用 exit 硬退。若将来有子窗拦截关闭，这里会留下残留进程。
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                use tauri::Manager;
                let app = window.app_handle();
                for (label, w) in app.webview_windows() {
                    if label != "main" {
                        let _ = w.close();
                    }
                }
            }
        })
        .setup(|app| {
            use tauri::Manager;
            let data_dir = data_dir(app.handle());
            // 是否处于便携模式（TIDEPAGE_DATA_DIR 显式指定）。仅便携模式才把
            // WebView2 用户数据目录（localStorage 等）也迁徙到该目录，默认行为不变。
            let portable = std::env::var("TIDEPAGE_DATA_DIR")
                .map(|p| !p.trim().is_empty())
                .unwrap_or(false);
            std::fs::create_dir_all(&data_dir).ok();
            // 启动打点/内存观察点：先 init 才能把「上一次启动是否走到书库」写进日志与状态文件。
            // 连续启动失败时本轮自动跳过封面重建（见 memlog::skip_cover_rebuild）。
            memlog::init(&data_dir);
            memlog::stage("setup:data_dir");
            let conn = match db::init(&data_dir.join("tidepage.db")) {
                Ok(c) => c,
                Err(e) => {
                    // 不静默退出：写日志 + 弹窗（见 memlog::fatal 的说明）。
                    memlog::fatal("db_init", &e);
                    return Err(e.into());
                }
            };
            memlog::stage("setup:db_init");
            if db::has_fts(&conn) {
                let _ = db::ensure_fts(&conn);
            }
            memlog::stage("setup:fts");
            db::migrate_book_ids(&conn);
            memlog::stage("setup:migrate_book_ids");
            // 切分算法升级（txt.rs 引入章节上限细分）：作废按旧 toc 写入的搜索缓存/FTS，
            // 避免划词全文搜索定位到错误章节；下次搜索按新 toc 懒重建。
            db::migrate_chapter_split(&conn);
            memlog::stage("setup:migrate_chapter_split");

            // 创建主窗口：注入 WebView2 字体渲染参数（需重启生效）。
            // 设置页现已简化为单个「灰度抗锯齿」复选框，只写入
            // --disable-lcd-text（勾选）或 --enable-lcd-text（取消）；
            // 旧版 GDI/自定义参数值仍原样尊重，保证升级用户配置不失效。
            {
                let webview_args = db::get_setting(&conn, "render.webviewArgs").unwrap_or_default();
                let raw = webview_args.trim().to_string();
                let render_arg = if raw.is_empty() {
                    "--disable-lcd-text".to_string()
                } else if raw.contains("disable-lcd-text")
                    || raw.contains("enable-lcd-text")
                    || raw.contains("disable-direct-write")
                {
                    raw
                } else {
                    format!("--disable-lcd-text {raw}")
                };
                let mut wb = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                    .title("TidePage 阅读")
                    .inner_size(1380.0, 880.0)
                    .min_inner_size(940.0, 620.0)
                    // 去掉系统原生标题栏（大黑边），改为前端自绘窗口控制按钮；
                    // 无边框窗口在 Windows 上默认仍可拖拽边缘缩放。
                    .decorations(false)
                    .center()
                    .visible(false);
                // 便携模式：把 WebView2 的用户数据目录（含 localStorage）固定到便携目录，
                // 使配置/凭据随 TIDEPAGE_DATA_DIR 一起搬走。非便携模式沿用 Tauri 默认目录。
                // 注意：JS 运行期创建的独立窗口（如 AI 窗口）仍使用 Tauri 默认数据目录，
                // 其持久化 localStorage 不跟随便携目录，但同一次运行内凭据经事件同步不受影响。
                if portable {
                    wb = wb.data_directory(data_dir.join("webview2"));
                }
                // wry 在设置了 additional_browser_args 时会替换默认参数，这里补回默认项
                // wry 程序化参数优先于 WEBVIEW2_* 环境变量，故 CDP 端口只能在此注入；
                // 仅 debug 构建编译该分支（release 二进制不含此代码路径与字符串）。
                // 严格校验堵住“绕过测试侧守卫直接启动 debug exe”：
                // ① 必须 ASCII 纯数字（不用 trim——空格/制表符/加号/附加参数一律拒绝）
                // ② 再 parse::<u16>（超范围拒绝）③ 拒绝 0 与 9222
                // 注入串只由解析后的数字重建；固定仅监听 127.0.0.1。
                #[cfg(debug_assertions)]
                let cdp_extra = match std::env::var("TP_TEST_CDP_PORT") {
                    Ok(raw) => {
                        let ascii_digits =
                            !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit());
                        match (ascii_digits, raw.parse::<u16>()) {
                            (true, Ok(port)) if port != 0 && port != 9222 => format!(
                                " --remote-debugging-address=127.0.0.1 --remote-debugging-port={port}"
                            ),
                            _ => String::new(),
                        }
                    }
                    Err(_) => String::new(),
                };
                #[cfg(not(debug_assertions))]
                let cdp_extra = String::new();
                let full = format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection {render_arg}{cdp_extra}"
                );
                // 记录本次启动实际生效的参数串：JS 端创建的子窗口必须使用
                // 完全相同的 WebView2 启动参数，否则同 user-data-dir 下新建
                // 环境会静默失败，产生无法加载页面的"幽灵窗口"
                let main_window_args = full.clone();
                wb = wb.additional_browser_args(&full);
                let main_window = match wb.build() {
                    Ok(w) => w,
                    Err(e) => {
                        memlog::fatal("window_build", &e.to_string());
                        return Err(Box::new(e));
                    }
                };
                // 建窗后立刻重算非客户区并手动恢复几何（防「每启动一次窗口长高一点」），
                // 仍早于其余 setup 工作，且不改变「140ms 后才 show」的启动观感。
                #[cfg(windows)]
                restore_main_window_state(&main_window);
                db::set_setting(&conn, "render.webviewArgs.applied", &main_window_args);
                memlog::stage("setup:window");
            }

            let covers_dir = data_dir.join("covers");
            std::fs::create_dir_all(&covers_dir).ok();
            db::reset_epub_covers(&conn, &covers_dir);
            // 封面自愈（每个进程最多一次）：cover_path 指向的文件已不存在时复位，交给扫描重建。
            // 必须落在 create_dir_all 之后、ensure_scan_worker（下一步）之前：worker 一旦开始认领，
            // 复位就会与在途的 meta_done=1 记录交叉（那本书会被当成"已完成"跳过，封面永远是空的）。
            // manual 模式下整段跳过：不复位、也不写任何标记。
            match db::get_setting(&conn, library::COVER_SCAN_MODE_KEY).as_deref() {
                Some("manual") => {
                    memlog::stage("setup:cover_heal_skipped_manual");
                }
                Some(_) => {
                    let rep = db::heal_missing_covers(&conn, &covers_dir);
                    library::mark_cover_heal_done();
                    library::log_cover_heal(&rep);
                    memlog::stage(&format!(
                        "setup:cover_heal scanned={} reset={} custom={}",
                        rep.scanned, rep.reset, rep.reset_custom
                    ));
                }
                // 键还不存在（升级后第一次启动）：此时无法判断手动/自动，先不动；
                // 前端启动时必定推一次 set_cover_scan，由那里补做（模式未知时不猜）。
                None => {
                    memlog::stage("setup:cover_heal_deferred");
                }
            }
            memlog::stage("setup:covers_dir");
            // 共享 client 不再全局放宽 TLS：公网目标严格校验证书。按目标本机/私有
            // 选择 client 的逻辑集中在 ai::build_client（带缓存），此字段仅作通用兜底。
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default();
            let state = Arc::new(AppState {
                db: db::Db(Mutex::new(conn)),
                app: app.handle().clone(),
                watcher: Mutex::new(None),
                last_fs_event: std::sync::atomic::AtomicU64::new(0),
                fs_paths: Mutex::new(Vec::new()),
                rules_ver: std::sync::atomic::AtomicU64::new(1),
                txt_cache: Mutex::new(std::collections::HashMap::new()),
                epub_cache: Mutex::new(std::collections::HashMap::new()),
                epub_stats_cache: Mutex::new(std::collections::HashMap::new()),
                chapter_cache: Mutex::new(library::ChapterCache::new(80)),
                image_cache: Mutex::new(std::collections::HashMap::new()),
                http,
                scan_running: std::sync::atomic::AtomicBool::new(false),
                scan_stop: std::sync::atomic::AtomicBool::new(false),
                scan_start_count: std::sync::atomic::AtomicU64::new(0),
                cover_scan_done: std::sync::atomic::AtomicU64::new(0),
                cover_scan_total: std::sync::atomic::AtomicU64::new(0),
                cover_scan_enabled: std::sync::atomic::AtomicBool::new(true),
                cover_scan_ws_base: std::sync::atomic::AtomicU64::new(0),
                cover_style_ver: std::sync::atomic::AtomicU64::new(1),
            });
            app.manage(state.clone());

            // 冷启动增量扫描：处理全部 `meta_done=0`（含存量与新增），逐本提交、扫完即静默。
            // 不再一次性把全部未处理 id 载入内存；worker 内部分批查询 + 限流（见 library::on_startup）。
            memlog::stage("setup:state_ready");
            library::ensure_scan_worker(&app.handle().clone());

            {
                let st = state.clone();
                let folders: Vec<String> = {
                    let conn = st.db.0.lock().unwrap();
                    db::list_folders(&conn)
                };
                // 启动不再做全量扫描（数万本要数分钟，期间前端进度恒为 0，看起来像卡死）。
                // 只安装监听：新增书籍由监听触发的、限定目录范围的扫描补齐；存量指纹刷新另做。
                if !folders.is_empty() {
                    library::install_watcher(&st);
                }
            }

            let app2 = app.handle().clone();
            std::thread::spawn(move || {
                library::spawn_watcher_loop(app2);
            });

            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(140));
                    use tauri::Manager;
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                });
            }

            Ok(())
        })
        .register_uri_scheme_protocol("tidepage", |ctx, request| {
            use tauri::Manager;
            let path = request.uri().path().trim_start_matches('/');
            memlog::watch("proto");
            let proto_requests = PROTO_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if proto_requests == 1 || proto_requests % 2000 == 0 {
                memlog::stage_note("proto:count", &format!("requests={proto_requests} path={path}"));
            }
            let dir = covers::covers_dir(ctx.app_handle());
            let dc_theme = parse_dc_theme(request.uri().query());

            let (mime, bytes) = if let Some(raw) = path.strip_prefix("epubimg/") {
                // EPUB 正文插图按需读取：epubimg/<book_id>/<b64url(zip内路径)>
                let (bid_s, enc) = raw.split_once('/').unwrap_or((raw, ""));
                let bid: i64 = bid_s.parse().unwrap_or(0);
                let target = {
                    use base64::Engine;
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(enc.as_bytes())
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .unwrap_or_default()
                };
                if bid <= 0 || target.is_empty() || target.contains("..") {
                    ("image/png".to_string(), Vec::new())
                } else {
                    match ctx
                        .app_handle()
                        .try_state::<std::sync::Arc<library::AppState>>()
                        .and_then(|st| library::read_epub_image(&st, bid, &target))
                    {
                        Some((m, b)) => (m, b),
                        None => ("image/png".to_string(), Vec::new()),
                    }
                }
            } else if let Some(raw) = path.strip_prefix("cover-default/") {
                // F2 共享默认封面：底图 + 实时叠字（书名/作者从 DB 取，不缓存每本封面）。
                let bid: i64 = raw.trim_end_matches('/').parse().unwrap_or(0);
                match ctx
                    .app_handle()
                    .try_state::<std::sync::Arc<library::AppState>>()
                {
                    Some(st) => {
                        let svg = library::render_default_cover(&st, bid, &dc_theme);
                        ("image/svg+xml".to_string(), svg.into_bytes())
                    }
                    None => (
                        "image/svg+xml".to_string(),
                        covers::placeholder_svg("未命名", "").into_bytes(),
                    ),
                }
            } else if let Some(raw) = path.strip_prefix("book-cover/") {
                // per-book 封面端点：/book-cover/<id>?v=<cover_style_ver>-<cover_ver>
                // id → DB cover_path → 文件；无内嵌封面时回落共享默认封面（底图+实时叠字）。
                // URL 含 per-book 分量（cover_ver），提取出新封面后立即替换底图、不吃缓存。
                let bid: i64 = raw.trim_end_matches('/').parse().unwrap_or(0);
                match ctx
                    .app_handle()
                    .try_state::<std::sync::Arc<library::AppState>>()
                {
                    Some(st) => {
                        let name = {
                            let conn = st.db.0.lock().unwrap();
                            db::get_book(&conn, bid).and_then(|b| b.cover_path)
                        };
                        // 与 covers/ 端点同一套校验与读取上限：只接受单一文件名，32 MiB 封顶。
                        let name = name.unwrap_or_default();
                        let served = resolve_under(&covers::covers_dir(ctx.app_handle()), &name)
                            .and_then(|f| {
                                read_protocol_file(&f).map(|b| (sniff_mime(&b).to_string(), b))
                            });
                        match served {
                            Some(v) => v,
                            None => {
                                let svg = library::render_default_cover(&st, bid, &dc_theme);
                                ("image/svg+xml".to_string(), svg.into_bytes())
                            }
                        }
                    }
                    None => (
                        "image/svg+xml".to_string(),
                        covers::placeholder_svg("未命名", "").into_bytes(),
                    ),
                }
            } else if let Some(raw) = path.strip_prefix("covers/") {
                // 解码后的名字必须仍是 covers 目录下的单一文件名：绝对路径/盘符/穿越一律拒绝。
                let name = percent_decode(raw);
                let base = covers::covers_dir(ctx.app_handle());
                let served = resolve_under(&base, &name).and_then(|f| {
                    read_protocol_file(&f).map(|b| (f, b))
                });
                match served {
                    Some((file, b)) => {
                        let ext = file
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        let m = match ext.as_str() {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "webp" => "image/webp",
                            "svg" => "image/svg+xml",
                            _ => "application/octet-stream",
                        };
                        (m.to_string(), b)
                    }
                    None => (
                        "image/svg+xml".to_string(),
                        covers::placeholder_svg("", "").into_bytes(),
                    ),
                }
            } else if let Some(raw) = path.strip_prefix("res/") {
                // MOBI/AZW3 提取出的正文插图：res/<book_id>/<file>，路径固定两段。
                // book_id 只接受纯数字目录名；文件名再走 resolve_under（只允许单一文件名）。
                let decoded = percent_decode(raw);
                let segs: Vec<&str> = decoded.split('/').collect();
                let served = if segs.len() == 2
                    && !segs[0].is_empty()
                    && segs[0].bytes().all(|b| b.is_ascii_digit())
                {
                    let base = dir.parent().unwrap_or(&dir).join("res").join(segs[0]);
                    resolve_under(&base, segs[1])
                        .and_then(|f| read_protocol_file(&f).map(|b| (f, b)))
                } else {
                    None
                };
                match served {
                    Some((file, b)) => {
                        let ext = file
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        let m = match ext.as_str() {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "webp" => "image/webp",
                            "bmp" => "image/bmp",
                            _ => "application/octet-stream",
                        };
                        (m.to_string(), b)
                    }
                    None => ("image/png".to_string(), Vec::new()),
                }
            } else if let Some(raw) = path.strip_prefix("bg/") {
                // 与 covers/res/epubimg 一致：空名、`..` 或含路径分隔的越界名一律拒绝，
                // 规范化为纯文件名后再读取，避免任意文件读取。
                let name = percent_decode(raw).replace('\\', "/");
                let file_name = name.rsplit('/').next().unwrap_or("");
                if name.is_empty()
                    || name.contains("..")
                    || file_name.is_empty()
                    || file_name.contains(':')
                {
                    ("image/png".into(), Vec::new())
                } else {
                    match std::fs::read(dir.join(file_name)) {
                        Ok(b) => (sniff_mime(&b).to_string(), b),
                        Err(_) => ("image/png".into(), Vec::new()),
                    }
                }
            } else if let Some(name) = path.strip_prefix("fonts/") {
                let file = name.replace('/', "").replace('\\', "");
                if file.contains("..") {
                    ("text/plain".into(), Vec::new())
                } else {
                    match std::fs::read(dir.parent().unwrap_or(&dir).join("fonts").join(&file)) {
                        Ok(b) => ("font/ttf".into(), b),
                        Err(_) => ("text/plain".into(), Vec::new()),
                    }
                }
            } else {
                (
                    "image/svg+xml".into(),
                    covers::placeholder_svg("", "").into_bytes(),
                )
            };
            if bytes.len() > 1024 * 1024 {
                memlog::stage_note(
                    "proto:big",
                    &format!("path={path} bytes={} mime={mime}", bytes.len()),
                );
            }
            memlog::watch("proto:body");
            // 仅跨域加载字体需要 CORS（@font-face 受 CORS 限制）；图片/封面由
            // <img>/CSS 直接显示，无需也不应回显 `*`，收紧为仅字体响应放行。
            let mut builder = tauri::http::Response::builder()
                .status(200)
                .header("Content-Type", mime);
            if path.starts_with("fonts/") {
                builder = builder.header("Access-Control-Allow-Origin", "*");
            }
            // 图片类 URL 由内容本身决定，可长期缓存：epubimg/<book_id>/<b64url(zip内路径)> 与
            // res/<book_id>/<提取文件名> 只在书内图片被替换（重新导入该书）时才变；covers/ 与
            // book-cover/<id>?v=<coverVer>、cover-default/<id>?…&db=<n> 的 URL 自带版本参数。
            // 目的：被重建的 .manga-block 直接命中内存/磁盘缓存，避免同一张几百 KB 的图反复
            // 重新解压 zip 条目、重新下载与重新解码。
            if let Some(cc) = cache_control_for(path) {
                builder = builder.header("Cache-Control", cc);
            }
            builder.body(bytes).unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            library::list_folders,
            library::add_folder,
            library::remove_folder,
            library::list_books,
            library::list_books_page,
            library::list_order_index,
            library::list_formats,
            library::set_read_state,
            library::list_books_by_ids,
            library::rescan,
            library::refresh_library,
            library::file_exists,
            library::get_last_scan_stale,
            library::path_is_dir,
            library::open_book,
            library::get_chapter,
            library::list_book_images,
            library::save_progress,
            library::load_progress,
            library::delete_books,
            library::preview_split,
            library::save_split_rules,
            library::convert_book,
            library::pick_file,
            library::pick_folder,
            library::pick_save,
            library::export_ai,
            library::set_mobi_encoding,
            library::set_custom_cover,
            library::read_book_bytes,
            library::pdf_file_size,
            // 超大 PDF 的原生渲染通道（见 pdfnative.rs）
            pdfnative::pdf_native_open,
            pdfnative::pdf_native_page_geometry,
            pdfnative::pdf_native_render_page,
            pdfnative::pdf_native_close,
    pdfscan::pdf_scan_open,
    pdfscan::pdf_scan_page,
    pdfscan::pdf_scan_page_bytes,
    pdfscan::pdf_scan_outline,
    pdfscan::pdf_scan_close,
            library::pdf_read_range,
            library::set_book_cover_image,
            library::set_cover_scan,
            library::library_ready,
            library::list_book_ids_page,
            library::pdf_books_missing_cover,
            library::set_pdf_page_text,
            library::webdav_test,
            library::webdav_backup_settings,
            library::webdav_restore_settings,
            library::webdav_sync_progress,
            library::webdav_read_progress,
            library::webdav_upload_book,
            library::webdav_download_books,
            library::webdav_download_book,
            library::webdav_list,
            library::webdav_delete,
            library::webdav_copy,
            library::webdav_move,
            library::webdav_download_file,
            library::webdav_download_progress,
            library::set_setting_kv,
            library::get_setting_kv,
            library::app_data_dir,
            library::storage_stats,
            library::clear_cache,
            library::meta_debug_stats,
            library::scan_progress,
            library::prioritize_covers,
            library::set_default_cover,
            library::clear_default_cover,
            library::has_default_cover,
            library::add_bookmark,
            library::list_bookmarks,
            library::delete_bookmark,
            library::add_stats,
            library::get_stats,
            library::ai_chat,
            library::ai_test,
            library::tts_synth,
            library::tts_voices,
            library::tts_ensure_fixed_voice,
            library::search_all,
            library::search_in_book,
            library::ensure_book_text_cmd,
            library::add_highlight,
            library::book_note_info,
            library::list_note_books,
            library::list_notes_of_book,
            library::set_highlight_note,
            library::append_highlight_note,
            library::list_highlights,
            library::search_highlights,
            library::delete_highlight,
            library::update_highlight,
            library::update_book_meta,
            library::write_book_meta,
            library::rebuild_fts,
            library::index_book_fts,
            library::reveal_in_explorer,
            library::set_background_image,
            library::clear_background_image,
            library::random_book,
            library::reextract_cover,
            library::start_reading_session,
            library::update_reading_session,
            library::get_book_sessions,
            library::suggest_meta,
            library::add_dropped_book,
            library::get_book_word_count,
            library::book_char_counts,
            library::random_book_filtered,
            raise_above,
            secret::secret_write,
            secret::secret_read,
            update::check_update,
            backup::host_name,
            backup::backup_seal,
            backup::backup_open,
            fonts::list_system_fonts,
            fonts::install_font_file,
            fonts::list_custom_fonts,
            fonts::delete_custom_font,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_paths_are_immutably_cached() {
        for p in [
            "epubimg/42/aW1nLzAucG5n",
            "res/42/r00001.jpg",
            "covers/a.png",
            "book-cover/42",
            "cover-default/42",
        ] {
            assert_eq!(
                cache_control_for(p),
                Some("public, max-age=31536000, immutable"),
                "{p} 应长期缓存"
            );
        }
        // 背景图：新图按时间戳+序号唯一命名（换图即换 URL），但端点仍要照顾历史遗留的同名文件
        assert_eq!(cache_control_for("bg/a.png"), Some("public, max-age=60"));
        assert_eq!(cache_control_for("bg/bg-1-0.png"), Some("public, max-age=60"));
        // 字体等保持原行为
        assert_eq!(cache_control_for("fonts/a.ttf"), None);
        assert_eq!(cache_control_for("other"), None);
    }

    /// 协议端点资源名解析：绝对路径、盘符、路径分隔与上跳目录一律拒绝。
    #[test]
    fn resolve_under_rejects_absolute_and_traversal() {
        let base = std::path::Path::new("C:\\tidepage\\covers");
        for bad in [
            "",
            ".",
            "..",
            "../a.png",
            "..\\a.png",
            "/etc/passwd",
            "C:/Windows/win.ini",
            "C:\\Windows\\win.ini",
            "covers/a.png",
            "a/b.png",
            "a\\b.png",
        ] {
            assert!(resolve_under(base, bad).is_none(), "{bad:?} 必须拒绝");
        }
        // 合法单一文件名（文件不存在时同样返回 None，但不会误判为越界）。
        assert_eq!(resolve_under(base, "ok.png"), None);
    }
}
