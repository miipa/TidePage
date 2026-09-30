use crate::models::{BookRow, Progress};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
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

/// 秒级时间戳（扫描重试元数据 last_attempt_at / cover_priority.queued_at 用）。
pub fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
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
            cover_ver INTEGER NOT NULL DEFAULT 0,
            -- 秩可寻址分页专用：最近阅读时间的普通持久列（= max(last_opened, progress.updated_at)）。
            -- 由写入路径（打开书 / 保存进度 / 进度重置）维护，供 ORDER BY 走索引、消除 TEMP B-TREE。
            last_read INTEGER NOT NULL DEFAULT 0,
            -- 反规范化的阅读进度（0–100，量纲同 progress.pct）：由写 progress.pct 的每条路径同步维护（红线 5），
            -- 供封面进度环与「阅读状态」过滤下推 SQLite，避免 JOIN progress。
            read_pct REAL NOT NULL DEFAULT 0,
            -- 手动状态覆盖：0=自动推导 / 1=手动已读 / 2=手动未读。
            read_state INTEGER NOT NULL DEFAULT 0
        );
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
        CREATE TABLE IF NOT EXISTS settings(
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL DEFAULT ''
        );
        -- 打开未扫描书籍时把该本封面扫描提升为最高优先级（持久化，非内存队列）。
        CREATE TABLE IF NOT EXISTS cover_priority(
            book_id INTEGER PRIMARY KEY,
            queued_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    for col in [
        "total_chars INTEGER NOT NULL DEFAULT 0",
        "total_text_chars INTEGER NOT NULL DEFAULT 0",
        "last_opened INTEGER NOT NULL DEFAULT 0",
        "cover_ver INTEGER NOT NULL DEFAULT 0",
        "last_read INTEGER NOT NULL DEFAULT 0",
        // 阅读状态相关反规范化列（老库补列；新库建表已含）。
        "read_pct REAL NOT NULL DEFAULT 0",
        "read_state INTEGER NOT NULL DEFAULT 0",
        // 扫描重试元数据（A5/A9）：失败原因、上次尝试秒级时间、失败次数。
        "meta_error TEXT",
        "last_attempt_at INTEGER",
        "fail_count INTEGER NOT NULL DEFAULT 0",
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
    // last_read 一次性回填（新列/旧库）：= max(last_opened, 该书 progress.updated_at)。
    // 原子性（🔴 修）：UPDATE 与「已回填」标记放**同一事务**，杜绝「标记在、数据缺」的半成品态；
    //   WHERE 只补落后的行 → 中断后重跑是廉价 no-op（幂等、可重入）。事务失败则不写标记、下次重试。
    if get_setting(&conn, "last_read_backfill_v1").is_none() {
        let r = (|| -> rusqlite::Result<()> {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE books SET last_read = MAX(
                    COALESCE(last_opened, 0),
                    COALESCE((SELECT updated_at FROM progress WHERE progress.book_id = books.id), 0)
                 )
                 WHERE last_read < MAX(
                    COALESCE(last_opened, 0),
                    COALESCE((SELECT updated_at FROM progress WHERE progress.book_id = books.id), 0)
                 )",
                [],
            )?;
            tx.execute(
                "INSERT INTO settings(key,value) VALUES('last_read_backfill_v1','1')
                 ON CONFLICT(key) DO UPDATE SET value='1'",
                [],
            )?;
            tx.commit()
        })();
        if let Err(e) = r {
            eprintln!("[db] last_read backfill failed (will retry next start): {e}");
        }
    }
    // read_pct 一次性回填（新列/旧库）：progress ↔ books 为 1:1（progress.book_id 主键），
    // 直接取该书 progress.pct，无记录留 0。与 last_read 回填同款：UPDATE 与标记同事务、幂等可重入。
    if get_setting(&conn, "read_pct_backfill_v1").is_none() {
        let r = (|| -> rusqlite::Result<()> {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE books SET read_pct = COALESCE(
                    (SELECT pct FROM progress WHERE progress.book_id = books.id), 0
                 )",
                [],
            )?;
            tx.execute(
                "INSERT INTO settings(key,value) VALUES('read_pct_backfill_v1','1')
                 ON CONFLICT(key) DO UPDATE SET value='1'",
                [],
            )?;
            tx.commit()
        })();
        if let Err(e) = r {
            eprintln!("[db] read_pct backfill failed (will retry next start): {e}");
        }
    }
    // 所有索引统一放在**补列逻辑之后**创建。
    // 关键：`idx_books_lastread_id` 依赖 `books.last_read`，而老库的该列是在上面 ALTER 补出的；
    // 若把索引放在建表批内，老库建表被跳过、列尚不存在 → `no such column: last_read` → setup panic。
    // 先补列、再建索引，对老库/新库都安全（新库建表已含 last_read，跳过 ALTER）。
    // 索引放在回填之后，避免回填 UPDATE 维护 last_read 索引的额外开销。
    conn.execute_batch(
        r#"
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
        -- 「按最近阅读」秩可寻址分页：与 ORDER BY last_read DESC, id DESC 逐列一致，含唯一 tiebreaker。
        CREATE INDEX IF NOT EXISTS idx_books_lastread_id ON books(last_read DESC, id DESC);
        -- 「按文件大小」：与 ORDER BY size DESC, id DESC 逐列一致（反向可用索引回扫）。
        CREATE INDEX IF NOT EXISTS idx_books_size_id ON books(size DESC, id DESC);
        CREATE INDEX IF NOT EXISTS idx_highlights_book ON highlights(book_id);
        CREATE INDEX IF NOT EXISTS idx_highlights_created ON highlights(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_sessions_book ON reading_sessions(book_id);
        CREATE INDEX IF NOT EXISTS idx_progress_book ON progress(book_id);
        CREATE INDEX IF NOT EXISTS idx_progress_updated ON progress(updated_at DESC);
        -- 增量扫描的关键索引：让 `WHERE meta_done=0 ORDER BY id LIMIT 200`
        -- 与 `COUNT(*) WHERE meta_done=0` 走索引，避免 16 万行时每批从头线性扫（二次方）。
        CREATE INDEX IF NOT EXISTS idx_books_meta_done ON books(meta_done, id);
        "#,
    )
    .map_err(|e| e.to_string())?;
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
        read_pct: r.get(14)?,
        read_state: r.get(15)?,
        last_opened: r.get(16)?,
    })
}

