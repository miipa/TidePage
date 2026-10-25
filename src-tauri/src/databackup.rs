//! D4：本地完整备份 / 预览 / 恢复。
//! 单文件版本化 JSON；跨库通过「绝对路径优先 + 书名/大小/作者退化」匹配 books，不存 book_id。
//! 不备份书籍文件、封面、解析缓存、FTS；config 由前端传脱敏 JSON。
use crate::db;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tauri::State;

pub const FORMAT: &str = "tidepage-backup";
pub const VERSION: u32 = 1;
const SHELVES_KEY: &str = "library.shelves.v1";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub config: Option<Value>,
    #[serde(default)]
    pub shelves: Option<Value>,
    #[serde(default)]
    pub progress: Vec<ProgressRec>,
    #[serde(default)]
    pub bookmarks: Vec<BookmarkRec>,
    #[serde(default)]
    pub highlights: Vec<HighlightRec>,
    #[serde(default)]
    pub tags: Vec<TagRec>,
    #[serde(default)]
    pub book_tags: Vec<BookTagRec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BookIdent {
    #[serde(default)]
    pub book_path: String,
    #[serde(default)]
    pub book_title: String,
    #[serde(default)]
    pub book_author: String,
    #[serde(default)]
    pub book_size: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProgressRec {
    #[serde(flatten)]
    pub book: BookIdent,
    pub chapter_index: i64,
    pub chapter_ratio: f64,
    pub pct: f64,
    pub updated_at: i64,
    #[serde(default)]
    pub device: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BookmarkRec {
    #[serde(flatten)]
    pub book: BookIdent,
    pub chapter_index: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub excerpt: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HighlightRec {
    #[serde(flatten)]
    pub book: BookIdent,
    pub chapter_index: i64,
    #[serde(default)]
    pub chapter_title: String,
    #[serde(default)]
    pub pos_start: i64,
    #[serde(default)]
    pub pos_end: i64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TagRec {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BookTagRec {
    #[serde(flatten)]
    pub book: BookIdent,
    pub tag_name: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub progress: usize,
    pub bookmarks: usize,
    pub highlights: usize,
    pub tags: usize,
    pub book_tags: usize,
    pub skipped: usize,
    pub size_bytes: u64,
    pub path: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub format: String,
    pub version: u32,
    pub app_version: String,
    pub created_at: i64,
    pub progress: usize,
    pub bookmarks: usize,
    pub highlights: usize,
    pub tags: usize,
    pub book_tags: usize,
    pub matched_books: usize,
    pub skipped: usize,
    pub conflicts: usize,
    pub has_config: bool,
    pub has_shelves: bool,
    pub empty: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    pub progress: usize,
    pub bookmarks: usize,
    pub highlights: usize,
    pub tags: usize,
    pub book_tags: usize,
    pub skipped: usize,
    pub conflicts: usize,
    pub overwrite: bool,
    pub config_json: Option<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn normalize_path(p: &str) -> String {
    p.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn norm_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// 书匹配索引：绝对路径（大小写不敏感）→ 书名+大小 → 书名+作者；任一命中多本视为歧义。
/// A6 优化：不再为每本书分配 Vec；唯一命中直接存 id，第二本冲突时记为 AMBIGUOUS。
pub struct BookIndex {
    by_path: HashMap<String, i64>,
    by_title_size: HashMap<String, i64>,
    by_title_author: HashMap<String, i64>,
}
const AMBIGUOUS: i64 = i64::MIN;

fn idx_insert(map: &mut HashMap<String, i64>, key: String, id: i64) {
    match map.entry(key) {
        std::collections::hash_map::Entry::Vacant(e) => { e.insert(id); }
        std::collections::hash_map::Entry::Occupied(mut e) => {
            if *e.get() != id { e.insert(AMBIGUOUS); }
        }
    }
}

impl BookIndex {
    pub fn build(conn: &Connection) -> Self {
        let mut idx = Self {
            by_path: HashMap::new(),
            by_title_size: HashMap::new(),
            by_title_author: HashMap::new(),
        };
        let mut stmt = match conn.prepare("SELECT id, path, title, COALESCE(author,''), COALESCE(size,0) FROM books") {
            Ok(s) => s,
            Err(_) => return idx,
        };
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (id, path, title, author, size) = row;
                idx_insert(&mut idx.by_path, normalize_path(&path), id);
                idx_insert(&mut idx.by_title_size, format!("{}{}", norm_key(&title), size), id);
                idx_insert(&mut idx.by_title_author, format!("{}{}", norm_key(&title), norm_key(&author)), id);
            }
        }
        idx
    }

    fn hit(&self, id: Option<&i64>) -> Option<i64> {
        match id {
            Some(&id) if id != AMBIGUOUS => Some(id),
            _ => None,
        }
    }

    pub fn resolve(&self, b: &BookIdent) -> Option<i64> {
        if !b.book_path.trim().is_empty() {
            if let Some(id) = self.hit(self.by_path.get(&normalize_path(&b.book_path))) {
                return Some(id);
            }
        }
        if !b.book_title.trim().is_empty() {
            let ts = format!("{}{}", norm_key(&b.book_title), b.book_size);
            if let Some(id) = self.hit(self.by_title_size.get(&ts)) {
                return Some(id);
            }
            let ta = format!("{}{}", norm_key(&b.book_title), norm_key(&b.book_author));
            if let Some(id) = self.hit(self.by_title_author.get(&ta)) {
                return Some(id);
            }
        }
        None
    }
}

fn ident_row(r: &rusqlite::Row<'_>, base: usize) -> rusqlite::Result<BookIdent> {
    Ok(BookIdent {
        book_path: r.get(base)?,
        book_title: r.get(base + 1)?,
        book_author: r.get(base + 2).unwrap_or_default(),
        book_size: r.get(base + 3).unwrap_or(0),
    })
}

fn collect_backup(conn: &Connection, config_json: Option<&str>) -> Result<Backup, String> {
    let config = match config_json {
        Some(s) if !s.trim().is_empty() => Some(serde_json::from_str::<Value>(s).map_err(|e| format!("配置 JSON 无效：{e}"))?),
        _ => None,
    };
    let shelves = db::get_setting(conn, SHELVES_KEY).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let mut backup = Backup {
        format: FORMAT.to_string(),
        version: VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: now_ms(),
        config,
        shelves,
        ..Default::default()
    };
    let ident = "b.path, b.title, COALESCE(b.author,''), COALESCE(b.size,0)";
    {
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, p.chapter_index, p.chapter_ratio, p.pct, p.updated_at, p.device FROM progress p JOIN books b ON b.id=p.book_id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(ProgressRec {
                book: ident_row(r, 0)?,
                chapter_index: r.get(4)?,
                chapter_ratio: r.get(5)?,
                pct: r.get(6)?,
                updated_at: r.get(7)?,
                device: r.get(8).unwrap_or_default(),
            })
        });
        backup.progress = rows.map_err(|e| e.to_string())?.flatten().collect();
    }
    {
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, bm.chapter_index, bm.title, bm.excerpt, bm.created_at FROM bookmarks bm JOIN books b ON b.id=bm.book_id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(BookmarkRec {
                book: ident_row(r, 0)?,
                chapter_index: r.get(4)?,
                title: r.get(5).unwrap_or_default(),
                excerpt: r.get(6).unwrap_or_default(),
                created_at: r.get(7).unwrap_or(0),
            })
        });
        backup.bookmarks = rows.map_err(|e| e.to_string())?.flatten().collect();
    }
    {
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, h.chapter_index, h.chapter_title, h.pos_start, h.pos_end, h.text, h.style, h.color, h.note, h.created_at FROM highlights h JOIN books b ON b.id=h.book_id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(HighlightRec {
                book: ident_row(r, 0)?,
                chapter_index: r.get(4)?,
                chapter_title: r.get(5).unwrap_or_default(),
                pos_start: r.get(6).unwrap_or(0),
                pos_end: r.get(7).unwrap_or(0),
                text: r.get(8).unwrap_or_default(),
                style: r.get(9).unwrap_or_default(),
                color: r.get(10).unwrap_or_default(),
                note: r.get(11).unwrap_or_default(),
                created_at: r.get(12).unwrap_or(0),
            })
        });
        backup.highlights = rows.map_err(|e| e.to_string())?.flatten().collect();
    }
    {
        let mut stmt = conn.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(TagRec { name: r.get(0)? }));
        backup.tags = rows.map_err(|e| e.to_string())?.flatten().collect();
    }
    {
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, t.name FROM book_tags bt JOIN books b ON b.id=bt.book_id JOIN tags t ON t.id=bt.tag_id ORDER BY t.name"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(BookTagRec {
                book: ident_row(r, 0)?,
                tag_name: r.get(4)?,
            })
        });
        backup.book_tags = rows.map_err(|e| e.to_string())?.flatten().collect();
    }
    Ok(backup)
}

