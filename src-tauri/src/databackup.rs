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
/// v2：新增 orphan_books（「删书保留」的孤儿划线及其书名/作者快照）。
/// v1 备份仍可读（缺失字段 serde default）；> VERSION 拒绝。
pub const VERSION: u32 = 2;
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
    /// v2：删书保留（keep_notes=true）产生的孤儿划线，按孤儿书分组。
    #[serde(default)]
    pub orphan_books: Vec<OrphanBookRec>,
}

/// v2：孤儿书的书名/作者快照 + 其下的带笔记划线。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct OrphanBookRec {
    #[serde(default)]
    pub book_id: i64,
    #[serde(default)]
    pub title_snap: String,
    #[serde(default)]
    pub author_snap: String,
    #[serde(default)]
    pub deleted_at: i64,
    #[serde(default)]
    pub highlights: Vec<OrphanHighlightRec>,
}

/// v2：孤儿划线（字段语义与 HighlightRec 相同，只是没有书籍身份）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct OrphanHighlightRec {
    #[serde(default)]
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
    /// v2 孤儿计数（导出/预览/恢复都含）。
    pub orphan_books: usize,
    pub orphan_highlights: usize,
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
    /// v2 孤儿计数（导出/预览/恢复都含）。
    pub orphan_books: usize,
    pub orphan_highlights: usize,
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
    /// v2 孤儿计数（导出/预览/恢复都含）。
    pub orphan_books: usize,
    pub orphan_highlights: usize,
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
    // v2：孤儿书（删书保留）单独采集，不走 JOIN books——它们本来就没有 books 行。
    {
        let mut by_book: HashMap<i64, OrphanBookRec> = HashMap::new();
        let mut stmt = conn
            .prepare("SELECT book_id, title_snap, author_snap, deleted_at FROM orphan_books ORDER BY book_id")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(OrphanBookRec {
                book_id: r.get(0)?,
                title_snap: r.get(1).unwrap_or_default(),
                author_snap: r.get(2).unwrap_or_default(),
                deleted_at: r.get(3).unwrap_or(0),
                highlights: Vec::new(),
            })
        });
        for rec in rows.map_err(|e| e.to_string())?.flatten() {
            by_book.insert(rec.book_id, rec);
        }
        let mut stmt = conn
            .prepare(
                "SELECT book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, created_at                  FROM highlights WHERE book_id<0 ORDER BY book_id, chapter_index, pos_start",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                OrphanHighlightRec {
                    chapter_index: r.get(1)?,
                    chapter_title: r.get(2).unwrap_or_default(),
                    pos_start: r.get(3).unwrap_or(0),
                    pos_end: r.get(4).unwrap_or(0),
                    text: r.get(5).unwrap_or_default(),
                    style: r.get(6).unwrap_or_default(),
                    color: r.get(7).unwrap_or_default(),
                    note: r.get(8).unwrap_or_default(),
                    created_at: r.get(9).unwrap_or(0),
                },
            ))
        });
        for row in rows.map_err(|e| e.to_string())?.flatten() {
            let (book_id, hl) = row;
            by_book
                .entry(book_id)
                .or_insert_with(|| OrphanBookRec { book_id, ..Default::default() })
                .highlights
                .push(hl);
        }
        let mut orphans: Vec<OrphanBookRec> = by_book.into_values().collect();
        orphans.sort_by_key(|o| o.book_id);
        backup.orphan_books = orphans;
    }
    Ok(backup)
}

/// 孤儿划线总数（导出/预览计数用）。
fn orphan_highlight_count(b: &Backup) -> usize {
    b.orphan_books.iter().map(|o| o.highlights.len()).sum()
}