/// `last_read`（下标 10）= 普通持久列 `books.last_read`（由写入路径维护）。
/// 不再跨表 JOIN progress 计算，故所有列表/单查询都不需要 `LEFT JOIN progress`。
/// read_pct（14）/ read_state（15）/ last_opened（16）为阅读状态相关列（红线 5 + 状态推导）。
const BOOK_COLS: &str = "b.id, b.path, b.title, b.author, b.format, b.size, b.mtime, b.added_at, b.cover_path, b.meta_done, b.last_read, COALESCE(b.total_chars, 0), COALESCE(b.total_text_chars, 0), b.cover_ver, COALESCE(b.read_pct, 0), COALESCE(b.read_state, 0), COALESCE(b.last_opened, 0)";

/// 阅读进度「已满」阈值（量纲 0–100，见 §0 第 1 条：progress.pct 为百分数）。
/// 取 99.5 以吸收浮点；SQL（build_book_filter）与前端 effectiveState 必须逐字同源使用同一阈值。
pub const READ_FULL_PCT: f64 = 99.5;

/// 生成「有效阅读状态」的 SQL CASE 表达式（与前端 effectiveState(row) 同一套映射）：
///   read_state==1 -> 'read'；read_state==2 -> 'unread'；
///   last_opened==0 -> 'unread'；read_pct>=READ_FULL_PCT -> 'read'；else 'reading'。
/// 用于 build_book_filter 拼接过滤条件；所有字段名为字面量、阈值为数值字面量，无用户输入拼接。
fn effective_state_case_sql() -> String {
    format!(
        "CASE \
         WHEN COALESCE(read_state,0)=1 THEN 'read' \
         WHEN COALESCE(read_state,0)=2 THEN 'unread' \
         WHEN COALESCE(last_opened,0)=0 THEN 'unread' \
         WHEN COALESCE(read_pct,0)>={READ_FULL_PCT} THEN 'read' \
         ELSE 'reading' END"
    )
}

