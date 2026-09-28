mod ai;
mod bookmeta;
mod convert;
mod covers;
mod dav;
mod db;
mod epub;
mod fonts;
mod library;
mod mobi;
mod models;
mod txt;

use library::AppState;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 应用数据目录（数据库/封面/字体/提取资源）。
/// 默认使用系统的 app_data_dir；当设置了 `FLOWREAD_DATA_DIR` 时改为该目录，
/// 用于「便携模式」以及测试时的数据隔离（默认不设置 → 行为完全不变）。
pub fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("FLOWREAD_DATA_DIR") {
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
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
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
fn parse_dc_theme(query: Option<&str>) -> library::DcTheme {
    let mut t = library::DcTheme::default();
    let Some(q) = query else { return t };
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let val = percent_decode(v);
        match k {
            "g1" => t.g1 = val,
            "g2" => t.g2 = val,
            "g3" => t.g3 = val,
            "tc" => t.tc = val,
            _ => {}
        }
    }
    t
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            use tauri::Manager;
            let data_dir = data_dir(app.handle());
            std::fs::create_dir_all(&data_dir).ok();
            let conn = db::init(&data_dir.join("flowread.db"))?;
            if db::has_fts(&conn) {
                let _ = db::ensure_fts(&conn);
            }
            db::migrate_book_ids(&conn);

            // 创建主窗口：注入 WebView2 字体渲染参数（需重启生效）。
            // 设置页现已简化为单个「灰度抗锯齿」复选框，只写入
            // --disable-lcd-text（勾选）或 --enable-lcd-text（取消）；
            // 旧版 GDI/自定义参数值仍原样尊重，保证升级用户配置不失效。
            let mut main_window_args = String::new();
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
                    .title("FlowRead 阅读")
                    .inner_size(1380.0, 880.0)
                    .min_inner_size(940.0, 620.0)
                    .center()
                    .visible(false);
                // wry 在设置了 additional_browser_args 时会替换默认参数，这里补回默认项
                // wry 程序化参数优先于 WEBVIEW2_* 环境变量，故 CDP 端口只能在此注入；
                // 仅 debug 构建编译该分支（release 二进制不含此代码路径与字符串）。
                // 严格校验堵住“绕过测试侧守卫直接启动 debug exe”：
                // ① 必须 ASCII 纯数字（不用 trim——空格/制表符/加号/附加参数一律拒绝）
                // ② 再 parse::<u16>（超范围拒绝）③ 拒绝 0 与 9222
                // 注入串只由解析后的数字重建；固定仅监听 127.0.0.1。
                #[cfg(debug_assertions)]
                let cdp_extra = match std::env::var("FR_TEST_CDP_PORT") {
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
                main_window_args = full.clone();
                wb = wb.additional_browser_args(&full);
                wb.build()?;
                db::set_setting(&conn, "render.webviewArgs.applied", &main_window_args);
            }

            let covers_dir = data_dir.join("covers");
            std::fs::create_dir_all(&covers_dir).ok();
            db::reset_epub_covers(&conn, &covers_dir);
            let http = reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default();
            let state = Arc::new(AppState {
                db: db::Db(Mutex::new(conn)),
                app: app.handle().clone(),
                watcher: Mutex::new(None),
                last_fs_event: std::sync::atomic::AtomicU64::new(0),
                rules_ver: std::sync::atomic::AtomicU64::new(1),
                txt_cache: Mutex::new(std::collections::HashMap::new()),
                epub_cache: Mutex::new(std::collections::HashMap::new()),
                chapter_cache: Mutex::new(library::ChapterCache::new(80)),
                image_cache: Mutex::new(std::collections::HashMap::new()),
                http,
                scan_running: std::sync::atomic::AtomicBool::new(false),
                scan_stop: std::sync::atomic::AtomicBool::new(false),
                scan_start_count: std::sync::atomic::AtomicU64::new(0),
                cover_scan_done: std::sync::atomic::AtomicU64::new(0),
                cover_scan_total: std::sync::atomic::AtomicU64::new(0),
                cover_style_ver: std::sync::atomic::AtomicU64::new(1),
            });
            app.manage(state.clone());

            // 冷启动增量扫描：处理全部 `meta_done=0`（含存量与新增），逐本提交、扫完即静默。
            // 不再一次性把全部未处理 id 载入内存；worker 内部分批查询 + 限流（见 library::on_startup）。
            library::ensure_scan_worker(&app.handle().clone());

            {
                let st = state.clone();
                let folders: Vec<String> = {
                    let conn = st.db.0.lock().unwrap();
                    db::list_folders(&conn)
                };
                if !folders.is_empty() {
                    let st2 = st.clone();
                    tauri::async_runtime::spawn(async move {
                        library::scan_folders(&st2).await;
                    });
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
        .register_uri_scheme_protocol("flowread", |ctx, request| {
            use tauri::Manager;
            let path = request.uri().path().trim_start_matches('/');
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
                        let safe = name.unwrap_or_default();
                        let valid = !safe.is_empty()
                            && !safe.contains("..")
                            && !safe.contains('/')
                            && !safe.contains('\\');
                        let served = if valid {
                            std::fs::read(covers::covers_dir(ctx.app_handle()).join(&safe))
                                .ok()
                                .map(|b| (sniff_mime(&b).to_string(), b))
                        } else {
                            None
                        };
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
                let name = percent_decode(raw);
                let safe = name.replace('\\', "/");
                let file = covers::covers_dir(ctx.app_handle()).join(&safe);
                if safe.is_empty() || safe.contains("..") || !file.is_file() {
                    (
                        "image/svg+xml".to_string(),
                        covers::placeholder_svg("", "").into_bytes(),
                    )
                } else {
                    let ext = safe.rsplit('.').next().unwrap_or("").to_lowercase();
                    match std::fs::read(&file) {
                        Ok(b) => {
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
                        Err(_) => (
                            "image/svg+xml".to_string(),
                            covers::placeholder_svg("", "").into_bytes(),
                        ),
                    }
                }
            } else if let Some(raw) = path.strip_prefix("res/") {
                // MOBI/AZW3 提取出的正文插图：res/<book_id>/<file>
                let name = percent_decode(raw);
                let safe = name.replace('\\', "/");
                let file = dir.parent().unwrap_or(&dir).join("res").join(&safe);
                if safe.is_empty() || safe.contains("..") || !file.is_file() {
                    ("image/png".to_string(), Vec::new())
                } else {
                    let ext = safe.rsplit('.').next().unwrap_or("").to_lowercase();
                    let m = match ext.as_str() {
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "gif" => "image/gif",
                        "webp" => "image/webp",
                        "bmp" => "image/bmp",
                        _ => "application/octet-stream",
                    };
                    match std::fs::read(&file) {
                        Ok(b) => (m.to_string(), b),
                        Err(_) => ("image/png".to_string(), Vec::new()),
                    }
                }
            } else if let Some(name) = path.strip_prefix("bg/") {
                match std::fs::read(dir.join(name)) {
                    Ok(b) => (sniff_mime(&b).to_string(), b),
                    Err(_) => ("image/png".into(), Vec::new()),
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
            tauri::http::Response::builder()
                .status(200)
                .header("Content-Type", mime)
                .header("Access-Control-Allow-Origin", "*")
                .body(bytes)
                .unwrap()
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
            library::webdav_test,
            library::webdav_sync_progress,
            library::webdav_read_progress,
            library::webdav_upload_book,
            library::webdav_download_books,
            library::webdav_download_book,
            library::webdav_list,
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
            fonts::list_system_fonts,
            fonts::install_font_file,
            fonts::list_custom_fonts,
            fonts::delete_custom_font,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