fn write_backup_atomic(path: &str, backup: &Backup) -> Result<u64, String> {
    let json = serde_json::to_vec(backup).map_err(|e| format!("序列化备份失败：{e}"))?;
    let target = Path::new(path);
    let tmp = target.with_extension("json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("写入备份失败：{e}"))?;
    if target.exists() {
        std::fs::remove_file(target).map_err(|e| format!("替换备份失败：{e}"))?;
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("保存备份失败：{e}")
    })?;
    Ok(json.len() as u64)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn export_backup_to_file(conn: &Connection, path: &str, config_json: Option<&str>) -> Result<ExportSummary, String> {
    let backup = collect_backup(conn, config_json)?;
    let skipped = 0usize; // 导出 JOIN books，孤儿行不会进入备份
    let size = write_backup_atomic(path, &backup)?;
    Ok(ExportSummary {
        progress: backup.progress.len(),
        bookmarks: backup.bookmarks.len(),
        highlights: backup.highlights.len(),
        tags: backup.tags.len(),
        book_tags: backup.book_tags.len(),
        skipped,
        size_bytes: size,
        path: path.to_string(),
    })
}

fn read_backup(path: &str) -> Result<Backup, String> {
    let data = std::fs::read(path).map_err(|e| format!("读取备份文件失败：{e}"))?;
    let b: Backup = serde_json::from_slice(&data).map_err(|e| format!("备份 JSON 损坏或字段缺失：{e}"))?;
    if b.format != FORMAT {
        return Err("不是 TidePage 备份文件".into());
    }
    if b.version != VERSION {
        return Err(format!("备份版本 {} 不受支持（当前支持 {}）", b.version, VERSION));
    }
    Ok(b)
}

/// 书架容器解析：备份/本机 settings 允许对象 {"v":1,"shelves":[...]} 或纯数组。
fn shelves_vec(v: &Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(o)) => o.get("shelves").and_then(|s| s.as_array()).cloned().unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn shelves_conflicts(conn: &Connection, backup_shelves: &Option<Value>) -> usize {
    let arr = shelves_vec(backup_shelves);
    if arr.is_empty() { return 0; }
    let existing: Vec<Value> = shelves_vec(
        &db::get_setting(conn, SHELVES_KEY).and_then(|s| serde_json::from_str::<Value>(&s).ok()),
    );
    let names: HashSet<String> = existing
        .iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(norm_key))
        .collect();
    arr.iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(norm_key))
        .filter(|n| names.contains(n))
        .count()
}