/// 【红线 3 落地】书库过滤子句的唯一来源：`order_index_ids` 与 `list_books_page` 都必须
/// 拼接本函数返回的同一 fragment、绑同一批 params，从结构上杜绝两处 WHERE 不同源、
/// 避免 rank→id 与取行错位。
///
/// - 返回 `(where_sql_fragment, params)`：fragment **不含** 前导 `WHERE`/`AND`，
///   由调用方与其它条件（如搜索 LIKE）用 `AND` 组合；无任何条件时返回空串。
/// - 格式维度：`format IN (?,?,…)`，占位符数量随选中项变化，值一律走参数化绑定。
/// - 状态维度：`(effective_read_state) IN (?,…)`，状态字符串走绑定（白名单校验后）。
/// - 空数组 / None 语义（红线 & Task D）：**不追加该维度条件**（回到完整书库），
///   绝不翻成 `IN ()` 或恒假条件。
pub fn build_book_filter(
    formats: &Option<Vec<String>>,
    states: &Option<Vec<String>>,
) -> (String, Vec<Value>) {
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if let Some(fs) = formats {
        if !fs.is_empty() {
            let ph = std::iter::repeat("?").take(fs.len()).collect::<Vec<_>>().join(",");
            conds.push(format!("b.format IN ({ph})"));
            for f in fs {
                params.push(Value::Text(f.clone()));
            }
        }
    }
    if let Some(ss) = states {
        // 仅保留白名单值（read/unread/reading），过滤脏数据；空则不追加条件。
        let valid: Vec<&str> = ss
            .iter()
            .map(|s| s.as_str())
            .filter(|s| matches!(*s, "read" | "unread" | "reading"))
            .collect();
        if !valid.is_empty() {
            let ph = std::iter::repeat("?").take(valid.len()).collect::<Vec<_>>().join(",");
            conds.push(format!("({}) IN ({ph})", effective_state_case_sql()));
            for s in valid {
                params.push(Value::Text(s.to_string()));
            }
        }
    }
    (conds.join(" AND "), params)
}

