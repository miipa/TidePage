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
use std::sync::{Arc, Mutex};

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
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("无法获取数据目录");
            std::fs::create_dir_all(&data_dir).ok();
            let conn = db::init(&data_dir.join("flowread.db"))?;
            if db::has_fts(&conn) {
                let _ = db::ensure_fts(&conn);
            }
            db::migrate_book_ids(&conn);

            // 创建主窗口：注入 WebView2 字体渲染参数（需重启生效）。
            // 根治粗体第三方字体（仓耳云黑等）在 ClearType 子像素渲染下"黑成一坨/发糊"：
            // --disable-lcd-text（灰度抗锯齿）现为强制默认项，不再依赖用户手动配置；
            // 显式写 --enable-lcd-text 可恢复子像素，--disable-direct-write（GDI 模式）原样尊重。
            let mut main_window_args = String::new();
            {
                let webview_args = db::get_setting(&conn, "render.webviewArgs").unwrap_or_default();
                let raw = webview_args.trim().to_string();
                let render_arg = if raw.is_empty() {
                    "--disable-lcd-text".to_string()
                } else if raw.contains("disable-lcd-text")
                    || raw.contains("disable-direct-write")
                    || raw.contains("enable-lcd-text")
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
                // wry 程序化参数优先于 WEBVIEW2_* 环境变量，故 CDP 端口只能在此注入：
                // 源码不含任何固定端口——仅当运行环境显式提供 FR_TEST_CDP_PORT（受守卫测试
                // 启动器注入、构建期拦截 9222）才开启远程调试；生产/发布环境无此变量=永无调试端口。
                let cdp_extra = match std::env::var("FR_TEST_CDP_PORT") {
                    Ok(p) if !p.is_empty() && p != "9222" => format!(" --remote-debugging-port={p}"),
                    _ => String::new(),
                };
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
                http,
            });
            app.manage(state.clone());

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
            let path = request.uri().path().trim_start_matches('/');
            let dir = covers::covers_dir(ctx.app_handle());

            let (mime, bytes) = if let Some(raw) = path.strip_prefix("covers/") {
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
            library::rescan,
            library::open_book,
            library::get_chapter,
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
            library::webdav_upload_book,
            library::webdav_download_books,
            library::webdav_moon_progress,
            library::webdav_moon_write,
            library::webdav_moon_read_po,
            library::webdav_moon_write_po,
            library::webdav_moon_po_count,
            library::webdav_list,
            library::webdav_download_file,
            library::webdav_download_progress,
            library::set_setting_kv,
            library::get_setting_kv,
            library::app_data_dir,
            library::add_bookmark,
            library::list_bookmarks,
            library::delete_bookmark,
            library::add_stats,
            library::get_stats,
            library::ai_chat,
            library::ai_test,
            library::search_all,
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