pub fn inspect_backup_file(conn: &Connection, path: &str) -> Result<BackupPreview, String> {
    let b = read_backup(path)?;
    let idx = BookIndex::build(conn);
    let mut matched: HashSet<i64> = HashSet::new();
    let mut skipped = 0usize;
    let mut conflicts = 0usize;
    for rec in &b.progress {
        match idx.resolve(&rec.book) {
            Some(id) => {
                matched.insert(id);
                let existing: Option<i64> = conn
                    .query_row("SELECT updated_at FROM progress WHERE book_id=?1", [id], |r| r.get(0))
                    .optional()
                    .unwrap_or(None);
                if existing.map(|e| e >= rec.updated_at).unwrap_or(false) {
                    conflicts += 1;
                }
            }
            None => skipped += 1,
        }
    }
    for rec in &b.bookmarks {
        match idx.resolve(&rec.book) {
            Some(id) => {
                matched.insert(id);
                let exists: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM bookmarks WHERE book_id=?1 AND chapter_index=?2 AND title=?3 AND excerpt=?4",
                        params![id, rec.chapter_index, rec.title, rec.excerpt],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if exists > 0 { conflicts += 1; }
            }
            None => skipped += 1,
        }
    }
    for rec in &b.highlights {
        match idx.resolve(&rec.book) {
            Some(id) => {
                matched.insert(id);
                let exists: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND chapter_index=?2 AND pos_start=?3 AND pos_end=?4 AND text=?5",
                        params![id, rec.chapter_index, rec.pos_start, rec.pos_end, rec.text],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if exists > 0 { conflicts += 1; }
            }
            None => skipped += 1,
        }
    }
    for rec in &b.book_tags {
        match idx.resolve(&rec.book) {
            Some(id) => { matched.insert(id); }
            None => skipped += 1,
        }
    }
    conflicts += shelves_conflicts(conn, &b.shelves);
    let total_rows = b.progress.len() + b.bookmarks.len() + b.highlights.len() + b.book_tags.len();
    Ok(BackupPreview {
        format: b.format,
        version: b.version,
        app_version: b.app_version,
        created_at: b.created_at,
        progress: b.progress.len(),
        bookmarks: b.bookmarks.len(),
        highlights: b.highlights.len(),
        tags: b.tags.len(),
        book_tags: b.book_tags.len(),
        matched_books: matched.len(),
        skipped,
        conflicts,
        has_config: b.config.is_some(),
        has_shelves: b.shelves.is_some(),
        empty: total_rows == 0 && b.tags.is_empty() && b.shelves.is_none(),
    })
}