/// 书库中实际存在的格式列表（Task B）：`SELECT DISTINCT format`，
/// 排序固定 `COUNT(*) DESC, format ASC`（写死，避免每次刷新顺序跳动）。
pub fn list_formats(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn
        .prepare("SELECT format FROM books GROUP BY format ORDER BY COUNT(*) DESC, format ASC")
    {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 手动设置阅读状态覆盖（Task F）：state=1 已读 / 2 未读 / 0 自动。仅写 read_state，不动 read_pct（真实进度不破坏）。
pub fn set_read_state(conn: &Connection, book_id: i64, state: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET read_state=?2 WHERE id=?1",
        params![book_id, state],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 记录「最近打开」时间（不改动阅读进度）。打开失败的书也能出现在「按最近阅读」里。
/// 同步维护 last_read（取大），保证秩可寻址分页的排序键与显示一致。
pub fn touch_book_opened(conn: &Connection, book_id: i64) {
    let now = now_ms();
    conn.execute(
        "UPDATE books SET last_opened=?1, last_read=MAX(last_read, ?1) WHERE id=?2",
        params![now, book_id],
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
        "SELECT {BOOK_COLS} FROM books b ORDER BY b.added_at DESC, b.id DESC"
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

/// 排序 ORDER BY 片段的**唯一来源**（list_books_page / order_index_ids / random_book_filtered
/// 三处共用，保证逐字同源、rank→id 不错位）。
/// - 主列走白名单字面量：lastread→b.last_read、title→b.title COLLATE NOCASE、recent→b.mtime、
///   size→b.size、其余回落 added→b.added_at；方向与列名都不拼接用户文本（红线：白名单）。
/// - `desc=true` 主列降序、`false` 升序；tiebreaker `b.id` 与主列同向，保证全序稳定。
pub fn order_by_clause(sort: &str, desc: bool) -> String {
    let col = match sort {
        "lastread" => "b.last_read",
        "title" => "b.title COLLATE NOCASE",
        "recent" => "b.mtime",
        "size" => "b.size",
        _ => "b.added_at",
    };
    let d = if desc { "DESC" } else { "ASC" };
    format!("{col} {d}, b.id {d}")
}

/// 超大书库分页查询：过滤/排序全部下推 SQLite（带索引），只返回窗口 rows + total。
/// - query 为空不施加搜索过滤；否则按 title/author 模糊匹配（LIKE 对 ASCII 不区分大小写）。
/// - formats/states 走 `build_book_filter` 下推（红线 3 同源）；空数组/None = 不约束该维度。
/// - sort 走白名单，绝不拼接用户输入；desc 决定主列与 tiebreaker 方向。
/// - limit 上限保护 500，offset 非负。
pub fn list_books_page(
    conn: &Connection,
    query: &str,
    sort: &str,
    offset: i64,
    limit: i64,
    formats: &Option<Vec<String>>,
    states: &Option<Vec<String>>,
    desc: bool,
) -> crate::models::BookPage {
    let order = order_by_clause(sort, desc);
    let limit = limit.clamp(1, 500);
    let offset = offset.max(0);
    let q = query.trim();

    // 组合 WHERE：搜索 LIKE（若有）+ build_book_filter fragment（红线 3 同源）。
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if !q.is_empty() {
        conds.push("(b.title LIKE ? ESCAPE '\\' OR b.author LIKE ? ESCAPE '\\')".to_string());
        let pat = format!("%{}%", escape_like(q));
        params.push(Value::Text(pat.clone()));
        params.push(Value::Text(pat));
    }
    let (frag, fparams) = build_book_filter(formats, states);
    if !frag.is_empty() {
        conds.push(frag);
        params.extend(fparams);
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };

    let total: i64 = {
        let sql = format!("SELECT COUNT(*) FROM books b{where_sql}");
        conn.query_row(&sql, params_from_iter(params.iter()), |r| r.get(0))
            .unwrap_or(0)
    };

    let sql = format!("SELECT {BOOK_COLS} FROM books b{where_sql} ORDER BY {order} LIMIT ? OFFSET ?");
    let mut page_params = params;
    page_params.push(Value::Integer(limit));
    page_params.push(Value::Integer(offset));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => {
            return crate::models::BookPage { total, rows: vec![] };
        }
    };
    let rows: Vec<BookRow> = stmt
        .query_map(params_from_iter(page_params.iter()), book_from_row)
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    crate::models::BookPage { total, rows }
}

/// 从「当前筛选结果」中随机取一本书（`query`/`sort`/`formats`/`states` 与 `list_books_page` 同源）。
/// 先 COUNT 满足条件的行数，再随机 offset + LIMIT 1，避免把整库/整份筛选结果载入内存。
pub fn random_book_filtered(
    conn: &Connection,
    query: &str,
    sort: &str,
    formats: &Option<Vec<String>>,
    states: &Option<Vec<String>>,
) -> Option<BookRow> {
    // 随机与顺序无关，方向固定（列仍走白名单）；offset 均匀分布即可。
    let order = order_by_clause(sort, false);
    let q = query.trim();
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if !q.is_empty() {
        conds.push("(b.title LIKE ? ESCAPE '\\' OR b.author LIKE ? ESCAPE '\\')".to_string());
        let pat = format!("%{}%", escape_like(q));
        params.push(Value::Text(pat.clone()));
        params.push(Value::Text(pat));
    }
    let (frag, fparams) = build_book_filter(formats, states);
    if !frag.is_empty() {
        conds.push(frag);
        params.extend(fparams);
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    let total: i64 = {
        let sql = format!("SELECT COUNT(*) FROM books b{where_sql}");
        conn.query_row(&sql, params_from_iter(params.iter()), |r| r.get(0))
            .unwrap_or(0)
    };
    if total <= 0 {
        return None;
    }
    let offset = (rand::random::<u64>() % (total as u64)) as i64;
    let sql = format!("SELECT {BOOK_COLS} FROM books b{where_sql} ORDER BY {order} LIMIT 1 OFFSET ?");
    let mut p = params;
    p.push(Value::Integer(offset));
    conn.query_row(&sql, params_from_iter(p.iter()), book_from_row).ok()
}

/// 全库随机取一本书（轻量：COUNT + 随机 offset，不一次载入整库）。
pub fn random_book_any(conn: &Connection) -> Option<BookRow> {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
        .unwrap_or(0);
    if total <= 0 {
        return None;
    }
    let offset = (rand::random::<u64>() % (total as u64)) as i64;
    conn.query_row(
        &format!("SELECT {BOOK_COLS} FROM books b ORDER BY b.id LIMIT 1 OFFSET ?"),
        params![offset],
        book_from_row,
    )
    .ok()
}

/// 秩可寻址分页：命令 1 的数据源。按白名单 `sort` + 可选 `query` / `formats` / `states` 过滤，返回**有序 books.id 序列**。
/// - `query`/`formats`/`states` 施加与 `list_books_page` **逐字同源**的 WHERE（后者由 `build_book_filter` 保证，红线 3），
///   保证 rank→id 与 `books_by_ids_aligned` 取到的行严格对齐；
/// - 不 JOIN progress；ORDER BY 片段为编译期字面量（禁止拼接用户输入）；
/// - 边收集边计数：超过 `max_rows` 立即中止并返回 `Err("order_index_too_large:N")`（禁止静默截断），
///   避免为超大结果集构建巨大 Vec。
pub fn order_index_ids(
    conn: &Connection,
    sort: &str,
    query: &str,
    max_rows: usize,
    formats: &Option<Vec<String>>,
    states: &Option<Vec<String>>,
    desc: bool,
) -> Result<Vec<i64>, String> {
    // 白名单 → 字面量 ORDER BY（与 list_books_page 同一函数，红线 3 同源）；未知一律回落 "added"。
    // 表别名 b 与 list_books_page 一致，令 build_book_filter 的 `b.format` 片段两处通用。
    let order = order_by_clause(sort, desc);
    let q = query.trim();
    // WHERE 与 list_books_page 同源：搜索 LIKE + build_book_filter fragment。
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if !q.is_empty() {
        conds.push("(b.title LIKE ? ESCAPE '\\' OR b.author LIKE ? ESCAPE '\\')".to_string());
        let pat = format!("%{}%", escape_like(q));
        params.push(Value::Text(pat.clone()));
        params.push(Value::Text(pat));
    }
    let (frag, fparams) = build_book_filter(formats, states);
    if !frag.is_empty() {
        conds.push(frag);
        params.extend(fparams);
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    let sql = format!("SELECT b.id FROM books b{where_sql} ORDER BY {order}");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut ids: Vec<i64> = Vec::new();
    let mut rows = stmt.query(params_from_iter(params.iter())).map_err(|e| e.to_string())?;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        if ids.len() >= max_rows {
            return Err(format!("order_index_too_large:{max_rows}"));
        }
        ids.push(r.get(0).map_err(|e| e.to_string())?);
    }
    Ok(ids)
}

/// books 总行数（诊断/兜底用）。
#[allow(dead_code)]
pub fn count_books(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
        .unwrap_or(0)
}

/// 秩可寻址分页：命令 2 的数据源。按入参 `ids` 批量取行，**返回顺序与长度恒等于 ids**：
/// 库中不存在的 id 以 `None` 占位（不跳过，保持秩对齐）。单次 `IN (...)` 查询，无循环单查。
pub fn books_by_ids_aligned(conn: &Connection, ids: &[i64]) -> Vec<Option<BookRow>> {
    if ids.is_empty() {
        return vec![];
    }
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT {BOOK_COLS} FROM books b WHERE b.id IN ({placeholders})");
    let mut by_id: std::collections::HashMap<i64, BookRow> =
        std::collections::HashMap::with_capacity(ids.len());
    if let Ok(mut stmt) = conn.prepare(&sql) {
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        if let Ok(rows) = stmt.query_map(params.as_slice(), book_from_row) {
            for r in rows.flatten() {
                by_id.insert(r.id, r);
            }
        }
    }
    // 按入参顺序对齐输出（去重的 id 会各自命中同一行的克隆）。
    ids.iter().map(|id| by_id.get(id).cloned()).collect()
}

pub fn get_book(conn: &Connection, id: i64) -> Option<BookRow> {
    let sql = format!(
        "SELECT {BOOK_COLS} FROM books b WHERE b.id=?1"
    );
    conn.query_row(&sql, params![id], book_from_row)
        .optional()
        .unwrap_or(None)
}

pub fn get_book_by_path(conn: &Connection, path: &str) -> Option<BookRow> {
    let sql = format!(
        "SELECT {BOOK_COLS} FROM books b WHERE b.path=?1"
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

/// 一次性读入全部书籍路径为 `HashSet`（`SELECT path FROM books`）。
/// 供 scan_folders 阶段二：单次短锁读取后立即释放，随后无锁求差集。
pub fn all_paths_set(conn: &Connection) -> std::collections::HashSet<String> {
    let mut stmt = match conn.prepare("SELECT path FROM books") {
        Ok(s) => s,
        Err(_) => return std::collections::HashSet::new(),
    };
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}


/// 增量扫描：取下一批待处理（`meta_done=0`）的 (id, path, format)，按 id 升序、上限 limit。
/// 走 idx_books_meta_done 索引，16 万行也不会每批从头线性扫。
pub fn next_meta_batch(conn: &Connection, limit: i64) -> Vec<(i64, String, String)> {
    let mut stmt = match conn
        .prepare("SELECT id, path, format FROM books WHERE meta_done = 0 ORDER BY id ASC LIMIT ?1")
    {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// 待处理书籍数（`meta_done=0`）。用于进度分母与每批「是否在推进」的判定（A6）。
pub fn count_meta_pending(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM books WHERE meta_done = 0", [], |r| r.get(0))
        .unwrap_or(0)
}

/// 崩溃残留回收（B4）：把上次中断时停在「处理中」(meta_done=3) 的书还原为未处理(0)。
pub fn recover_processing(conn: &Connection) -> i64 {
    conn.execute("UPDATE books SET meta_done = 0 WHERE meta_done = 3", [])
        .map(|n| n as i64)
        .unwrap_or(0)
}

/// 读取单本 meta_done（B6：批次内已取到内存的书，处理前重读状态，避免二次读盘）。
pub fn meta_done_of(conn: &Connection, id: i64) -> Option<i64> {
    conn.query_row("SELECT meta_done FROM books WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .unwrap_or(None)
}

/// 读取单本 (meta_done, meta_error)。打开书籍时据 meta_error 判断是否值得插队重试。
/// 读取单本 (meta_done, meta_error, fail_count)。打开书籍时据 meta_error 类型与
/// fail_count 判断是否插队重试。
pub fn meta_state_of(conn: &Connection, id: i64) -> Option<(i64, Option<String>, i64)> {
    conn.query_row(
        "SELECT meta_done, meta_error, fail_count FROM books WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .unwrap_or(None)
}

/// CAS 认领（B5）：仅当当前状态在 `allow` 集合内时置为 3(处理中)。
/// 返回 true 表示本线程成功认领（changes()==1），可安全解析该本；否则说明已被处理/认领。
/// 启动批次传 &[0]；打开插队重试传 &[0, 2]（允许对失败项重试）。
pub fn claim_meta(conn: &Connection, id: i64, allow: &[i64]) -> bool {
    let list = allow
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("UPDATE books SET meta_done = 3 WHERE id = ?1 AND meta_done IN ({list})");
    conn.execute(&sql, params![id]).map(|n| n == 1).unwrap_or(false)
}

/// 写扫描状态（A5）：state=1 完成 / 2 失败。同时更新 meta_error、last_attempt_at(秒)、
/// fail_count（state=2 递增，state=1 归零）。**不写标题/封面等业务字段**（由 set_book_meta_ex 负责）。
/// 返回 `Result`：写入失败不再吞错，交由调用方记录/降级。
pub fn mark_meta(conn: &Connection, id: i64, state: i64, err: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE books SET meta_done = ?2, meta_error = ?3, last_attempt_at = ?4,
         fail_count = CASE WHEN ?2 = 2 THEN fail_count + 1 ELSE 0 END WHERE id = ?1",
        params![id, state, err, now_s()],
    )?;
    Ok(())
}

/// 打开书籍时把该本写入优先表（B1）。已存在则刷新 queued_at。
/// 返回 `Result`：写入失败不再吞错（否则 enqueued 计数与日志会失真）。
pub fn enqueue_priority(conn: &Connection, book_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO cover_priority(book_id, queued_at) VALUES(?1, ?2)
         ON CONFLICT(book_id) DO UPDATE SET queued_at = excluded.queued_at",
        params![book_id, now_s()],
    )?;
    Ok(())
}

/// 取优先表队首且仍需处理的一本（B3）：meta_done IN (0,2,3)，按 queued_at **降序**
/// （最近关注的优先——列表滚到的可见项 / 刚打开的书排在最前）。
pub fn next_priority(conn: &Connection) -> Option<(i64, String, String)> {
    conn.query_row(
        "SELECT b.id, b.path, b.format FROM cover_priority c
         JOIN books b ON b.id = c.book_id
         WHERE b.meta_done IN (0, 2, 3)
         ORDER BY c.queued_at DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .unwrap_or(None)
}

/// 处理完成后从优先表移除（B5：无论成败都 DELETE）。
pub fn delete_priority(conn: &Connection, book_id: i64) {
    conn.execute("DELETE FROM cover_priority WHERE book_id = ?1", params![book_id])
        .ok();
}

/// 只更新封面路径一列（扫描路径专用；不写 title/author/chars 等元数据）。
/// cover_path 由 NULL→文件名会让前端封面 URL 从「默认底图端点」切到「per-book 端点」而刷新。
pub fn set_cover_path(conn: &Connection, id: i64, cover_path: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE books SET cover_path = ?2 WHERE id = ?1",
        params![id, cover_path],
    )?;
    Ok(())
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
    // 同步维护 books.last_read（取大）：秩可寻址分页「按最近阅读」的排序键。
    // 同步维护 books.read_pct（反规范化，红线 5）：所有写 progress.pct 的路径都经由本函数，
    // 故这一处覆盖 §0 第 4 条列出的全部写入点（save_progress / webdav 合并回写）。
    conn.execute(
        "UPDATE books SET last_read=MAX(last_read, ?2), read_pct=?3 WHERE id=?1",
        params![book_id, p.updated_at, p.pct],
    )
    .ok();
    Ok(())
}

/// 进度重置/清除时调用：把 last_read 回落到 last_opened（不再受已删进度影响），
/// 并同步清 0 反规范化的 read_pct（红线 5：进度重置路径同步清 0）。
/// 当前无「重置进度」命令，保留供将来接入；书籍删除走整行删除，不需调用本函数。
#[allow(dead_code)]
pub fn reset_last_read(conn: &Connection, book_id: i64) {
    conn.execute(
        "UPDATE books SET last_read=COALESCE(last_opened, 0), read_pct=0 WHERE id=?1",
        params![book_id],
    )
    .ok();
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
    // day 用 SQLite 本地日期 date('now','localtime')，与 get_stats 查询口径一致，
    // 避免东八区等地在本地凌晨的阅读被 UTC 记到错误日期（原 day_string() 为 UTC）。
    conn.execute(
        "INSERT INTO reading_sessions(book_id, day, started_at, seconds, chars)
         VALUES(?1, date('now','localtime'), ?2, 0, 0)",
        params![book_id, now_ms()],
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

/// 笔记界面列表：按书分组聚合（**有划线即入列**，不要求 note!=''）。
/// 书名优先级：books 实时名 → orphan_books 快照名 → 空（前端显示"已删除"）。
/// 返回 (book_id, title, author, deleted, hl_count, note_count, last_at, cover_path, cover_ver)。
pub fn list_note_books(
    conn: &Connection,
) -> Vec<(i64, String, String, bool, i64, i64, i64, String, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT h.book_id, \
                COALESCE(NULLIF(b.title,''), o.title_snap, '') AS t, \
                COALESCE(NULLIF(b.author,''), o.author_snap, '') AS a, \
                CASE WHEN b.id IS NULL THEN 1 ELSE 0 END AS deleted, \
                COUNT(*), \
                SUM(CASE WHEN h.note != '' THEN 1 ELSE 0 END), \
                MAX(h.created_at), \
                COALESCE(b.cover_path,''), \
                COALESCE(b.cover_ver, 0) \
         FROM highlights h \
         LEFT JOIN books b ON b.id = h.book_id \
         LEFT JOIN orphan_books o ON o.book_id = h.book_id \
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
            r.get::<_, i64>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, i64>(8)?,
        ))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// 某书的划线明细（供笔记界面下钻）：**含未写感想的划线**（note 可能为空串）。
pub fn list_notes_of_book(conn: &Connection, book_id: i64) -> Vec<(i64, i64, String, String, String, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT id, chapter_index, chapter_title, text, note, created_at \
         FROM highlights WHERE book_id=?1 ORDER BY chapter_index, pos_start",
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

    /// 老库升级路径：books 表不含 last_read、user_version=0。
    /// 旧实现会在建表批内执行 `CREATE INDEX ... ON books(last_read ...)` → no such column → setup panic。
    /// 修复后：先按 pragma_table_info 补列，再统一建索引。
    #[test]
    fn init_upgrades_old_books_table_missing_last_read() {
        let p = std::env::temp_dir().join(format!("fr_old_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        {
            let old = Connection::open(&p).unwrap();
            old.execute_batch(
                "CREATE TABLE books(
                    id INTEGER PRIMARY KEY,
                    path TEXT UNIQUE NOT NULL,
                    title TEXT NOT NULL DEFAULT '',
                    author TEXT NOT NULL DEFAULT '',
                    format TEXT NOT NULL,
                    size INTEGER NOT NULL DEFAULT 0,
                    mtime INTEGER NOT NULL DEFAULT 0,
                    added_at INTEGER NOT NULL DEFAULT 0,
                    cover_path TEXT,
                    meta_done INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO books(id,path,title,format,size,mtime,added_at,meta_done)
                    VALUES(1,'/a.txt','a','txt',1,1,1,0),(2,'/b.txt','b','txt',1,1,1,0);",
            )
            .unwrap();
            let uv: i64 = old
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(uv, 0, "老库 user_version 应为 0");
            let has: i64 = old
                .query_row(
                    "SELECT count(*) FROM pragma_table_info('books') WHERE name='last_read'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(has, 0, "老库不应含 last_read");
        }
        let conn = init(&p).expect("老库 init 不应报错");
        let has: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('books') WHERE name='last_read'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has, 1, "升级后应补出 last_read 列");
        let idx: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_books_lastread_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "应创建 idx_books_lastread_id");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "升级不得改变行数");
        let _ = std::fs::remove_file(&p);
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
        // 语义：有划线即入列（不要求 note!=''），故两本都出现。
        let books = list_note_books(&conn);
        assert_eq!(books.len(), 2, "纯划线书也应入列");
        // 用 book_id 定位，不依赖返回顺序。
        let pure = books.iter().find(|x| x.0 == b).expect("纯划线书应在列表中");
        assert_eq!(pure.5, 0, "纯划线 note_count==0");
        assert_eq!(pure.4, 1, "纯划线 hl_count==1");
        let withnote = books.iter().find(|x| x.0 == a).expect("带感想书应在列表中");
        assert_eq!(withnote.5, 1, "带感想 note_count==1");
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
        // 有划线即入列：孤儿书A（带感想）与重导的《书B》（纯划线）各一组。
        let books = list_note_books(&conn);
        assert_eq!(books.len(), 2);
        let a_grp = books.iter().find(|x| x.0 < 0).expect("孤儿书A分组");
        assert_eq!(a_grp.1, "书A", "笔记界面显示 A 的原书名");
        assert!(a_grp.3, "孤儿书A标记为已删除");
        assert_eq!(a_grp.5, 1, "书A note_count==1");
        let b_grp = books.iter().find(|x| x.0 >= 0).expect("书B分组");
        assert_eq!(b_grp.5, 0, "书B 只有纯划线，note_count==0");
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
    /// 因此增量扫描（`count_meta_pending` / `next_meta_batch`）不会再把它们捞出来。
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
        assert_eq!(count_meta_pending(&conn), 3);

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
        assert_eq!(count_meta_pending(&conn), 0, "扫完的书不应再入队");
        assert!(next_meta_batch(&conn, 200).is_empty());
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

#[cfg(test)]
mod fts_snippet_tests {
    use super::*;

    /// 校验 search_all_inner 使用的 FTS5 snippet() 查询在 trigram 分词下可用，
    /// 且用哨兵参数能标出命中（Rust 侧再转义并还原为 <mark>）。
    #[test]
    fn snippet_sql_marks_hits_with_sentinels() {
        let p = std::env::temp_dir().join(format!("fr_ftssnip_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let conn = init(&p).unwrap();
        if !has_fts(&conn) {
            let _ = std::fs::remove_file(&p);
            return;
        }
        ensure_fts(&conn).unwrap();
        conn.execute(
            "INSERT INTO fts(book_id, chapter_index, title, body) VALUES(1,0,'第一章','这是一个用于全局搜索性能验证的中文正文段落')",
            [],
        )
        .unwrap();
        let sql = "SELECT fts.book_id, fts.chapter_index, fts.title, COALESCE(b.title,''), \
                   snippet(fts, 3, ?2, ?3, '…', 12) \
                   FROM fts LEFT JOIN books b ON b.id=fts.book_id \
                   WHERE fts.body MATCH ?1 LIMIT 80";
        let mut stmt = conn.prepare(sql).unwrap();
        let row: (i64, i64, String, String, String) = stmt
            .query_row(rusqlite::params!["全局搜索", "\u{1}", "\u{2}"], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        assert_eq!(row.0, 1);
        assert!(row.4.contains('\u{1}'), "snippet 应含起始哨兵: {}", row.4);
        assert!(row.4.contains('\u{2}'), "snippet 应含结束哨兵: {}", row.4);
        let _ = std::fs::remove_file(&p);
    }
}