fn write_backup_atomic(path: &str, backup: &Backup) -> Result<u64, String> {
    let json = serde_json::to_vec(backup).map_err(|e| format!("序列化备份失败：{e}"))?;
    let target = Path::new(path);
    let tmp = target.with_extension("json.tmp");
    // S7：替换不再「先删旧文件再 rename」——那两步之间失败会新旧备份同时丢失。
    // 改为 target → target.bak → 新文件 → 删 .bak；任一步失败回滚。
    let bak = target.with_extension("json.bak");
    std::fs::write(&tmp, &json).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("写入备份临时文件失败：{e}")
    })?;
    let had_target = target.exists();
    if had_target {
        if let Err(e) = std::fs::rename(target, &bak) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("备份替换失败（旧文件转入 .bak）：{e}"));
        }
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        // 回滚：旧文件还在 .bak 里且目标位空缺时换回去，保证旧备份不丢。
        if had_target && bak.exists() && !target.exists() {
            let _ = std::fs::rename(&bak, target);
        }
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("备份保存失败（临时文件替换目标）：{e}"));
    }
    if had_target {
        let _ = std::fs::remove_file(&bak);
    }
    Ok(json.len() as u64)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn export_backup_to_file(conn: &Connection, path: &str, config_json: Option<&str>) -> Result<ExportSummary, String> {
    let backup = collect_backup(conn, config_json)?;
    let skipped = 0usize; // 普通表按 JOIN books 采集；孤儿已在 orphan_books 中单独计入
    let size = write_backup_atomic(path, &backup)?;
    Ok(ExportSummary {
        progress: backup.progress.len(),
        bookmarks: backup.bookmarks.len(),
        highlights: backup.highlights.len(),
        tags: backup.tags.len(),
        book_tags: backup.book_tags.len(),
        orphan_books: backup.orphan_books.len(),
        orphan_highlights: orphan_highlight_count(&backup),
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
    // v2 读 v1（缺失的 orphan 字段按空）；拒绝 0 与未来版本。
    if b.version == 0 || b.version > VERSION {
        return Err(format!(
            "备份版本 {} 不受支持（当前支持 1..={}）",
            b.version, VERSION
        ));
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

/// P1-5：读文件 + JSON 解析在锁外；core 只做 DB 查询/冲突判断（调用方负责持锁）。
/// 保留包装供测试/兼容调用；生产命令直接读解析后调 core。
#[cfg_attr(not(test), allow(dead_code))]
pub fn inspect_backup_file(conn: &Connection, path: &str) -> Result<BackupPreview, String> {
    let b = read_backup(path)?;
    Ok(inspect_backup_core(conn, &b))
}

/// 已解析备份的检查核心：必须在持锁段内调用（BookIndex/冲突查询基于当前 DB 状态）。
pub fn inspect_backup_core(conn: &Connection, b: &Backup) -> BackupPreview {
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
    let orphan_highlights = orphan_highlight_count(&b);
    let total_rows = b.progress.len()
        + b.bookmarks.len()
        + b.highlights.len()
        + b.book_tags.len()
        + b.orphan_books.len()
        + orphan_highlights;
    BackupPreview {
        format: b.format.clone(),
        version: b.version,
        app_version: b.app_version.clone(),
        created_at: b.created_at,
        progress: b.progress.len(),
        bookmarks: b.bookmarks.len(),
        highlights: b.highlights.len(),
        tags: b.tags.len(),
        book_tags: b.book_tags.len(),
        orphan_books: b.orphan_books.len(),
        orphan_highlights,
        matched_books: matched.len(),
        skipped,
        conflicts,
        has_config: b.config.is_some(),
        has_shelves: b.shelves.is_some(),
        empty: total_rows == 0 && b.tags.is_empty() && b.shelves.is_none(),
    }
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

/// 本机书籍的 (路径, 跨设备身份 key)：恢复笔记后清理墓碑要用。
fn book_identity_for(conn: &Connection, book_id: i64) -> Option<(String, String)> {
    conn.query_row(
        "SELECT path, title, COALESCE(author,''), COALESCE(size,0) FROM books WHERE id=?1",
        [book_id],
        |r| {
            let path: String = r.get(0)?;
            let title: String = r.get(1)?;
            let author: String = r.get(2)?;
            let size: i64 = r.get(3)?;
            Ok((
                path.clone(),
                db::book_sync_identity(&title, &author, size, &path),
            ))
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// H19：恢复 bookmarks/highlights 后清掉对应墓碑（新身份 key + legacy 路径 key）。
/// 不清的话，下次 WebDAV 同步会被旧墓碑（早于恢复时间）重新删除。
fn clear_note_tombstones(
    conn: &Connection,
    kind: &str,
    book_id: i64,
    chapter_index: i64,
    pos_start: i64,
    pos_end: i64,
    title: &str,
    text: &str,
) {
    if let Some((path, ident)) = book_identity_for(conn, book_id) {
        let legacy = if kind == "bookmark" {
            db::bookmark_tomb_key(&path, chapter_index, title, text)
        } else {
            db::highlight_tomb_key(&path, chapter_index, pos_start, pos_end, text)
        };
        let new_key = if kind == "bookmark" {
            db::bookmark_sync_key(&ident, chapter_index, title, text)
        } else {
            db::highlight_sync_key(&ident, chapter_index, pos_start, pos_end, text)
        };
        let _ = db::remove_tombstone(conn, kind, &legacy);
        let _ = db::remove_tombstone(conn, kind, &new_key);
    }
}

/// P0-1：恢复前预解析备份能匹配到本机的书 id 集合。
/// 用于两件事：0 匹配时在写任何表之前拒绝恢复；overwrite 时只清匹配书的数据。
fn backup_matched_book_ids(conn: &Connection, b: &Backup) -> HashSet<i64> {
    let idx = BookIndex::build(conn);
    let mut matched: HashSet<i64> = HashSet::new();
    for rec in &b.progress {
        if let Some(id) = idx.resolve(&rec.book) { matched.insert(id); }
    }
    for rec in &b.bookmarks {
        if let Some(id) = idx.resolve(&rec.book) { matched.insert(id); }
    }
    for rec in &b.highlights {
        if let Some(id) = idx.resolve(&rec.book) { matched.insert(id); }
    }
    for rec in &b.book_tags {
        if let Some(id) = idx.resolve(&rec.book) { matched.insert(id); }
    }
    matched
}

/// P0-1：备份是否有可恢复内容。只有孤儿书（删书保留的划线）也算可用；
/// 纯配置/书架/标签定义没有书籍归属数据，不能单独触发写库恢复。
fn backup_has_restorable_data(matched: &HashSet<i64>, b: &Backup) -> bool {
    !matched.is_empty() || !b.orphan_books.is_empty()
}

/// P1-5：读文件 + JSON 解析在锁外；core 在锁内完成校验与写库（同一把锁、同一事务）。
/// 保留包装供测试/兼容调用；生产命令直接读解析后调 core。
#[cfg_attr(not(test), allow(dead_code))]
pub fn restore_backup_file(
    conn: &mut Connection,
    path: &str,
    overwrite: bool,
) -> Result<RestoreSummary, String> {
    let b = read_backup(path)?;
    restore_backup_core(conn, &b, overwrite)
}

/// 已解析备份的恢复核心：校验（0 匹配/matched/冲突）与写入必须在同一持锁段内，
/// 不能基于解析前的旧状态判断。
pub fn restore_backup_core(
    conn: &mut Connection,
    b: &Backup,
    overwrite: bool,
) -> Result<RestoreSummary, String> {
    // P0-1：先做「能否恢复」判定，再开事务/写表。旧实现先无条件 DELETE，
    // 0 匹配时会清空本机全部进度/划线/标签并返回 Ok（不可恢复的数据丢失）。
    let matched = backup_matched_book_ids(conn, b);
    if !backup_has_restorable_data(&matched, b) {
        return Err("备份与本机书库没有匹配的书籍，已取消恢复（未修改本地数据）".into());
    }
    let idx = BookIndex::build(conn);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if overwrite {
        // P0-1：覆盖语义只作用于「备份匹配到的书」；非匹配书的进度/划线/标签原样保留。
        // 标签定义（tags）是全局共享的，不再整体 DELETE，恢复时走 create_tag 合并。
        let ids: Vec<i64> = matched.iter().copied().collect();
        if !ids.is_empty() {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            for table in ["progress", "bookmarks", "book_tags"] {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE book_id IN ({placeholders})"),
                    rusqlite::params_from_iter(ids.iter()),
                )
                .map_err(|e| e.to_string())?;
            }
            // 本机孤儿划线（book_id<0）不在 matched 里，天然保留。
            tx.execute(
                &format!("DELETE FROM highlights WHERE book_id IN ({placeholders})"),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| e.to_string())?;
            // 只重置匹配书的反规范化进度；非匹配书保持不变（旧实现全部清零）。
            tx.execute(
                &format!(
                    "UPDATE books SET last_read = COALESCE(last_opened, 0), read_pct = 0 WHERE id IN ({placeholders})"
                ),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| e.to_string())?;
        }
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
        // H19：无论本条是否已存在都清对应墓碑（恢复语义 = 该记录重新存在，不应被旧墓碑再删）。
        clear_note_tombstones(&tx, "bookmark", book_id, rec.chapter_index, 0, 0, &rec.title, &rec.excerpt);
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM bookmarks WHERE book_id=?1 AND chapter_index=?2 AND title=?3 AND excerpt=?4",
                params![book_id, rec.chapter_index, rec.title, rec.excerpt],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 { summary.conflicts += 1; continue; }
        // H19：写 updated_at=now_ms()——旧实现只写 created_at，表默认 updated_at=0，
        // 恢复出来的条目会被早于现在的墓碑再次删除。
        tx.execute(
            "INSERT INTO bookmarks(book_id, chapter_index, title, excerpt, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![book_id, rec.chapter_index, rec.title, rec.excerpt, rec.created_at, now_ms()],
        )
        .map_err(|e| e.to_string())?;
        summary.bookmarks += 1;
    }
    for rec in &b.highlights {
        let Some(book_id) = idx.resolve(&rec.book) else { summary.skipped += 1; continue };
        // H19：冲突（已存在）同样清墓碑，避免旧墓碑在下次同步再删一次。
        clear_note_tombstones(&tx, "highlight", book_id, rec.chapter_index, rec.pos_start, rec.pos_end, "", &rec.text);
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND chapter_index=?2 AND pos_start=?3 AND pos_end=?4 AND text=?5",
                params![book_id, rec.chapter_index, rec.pos_start, rec.pos_end, rec.text],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 { summary.conflicts += 1; continue; }
        // H19：updated_at=now_ms()，并清掉对应墓碑（新身份 key + legacy 路径 key）。
        tx.execute(
            "INSERT INTO highlights(book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![book_id, rec.chapter_index, rec.chapter_title, rec.pos_start, rec.pos_end, rec.text, rec.style, rec.color, rec.note, rec.created_at, now_ms()],
        )
        .map_err(|e| e.to_string())?;
        summary.highlights += 1;
    }
    for rec in &b.tags {
        if let Ok((_id, _name)) = db::create_tag(&tx, &rec.name) {
            // H19：标签恢复同样清 tag 墓碑（create_tag 仅新建时清，已存在时这里补一次）。
            let _ = db::remove_tombstone(&tx, "tag", &norm_key(&rec.name));
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
        // H19/H20：恢复的书籍标签清 book_tag 墓碑（新身份 + legacy 两把 key）。
        if let Some((path, ident)) = book_identity_for(&tx, book_id) {
            let _ = db::remove_tombstone(&tx, "book_tag", &db::book_tag_tomb_key(&path, &rec.tag_name));
            let _ = db::remove_tombstone(&tx, "book_tag", &db::book_tag_sync_key(&ident, &rec.tag_name));
        }
        if n > 0 { summary.book_tags += 1; } else { summary.conflicts += 1; }
    }
    // C2：恢复孤儿（删书保留的划线）。本机已有同快照的孤儿书复用其负 id；否则用
    // db::next_orphan_id 分配新负 id（禁止用 path_id 造负 id，那会和其它批次撞键）。
    for o in &b.orphan_books {
        let t = o.title_snap.trim();
        let a = o.author_snap.trim();
        let existing: Option<i64> = tx
            .query_row(
                "SELECT book_id FROM orphan_books WHERE trim(title_snap)=?1 AND trim(author_snap)=?2 AND book_id<0 ORDER BY book_id DESC LIMIT 1",
                params![t, a],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None);
        let book_id = match existing {
            Some(id) => id,
            None => {
                let id = db::next_orphan_id(&tx);
                tx.execute(
                    "INSERT OR IGNORE INTO orphan_books(book_id,title_snap,author_snap,deleted_at) VALUES(?1,?2,?3,?4)",
                    params![id, o.title_snap, o.author_snap, o.deleted_at],
                )
                .map_err(|e| e.to_string())?;
                id
            }
        };
        summary.orphan_books += 1;
        for h in &o.highlights {
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND chapter_index=?2 AND pos_start=?3 AND pos_end=?4 AND text=?5",
                    params![book_id, h.chapter_index, h.pos_start, h.pos_end, h.text],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists > 0 {
                summary.conflicts += 1;
                continue;
            }
            tx.execute(
                "INSERT INTO highlights(book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![book_id, h.chapter_index, h.chapter_title, h.pos_start, h.pos_end, h.text, h.style, h.color, h.note, h.created_at, now_ms()],
            )
            .map_err(|e| e.to_string())?;
            summary.orphan_highlights += 1;
        }
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
        // P0-1：空备份/0 匹配不得进入写库，直接返回可读 Err（旧实现返回 Ok 且可能清空数据）。
        let err = restore_backup_file(&mut conn2, ep.to_str().unwrap(), false).unwrap_err();
        assert!(err.contains("未修改本地数据"), "{err}");
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
        // P0-1：备份只匹配到 b1，b2 不在 matched 中，覆盖恢复不得动它的任何数据。
        assert_eq!(pct2, 88.0, "非匹配书的 read_pct 必须保留");
        assert_eq!(last2, 999, "非匹配书的 last_read 必须保留");
        // 有备份进度的书：按备份 pct/updated_at 重建。
        let (pct1, last1): (f64, i64) = conn2
            .query_row("SELECT read_pct, last_read FROM books WHERE id=?1", [b1], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert!((pct1 - 30.0).abs() < 1e-6, "有进度书 read_pct 应按备份重建: {pct1}");
        assert!(last1 >= 100, "有进度书 last_read 应取备份 updated_at: {last1}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// C2：v2 备份包含「删书保留」的孤儿划线；覆盖恢复保留本机孤儿并恢复备份孤儿；重复恢复不重复。
    #[test]
    fn backup_v2_orphans_roundtrip_and_overwrite_keeps_local_orphans() {
        let root = tmp_root("orphans");
        let (conn, _path, sp) = setup_book(&root);
        let book_id: i64 = conn.query_row("SELECT id FROM books", [], |r| r.get(0)).unwrap();
        // 一条带笔记（删书后进孤儿）、一条纯划线（keep_notes 会删）
        conn.execute(
            "INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,1,'第一章',10,20,'孤儿正文','underline','','我的感想',100)",
            [book_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,2,'第二章',1,2,'纯划线','underline','','',200)",
            [book_id],
        )
        .unwrap();
        db::delete_books_policy(&conn, &[sp.clone()], true).unwrap();
        let (snap_t, snap_a): (String, String) = conn
            .query_row("SELECT title_snap, author_snap FROM orphan_books", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(snap_t, "书名", "删书保留应写入孤儿快照");

        let bk = root.join("bk.json").to_string_lossy().into_owned();
        let sum = export_backup_to_file(&conn, &bk, None).unwrap();
        assert_eq!((sum.orphan_books, sum.orphan_highlights), (1, 1), "导出计数应含孤儿: {sum:?}");
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&bk).unwrap()).unwrap();
        assert_eq!(raw["version"], 2, "写盘版本应为 2");
        assert_eq!(raw["orphan_books"][0]["title_snap"], snap_t);
        assert_eq!(raw["orphan_books"][0]["author_snap"], snap_a);
        assert_eq!(raw["orphan_books"][0]["highlights"][0]["note"], "我的感想");
        assert_eq!(raw["orphan_books"][0]["highlights"][0]["text"], "孤儿正文");
        // 纯划线不得进入备份
        let hl_total: i64 = conn.query_row("SELECT COUNT(*) FROM highlights", [], |r| r.get(0)).unwrap();
        assert_eq!(hl_total, 1, "keep_notes 已删纯划线");

        let pv = inspect_backup_file(&conn, &bk).unwrap();
        assert_eq!((pv.version, pv.orphan_books, pv.orphan_highlights), (2, 1, 1));
        assert!(!pv.empty, "只有孤儿也不得判空");

        // 新库：先放一个无关的本机孤儿，覆盖恢复不得删除它。
        let root2 = tmp_root("orphans-new");
        let mut conn2 = db::init(&root2.join("t.db")).unwrap();
        conn2
            .execute("INSERT INTO orphan_books(book_id,title_snap,author_snap,deleted_at) VALUES(-5,'别的书','',1)", [])
            .unwrap();
        conn2
            .execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(-5,0,'','',0,'保留我','underline','','老感想',1)", [])
            .unwrap();
        let r = restore_backup_file(&mut conn2, &bk, true).unwrap();
        assert_eq!((r.orphan_books, r.orphan_highlights), (1, 1), "恢复计数应含孤儿: {r:?}");
        let kept: i64 = conn2
            .query_row("SELECT COUNT(*) FROM highlights WHERE book_id=-5", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "覆盖恢复只删书相关划线，本机孤儿必须保留");
        let kept_book: i64 = conn2
            .query_row("SELECT COUNT(*) FROM orphan_books WHERE book_id=-5", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept_book, 1, "本机孤儿快照不得删除");
        // 备份孤儿已恢复（负 id、快照、正文、感想）
        let (bid, title, author, note): (i64, String, String, String) = conn2
            .query_row(
                "SELECT o.book_id,o.title_snap,o.author_snap,h.note FROM orphan_books o JOIN highlights h ON h.book_id=o.book_id WHERE h.text='孤儿正文'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert!(bid < 0, "孤儿 id 必须是负数: {bid}");
        assert_eq!((title.as_str(), author.as_str(), note.as_str()), (snap_t.as_str(), snap_a.as_str(), "我的感想"));
        // 重复恢复（合并模式）不重复插入
        let r2 = restore_backup_file(&mut conn2, &bk, false).unwrap();
        assert_eq!(r2.orphan_highlights, 0, "重复恢复不得重复插入孤儿划线: {r2:?}");
        assert!(r2.conflicts >= 1);
        let n: i64 = conn2
            .query_row("SELECT COUNT(*) FROM highlights WHERE text='孤儿正文'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    /// C2：v1 旧备份（无 orphan_books 字段）仍可预览与恢复。
    #[test]
    fn v1_backup_without_orphan_fields_still_works() {
        let root = tmp_root("v1compat");
        let (mut conn, _p, _sp) = setup_book(&root);
        let v1 = r#"{"format":"tidepage-backup","version":1,"appVersion":"0.1.0","createdAt":1,"progress":[],"bookmarks":[],"highlights":[],"tags":[],"bookTags":[]}"#;
        let p = root.join("v1.json");
        std::fs::write(&p, v1).unwrap();
        let pv = inspect_backup_file(&conn, p.to_str().unwrap()).unwrap();
        assert_eq!((pv.version, pv.orphan_books, pv.orphan_highlights), (1, 0, 0));
        assert!(pv.empty);
        // P0-1：v1 空备份没有可恢复条目 → Err（旧实现返回 Ok）。
        let err = restore_backup_file(&mut conn, p.to_str().unwrap(), true).unwrap_err();
        assert!(err.contains("未修改本地数据"), "{err}");
        // version 0 / 未来版本拒绝
        let v0 = root.join("v0.json");
        std::fs::write(&v0, r#"{"format":"tidepage-backup","version":0}"#).unwrap();
        assert!(inspect_backup_file(&conn, v0.to_str().unwrap()).is_err());
        let v3 = root.join("v3.json");
        std::fs::write(&v3, r#"{"format":"tidepage-backup","version":3}"#).unwrap();
        assert!(inspect_backup_file(&conn, v3.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H19：恢复 bookmarks/highlights 写 updated_at=now_ms()，并清掉新+legacy 墓碑。
    #[test]
    fn restore_refreshes_updated_at_and_clears_tombstones() {
        let root = tmp_root("tomb19");
        let (mut conn, _path, sp) = setup_book(&root);
        let book_id: i64 = conn.query_row("SELECT id FROM books", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO bookmarks(book_id,chapter_index,title,excerpt,created_at) VALUES(?1,0,'t','e',100)",
            [book_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,1,'ch',10,20,'hl','underline','','感想',200)",
            [book_id],
        )
        .unwrap();
        let (tag_id, _) = db::create_tag(&conn, "标签").unwrap();
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", params![book_id, tag_id]).unwrap();
        let bk = root.join("bk.json").to_string_lossy().into_owned();
        export_backup_to_file(&conn, &bk, None).unwrap();

        // 删掉本地条目，并写入旧墓碑（新身份 key + legacy 路径 key + tag/book_tag）
        conn.execute("DELETE FROM bookmarks", []).unwrap();
        conn.execute("DELETE FROM highlights", []).unwrap();
        conn.execute("DELETE FROM book_tags", []).unwrap();
        let ident = db::book_sync_identity("书名", "", 123, &sp);
        for (kind, key) in [
            ("bookmark", db::bookmark_tomb_key(&sp, 0, "t", "e")),
            ("bookmark", db::bookmark_sync_key(&ident, 0, "t", "e")),
            ("highlight", db::highlight_tomb_key(&sp, 1, 10, 20, "hl")),
            ("highlight", db::highlight_sync_key(&ident, 1, 10, 20, "hl")),
            ("book_tag", db::book_tag_tomb_key(&sp, "标签")),
            ("book_tag", db::book_tag_sync_key(&ident, "标签")),
            ("tag", "标签".to_lowercase()),
        ] {
            db::record_tombstone(&conn, kind, &key, 50).unwrap();
        }
        let t0 = db::now_ms();
        let r = restore_backup_file(&mut conn, &bk, false).unwrap();
        assert_eq!((r.bookmarks, r.highlights, r.book_tags), (1, 1, 1), "{r:?}");
        let (created, updated): (i64, i64) = conn
            .query_row("SELECT created_at, updated_at FROM bookmarks LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(created, 100, "created_at 应保留原值");
        assert!(updated >= t0, "恢复应写 updated_at=now_ms: {updated} < {t0}");
        let hl_updated: i64 = conn
            .query_row("SELECT updated_at FROM highlights LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(hl_updated >= t0, "划线恢复同样写 updated_at");
        let tombs: i64 = conn.query_row("SELECT COUNT(*) FROM sync_tombstones", [], |r| r.get(0)).unwrap();
        assert_eq!(tombs, 0, "恢复后对应墓碑必须清掉: {tombs}");
        let _ = std::fs::remove_dir_all(&root);
    }
    /// S7：替换失败时旧备份不丢、tmp 清理、错误可读。
    #[test]
    fn write_backup_atomic_keeps_old_on_replace_failure() {
        let dir = std::env::temp_dir().join(format!("tp_r6_backup_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("TidePage-backup.json");
        std::fs::write(&target, b"old-content").unwrap();
        // 把 .bak 占成目录：Windows 上 rename target→bak 必然失败（目录已存在）。
        let bak = target.with_extension("json.bak");
        std::fs::create_dir_all(&bak).unwrap();
        let tmp = target.with_extension("json.tmp");
        let err = write_backup_atomic(&target.to_string_lossy(), &Backup::default()).unwrap_err();
        assert!(err.contains("备份替换失败"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old-content", "旧备份内容必须保留");
        assert!(!tmp.exists(), "临时文件必须清理");
        // 清理故障后正常替换：新内容落盘，无 tmp/bak 残留。
        std::fs::remove_dir_all(&bak).unwrap();
        let n = write_backup_atomic(&target.to_string_lossy(), &Backup::default()).unwrap();
        assert!(n > 0);
        assert!(std::fs::read(&target).unwrap().starts_with(b"{"), "目标应写入新 JSON");
        assert!(!tmp.exists() && !bak.exists(), "成功后不留中间文件");
        // 首次创建（目标不存在）也正常。
        let fresh = dir.join("fresh.json");
        assert!(write_backup_atomic(&fresh.to_string_lossy(), &Backup::default()).is_ok());
        assert!(fresh.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P0-1：0 匹配备份不得修改任何本地表（旧实现先 DELETE 全表）。
    #[test]
    fn restore_zero_match_keeps_local_data() {
        let root = tmp_root("zero-match");
        let (mut conn, _p, sp) = setup_book(&root);
        let bid: i64 = conn.query_row("SELECT id FROM books", [], |r| r.get(0)).unwrap();
        db::set_progress(&conn, bid, &crate::models::Progress {
            chapter_index: 3, chapter_ratio: 0.5, pct: 55.0, updated_at: 111, device: "dev".into(),
        }).unwrap();
        conn.execute("INSERT INTO bookmarks(book_id,chapter_index,title,excerpt,created_at) VALUES(?1,1,'bm','ex',1)", [bid]).unwrap();
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,1,'ch',1,2,'hl','underline','','n',1)", [bid]).unwrap();
        let (tid, _) = db::create_tag(&conn, "本地标签").unwrap();
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", rusqlite::params![bid, tid]).unwrap();
        conn.execute("UPDATE books SET last_read=222, read_pct=55, last_opened=200 WHERE id=?1", [bid]).unwrap();
        let snapshot = |c: &Connection| -> Vec<i64> {
            vec![
                c.query_row("SELECT COUNT(*) FROM progress", [], |r| r.get(0)).unwrap(),
                c.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap(),
                c.query_row("SELECT COUNT(*) FROM highlights", [], |r| r.get(0)).unwrap(),
                c.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0)).unwrap(),
                c.query_row("SELECT COUNT(*) FROM book_tags", [], |r| r.get(0)).unwrap(),
                c.query_row("SELECT updated_at FROM progress WHERE book_id=?1", [bid], |r| r.get(0)).unwrap(),
                c.query_row("SELECT last_read FROM books WHERE id=?1", [bid], |r| r.get(0)).unwrap(),
            ]
        };
        let before = snapshot(&conn);
        let foreign = Backup {
            format: FORMAT.into(), version: VERSION,
            progress: vec![ProgressRec {
                book: BookIdent { book_path: "C:/别的机器/不存在.epub".into(), book_title: "别的书".into(), book_author: "".into(), book_size: 999 },
                chapter_index: 1, chapter_ratio: 0.2, pct: 10.0, updated_at: 500, device: "".into(),
            }],
            config: Some(serde_json::json!({"reader": {}})),
            ..Default::default()
        };
        let path = root.join("zero.json").to_string_lossy().into_owned();
        std::fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
        for ow in [false, true] {
            let err = restore_backup_file(&mut conn, &path, ow).unwrap_err();
            assert!(err.contains("未修改本地数据"), "{err}");
            assert_eq!(snapshot(&conn), before, "0 匹配恢复不得修改任何表（overwrite={ow}）");
        }
        // 路径本身仍匹配的书可正常恢复（确认失败不是一概拒绝）。
        let ok = Backup {
            format: FORMAT.into(), version: VERSION,
            progress: vec![ProgressRec {
                book: BookIdent { book_path: sp.clone(), book_title: "".into(), book_author: "".into(), book_size: 0 },
                chapter_index: 0, chapter_ratio: 0.0, pct: 1.0, updated_at: 900, device: "".into(),
            }],
            ..Default::default()
        };
        let okp = root.join("ok.json").to_string_lossy().into_owned();
        std::fs::write(&okp, serde_json::to_vec(&ok).unwrap()).unwrap();
        assert!(restore_backup_file(&mut conn, &okp, false).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// P0-1：部分匹配时只替换匹配书数据，非匹配书的进度/划线/标签保留。
    #[test]
    fn restore_partial_match_preserves_unmatched_books() {
        let root = tmp_root("partial");
        let conn = db::init(&root.join("t.db")).unwrap();
        let pa = root.join("a.epub");
        let pb = root.join("b.epub");
        std::fs::write(&pa, b"a").unwrap();
        std::fs::write(&pb, b"b").unwrap();
        let a = db::upsert_book(&conn, &pa.to_string_lossy(), "A书", "epub", 10, 1).unwrap();
        let b = db::upsert_book(&conn, &pb.to_string_lossy(), "B书", "epub", 20, 2).unwrap();
        for (id, pct) in [(a, 10.0), (b, 77.0)] {
            db::set_progress(&conn, id, &crate::models::Progress {
                chapter_index: 1, chapter_ratio: 0.4, pct, updated_at: 100, device: "".into(),
            }).unwrap();
            conn.execute("INSERT INTO bookmarks(book_id,chapter_index,title,excerpt,created_at) VALUES(?1,1,'本地书签','ex',1)", [id]).unwrap();
            conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,1,'ch',1,2,'本地划线','underline','','n',1)", [id]).unwrap();
        }
        let (ta, _) = db::create_tag(&conn, "A标签").unwrap();
        let (tb, _) = db::create_tag(&conn, "B标签").unwrap();
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", rusqlite::params![a, ta]).unwrap();
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", rusqlite::params![b, tb]).unwrap();
        let mut conn = conn;
        let backup = Backup {
            format: FORMAT.into(), version: VERSION,
            progress: vec![ProgressRec {
                book: BookIdent { book_path: pa.to_string_lossy().into_owned(), book_title: "".into(), book_author: "".into(), book_size: 0 },
                chapter_index: 5, chapter_ratio: 0.9, pct: 90.0, updated_at: 500, device: "".into(),
            }],
            bookmarks: vec![BookmarkRec {
                book: BookIdent { book_path: pa.to_string_lossy().into_owned(), book_title: "".into(), book_author: "".into(), book_size: 0 },
                chapter_index: 5, title: "备份书签".into(), excerpt: "be".into(), created_at: 2,
            }],
            highlights: vec![HighlightRec {
                book: BookIdent { book_path: pa.to_string_lossy().into_owned(), book_title: "".into(), book_author: "".into(), book_size: 0 },
                chapter_index: 5, chapter_title: "备份章".into(), pos_start: 1, pos_end: 3, text: "备份划线".into(),
                style: "underline".into(), color: "".into(), note: "".into(), created_at: 2,
            }],
            ..Default::default()
        };
        let path = root.join("partial.json").to_string_lossy().into_owned();
        std::fs::write(&path, serde_json::to_vec(&backup).unwrap()).unwrap();
        restore_backup_file(&mut conn, &path, true).unwrap();
        // A：按备份替换。
        let (apct, amark, ahl): (f64, i64, i64) = conn.query_row(
            "SELECT (SELECT pct FROM progress WHERE book_id=?1), (SELECT COUNT(*) FROM bookmarks WHERE book_id=?1), (SELECT COUNT(*) FROM highlights WHERE book_id=?1 AND text='备份划线')",
            [a], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(apct, 90.0);
        assert_eq!(amark, 1, "A 的旧书签应被覆盖替换");
        assert_eq!(ahl, 1);
        // B：完全原样。
        let (bpct, bmark, bhl, btag): (f64, i64, i64, i64) = conn.query_row(
            "SELECT (SELECT pct FROM progress WHERE book_id=?1), (SELECT COUNT(*) FROM bookmarks WHERE book_id=?1), (SELECT COUNT(*) FROM highlights WHERE book_id=?1), (SELECT COUNT(*) FROM book_tags WHERE book_id=?1)",
            [b], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!((bpct, bmark, bhl, btag), (77.0, 1, 1, 1), "非匹配书数据必须原样保留");
        // 标签定义全局共享：B 的标签仍在。
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM tags WHERE name='B标签'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// P0-1：备份只有配置/书架、没有书籍条目 → Err 且不应用任何配置/书架。
    #[test]
    fn restore_config_only_backup_rejected_without_changes() {
        let root = tmp_root("config-only");
        let (mut conn, _p, _sp) = setup_book(&root);
        db::set_setting(&conn, SHELVES_KEY, "{\"v\":1,\"shelves\":[{\"id\":\"keep\",\"name\":\"本机书架\"}]}");
        let before = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let b = Backup {
            format: FORMAT.into(), version: VERSION,
            config: Some(serde_json::json!({"reader": {}})),
            shelves: Some(serde_json::json!([{"id":"new","name":"备份书架"}])),
            ..Default::default()
        };
        let path = root.join("cfg.json").to_string_lossy().into_owned();
        std::fs::write(&path, serde_json::to_vec(&b).unwrap()).unwrap();
        let err = restore_backup_file(&mut conn, &path, true).unwrap_err();
        assert!(err.contains("未修改本地数据"), "{err}");
        assert_eq!(db::get_setting(&conn, SHELVES_KEY).unwrap(), before, "配置/书架也不得先应用");
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
            orphan_books: backup.orphan_books.len(),
            orphan_highlights: orphan_highlight_count(&backup),
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
        // P1-5：文件读取 + JSON 解析不持锁（大备份期间 DB 命令仍可响应）。
        let b = read_backup(&path)?;
        let conn = st.db.0.lock().unwrap();
        Ok(inspect_backup_core(&conn, &b))
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
        // P1-5：解析在锁外；随后同一把锁内完成校验+写入。
        let b = read_backup(&path)?;
        let mut conn = st.db.0.lock().unwrap();
        restore_backup_core(&mut conn, &b, overwrite)
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