fn merge_shelves(conn: &Connection, backup_shelves: &Option<Value>, overwrite: bool) -> Result<(), String> {
    let incoming = shelves_vec(backup_shelves);
    if incoming.is_empty() && backup_shelves.is_none() {
        if overwrite { db::set_setting(conn, SHELVES_KEY, "{\"v\":1,\"shelves\":[]}"); }
        return Ok(());
    }
    if overwrite {
        db::set_setting(conn, SHELVES_KEY, &serde_json::json!({"v":1,"shelves":incoming}).to_string());
        return Ok(());
    }
    let mut existing: Vec<Value> = shelves_vec(
        &db::get_setting(conn, SHELVES_KEY).and_then(|s| serde_json::from_str::<Value>(&s).ok()),
    );
    let mut names: HashSet<String> = existing
        .iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(norm_key))
        .collect();
    let mut ids: HashSet<String> = existing
        .iter()
        .filter_map(|s| s.get("id").and_then(|n| n.as_str()).map(|x| x.to_string()))
        .collect();
    for mut s in incoming.clone() {
        let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
        if name.is_empty() || names.contains(&norm_key(&name)) { continue; }
        let mut id = s.get("id").and_then(|n| n.as_str()).unwrap_or("").to_string();
        if id.is_empty() || ids.contains(&id) {
            id = format!("shelf-{}-{}", now_ms(), existing.len());
            if let Some(o) = s.as_object_mut() { o.insert("id".into(), Value::String(id.clone())); }
        }
        names.insert(norm_key(&name));
        ids.insert(id);
        existing.push(s);
    }
    db::set_setting(conn, SHELVES_KEY, &serde_json::json!({"v":1,"shelves":existing}).to_string());
    Ok(())
}

