use crate::models::{BookRow, Progress};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Db(pub Mutex<Connection>);

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn path_id(path: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h & 0xF_FFFF_FFFF_FFFF) as i64
}

pub fn init(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS folders(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT UNIQUE NOT NULL
        );
        CREATE TABLE IF NOT EXISTS books(
            id INTEGER PRIMARY KEY,
            path TEXT UNIQUE NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            author TEXT NOT NULL DEFAULT '',
            format TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            mtime INTEGER NOT NULL DEFAULT 0,
            added_at INTEGER NOT NULL DEFAULT 0,
            last_opened INTEGER NOT NULL DEFAULT 0,
            cover_path TEXT,
            meta_done INTEGER NOT NULL DEFAULT 0,
            total_chars INTEGER NOT NULL DEFAULT 0,
            total_text_chars INTEGER NOT NULL DEFAULT 0,
            cover_ver INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_books_title ON books(title);
        CREATE INDEX IF NOT EXISTS idx_books_title_nocase ON books(title COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_books_author ON books(author);
        CREATE INDEX IF NOT EXISTS idx_books_added ON books(added_at DESC);
        CREATE INDEX IF NOT EXISTS idx_books_mtime ON books(mtime DESC);
        -- 复合覆盖索引：让 ORDER BY <列> DESC, id DESC + LIMIT/OFFSET 走索引顺序，
        -- 避免对 16 万行做临时 B-tree 排序（深 offset 场景）。
        CREATE INDEX IF NOT EXISTS idx_books_added_id ON books(added_at DESC, id DESC);
        CREATE INDEX IF NOT EXISTS idx_books_mtime_id ON books(mtime DESC, id DESC);
        CREATE INDEX IF NOT EXISTS idx_books_title_nocase_id ON books(title COLLATE NOCASE, id);
        CREATE TABLE IF NOT EXISTS progress(
            book_id INTEGER PRIMARY KEY,
            chapter_index INTEGER NOT NULL DEFAULT 0,
            chapter_ratio REAL NOT NULL DEFAULT 0,
            pct REAL NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0,
            device TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS bookmarks(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id INTEGER NOT NULL,
            chapter_index INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '',
            excerpt TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS highlights(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id INTEGER NOT NULL,
            chapter_index INTEGER NOT NULL DEFAULT 0,
            chapter_title TEXT NOT NULL DEFAULT '',
            pos_start INTEGER NOT NULL DEFAULT 0,
            pos_end INTEGER NOT NULL DEFAULT 0,
            text TEXT NOT NULL DEFAULT '',
            style TEXT NOT NULL DEFAULT 'underline',
            color TEXT NOT NULL DEFAULT '',
            note TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_highlights_book ON highlights(book_id);
        CREATE INDEX IF NOT EXISTS idx_highlights_created ON highlights(created_at DESC);
        -- 每书一行：文件指纹（大小+首尾64KB 哈希）+ 创建划线时的书名/作者快照。
        -- 用于「重导按新文件处理、旧笔记不复活」与「书删后仍能显示书名」的判定；
        -- 不在每条 highlight 上冗余存储。
        CREATE TABLE IF NOT EXISTS book_fingerprint(
            book_id INTEGER PRIMARY KEY,
            size INTEGER NOT NULL DEFAULT 0,
            mtime INTEGER NOT NULL DEFAULT 0,
            hash TEXT NOT NULL DEFAULT '',
            title_snap TEXT NOT NULL DEFAULT '',
            author_snap TEXT NOT NULL DEFAULT ''
        );
        -- 删除书籍且用户选择「保留笔记」时，把书名/作者快照落到此表，
        -- 保证笔记界面在 books 行已消失后仍能显示原书名（而非只有"已删除"）。
        CREATE TABLE IF NOT EXISTS orphan_books(
            book_id INTEGER PRIMARY KEY,
            title_snap TEXT NOT NULL DEFAULT '',
            author_snap TEXT NOT NULL DEFAULT '',
            deleted_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS txt_cache(
            book_id INTEGER PRIMARY KEY,
            offsets TEXT NOT NULL,
            rules_hash TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS chapter_text(
            book_id INTEGER NOT NULL,
            chapter_index INTEGER NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT '',
            PRIMARY KEY(book_id, chapter_index)
        );
        CREATE TABLE IF NOT EXISTS stats(
            day TEXT NOT NULL,
            book_id INTEGER NOT NULL,
            seconds INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(day, book_id)
        );
        CREATE TABLE IF NOT EXISTS reading_sessions(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id INTEGER NOT NULL,
            day TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            seconds INTEGER NOT NULL DEFAULT 0,
            chars INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_book ON reading_sessions(book_id);
        CREATE INDEX IF NOT EXISTS idx_progress_book ON progress(book_id);
        CREATE INDEX IF NOT EXISTS idx_progress_updated ON progress(updated_at DESC);
        CREATE TABLE IF NOT EXISTS settings(
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL DEFAULT ''
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    for col in [
        "total_chars INTEGER NOT NULL DEFAULT 0",
        "total_text_chars INTEGER NOT NULL DEFAULT 0",
        "last_opened INTEGER NOT NULL DEFAULT 0",
        "cover_ver INTEGER NOT NULL DEFAULT 0",
    ] {
        let name = col.split(' ').next().unwrap_or("");
        let exists: bool = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('books') WHERE name=?1",
                params![name],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !exists {
            conn.execute(&format!("ALTER TABLE books ADD COLUMN {col}"), [])
                .ok();
        }
    }
    // highlights.color：旧库补列（新库建表时已含）
    let has_hl_color: bool = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('highlights') WHERE name='color'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if !has_hl_color {
        conn.execute("ALTER TABLE highlights ADD COLUMN color TEXT NOT NULL DEFAULT ''", [])
            .ok();
    }
    Ok(conn)
}

pub fn migrate_book_ids(conn: &Connection) {
    let rows: Vec<(i64, String)> = {
        let mut stmt = match conn.prepare("SELECT id, path FROM books") {
            Ok(s) => s,
            Err(_) => return,
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rs| rs.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    };
    let once: bool = get_setting(conn, "id_migrated_v2").is_none();
    if once {
        for (old_id, path) in &rows {
            let new_id = path_id(path);
            if new_id == *old_id {
                continue;
            }
            let occupied: bool = conn
                .query_row("SELECT 1 FROM books WHERE id=?1", params![new_id], |_| Ok(true))
                .optional()
                .unwrap_or(None)
                .is_some();
            if occupied {
                continue;
            }
            if conn
                .execute("UPDATE books SET id=?1 WHERE id=?2", params![new_id, old_id])
                .is_ok()
            {
                for table in ["progress", "txt_cache", "bookmarks", "reading_sessions", "highlights"] {
                    conn.execute(
                        &format!("UPDATE {table} SET book_id=?1 WHERE book_id=?2"),
                        params![new_id, old_id],
                    )
                    .ok();
                }
                conn.execute(
                    "UPDATE chapter_text SET book_id=?1 WHERE book_id=?2",
                    params![new_id, old_id],
                )
                .ok();
            }
        }
        set_setting(conn, "id_migrated_v2", "1");
    }
}

pub fn reset_epub_covers(conn: &Connection, covers_dir: &std::path::Path) {
    const KEY: &str = "meta_reset_v4";
    if get_setting(conn, KEY).is_some() {
        return;
    }
    conn
        .execute(
            "UPDATE books SET cover_path=NULL, meta_done=0 WHERE meta_done=1",
            [],
        )
        .ok();
    if let Ok(entries) = std::fs::read_dir(covers_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".svg") || name.ends_with(".png") {
                std::fs::remove_file(e.path()).ok();
            }
        }
    }
    set_setting(conn, "cover_reset_done", "1");
    set_setting(conn, "cover_reset_v2", "1");
    set_setting(conn, "cover_reset_v3", "1");
    set_setting(conn, KEY, "1");
}

pub fn has_fts(conn: &Connection) -> bool {
    conn.query_row("SELECT count(*) FROM pragma_compile_options WHERE compile_options LIKE '%ENABLE_FTS5%'", [], |r| r.get::<_, i64>(0))
        .map(|c| c > 0)
        .unwrap_or(false)
}

pub fn ensure_fts(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(book_id UNINDEXED, chapter_index UNINDEXED, title, body, tokenize='trigram');",
    )
    .map_err(|e| e.to_string())
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key=?1", params![key], |r| r.get(0))
        .optional()
        .unwrap_or(None)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
        params![key, value],
    )
    .ok();
}

pub fn add_folder(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("INSERT OR IGNORE INTO folders(path) VALUES(?1)", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_folder(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM folders WHERE path=?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_folders(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT path FROM folders ORDER BY id") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    rows
}

fn book_from_row(r: &Row) -> rusqlite::Result<BookRow> {
    Ok(BookRow {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        author: r.get(3)?,
        format: r.get(4)?,
        size: r.get(5)?,
        mtime: r.get(6)?,
        added_at: r.get(7)?,
        cover_path: r.get(8)?,
        meta_done: r.get(9)?,
        last_read: r.get(10)?,
        total_chars: r.get(11)?,
        total_text_chars: r.get(12)?,
        cover_ver: r.get(13)?,
    })
}

/// `last_read`（下标 10）= 阅读进度更新时间与「最后打开时间」取大者：
/// 这样只要打开过（哪怕打开失败）就会进入「按最近阅读」，不再依赖是否产生过进度。
const BOOK_COLS: &str = "b.id, b.path, b.title, b.author, b.format, b.size, b.mtime, b.added_at, b.cover_path, b.meta_done, MAX(COALESCE(p.updated_at, 0), COALESCE(b.last_opened, 0)), COALESCE(b.total_chars, 0), COALESCE(b.total_text_chars, 0), b.cover_ver";

/// 记录「最近打开」时间（不改动阅读进度）。打开失败的书也能出现在「按最近阅读」里。
pub fn touch_book_opened(conn: &Connection, book_id: i64) {
    conn.execute(
        "UPDATE books SET last_opened=?1 WHERE id=?2",
        params![now_ms(), book_id],
    )
    .ok();
}

pub fn upsert_book(
    conn: &Connection,
    path: &str,
    title: &str,
    format: &str,
    size: i64,
    mtime: i64,
) -> Result<i64, String> {
    let id = path_id(path);
    let res = conn.execute(
        "INSERT INTO books(id, path, title, author, format, size, mtime, added_at, meta_done)
         VALUES(?1,?2,?3,'',?4,?5,?6,?7,0)
         ON CONFLICT(path) DO UPDATE SET id=?1, size=?5, mtime=?6, meta_done=0",
        params![id, path, title, format, size, mtime, now_ms()],
    );
    if res.is_err() {
        conn.execute("DELETE FROM books WHERE path=?1", params![path])
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO books(id, path, title, author, format, size, mtime, added_at, meta_done)
             VALUES(?1,?2,?3,'',?4,?5,?6,?7,0)",
            params![id, path, title, format, size, mtime, now_ms()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(id)
}

pub fn list_books(conn: &Connection) -> Vec<BookRow> {
    let sql = format!(
        "SELECT {BOOK_COLS} FROM books b LEFT JOIN progress p ON p.book_id = b.id ORDER BY b.added_at DESC, b.id DESC"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], book_from_row)
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 转义 LIKE 通配符，配合 SQL 的 `ESCAPE '\'` 使用，避免用户输入 `%`/`_` 被当作模式。
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 超大书库分页查询：过滤/排序全部下推 SQLite（带索引），只返回窗口 rows + total。
/// - query 为空不施加过滤；否则按 title/author 模糊匹配（LIKE 对 ASCII 不区分大小写）。
/// - sort 走白名单，绝不拼接用户输入。
/// - limit 上限保护 500，offset 非负。
pub fn list_books_page(
    conn: &Connection,
    query: &str,
    sort: &str,
    offset: i64,
    limit: i64,
) -> crate::models::BookPage {
    let order = match sort {
        "lastread" => "MAX(COALESCE(p.updated_at, 0), COALESCE(b.last_opened, 0)) DESC, b.id DESC",
        "title" => "b.title COLLATE NOCASE ASC, b.id ASC",
        "recent" => "b.mtime DESC, b.id DESC",
        // "added" 为默认
        _ => "b.added_at DESC, b.id DESC",
    };
    let limit = limit.clamp(1, 500);
    let offset = offset.max(0);
    let q = query.trim();
    let pat = if q.is_empty() {
        None
    } else {
        Some(format!("%{}%", escape_like(q)))
    };
    let where_sql = if pat.is_some() {
        " WHERE (b.title LIKE ?1 ESCAPE '\\' OR b.author LIKE ?1 ESCAPE '\\')"
    } else {
        ""
    };

    let total: i64 = {
        let sql = format!("SELECT COUNT(*) FROM books b{where_sql}");
        match &pat {
            Some(p) => conn.query_row(&sql, params![p], |r| r.get(0)).unwrap_or(0),
            None => conn.query_row(&sql, [], |r| r.get(0)).unwrap_or(0),
        }
    };

    let sql = if pat.is_some() {
        format!(
            "SELECT {BOOK_COLS} FROM books b LEFT JOIN progress p ON p.book_id = b.id{where_sql} ORDER BY {order} LIMIT ?2 OFFSET ?3"
        )
    } else {
        format!(
            "SELECT {BOOK_COLS} FROM books b LEFT JOIN progress p ON p.book_id = b.id ORDER BY {order} LIMIT ?1 OFFSET ?2"
        )
    };
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => {
            return crate::models::BookPage { total, rows: vec![] };
        }
    };
    let rows: Vec<BookRow> = match &pat {
        Some(p) => stmt
            .query_map(params![p, limit, offset], book_from_row)
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default(),
        None => stmt
            .query_map(params![limit, offset], book_from_row)
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default(),
    };
    crate::models::BookPage { total, rows }
}

pub fn get_book(conn: &Connection, id: i64) -> Option<BookRow> {
    let sql = format!(
        "SELECT {BOOK_COLS} FROM books b LEFT JOIN progress p ON p.book_id = b.id WHERE b.id=?1"
    );
    conn.query_row(&sql, params![id], book_from_row)
        .optional()
        .unwrap_or(None)
}

pub fn get_book_by_path(conn: &Connection, path: &str) -> Option<BookRow> {
    let sql = format!(
        "SELECT {BOOK_COLS} FROM books b LEFT JOIN progress p ON p.book_id = b.id WHERE b.path=?1"
    );
    conn.query_row(&sql, params![path], book_from_row)
        .optional()
        .unwrap_or(None)
}

pub fn all_book_paths(conn: &Connection) -> Vec<(i64, String)> {
    let mut stmt = match conn.prepare("SELECT id, path FROM books") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 启动续扫用：所有 `meta_done=0`（未处理/上次中断）的书 id。
/// 已扫完的（`meta_done=1`）与可重试的（`meta_done=2`）都不会被捞出。
pub fn ids_needing_meta(conn: &Connection) -> Vec<i64> {
    let mut stmt = match conn.prepare("SELECT id FROM books WHERE meta_done=0") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| r.get::<_, i64>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 按 id 批量取 (id, path, format)：队列 worker 消费时用。
pub fn books_brief_by_ids(conn: &Connection, ids: &[i64]) -> Vec<(i64, String, String)> {
    if ids.is_empty() {
        return vec![];
    }
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT id, path, format FROM books WHERE id IN ({placeholders})");
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
    stmt.query_map(params.as_slice(), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 写入提取结果。`meta_done` 由调用方决定（1=完成/无封面，2=可重试失败），**永不为 0**。
/// `bump_cover_ver=true` 时该书 `cover_ver+1`（提取到新封面，令前端 per-book URL 缓存失效）。
#[allow(clippy::too_many_arguments)]
pub fn set_book_meta_ex(
    conn: &Connection,
    id: i64,
    title: &str,
    author: &str,
    cover_path: Option<&str>,
    total_chars: i64,
    total_text_chars: i64,
    meta_done: i64,
    bump_cover_ver: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET title=?2, author=?3, cover_path=?4, meta_done=?7,
         total_chars=?5, total_text_chars=?6, cover_ver = cover_ver + ?8 WHERE id=?1",
        params![
            id,
            title,
            author,
            cover_path,
            total_chars,
            total_text_chars,
            meta_done,
            if bump_cover_ver { 1 } else { 0 }
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除书籍（G1 分级）。`keep_notes=true` 时保留带笔记的划线（并落 orphan 快照），
/// 否则连同划线一起删；两种路径都删书签与指纹。默认（无参）为「保留笔记」。
pub fn delete_books(conn: &Connection, paths: &[String]) -> Result<(), String> {
    delete_books_policy(conn, paths, true)
}

/// 按策略删除：keep_notes=true 保留带笔记的划线；false 全删。
pub fn delete_books_policy(
    conn: &Connection,
    paths: &[String],
    keep_notes: bool,
) -> Result<(), String> {
    for p in paths {
        delete_book_with_policy(conn, p, keep_notes)?;
        let id = path_id(p);
        if has_fts(conn) {
            conn.execute("DELETE FROM fts WHERE book_id=?1", params![id])
                .ok();
        }
    }
    Ok(())
}

pub fn get_progress(conn: &Connection, book_id: i64) -> Option<Progress> {
    conn.query_row(
        "SELECT chapter_index, chapter_ratio, pct, updated_at, device FROM progress WHERE book_id=?1",
        params![book_id],
        |r| {
            Ok(Progress {
                chapter_index: r.get(0)?,
                chapter_ratio: r.get(1)?,
                pct: r.get(2)?,
                updated_at: r.get(3)?,
                device: r.get(4)?,
            })
        },
    )
    .optional()
    .unwrap_or(None)
}

pub fn set_progress(conn: &Connection, book_id: i64, p: &Progress) -> Result<(), String> {
    conn.execute(
        "INSERT INTO progress(book_id, chapter_index, chapter_ratio, pct, updated_at, device)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(book_id) DO UPDATE SET chapter_index=?2, chapter_ratio=?3, pct=?4, updated_at=?5, device=?6",
        params![book_id, p.chapter_index, p.chapter_ratio, p.pct, p.updated_at, p.device],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_txt_cache(conn: &Connection, book_id: i64) -> Option<(String, String)> {
    conn.query_row(
        "SELECT offsets, rules_hash FROM txt_cache WHERE book_id=?1",
        params![book_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .unwrap_or(None)
}

pub fn set_txt_cache(conn: &Connection, book_id: i64, offsets: &str, rules_hash: &str) {
    conn.execute(
        "INSERT INTO txt_cache(book_id, offsets, rules_hash) VALUES(?1,?2,?3)
         ON CONFLICT(book_id) DO UPDATE SET offsets=?2, rules_hash=?3",
        params![book_id, offsets, rules_hash],
    )
    .ok();
}

pub fn add_bookmark(
    conn: &Connection,
    book_id: i64,
    chapter_index: i64,
    title: &str,
    excerpt: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO bookmarks(book_id, chapter_index, title, excerpt, created_at) VALUES(?1,?2,?3,?4,?5)",
        params![book_id, chapter_index, title, excerpt, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_bookmarks(conn: &Connection, book_id: i64) -> Vec<(i64, i64, String, String, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT id, chapter_index, title, excerpt, created_at FROM bookmarks WHERE book_id=?1 ORDER BY created_at DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(params![book_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

pub fn delete_bookmark(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM bookmarks WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn add_highlight(
    conn: &Connection,
    book_id: i64,
    chapter_index: i64,
    chapter_title: &str,
    pos_start: i64,
    pos_end: i64,
    text: &str,
    style: &str,
    color: &str,
    note: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO highlights(book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn highlight_from_row(r: &Row) -> rusqlite::Result<crate::models::HighlightRow> {
    Ok(crate::models::HighlightRow {
        id: r.get(0)?,
        book_id: r.get(1)?,
        book_title: r.get(2)?,
        chapter_index: r.get(3)?,
        chapter_title: r.get(4)?,
        pos_start: r.get(5)?,
        pos_end: r.get(6)?,
        text: r.get(7)?,
        style: r.get(8)?,
        color: r.get(9)?,
        note: r.get(10)?,
        created_at: r.get(11)?,
    })
}

const HIGHLIGHT_COLS: &str = "h.id, h.book_id, COALESCE(b.title,''), h.chapter_index, h.chapter_title, h.pos_start, h.pos_end, h.text, h.style, h.color, h.note, h.created_at";

pub fn list_highlights(conn: &Connection, book_id: i64) -> Vec<crate::models::HighlightRow> {
    let sql = format!(
        "SELECT {HIGHLIGHT_COLS} FROM highlights h LEFT JOIN books b ON b.id=h.book_id WHERE h.book_id=?1 ORDER BY h.chapter_index, h.pos_start"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(params![book_id], highlight_from_row)
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 全库划线索引查询：按正文/书名/章节名模糊匹配，供「划线」面板检索。
pub fn search_highlights(conn: &Connection, query: &str) -> Vec<crate::models::HighlightRow> {
    let q = query.trim();
    let sql = if q.is_empty() {
        format!("SELECT {HIGHLIGHT_COLS} FROM highlights h LEFT JOIN books b ON b.id=h.book_id ORDER BY h.created_at DESC LIMIT 500")
    } else {
        format!("SELECT {HIGHLIGHT_COLS} FROM highlights h LEFT JOIN books b ON b.id=h.book_id WHERE h.text LIKE ?1 ESCAPE '\\' OR b.title LIKE ?1 ESCAPE '\\' OR h.chapter_title LIKE ?1 ESCAPE '\\' OR h.note LIKE ?1 ESCAPE '\\' ORDER BY h.created_at DESC LIMIT 500")
    };
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    if q.is_empty() {
        stmt.query_map([], highlight_from_row)
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    } else {
        let pat = format!("%{}%", escape_like(q));
        stmt.query_map(params![pat], highlight_from_row)
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }
}

pub fn delete_highlight(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM highlights WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_highlight(
    conn: &Connection,
    id: i64,
    style: &str,
    color: &str,
    note: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE highlights SET style=?2, color=?3, note=?4 WHERE id=?1",
        params![id, style, color, note],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_book_title_author(
    conn: &Connection,
    book_id: i64,
    title: &str,
    author: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET title=?2, author=?3 WHERE id=?1",
        params![book_id, title, author],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn add_stats(conn: &Connection, book_id: i64, seconds: i64) -> Result<(), String> {
    let day = day_string();
    conn.execute(
        "INSERT INTO stats(day, book_id, seconds) VALUES(?1,?2,?3)
         ON CONFLICT(day, book_id) DO UPDATE SET seconds = seconds + ?3",
        params![day, book_id, seconds],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn start_session(conn: &Connection, book_id: i64) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO reading_sessions(book_id, day, started_at, seconds, chars) VALUES(?1,?2,?3,0,0)",
        params![book_id, day_string(), now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn update_session(conn: &Connection, session_id: i64, seconds: i64, chars: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE reading_sessions SET seconds = seconds + ?2, chars = chars + ?3 WHERE id=?1",
        params![session_id, seconds, chars],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_book_sessions(conn: &Connection, book_id: i64) -> Vec<(String, i64, i64, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT day, started_at, seconds, chars FROM reading_sessions WHERE book_id=?1 ORDER BY started_at DESC LIMIT 400",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(params![book_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

pub fn get_stats(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT day, SUM(seconds) FROM reading_sessions WHERE day >= date('now','localtime','-13 day') GROUP BY day ORDER BY day",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

pub fn day_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// ===================== 书指纹 / 孤儿快照（G 数据层） =====================

/// 计算文件指纹：大小 + 首64KB + 尾64KB 做 sha256；<128KB 则全量。
/// 返回 (size, mtime, hash_hex)；文件不可读时 hash 为空。
pub fn file_fingerprint(path: &str) -> (i64, i64, String) {
    use sha2::{Digest, Sha256};
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return (0, 0, String::new()),
    };
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(size.to_le_bytes());
    const CHUNK: u64 = 64 * 1024;
    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::{Read, Seek, SeekFrom};
        if size < 2 * CHUNK {
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_ok() {
                hasher.update(&buf);
            }
        } else {
            let mut head = vec![0u8; CHUNK as usize];
            if f.read_exact(&mut head).is_ok() {
                hasher.update(&head);
            }
            if f.seek(SeekFrom::End(-(CHUNK as i64))).is_ok() {
                let mut tail = vec![0u8; CHUNK as usize];
                if f.read_exact(&mut tail).is_ok() {
                    hasher.update(&tail);
                }
            }
        }
    }
    (size as i64, mtime, format!("{:x}", hasher.finalize()))
}

/// 创建划线时 upsert 该书指纹行（含书名/作者快照）。每书一行，不冗余进 highlight。
pub fn upsert_book_fingerprint(
    conn: &Connection,
    book_id: i64,
    size: i64,
    mtime: i64,
    hash: &str,
    title_snap: &str,
    author_snap: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO book_fingerprint(book_id,size,mtime,hash,title_snap,author_snap) \
         VALUES(?1,?2,?3,?4,?5,?6) \
         ON CONFLICT(book_id) DO UPDATE SET size=?2,mtime=?3,hash=?4,title_snap=?5,author_snap=?6",
        params![book_id, size, mtime, hash, title_snap, author_snap],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 某书是否存在带笔记(note!='')的划线。
pub fn book_has_notes(conn: &Connection, book_id: i64) -> bool {
    conn.query_row(
        "SELECT 1 FROM highlights WHERE book_id=?1 AND note!='' LIMIT 1",
        params![book_id],
        |_| Ok(()),
    )
    .is_ok()
}

/// 删除书籍（G1 分级）：
/// - keep_notes=true：保留 note!='' 的划线，删除纯划线(note='')；bookmarks 全删。
/// - keep_notes=false：highlights/bookmarks 全删。
/// - book_fingerprint 一律删（删除不可恢复，旧笔记永不复活）。
/// - keep_notes 且确有笔记时，先把书名/作者快照写入 orphan_books。
/// 取下一个「孤儿命名空间」唯一 id：负数，且不复用 -path_id。
/// 保证同一路径被多次「删书 + 保留笔记」时，各批笔记互不覆盖。
/// 实现：取所有现有负数 book_id（orphan_books 与 highlights）的最小值再 -1。
fn next_orphan_id(conn: &Connection) -> i64 {
    let min_orphan: i64 = conn
        .query_row("SELECT COALESCE(MIN(book_id), 0) FROM orphan_books", [], |r| r.get(0))
        .unwrap_or(0);
    let min_hl: i64 = conn
        .query_row("SELECT COALESCE(MIN(book_id), 0) FROM highlights", [], |r| r.get(0))
        .unwrap_or(0);
    let min_neg = min_orphan.min(min_hl).min(0);
    min_neg - 1
}

pub fn delete_book_with_policy(
    conn: &Connection,
    path: &str,
    keep_notes: bool,
) -> Result<(), String> {
    let id = path_id(path);
    // 保留笔记：先把书名/作者快照落到 orphan_books，并把带笔记的划线改挂到
    // 「孤儿命名空间」唯一 id（负数，**不复用 -path_id**，避免同一路径多次删书时
    // 两批笔记同 key 相互覆盖/污染）。同路径重导的新书是正 id，绝不会命中旧笔记。
    if keep_notes && book_has_notes(conn, id) {
        let orphan_id = next_orphan_id(conn);
        let snap: Option<(String, String)> = conn
            .query_row(
                "SELECT title_snap, author_snap FROM book_fingerprint WHERE book_id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap_or(None);
        let (t, a) = snap
            .or_else(|| {
                conn.query_row("SELECT title, author FROM books WHERE id=?1", params![id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .optional()
                .unwrap_or(None)
            })
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO orphan_books(book_id,title_snap,author_snap,deleted_at) VALUES(?1,?2,?3,?4) \
             ON CONFLICT(book_id) DO UPDATE SET title_snap=?2,author_snap=?3,deleted_at=?4",
            params![orphan_id, t, a, now_ms()],
        )
        .ok();
        // 把带笔记的划线移入孤儿命名空间；纯划线删除。
        conn.execute(
            "UPDATE highlights SET book_id=?2 WHERE book_id=?1 AND note!=''",
            params![id, orphan_id],
        )
        .ok();
        conn.execute("DELETE FROM highlights WHERE book_id=?1 AND note=''", params![id]).ok();
    } else if keep_notes {
        // 无笔记：直接删（下面 highlights 全删）
        conn.execute("DELETE FROM highlights WHERE book_id=?1", params![id]).ok();
    } else {
        conn.execute("DELETE FROM highlights WHERE book_id=?1", params![id]).ok();
    }
    // 书本身与关联数据
    conn.execute("DELETE FROM books WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM progress WHERE book_id=?1", params![id]).ok();
    conn.execute("DELETE FROM txt_cache WHERE book_id=?1", params![id]).ok();
    conn.execute("DELETE FROM chapter_text WHERE book_id=?1", params![id]).ok();
    conn.execute("DELETE FROM bookmarks WHERE book_id=?1", params![id]).ok();
    // 删除不可恢复：指纹一律删。
    conn.execute("DELETE FROM book_fingerprint WHERE book_id=?1", params![id]).ok();
    Ok(())
}

/// 笔记界面列表：按书分组聚合（仅 note!='' 的划线）。
/// 书名优先级：books 实时名 → orphan_books 快照名 → 空（前端显示"已删除"）。
/// 返回 (book_id, title, author, deleted, note_count, last_note_at)。
pub fn list_note_books(
    conn: &Connection,
) -> Vec<(i64, String, String, bool, i64, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT h.book_id, \
                COALESCE(NULLIF(b.title,''), o.title_snap, '') AS t, \
                COALESCE(NULLIF(b.author,''), o.author_snap, '') AS a, \
                CASE WHEN b.id IS NULL THEN 1 ELSE 0 END AS deleted, \
                COUNT(*), MAX(h.created_at) \
         FROM highlights h \
         LEFT JOIN books b ON b.id = h.book_id \
         LEFT JOIN orphan_books o ON o.book_id = h.book_id \
         WHERE h.note != '' \
         GROUP BY h.book_id \
         ORDER BY MAX(h.created_at) DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)? != 0,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// 某书带笔记的划线明细（供笔记界面下钻）。
pub fn list_notes_of_book(conn: &Connection, book_id: i64) -> Vec<(i64, i64, String, String, String, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT id, chapter_index, chapter_title, text, note, created_at \
         FROM highlights WHERE book_id=?1 AND note!='' ORDER BY chapter_index, pos_start",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(params![book_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// 设置/追加某条划线的 note。
pub fn set_highlight_note(conn: &Connection, id: i64, note: &str) -> Result<(), String> {
    conn.execute("UPDATE highlights SET note=?2 WHERE id=?1", params![id, note])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 追加 note（AI 添加到笔记时用，避免覆盖已有感想）。
pub fn append_highlight_note(conn: &Connection, id: i64, addition: &str) -> Result<String, String> {
    let cur: String = conn
        .query_row("SELECT note FROM highlights WHERE id=?1", params![id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let next = if cur.trim().is_empty() {
        addition.trim().to_string()
    } else {
        format!("{}\n\n{}", cur.trim_end(), addition.trim())
    };
    conn.execute("UPDATE highlights SET note=?2 WHERE id=?1", params![id, next])
        .map_err(|e| e.to_string())?;
    Ok(next)
}

#[cfg(test)]
mod notes_tests {
    use super::*;
    use rusqlite::Connection;

    fn tmpdb(tag: &str) -> Connection {
        let p = std::env::temp_dir().join(format!("fr_notes_{}_{}.db", tag, std::process::id()));
        let _ = std::fs::remove_file(&p);
        init(&p).unwrap()
    }

    fn add_book(conn: &Connection, path: &str, title: &str) {
        upsert_book(conn, path, title, "txt", 10, 1).unwrap();
    }

    fn add_hl(conn: &Connection, bid: i64, text: &str, note: &str) -> i64 {
        add_highlight(conn, bid, 0, "第一章", 0, 5, text, "underline", "", note).unwrap()
    }

    #[test]
    fn fingerprint_one_row_per_book_even_with_5000_highlights() {
        let conn = tmpdb("fp");
        let path = "/x/big.txt";
        add_book(&conn, path, "大书");
        let bid = path_id(path);
        for i in 0..5000 {
            add_hl(&conn, bid, &format!("划线{i}"), "");
        }
        // 手动 upsert 指纹（模拟 add_highlight 命令的行为）
        upsert_book_fingerprint(&conn, bid, 111, 222, "deadbeef", "大书", "作者").unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM book_fingerprint WHERE book_id=?1", rusqlite::params![bid], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "fingerprint must be exactly 1 row per book");
        let hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights WHERE book_id=?1", rusqlite::params![bid], |r| r.get(0)).unwrap();
        assert_eq!(hl, 5000);
    }

    #[test]
    fn note_list_filters_out_pure_highlights() {
        let conn = tmpdb("filter");
        add_book(&conn, "/x/withnote.txt", "有笔记");
        add_book(&conn, "/x/pure.txt", "纯划线");
        let a = path_id("/x/withnote.txt");
        let b = path_id("/x/pure.txt");
        add_hl(&conn, a, "被划线A", "我的感想A");
        add_hl(&conn, b, "被划线B", ""); // 纯划线
        let books = list_note_books(&conn);
        assert_eq!(books.len(), 1, "只有带 note 的书进入列表");
        assert_eq!(books[0].0, a);
        assert_eq!(books[0].4, 1); // note_count
    }

    #[test]
    fn delete_keep_notes_preserves_note_and_orphan_snapshot() {
        let conn = tmpdb("keep");
        let path = "/x/keep.txt";
        add_book(&conn, path, "保留书");
        let id = path_id(path);
        add_hl(&conn, id, "有感想", "保留我");
        add_hl(&conn, id, "纯划线", "");
        upsert_book_fingerprint(&conn, id, 1, 2, "h", "保留书", "作者X").unwrap();
        let before_hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        assert_eq!(before_hl, 2);
        delete_books_policy(&conn, &[path.to_string()], true).unwrap();
        // 原正 id 下不应再有划线（纯划线已删、带笔记的已移入孤儿命名空间）
        let at_orig: i64 = conn.query_row("SELECT COUNT(*) FROM highlights WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        let fp: i64 = conn.query_row("SELECT COUNT(*) FROM book_fingerprint WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        let book: i64 = conn.query_row("SELECT COUNT(*) FROM books WHERE id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        // 所有带笔记的划线都挂在某个负 id（唯一 orphan）上
        let orphan_hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights WHERE book_id<0 AND note!=''", [], |r| r.get(0)).unwrap();
        let orphan: i64 = conn.query_row("SELECT COUNT(*) FROM orphan_books", [], |r| r.get(0)).unwrap();
        assert_eq!(orphan_hl, 1, "带笔记的划线移入孤儿命名空间");
        assert_eq!(at_orig, 0, "原 id 下不残留划线");
        assert_eq!(fp, 0, "指纹一律删");
        assert_eq!(book, 0);
        assert_eq!(orphan, 1, "保留笔记需落 orphan 快照");
        // 笔记列表仍显示书名（来自 orphan 快照）
        let books = list_note_books(&conn);
        assert_eq!(books.len(), 1);
        assert!(books[0].0 < 0, "orphan id 为负数");
        assert_eq!(books[0].1, "保留书", "书名来自 orphan 快照");
        assert!(books[0].3, "标记为已删除");
    }

    #[test]
    fn delete_without_notes_removes_all() {
        let conn = tmpdb("nonote");
        let path = "/x/nonote.txt";
        add_book(&conn, path, "纯划线书");
        let id = path_id(path);
        add_hl(&conn, id, "划线", "");
        assert!(!book_has_notes(&conn, id));
        delete_books_policy(&conn, &[path.to_string()], true).unwrap();
        let hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        assert_eq!(hl, 0);
        let orphan: i64 = conn.query_row("SELECT COUNT(*) FROM orphan_books WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        assert_eq!(orphan, 0, "无笔记不落 orphan");
    }

    #[test]
    fn delete_removes_notes_when_chosen() {
        let conn = tmpdb("delnotes");
        let path = "/x/dn.txt";
        add_book(&conn, path, "删笔记");
        let id = path_id(path);
        add_hl(&conn, id, "有感想", "删掉我");
        upsert_book_fingerprint(&conn, id, 1, 2, "h", "删笔记", "").unwrap();
        delete_books_policy(&conn, &[path.to_string()], false).unwrap();
        let hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        assert_eq!(hl, 0);
        let orphan: i64 = conn.query_row("SELECT COUNT(*) FROM orphan_books WHERE book_id=?1", rusqlite::params![id], |r| r.get(0)).unwrap();
        assert_eq!(orphan, 0);
    }

    #[test]
    fn same_path_deleted_twice_keeps_both_batches() {
        // 同一路径「删书+保留笔记」两次：两批笔记必须都在、互不覆盖。
        let conn = tmpdb("twice");
        let path = "/x/twice.txt";
        // 第一批
        add_book(&conn, path, "第一版");
        let id = path_id(path);
        add_hl(&conn, id, "第一批划线", "第一批感想");
        upsert_book_fingerprint(&conn, id, 1, 1, "h1", "第一版", "作者1").unwrap();
        delete_books_policy(&conn, &[path.to_string()], true).unwrap();
        // 第二批（同路径重导后再删）
        add_book(&conn, path, "第二版");
        add_hl(&conn, path_id(path), "第二批划线", "第二批感想");
        upsert_book_fingerprint(&conn, path_id(path), 2, 2, "h2", "第二版", "作者2").unwrap();
        delete_books_policy(&conn, &[path.to_string()], true).unwrap();
        // 两批笔记都在
        let n1: i64 = conn
            .query_row("SELECT COUNT(*) FROM highlights WHERE note='第一批感想'", [], |r| r.get(0))
            .unwrap();
        let n2: i64 = conn
            .query_row("SELECT COUNT(*) FROM highlights WHERE note='第二批感想'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n1, 1, "第一批笔记仍在");
        assert_eq!(n2, 1, "第二批笔记仍在");
        // 两个不同的 orphan id，互不覆盖
        let orphans = list_note_books(&conn);
        assert_eq!(orphans.len(), 2, "两批笔记应为两个独立 orphan 分组");
        let titles: Vec<String> = orphans.iter().map(|x| x.1.clone()).collect();
        assert!(titles.contains(&"第一版".to_string()));
        assert!(titles.contains(&"第二版".to_string()));
    }

    #[test]
    fn reimport_does_not_revive_notes() {
        // 《A》带 note → 删书(保留笔记) → 同路径放《B》重导 → 《B》不冒出《A》的划线
        let conn = tmpdb("revive");
        let path = "/x/same.txt";
        add_book(&conn, path, "书A");
        let id = path_id(path);
        add_hl(&conn, id, "A的划线", "A的感想");
        upsert_book_fingerprint(&conn, id, 99, 88, "hashA", "书A", "作者A").unwrap();
        delete_books_policy(&conn, &[path.to_string()], true).unwrap();
        // 重导同路径《B》（book_id 与 A 相同）
        upsert_book(&conn, path, "书B", "txt", 20, 2).unwrap();
        add_hl(&conn, path_id(path), "B的划线", "");
        // B（正 id）不应有任何 A 的划线
        let a_under_b: i64 = conn
            .query_row("SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND text='A的划线'", rusqlite::params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(a_under_b, 0, "B 不冒出 A 的划线");
        // A 的笔记仍在（孤儿命名空间，负数 id）
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM highlights WHERE book_id<0 AND note='A的感想'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "A 的笔记仍在笔记界面");
        let books = list_note_books(&conn);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].1, "书A", "笔记界面显示 A 的原书名");
    }
}

#[cfg(test)]
mod cover_scan_tests {
    use super::*;
    use rusqlite::Connection;

    fn tmpdb(tag: &str) -> Connection {
        let p = std::env::temp_dir().join(format!("fr_cscan_{}_{}.db", tag, std::process::id()));
        let _ = std::fs::remove_file(&p);
        init(&p).unwrap()
    }

    /// 需求2：Some/None 写 meta_done=1，Err 写 meta_done=2；三种情况都**不再是 0**，
    /// 因此 `ids_needing_meta`（启动续扫用）不会再把它们捞出来。
    #[test]
    fn meta_done_never_stays_zero_after_scan() {
        let conn = tmpdb("md");
        let p1 = "/x/with_cover.epub";
        let p2 = "/x/no_cover.txt";
        let p3 = "/x/broken.epub";
        let id1 = upsert_book(&conn, p1, "有封面", "epub", 10, 1).unwrap();
        let id2 = upsert_book(&conn, p2, "无封面", "txt", 10, 1).unwrap();
        let id3 = upsert_book(&conn, p3, "损坏", "epub", 10, 1).unwrap();
        // 初始全部 meta_done=0（导入书籍落库后的状态）
        assert_eq!(ids_needing_meta(&conn).len(), 3);

        // Some：有内嵌封面 → meta_done=1，cover_ver+1
        set_book_meta_ex(&conn, id1, "有封面", "作者", Some("a.png"), 100, 90, 1, true).unwrap();
        // None：确定无封面 → meta_done=1
        set_book_meta_ex(&conn, id2, "无封面", "作者", None, 50, 45, 1, false).unwrap();
        // Err：失败 → meta_done=2（可重试，但启动不自动重扫）
        set_book_meta_ex(&conn, id3, "损坏", "作者", None, 0, 0, 2, false).unwrap();

        let b1 = get_book(&conn, id1).unwrap();
        assert_eq!(b1.meta_done, 1);
        assert_eq!(b1.cover_path.as_deref(), Some("a.png"));
        assert_eq!(b1.cover_ver, 1, "提取到封面应 cover_ver+1");
        let b2 = get_book(&conn, id2).unwrap();
        assert_eq!(b2.meta_done, 1);
        assert_eq!(b2.cover_path, None);
        let b3 = get_book(&conn, id3).unwrap();
        assert_eq!(b3.meta_done, 2, "失败必须写 2，不能留 0");
        // 启动只捞 meta_done=0 → 三本都不再被自动重扫
        assert!(ids_needing_meta(&conn).is_empty(), "扫完的书不应再入队");
    }

    /// 再次提取到新封面：cover_ver 递增（per-book URL 缓存失效）。
    #[test]
    fn cover_ver_increments_on_new_cover() {
        let conn = tmpdb("cv");
        let p = "/x/b.epub";
        let id = upsert_book(&conn, p, "书", "epub", 10, 1).unwrap();
        set_book_meta_ex(&conn, id, "书", "", Some("b.png"), 1, 1, 1, true).unwrap();
        set_book_meta_ex(&conn, id, "书", "", Some("b.png"), 1, 1, 1, true).unwrap();
        assert_eq!(get_book(&conn, id).unwrap().cover_ver, 2);
        // 无封面（None）不递增
        set_book_meta_ex(&conn, id, "书", "", None, 1, 1, 1, false).unwrap();
        assert_eq!(get_book(&conn, id).unwrap().cover_ver, 2);
    }

    #[test]
    fn brief_by_ids_returns_path_and_format() {
        let conn = tmpdb("brief");
        let a = upsert_book(&conn, "/x/a.epub", "A", "epub", 1, 1).unwrap();
        let b = upsert_book(&conn, "/x/b.txt", "B", "txt", 1, 1).unwrap();
        let mut got = books_brief_by_ids(&conn, &[a, b, 999999]);
        got.sort_by_key(|x| x.0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], (a, "/x/a.epub".into(), "epub".into()));
        assert_eq!(got[1], (b, "/x/b.txt".into(), "txt".into()));
    }
}
