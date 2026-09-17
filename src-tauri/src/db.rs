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
            cover_path TEXT,
            meta_done INTEGER NOT NULL DEFAULT 0,
            total_chars INTEGER NOT NULL DEFAULT 0,
            total_text_chars INTEGER NOT NULL DEFAULT 0
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
    for col in ["total_chars INTEGER NOT NULL DEFAULT 0", "total_text_chars INTEGER NOT NULL DEFAULT 0"] {
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
                for table in ["progress", "txt_cache", "bookmarks", "reading_sessions"] {
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
    })
}

const BOOK_COLS: &str = "b.id, b.path, b.title, b.author, b.format, b.size, b.mtime, b.added_at, b.cover_path, b.meta_done, COALESCE(p.updated_at, 0), COALESCE(b.total_chars, 0), COALESCE(b.total_text_chars, 0)";

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
        "lastread" => "COALESCE(p.updated_at, 0) DESC, b.id DESC",
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

pub fn books_needing_meta(conn: &Connection) -> Vec<(i64, String, String)> {
    let mut stmt = match conn.prepare("SELECT id, path, format FROM books WHERE meta_done=0") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

pub fn set_book_meta(
    conn: &Connection,
    id: i64,
    title: &str,
    author: &str,
    cover_path: Option<&str>,
    total_chars: i64,
    total_text_chars: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET title=?2, author=?3, cover_path=?4, meta_done=1, total_chars=?5, total_text_chars=?6 WHERE id=?1",
        params![id, title, author, cover_path, total_chars, total_text_chars],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_books(conn: &Connection, paths: &[String]) -> Result<(), String> {
    for p in paths {
        let id = path_id(p);
        conn.execute("DELETE FROM books WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM progress WHERE book_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM txt_cache WHERE book_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM chapter_text WHERE book_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM bookmarks WHERE book_id=?1", params![id])
            .map_err(|e| e.to_string())?;
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