pub fn restore_backup_file(conn: &mut Connection, path: &str, overwrite: bool) -> Result<RestoreSummary, String> {
    let b = read_backup(path)?;
    let idx = BookIndex::build(conn);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if overwrite {
        for table in ["progress", "bookmarks", "highlights", "tags"] {
            tx.execute(&format!("DELETE FROM {table}"), []).map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM book_tags", []).map_err(|e| e.to_string())?;
        // 覆盖模式重置 books 上的反规范化进度：无备份进度的书回到未读；
        // 有进度的书随后由 set_progress 按备份 updated_at/pct 重建。
        tx.execute(
            "UPDATE books SET last_read = COALESCE(last_opened, 0), read_pct = 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    let mut summary = RestoreSummary { overwrite, ..Default::default() };

    for rec in &b.progress {
        let Some(book_id) = idx.resolve(&rec.book) else { summary.skipped += 1; continue };
        let existing: Option<i64> = tx
            .query_row("SELECT updated_at FROM progress WHERE book_id=?1", [book_id], |r| r.get(0))
            .optional()
            .unwrap_or(None);
        if existing.map(|e| e >= rec.updated_at).unwrap_or(false) {
            summary.conflicts += 1;
            continue;
        }
        db::set_progress(
            &tx,
            book_id,
            &crate::models::Progress {
                chapter_index: rec.chapter_index,
                chapter_ratio: rec.chapter_ratio,
                pct: rec.pct,
                updated_at: rec.updated_at,
                device: rec.device.clone(),
            },
        )?;
        summary.progress += 1;
    }
    for rec in &b.bookmarks {
        let Some(book_id) = idx.resolve(&rec.book) else { summary.skipped += 1; continue };
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM bookmarks WHERE book_id=?1 AND chapter_index=?2 AND title=?3 AND excerpt=?4",
                params![book_id, rec.chapter_index, rec.title, rec.excerpt],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 { summary.conflicts += 1; continue; }
        tx.execute(
            "INSERT INTO bookmarks(book_id, chapter_index, title, excerpt, created_at) VALUES(?1,?2,?3,?4,?5)",
            params![book_id, rec.chapter_index, rec.title, rec.excerpt, rec.created_at],
        )
        .map_err(|e| e.to_string())?;
        summary.bookmarks += 1;
    }
    for rec in &b.highlights {
        let Some(book_id) = idx.resolve(&rec.book) else { summary.skipped += 1; continue };
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND chapter_index=?2 AND pos_start=?3 AND pos_end=?4 AND text=?5",
                params![book_id, rec.chapter_index, rec.pos_start, rec.pos_end, rec.text],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 { summary.conflicts += 1; continue; }
        tx.execute(
            "INSERT INTO highlights(book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![book_id, rec.chapter_index, rec.chapter_title, rec.pos_start, rec.pos_end, rec.text, rec.style, rec.color, rec.note, rec.created_at],
        )
        .map_err(|e| e.to_string())?;
        summary.highlights += 1;
    }
    for rec in &b.tags {
        if let Ok((_id, _name)) = db::create_tag(&tx, &rec.name) {
            summary.tags += 1;
        }
    }
    for rec in &b.book_tags {
        let Some(book_id) = idx.resolve(&rec.book) else { summary.skipped += 1; continue };
        let tag_id: Option<i64> = tx
            .query_row("SELECT id FROM tags WHERE norm=?1", [norm_key(&rec.tag_name)], |r| r.get(0))
            .optional()
            .unwrap_or(None);
        let Some(tag_id) = tag_id else { summary.skipped += 1; continue };
        let n = tx
            .execute("INSERT OR IGNORE INTO book_tags(book_id, tag_id) VALUES(?1,?2)", params![book_id, tag_id])
            .map_err(|e| e.to_string())?;
        if n > 0 { summary.book_tags += 1; } else { summary.conflicts += 1; }
    }
    merge_shelves(&tx, &b.shelves, overwrite)?;
    tx.commit().map_err(|e| e.to_string())?;
    summary.config_json = b.config.as_ref().map(|v| v.to_string());
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ocr_book_index_composite_key_no_collision() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE books(id INTEGER PRIMARY KEY, path TEXT, title TEXT, author TEXT, size INTEGER);").unwrap();
        conn.execute("INSERT INTO books VALUES(1,'C:/a.epub','x1','y1',23)", []).unwrap();
        conn.execute("INSERT INTO books VALUES(2,'C:/b.epub','x12','y12',3)", []).unwrap();
        conn.execute("INSERT INTO books VALUES(3,'C:/c.epub','z9','y12',99)", []).unwrap();
        let idx = BookIndex::build(&conn);
        // title+size 组合不得把 ("x1",23) 与 ("x12",3) 当作同键。
        let b1 = BookIdent { book_path: "".into(), book_title: "x1".into(), book_author: "".into(), book_size: 23 };
        let b2 = BookIdent { book_path: "".into(), book_title: "x12".into(), book_author: "".into(), book_size: 3 };
        assert_eq!(idx.resolve(&b1), Some(1));
        assert_eq!(idx.resolve(&b2), Some(2));
        // title+author 同理。
        let a1 = BookIdent { book_path: "".into(), book_title: "x1".into(), book_author: "y1".into(), book_size: 0 };
        let a2 = BookIdent { book_path: "".into(), book_title: "x12".into(), book_author: "y12".into(), book_size: 0 };
        assert_eq!(idx.resolve(&a1), Some(1));
        assert_eq!(idx.resolve(&a2), Some(2));
    }


    fn tmp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp-d4-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn setup_book(root: &Path) -> (Connection, PathBuf, String) {
        let conn = db::init(&root.join("t.db")).unwrap();
        let path = root.join("book.epub");
        std::fs::write(&path, b"x").unwrap();
        let sp = path.to_string_lossy().into_owned();
        db::upsert_book(&conn, &sp, "书名", "epub", 123, 1).unwrap();
        (conn, path, sp)
    }

    #[test]
    fn roundtrip_merge_overwrite_and_config() {
        let root = tmp_root("roundtrip");
        let (mut conn, _path, _sp) = setup_book(&root);
        let book_id: i64 = conn.query_row("SELECT id FROM books", [], |r| r.get(0)).unwrap();
        db::set_progress(&conn, book_id, &crate::models::Progress {
            chapter_index: 2, chapter_ratio: 0.25, pct: 30.0, updated_at: 100, device: "dev".into(),
        }).unwrap();
        conn.execute("INSERT INTO bookmarks(book_id,chapter_index,title,excerpt,created_at) VALUES(?1,2,'t','e',1)", [book_id]).unwrap();
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,2,'ch',1,5,'hello','underline','#ff0','n',10)", [book_id]).unwrap();
        let (tag_id, _) = db::create_tag(&conn, "标签").unwrap();
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", params![book_id, tag_id]).unwrap();
        db::set_setting(&conn, SHELVES_KEY, "{\"v\":1,\"shelves\":[{\"id\":\"s1\",\"name\":\"书架\",\"formats\":[],\"states\":[],\"tags\":[],\"folder\":null,\"query\":\"\",\"createdAt\":1}]}");
        let path = root.join("backup.json").to_string_lossy().into_owned();
        let sum = export_backup_to_file(&conn, &path, Some("{\"reader\":{}}")).unwrap();
        assert_eq!((sum.progress, sum.bookmarks, sum.highlights, sum.tags, sum.book_tags), (1, 1, 1, 1, 1));
        assert!(std::path::Path::new(&path).exists());
        assert!(!root.join("backup.json.tmp").exists());
        // 覆盖已存在的目标文件：同一路径再导出一次应成功，临时文件不残留。
        let sum2 = export_backup_to_file(&conn, &path, Some("{\"reader\":{}}")).unwrap();
        assert_eq!(sum2.progress, 1);
        assert!(!root.join("backup.json.tmp").exists());
        let preview = inspect_backup_file(&conn, &path).unwrap();
        assert_eq!(preview.matched_books, 1);
        assert_eq!(preview.skipped, 0);
        assert!(preview.has_config && preview.has_shelves && !preview.empty);
        assert_eq!((preview.progress, preview.bookmarks, preview.highlights, preview.tags, preview.book_tags), (1,1,1,1,1));

        for t in ["progress", "bookmarks", "highlights", "book_tags", "tags"] {
            conn.execute(&format!("DELETE FROM {t}"), []).unwrap();
        }
        // 本机书架清空后再恢复，验证对象形状 {"v":1,"shelves":[...]} 能正确导入。
        db::set_setting(&conn, SHELVES_KEY, "{\"v\":1,\"shelves\":[]}");
        let restored = restore_backup_file(&mut conn, &path, false).unwrap();
        assert_eq!((restored.progress, restored.bookmarks, restored.highlights), (1,1,1));
        assert_eq!(restored.tags, 1);
        assert!(restored.config_json.is_some());
        let pct: f64 = conn.query_row("SELECT read_pct FROM books WHERE id=?1", [book_id], |r| r.get(0)).unwrap();
        assert!(pct > 0.0, "progress 恢复同步 books.read_pct");
        let sh = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let shv: Value = serde_json::from_str(&sh).unwrap();
        let names: Vec<String> = shv["shelves"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or("").to_string()).collect();
        assert!(names.contains(&"书架".to_string()), "D4 恢复应导入对象形状书架: {names:?}");

        let again = restore_backup_file(&mut conn, &path, false).unwrap();
        assert_eq!((again.progress, again.bookmarks, again.highlights), (0, 0, 0));
        assert!(again.conflicts >= 2, "较旧/重复应计 conflict: {}", again.conflicts);

        for t in ["progress", "bookmarks", "highlights", "book_tags", "tags"] {
            conn.execute(&format!("DELETE FROM {t}"), []).unwrap();
        }
        let ow = restore_backup_file(&mut conn, &path, true).unwrap();
        assert_eq!((ow.progress, ow.bookmarks, ow.highlights, ow.tags), (1,1,1,1));
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM book_tags", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reject_bad_version_and_corrupt_and_empty() {
        let root = tmp_root("bad");
        let (conn, _p, _sp) = setup_book(&root);
        let bad = root.join("bad.json");
        std::fs::write(&bad, r#"{"format":"tidepage-backup","version":99}"#).unwrap();
        let err = inspect_backup_file(&conn, bad.to_str().unwrap()).unwrap_err();
        assert!(err.contains("版本"), "{err}");
        let wrong = root.join("wrong.json");
        std::fs::write(&wrong, r#"{"format":"other","version":1}"#).unwrap();
        assert!(inspect_backup_file(&conn, wrong.to_str().unwrap()).is_err());
        let corrupt = root.join("corrupt.json");
        std::fs::write(&corrupt, b"{not json").unwrap();
        assert!(inspect_backup_file(&conn, corrupt.to_str().unwrap()).is_err());
        let empty = Backup { format: FORMAT.into(), version: VERSION, ..Default::default() };
        let ep = root.join("empty.json");
        std::fs::write(&ep, serde_json::to_vec(&empty).unwrap()).unwrap();
        let pv = inspect_backup_file(&conn, ep.to_str().unwrap()).unwrap();
        assert!(pv.empty && pv.progress == 0 && !pv.has_config);
        let mut conn2 = conn;
        let r = restore_backup_file(&mut conn2, ep.to_str().unwrap(), false).unwrap();
        assert_eq!(r.progress + r.bookmarks + r.highlights, 0);
        let _ = std::fs::remove_dir_all(&root);
    }


    /// A6：小库前后计时证据（旧 Vec 索引 vs 新唯一 id 索引），打印毫秒；不做性能硬断言。
    #[test]
    fn book_index_timing_evidence() {
        let root = tmp_root("a6-timing");
        let conn = db::init(&root.join("t.db")).unwrap();
        for i in 0..3000 {
            let p = root.join(format!("b{i}.epub"));
            std::fs::write(&p, b"x").unwrap();
            db::upsert_book(&conn, &p.to_string_lossy(), &format!("书{i}"), "epub", 100 + i, i as i64).unwrap();
        }
        // 旧结构复刻：每本 Vec 分配。
        let t_old = std::time::Instant::now();
        let mut by_path: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
        let mut by_ts: std::collections::HashMap<(String, i64), Vec<i64>> = std::collections::HashMap::new();
        let mut by_ta: std::collections::HashMap<(String, String), Vec<i64>> = std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT id,path,title,COALESCE(author,''),COALESCE(size,0) FROM books").unwrap();
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))).unwrap();
            for row in rows.flatten() {
                let (id, path, title, author, size) = row;
                let np = path.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase();
                by_path.entry(np).or_default().push(id);
                by_ts.entry((title.trim().to_lowercase(), size)).or_default().push(id);
                by_ta.entry((title.trim().to_lowercase(), author.trim().to_lowercase())).or_default().push(id);
            }
        }
        let old_ms = t_old.elapsed().as_millis();
        let t_new = std::time::Instant::now();
        let idx = BookIndex::build(&conn);
        let mut matched = 0;
        for i in 0..1000 {
            let p = root.join(format!("b{i}.epub"));
            let b = BookIdent { book_path: p.to_string_lossy().into_owned(), book_title: format!("书{i}"), book_author: String::new(), book_size: 100 + i };
            if idx.resolve(&b).is_some() { matched += 1; }
        }
        let new_ms = t_new.elapsed().as_millis();
        eprintln!("[A6] legacy build={old_ms}ms; new build+1000 resolve={new_ms}ms; matched={matched}");
        assert_eq!(matched, 1000);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fallback_match_and_ambiguity_skip() {
        let root = tmp_root("match");
        let conn = db::init(&root.join("t.db")).unwrap();
        let p1 = root.join("a.epub");
        let p2 = root.join("b.epub");
        std::fs::write(&p1, b"1").unwrap();
        std::fs::write(&p2, b"2").unwrap();
        let id1 = db::upsert_book(&conn, &p1.to_string_lossy(), "同名", "epub", 10, 1).unwrap();
        let _id2 = db::upsert_book(&conn, &p2.to_string_lossy(), "同名", "epub", 10, 2).unwrap();
        let backup = Backup {
            format: FORMAT.into(),
            version: VERSION,
            bookmarks: vec![BookmarkRec {
                book: BookIdent { book_path: "C:/不存在.epub".into(), book_title: "同名".into(), book_author: "".into(), book_size: 10 },
                chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: 1,
            }],
            progress: vec![ProgressRec {
                book: BookIdent { book_path: "C:/也不存在.epub".into(), book_title: "同名".into(), book_author: "".into(), book_size: 10 },
                chapter_index: 0, chapter_ratio: 0.0, pct: 1.0, updated_at: 1, device: "".into(),
            }],
            ..Default::default()
        };
        let path = root.join("amb.json").to_string_lossy().into_owned();
        std::fs::write(&path, serde_json::to_vec(&backup).unwrap()).unwrap();
        let pv = inspect_backup_file(&conn, &path).unwrap();
        assert_eq!(pv.matched_books, 0);
        assert_eq!(pv.skipped, 2, "歧义应跳过");
        conn.execute("DELETE FROM books WHERE id<>?1", [id1]).unwrap();
        let pv2 = inspect_backup_file(&conn, &path).unwrap();
        assert_eq!(pv2.matched_books, 1);
        assert_eq!(pv2.skipped, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overwrite_resets_books_progress_denorm() {
        let root = tmp_root("overwrite-denorm");
        let conn = db::init(&root.join("t.db")).unwrap();
        let p1 = root.join("a.epub");
        let p2 = root.join("b.epub");
        std::fs::write(&p1, b"1").unwrap();
        std::fs::write(&p2, b"2").unwrap();
        let b1 = db::upsert_book(&conn, &p1.to_string_lossy(), "有进度", "epub", 10, 1).unwrap();
        let b2 = db::upsert_book(&conn, &p2.to_string_lossy(), "无进度", "epub", 20, 2).unwrap();
        // 本机状态：两本都有反规范化进度，备份只含第一本的 progress。
        db::set_progress(&conn, b1, &crate::models::Progress {
            chapter_index: 1, chapter_ratio: 0.5, pct: 30.0, updated_at: 100, device: "".into(),
        }).unwrap();
        conn.execute("UPDATE books SET last_read=999, read_pct=88, last_opened=50 WHERE id=?1", [b2]).unwrap();
        conn.execute("UPDATE books SET last_opened=10 WHERE id=?1", [b1]).unwrap();

        let backup = Backup {
            format: FORMAT.into(),
            version: VERSION,
            progress: vec![ProgressRec {
                book: BookIdent {
                    book_path: p1.to_string_lossy().into_owned(),
                    book_title: "有进度".into(), book_author: "".into(), book_size: 10,
                },
                chapter_index: 1, chapter_ratio: 0.5, pct: 30.0, updated_at: 100, device: "".into(),
            }],
            ..Default::default()
        };
        let path = root.join("ow.json").to_string_lossy().into_owned();
        std::fs::write(&path, serde_json::to_vec(&backup).unwrap()).unwrap();
        let mut conn2 = conn;
        restore_backup_file(&mut conn2, &path, true).unwrap();

        // 无备份进度的书：反规范化状态回到「未读」，last_read 回落 last_opened。
        let (pct2, last2): (f64, i64) = conn2
            .query_row("SELECT read_pct, last_read FROM books WHERE id=?1", [b2], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(pct2, 0.0, "无备份进度的书 read_pct 应清零");
        assert_eq!(last2, 50, "last_read 应回落 COALESCE(last_opened,0)");
        // 有备份进度的书：按备份 pct/updated_at 重建。
        let (pct1, last1): (f64, i64) = conn2
            .query_row("SELECT read_pct, last_read FROM books WHERE id=?1", [b1], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert!((pct1 - 30.0).abs() < 1e-6, "有进度书 read_pct 应按备份重建: {pct1}");
        assert!(last1 >= 100, "有进度书 last_read 应取备份 updated_at: {last1}");
        let _ = std::fs::remove_dir_all(&root);
    }
}


/* ---------------- Tauri 命令 ---------------- */

#[tauri::command]
pub async fn export_data_backup(
    state: State<'_, Arc<crate::library::AppState>>,
    path: String,
    config_json: Option<String>,
) -> Result<ExportSummary, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let backup = {
            let conn = st.db.0.lock().unwrap();
            collect_backup(&conn, config_json.as_deref())?
        };
        let size = write_backup_atomic(&path, &backup)?;
        Ok(ExportSummary {
            progress: backup.progress.len(),
            bookmarks: backup.bookmarks.len(),
            highlights: backup.highlights.len(),
            tags: backup.tags.len(),
            book_tags: backup.book_tags.len(),
            skipped: 0,
            size_bytes: size,
            path: path.clone(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn inspect_data_backup(
    state: State<'_, Arc<crate::library::AppState>>,
    path: String,
) -> Result<BackupPreview, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.0.lock().unwrap();
        inspect_backup_file(&conn, &path)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn restore_data_backup(
    state: State<'_, Arc<crate::library::AppState>>,
    path: String,
    overwrite: bool,
) -> Result<RestoreSummary, String> {
    let st = state.inner().clone();
    let summary = tauri::async_runtime::spawn_blocking(move || {
        let mut conn = st.db.0.lock().unwrap();
        restore_backup_file(&mut conn, &path, overwrite)
    })
    .await
    .map_err(|e| e.to_string())??;
    // 恢复后刷新多窗口（配置由前端应用后自行广播）。
    use tauri::Emitter;
    let app = state.app.clone();
    let _ = app.emit("library-changed", ());
    let _ = app.emit("notes-changed", ());
    let _ = app.emit("library-tags-changed", ());
    let _ = app.emit("library-shelves-changed", ());
    Ok(summary)
}
