//! D5：WebDAV 书签/划线笔记/标签/书架同步（单文件 TidePage/data/sync.json）。
//! 进度与配置继续走各自现有同步；本模块不碰 books 表结构。
use crate::databackup::{BookIdent, BookIndex};
use crate::db;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::State;

pub const FORMAT: &str = "tidepage-data-sync";
pub const VERSION: u32 = 2;
/// v1 路径：旧客户端只读此文件；新客户端只写 v2，避免旧客户端丢元数据。
pub const REMOTE_PATH: &str = "TidePage/data/sync.json";
pub const REMOTE_PATH_V2: &str = "TidePage/data/sync-v2.json";
const TOMBSTONE_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;
const SHELVES_KEY: &str = "library.shelves.v1";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncData {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub bookmarks: Vec<SyncBookmark>,
    #[serde(default)]
    pub highlights: Vec<SyncHighlight>,
    #[serde(default)]
    pub tags: Vec<SyncTag>,
    #[serde(default)]
    pub book_tags: Vec<SyncBookTag>,
    #[serde(default)]
    pub shelves: Option<Value>,
    #[serde(default)]
    pub tombstones: Vec<SyncTombstone>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncBookmark {
    #[serde(flatten)]
    pub book: BookIdent,
    #[serde(default)]
    pub chapter_index: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub excerpt: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncHighlight {
    #[serde(flatten)]
    pub book: BookIdent,
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
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncTag {
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncBookTag {
    #[serde(flatten)]
    pub book: BookIdent,
    pub tag_name: String,
}

/// B9：删除墓碑（kind=bookmark/highlight/tag/book_tag/shelf，key 为稳定键）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncTombstone {
    pub kind: String,
    pub key: String,
    #[serde(default)]
    pub deleted_at: i64,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncOptions {
    #[serde(default)]
    pub sync_notes: bool,
    #[serde(default)]
    pub sync_tags: bool,
    #[serde(default)]
    pub sync_shelves: bool,
}

impl SyncOptions {
    fn any(&self) -> bool {
        self.sync_notes || self.sync_tags || self.sync_shelves
    }
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub key: String,
    pub kind: String,
    pub book: String,
    pub summary: String,
    pub local_time: i64,
    pub remote_time: i64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResolveChoice {
    pub key: String,
    pub choice: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub conflicts: usize,
    pub conflicts_preview: Vec<SyncConflict>,
    pub remote_found: bool,
    pub uploaded: bool,
    pub path: String,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Policy {
    Unresolved,
    Local,
    Remote,
}

fn parse_policy(s: Option<&str>) -> Policy {
    match s.unwrap_or("") {
        "local" => Policy::Local,
        "remote" => Policy::Remote,
        _ => Policy::Unresolved,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// 书架容器解析：本机 settings 是 {"v":1,"shelves":[...]}，远端 sync.json 沿用数组；
/// 对象优先取 shelves，数组直接使用，null/其它按空。
fn shelf_array(v: &Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(o)) => o.get("shelves").and_then(|s| s.as_array()).cloned().unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn shelf_name(v: &Value) -> String {
    v.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string()
}

fn shelf_created(v: &Value) -> i64 {
    v.get("createdAt").and_then(|n| n.as_i64()).unwrap_or(0)
}

/// B9：书架有效时间 = updatedAt>0 ? updatedAt : createdAt。
fn shelf_time(v: &Value) -> i64 {
    let up = v.get("updatedAt").and_then(|n| n.as_i64()).unwrap_or(0);
    if up > 0 { up } else { shelf_created(v) }
}

pub(crate) struct LocalData {
    bookmarks: Vec<SyncBookmark>,
    highlights: Vec<SyncHighlight>,
    tags: Vec<SyncTag>,
    book_tags: Vec<SyncBookTag>,
    shelves: Option<Value>,
    tombstones: Vec<SyncTombstone>,
}

fn book_ident(conn: &Connection, id: i64) -> Option<BookIdent> {
    conn.query_row(
        "SELECT path, title, COALESCE(author,''), COALESCE(size,0) FROM books WHERE id=?1",
        [id],
        |r| Ok(BookIdent {
            book_path: r.get(0)?,
            book_title: r.get(1)?,
            book_author: r.get(2).unwrap_or_default(),
            book_size: r.get(3).unwrap_or(0),
        }),
    )
    .ok()
}

fn collect_local(conn: &Connection, opts: &SyncOptions) -> Result<LocalData, String> {
    // B9：同步前清理 90 天前墓碑（本地与上传共用）。
    let _ = db::gc_tombstones(conn, now_ms() - TOMBSTONE_TTL_MS);
    let mut out = LocalData {
        bookmarks: vec![],
        highlights: vec![],
        tags: vec![],
        book_tags: vec![],
        shelves: db::get_setting(conn, SHELVES_KEY).and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        tombstones: Vec::new(),
    };
    if opts.sync_notes {
        let ident = "b.path, b.title, COALESCE(b.author,''), COALESCE(b.size,0)";
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, bm.chapter_index, bm.title, bm.excerpt, bm.created_at, bm.updated_at FROM bookmarks bm JOIN books b ON b.id=bm.book_id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(SyncBookmark {
                book: BookIdent { book_path: r.get(0)?, book_title: r.get(1)?, book_author: r.get(2).unwrap_or_default(), book_size: r.get(3).unwrap_or(0) },
                chapter_index: r.get(4)?, title: r.get(5).unwrap_or_default(), excerpt: r.get(6).unwrap_or_default(), created_at: r.get(7).unwrap_or(0), updated_at: r.get(8).unwrap_or(0),
            })
        }).map_err(|e| e.to_string())?;
        out.bookmarks = rows.flatten().collect();
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, h.chapter_index, h.chapter_title, h.pos_start, h.pos_end, h.text, h.style, h.color, h.note, h.created_at, h.updated_at FROM highlights h JOIN books b ON b.id=h.book_id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok(SyncHighlight {
                book: BookIdent { book_path: r.get(0)?, book_title: r.get(1)?, book_author: r.get(2).unwrap_or_default(), book_size: r.get(3).unwrap_or(0) },
                chapter_index: r.get(4)?, chapter_title: r.get(5).unwrap_or_default(), pos_start: r.get(6).unwrap_or(0), pos_end: r.get(7).unwrap_or(0),
                text: r.get(8).unwrap_or_default(), style: r.get(9).unwrap_or_default(), color: r.get(10).unwrap_or_default(), note: r.get(11).unwrap_or_default(), created_at: r.get(12).unwrap_or(0), updated_at: r.get(13).unwrap_or(0),
            })
        }).map_err(|e| e.to_string())?;
        out.highlights = rows.flatten().collect();
    }
    if opts.sync_tags {
        let mut stmt = conn.prepare("SELECT name, created_at, updated_at FROM tags ORDER BY name COLLATE NOCASE").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(SyncTag { name: r.get(0)?, created_at: r.get(1).unwrap_or(0), updated_at: r.get(2).unwrap_or(0) })).map_err(|e| e.to_string())?;
        out.tags = rows.flatten().collect();
        let ident = "b.path, b.title, COALESCE(b.author,''), COALESCE(b.size,0)";
        let mut stmt = conn
            .prepare(&format!("SELECT {ident}, t.name FROM book_tags bt JOIN books b ON b.id=bt.book_id JOIN tags t ON t.id=bt.tag_id ORDER BY t.name"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(SyncBookTag {
            book: BookIdent { book_path: r.get(0)?, book_title: r.get(1)?, book_author: r.get(2).unwrap_or_default(), book_size: r.get(3).unwrap_or(0) },
            tag_name: r.get(4)?,
        })).map_err(|e| e.to_string())?;
        out.book_tags = rows.flatten().collect();
    }
    out.tombstones = db::list_tombstones(conn)
        .into_iter()
        .map(|(kind, key, deleted_at)| SyncTombstone { kind, key, deleted_at })
        .collect();
    Ok(out)
}

/// B9：条目有效时间 = updated_at>0 ? updated_at : created_at。
fn eff(updated_at: i64, created_at: i64) -> i64 { if updated_at > 0 { updated_at } else { created_at } }

fn tomb_map(list: &[SyncTombstone]) -> HashMap<(String, String), i64> {
    let mut m = HashMap::new();
    for t in list {
        m.entry((t.kind.clone(), t.key.clone()))
            .and_modify(|v: &mut i64| *v = (*v).max(t.deleted_at))
            .or_insert(t.deleted_at);
    }
    m
}

/// B9：远端墓碑是否应删除本地条目（严格更新；相等留给冲突，不静默删）。
fn tomb_should_delete(tombs: &HashMap<(String, String), i64>, kind: &str, key: &str, item_time: i64) -> bool {
    tombs.get(&(kind.to_string(), key.to_string())).map(|&at| at > item_time && at != 0).unwrap_or(false)
}

fn same_highlight(l: &SyncHighlight, r: &SyncHighlight) -> bool {
    l.chapter_title == r.chapter_title
        && l.style == r.style
        && l.color == r.color
        && l.note == r.note
        && eff(l.updated_at, l.created_at) == eff(r.updated_at, r.created_at)
}

fn shelf_same_content(a: &Value, b: &Value) -> bool {
    // id 不参与比较；其余展示/筛选字段全部一致才算「无变化」。
    ["name", "createdAt", "folder", "formats", "states", "tags", "query"]
        .iter()
        .all(|f| a.get(*f).cloned().unwrap_or(Value::Null) == b.get(*f).cloned().unwrap_or(Value::Null))
}

/// H20：书籍同步身份（路径无关：归一化 title+author+size；无书名回退旧路径身份）。
fn book_identity_of(b: &BookIdent) -> String {
    db::book_sync_identity(&b.book_title, &b.book_author, b.book_size, &b.book_path)
}

fn bm_key(b: &SyncBookmark) -> String {
    db::bookmark_sync_key(&book_identity_of(&b.book), b.chapter_index, &b.title, &b.excerpt)
}

/// 旧路径 key：历史墓碑与旧客户端沿用，比较/输出时与新 key 并行。
fn bm_legacy_key(b: &SyncBookmark) -> String {
    db::bookmark_tomb_key(&b.book.book_path, b.chapter_index, &b.title, &b.excerpt)
}

fn hl_key(h: &SyncHighlight) -> String {
    db::highlight_sync_key(
        &book_identity_of(&h.book),
        h.chapter_index,
        h.pos_start,
        h.pos_end,
        &h.text,
    )
}

fn hl_legacy_key(h: &SyncHighlight) -> String {
    db::highlight_tomb_key(&h.book.book_path, h.chapter_index, h.pos_start, h.pos_end, &h.text)
}

fn bt_key(b: &SyncBookTag) -> String {
    db::book_tag_sync_key(&book_identity_of(&b.book), &b.tag_name)
}

fn bt_legacy_key(b: &SyncBookTag) -> String {
    db::book_tag_tomb_key(&b.book.book_path, &b.tag_name)
}

/// 墓碑是否应作用于该条目（删除/过滤）：
/// - legacy 路径 key 命中：保持旧行为，直接生效（不套碰撞守卫）；
/// - 新身份 key 命中：本机同身份书不唯一时保守跳过，宁可保留也不误删。
fn tomb_hits_guarded(
    conn: &Connection,
    tombs: &HashMap<(String, String), i64>,
    kind: &str,
    new_key: &str,
    legacy_key: &str,
    item_time: i64,
    book: &BookIdent,
) -> bool {
    if tomb_should_delete(tombs, kind, legacy_key, item_time) {
        return true;
    }
    tomb_should_delete(tombs, kind, new_key, item_time) && identity_unique(conn, book)
}

/// book_tag 无条目时间：墓碑存在即生效；新身份 key 同样套碰撞守卫。
fn tomb_exists_guarded(
    conn: &Connection,
    tombs: &HashMap<(String, String), i64>,
    kind: &str,
    new_key: &str,
    legacy_key: &str,
    book: &BookIdent,
) -> bool {
    if tombs.contains_key(&(kind.to_string(), legacy_key.to_string())) {
        return true;
    }
    tombs.contains_key(&(kind.to_string(), new_key.to_string())) && identity_unique(conn, book)
}

/// H20 身份碰撞保守策略：同 title+author+size 的本机书不止一本时无法唯一确定目标，
/// 远端身份墓碑不得删除任何一本的条目（宁可保留，不误删）。legacy 路径 key 路径唯一，不受此限。
fn identity_unique(conn: &Connection, b: &BookIdent) -> bool {
    if b.book_title.trim().is_empty() {
        return true;
    }
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM books WHERE lower(trim(title))=lower(trim(?1)) AND lower(trim(COALESCE(author,'')))=lower(trim(?2)) AND COALESCE(size,0)=?3",
            params![b.book_title, b.book_author, b.book_size],
            |r| r.get(0),
        )
        .unwrap_or(1);
    n <= 1
}

fn ident_with_local(conn: &Connection, rec: &BookIdent, id: i64) -> BookIdent {
    if let Some(mut local) = book_ident(conn, id) {
        // 用本机路径作为稳定键的一部分，跨设备路径不同也能对齐。
        local.book_author = rec.book_author.clone();
        local.book_size = rec.book_size;
        local
    } else {
        rec.clone()
    }
}

fn push_conflict(summary: &mut SyncSummary, policy: Policy, key: &str, kind: &str, book: &str, text: &str, lt: i64, rt: i64) {
    summary.conflicts += 1;
    if policy == Policy::Unresolved && summary.conflicts_preview.len() < 20 {
        summary.conflicts_preview.push(SyncConflict {
            key: key.into(),
            kind: kind.into(),
            book: book.into(),
            summary: text.chars().take(80).collect(),
            local_time: lt,
            remote_time: rt,
        });
    }
}

/// B9：仅首次迁移时允许读 v1；v2 缺失且已迁移时按空远端处理，避免旧 v1 复活。
pub(crate) fn should_read_v1(v2_found: bool, migrated: bool) -> bool {
    !v2_found && !migrated
}

pub fn parse_sync(bytes: &[u8]) -> Result<SyncData, String> {
    // G18：稳定错误码前缀（前端 ERR_CODES 映射成文案）；冒号后的中文仅供日志/诊断，
    // errText 按前缀命中后会整句替换，不会把内部细节上屏。
    let d: SyncData = serde_json::from_slice(bytes)
        .map_err(|e| format!("dataSync.jsonBroken：同步文件 JSON 损坏或字段缺失：{e}"))?;
    if d.format != FORMAT {
        return Err("dataSync.notFile：不是 TidePage 数据同步文件".into());
    }
    if d.version != 1 && d.version != VERSION {
        return Err(format!(
            "dataSync.versionUnsupported：同步文件版本 {} 不受支持（当前支持 1/{}）",
            d.version, VERSION
        ));
    }
    Ok(d)
}

/// 合并远端到本地并生成要上传的完整文件；DB 变更在单事务内，失败回滚。
pub(crate) fn merge_local_remote(
    conn: &mut Connection,
    local: LocalData,
    remote: &SyncData,
    opts: &SyncOptions,
    policy: Policy,
    choices: Option<&HashMap<String, String>>,
) -> Result<(SyncData, SyncSummary), String> {
    let idx = BookIndex::build(conn);
    let mut summary = SyncSummary { remote_found: true, ..Default::default() };
    let remote_tombs = tomb_map(&remote.tombstones);
    let local_tombs = tomb_map(&local.tombstones);
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    // ---- bookmarks ----
    let mut bm_out: Vec<SyncBookmark> = Vec::new();
    let mut bm_map: HashMap<String, SyncBookmark> = HashMap::new();
    for l in &local.bookmarks {
        bm_map.insert(bm_key(l), l.clone());
    }
    // B9：远端墓碑 > 本地条目时间 → 删除本地，不上传旧条目。
    // 关闭笔记同步时不得写本地/删本地，只原样输出远端类别。
    if opts.sync_notes {
        let bm_tomb: Vec<String> = bm_map
            .iter()
            .filter(|(k, l)| {
                let lk = bm_legacy_key(l);
                tomb_hits_guarded(&tx, &remote_tombs, "bookmark", k, &lk, eff(l.updated_at, l.created_at), &l.book)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in bm_tomb {
            if let Some(l) = bm_map.remove(&k) {
                let _ = tx.execute("DELETE FROM bookmarks WHERE book_id IN (SELECT id FROM books WHERE COALESCE(path,'')=?1) AND chapter_index=?2 AND title=?3 AND excerpt=?4",
                    params![l.book.book_path, l.chapter_index, l.title, l.excerpt]);
                summary.updated += 1;
            }
        }
    }
    if opts.sync_notes {
        for rec in &remote.bookmarks {
            let matched = idx.resolve(&rec.book);
            let (ident, id) = match matched {
                Some(id) => (ident_with_local(&tx, &rec.book, id), Some(id)),
                None => (rec.book.clone(), None),
            };
            let mut r = rec.clone();
            r.book = ident;
            let k = bm_key(&r);
            let lk = bm_legacy_key(&r);
            // B9：本地墓碑 > 远端条目 → 不上传旧条目；远端同 key 墓碑 > 远端旧条目 → 丢弃，避免删后又重加。
            // H20：同时查新身份 key 与 legacy 路径 key；新 key 命中时套身份唯一性守卫。
            if tomb_hits_guarded(&tx, &local_tombs, "bookmark", &k, &lk, eff(r.updated_at, r.created_at), &r.book) { continue; }
            if tomb_hits_guarded(&tx, &remote_tombs, "bookmark", &k, &lk, eff(r.updated_at, r.created_at), &r.book) { continue; }
            match bm_map.get(&k) {
                Some(l) => {
                    let (lt, rt) = (eff(l.updated_at, l.created_at), eff(r.updated_at, r.created_at));
                    // 同 key（book/chapter/title/excerpt）且有效时间相同 = 内容完全相同：无变化。
                    if lt == rt { continue; }
                    if policy == Policy::Remote || (policy == Policy::Unresolved && rt > lt && rt != 0) {
                        if rt != lt {
                            tx.execute("UPDATE bookmarks SET created_at=?1, updated_at=?2 WHERE book_id=?3 AND chapter_index=?4 AND title=?5 AND excerpt=?6",
                                params![r.created_at, r.updated_at, id, r.chapter_index, r.title, r.excerpt]).map_err(|e| e.to_string())?;
                        }
                        bm_map.insert(k, r);
                        summary.updated += 1;
                    } else if let Some(choices) = choices {
                        let ck = format!("bookmark:{k}");
                        if choices.get(&ck).map(|c| c == "remote").unwrap_or(false) {
                            if let Some(id) = id {
                                tx.execute("UPDATE bookmarks SET created_at=?1, updated_at=?2 WHERE book_id=?3 AND chapter_index=?4 AND title=?5 AND excerpt=?6",
                                    params![r.created_at, r.updated_at, id, r.chapter_index, r.title, r.excerpt]).map_err(|e| e.to_string())?;
                            }
                            bm_map.insert(k, r);
                            summary.updated += 1;
                        }
                        // 明确选择保留本机：上传也保持本机值，避免下次再冲突。
                    } else if policy == Policy::Unresolved && (rt == lt || rt == 0 || lt == 0) {
                        let ck = format!("bookmark:{k}");
                        push_conflict(&mut summary, policy, &ck, "bookmark", &r.book.book_title, &r.excerpt, lt, rt);
                        bm_map.insert(k, r);
                    }
                }
                None => {
                    if let Some(id) = id {
                        tx.execute("INSERT INTO bookmarks(book_id, chapter_index, title, excerpt, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6)",
                            params![id, r.chapter_index, r.title, r.excerpt, r.created_at, r.updated_at]).map_err(|e| e.to_string())?;
                        summary.added += 1;
                    } else {
                        summary.skipped += 1;
                    }
                    bm_map.insert(k, r);
                }
            }
        }
    }
    if opts.sync_notes {
        bm_out.extend(bm_map.into_values());
    } else {
        bm_out = remote.bookmarks.clone();
    }

    // ---- highlights ----
    let mut hl_out: Vec<SyncHighlight> = Vec::new();
    let mut hl_map: HashMap<String, SyncHighlight> = HashMap::new();
    for l in &local.highlights { hl_map.insert(hl_key(l), l.clone()); }
    if opts.sync_notes {
        let hl_tomb: Vec<String> = hl_map
            .iter()
            .filter(|(k, l)| {
                let lk = hl_legacy_key(l);
                tomb_hits_guarded(&tx, &remote_tombs, "highlight", k, &lk, eff(l.updated_at, l.created_at), &l.book)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in hl_tomb {
            if let Some(l) = hl_map.remove(&k) {
                let _ = tx.execute("DELETE FROM highlights WHERE book_id IN (SELECT id FROM books WHERE COALESCE(path,'')=?1) AND chapter_index=?2 AND pos_start=?3 AND pos_end=?4 AND text=?5",
                    params![l.book.book_path, l.chapter_index, l.pos_start, l.pos_end, l.text]);
                summary.updated += 1;
            }
        }
    }
    if opts.sync_notes {
        for rec in &remote.highlights {
            let matched = idx.resolve(&rec.book);
            let (ident, id) = match matched { Some(id) => (ident_with_local(&tx, &rec.book, id), Some(id)), None => (rec.book.clone(), None) };
            let mut r = rec.clone(); r.book = ident;
            let k = hl_key(&r);
            let lk = hl_legacy_key(&r);
            // H20：新身份 key 与 legacy 路径 key 并行比对；新 key 命中时套身份唯一性守卫。
            if tomb_hits_guarded(&tx, &local_tombs, "highlight", &k, &lk, eff(r.updated_at, r.created_at), &r.book) { continue; }
            if tomb_hits_guarded(&tx, &remote_tombs, "highlight", &k, &lk, eff(r.updated_at, r.created_at), &r.book) { continue; }
            match hl_map.get(&k) {
                Some(l) => {
                    let (lt, rt) = (eff(l.updated_at, l.created_at), eff(r.updated_at, r.created_at));
                    // key 只含位置/文本；内容相同但远端 updated_at 更新时只刷新本地时间戳。
                    if same_highlight(l, &r) {
                        let (lt, rt) = (eff(l.updated_at, l.created_at), eff(r.updated_at, r.created_at));
                        if rt > lt && rt != 0 {
                            if let Some(id) = id {
                                tx.execute("UPDATE highlights SET updated_at=?1 WHERE book_id=?2 AND chapter_index=?3 AND pos_start=?4 AND pos_end=?5 AND text=?6",
                                    params![r.updated_at, id, r.chapter_index, r.pos_start, r.pos_end, r.text]).map_err(|e| e.to_string())?;
                                summary.updated += 1;
                            }
                        }
                        continue;
                    }
                    if policy == Policy::Remote || (policy == Policy::Unresolved && rt > lt && rt != 0) {
                        if rt != lt || l.style != r.style || l.color != r.color || l.note != r.note || l.chapter_title != r.chapter_title {
                            tx.execute("UPDATE highlights SET chapter_title=?1, style=?2, color=?3, note=?4, created_at=?5, updated_at=?6 WHERE book_id=?7 AND chapter_index=?8 AND pos_start=?9 AND pos_end=?10 AND text=?11",
                                params![r.chapter_title, r.style, r.color, r.note, r.created_at, r.updated_at, id, r.chapter_index, r.pos_start, r.pos_end, r.text]).map_err(|e| e.to_string())?;
                        }
                        hl_map.insert(k, r); summary.updated += 1;
                    } else if let Some(choices) = choices {
                        let ck = format!("highlight:{k}");
                        if choices.get(&ck).map(|c| c == "remote").unwrap_or(false) {
                            if let Some(id) = id {
                                tx.execute("UPDATE highlights SET chapter_title=?1, style=?2, color=?3, note=?4, created_at=?5, updated_at=?6 WHERE book_id=?7 AND chapter_index=?8 AND pos_start=?9 AND pos_end=?10 AND text=?11",
                                    params![r.chapter_title, r.style, r.color, r.note, r.created_at, r.updated_at, id, r.chapter_index, r.pos_start, r.pos_end, r.text]).map_err(|e| e.to_string())?;
                            }
                            hl_map.insert(k, r);
                            summary.updated += 1;
                        }
                    } else if policy == Policy::Unresolved && (rt == lt || rt == 0 || lt == 0) {
                        let ck = format!("highlight:{k}");
                        push_conflict(&mut summary, policy, &ck, "highlight", &r.book.book_title, &r.text, lt, rt);
                        hl_map.insert(k, r);
                    }
                }
                None => {
                    if let Some(id) = id {
                        tx.execute("INSERT INTO highlights(book_id, chapter_index, chapter_title, pos_start, pos_end, text, style, color, note, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                            params![id, r.chapter_index, r.chapter_title, r.pos_start, r.pos_end, r.text, r.style, r.color, r.note, r.created_at, r.updated_at]).map_err(|e| e.to_string())?;
                        summary.added += 1;
                    } else { summary.skipped += 1; }
                    hl_map.insert(k, r);
                }
            }
        }
    }
    if opts.sync_notes {
        hl_out.extend(hl_map.into_values());
    } else {
        hl_out = remote.highlights.clone();
    }

    // ---- tags ----
    let mut tag_out: Vec<SyncTag> = Vec::new();
    let mut tag_map: HashMap<String, (i64, String)> = HashMap::new();
    if opts.sync_tags {
        for l in &local.tags {
            if let Some(id) = tx.query_row("SELECT id FROM tags WHERE norm=?1", [norm(&l.name)], |r| r.get::<_, i64>(0)).ok() {
                tag_map.insert(norm(&l.name), (id, l.name.clone()));
            }
            tag_out.push(l.clone());
        }
        for rec in &remote.tags {
            let n = norm(&rec.name);
            if n.is_empty() { continue; }
            // B9：本地/远端 tag 墓碑 > 远端条目 → 不上传或丢弃旧标签（远端比墓碑新则复活）。
            if tomb_should_delete(&local_tombs, "tag", &n, eff(rec.updated_at, rec.created_at)) { continue; }
            if tomb_should_delete(&remote_tombs, "tag", &n, eff(rec.updated_at, rec.created_at)) { continue; }
            if let Some(&(id, _)) = tag_map.get(&n) {
                // B9：标签比较用有效时间；远端更新则刷新本地 updated_at（名称按 norm 对齐）。
                let lt = local.tags.iter().find(|t| norm(&t.name) == n).map(|t| eff(t.updated_at, t.created_at)).unwrap_or(0);
                let rt = eff(rec.updated_at, rec.created_at);
                if rt > lt && rt != 0 {
                    tx.execute("UPDATE tags SET updated_at=?1 WHERE id=?2", params![rt, id]).map_err(|e| e.to_string())?;
                    if let Some(t) = tag_out.iter_mut().find(|t| norm(&t.name) == n) { t.updated_at = rt; }
                    summary.updated += 1;
                }
                continue;
            }
            let tag_id = db::create_tag(&tx, &rec.name).map_err(|e| e.to_string())?.0;
            tag_map.insert(n, (tag_id, rec.name.clone()));
            tag_out.push(rec.clone());
            summary.added += 1;
        }
    } else {
        tag_out = remote.tags.clone();
    }
    // B9：远端 tag 墓碑 > 本地标签时间 → 删除本地标签及其关联（仅同步标签时）。
    if opts.sync_tags {
        let local_tag_time: HashMap<String, i64> = local.tags.iter().map(|t| (norm(&t.name), eff(t.updated_at, t.created_at))).collect();
        let mut dead: Vec<String> = Vec::new();
        for (n, &at) in remote_tombs.iter().filter(|((k, _), _)| k == "tag").map(|((_, key), at)| (key, at)) {
            if local_tag_time.get(n).map(|&t| at > t && at != 0).unwrap_or(false) { dead.push(n.clone()); }
        }
        for n in dead {
            if let Some(id) = tag_map.get(&n).map(|v| v.0) {
                let _ = tx.execute("DELETE FROM book_tags WHERE tag_id=?1", params![id]);
                let _ = tx.execute("DELETE FROM tags WHERE id=?1", params![id]);
                tag_map.remove(&n);
                tag_out.retain(|t| norm(&t.name) != n);
                summary.updated += 1;
            }
        }
    }

    // ---- book_tags ----
    let mut bt_out: Vec<SyncBookTag> = Vec::new();
    let mut seen_bt: HashSet<String> = HashSet::new();
    if opts.sync_tags {
        for l in &local.book_tags {
            let k = bt_key(l);
            let lk = bt_legacy_key(l);
            // C15/OCR：book_tag 无条目时间，墓碑优先；命中本地/远端墓碑的本地关联必须删除且不进输出。
            // H20：新身份 key 套身份唯一性守卫，legacy 路径 key 保持旧行为。
            let dead = tomb_exists_guarded(&tx, &local_tombs, "book_tag", &k, &lk, &l.book)
                || tomb_exists_guarded(&tx, &remote_tombs, "book_tag", &k, &lk, &l.book);
            if dead {
                if let Some(id) = idx.resolve(&l.book) {
                    if let Some((tag_id, _)) = tag_map.get(&norm(&l.tag_name)) {
                        let _ = tx.execute("DELETE FROM book_tags WHERE book_id=?1 AND tag_id=?2", params![id, tag_id]);
                    }
                }
                summary.updated += 1;
                continue;
            }
            seen_bt.insert(k);
            bt_out.push(l.clone());
        }
        for rec in &remote.book_tags {
            let Some(id) = idx.resolve(&rec.book) else { summary.skipped += 1; bt_out.push(rec.clone()); continue };
            let ident = ident_with_local(&tx, &rec.book, id);
            let tn = norm(&rec.tag_name);
            let Some((tag_id, _)) = tag_map.get(&tn) else { summary.skipped += 1; continue };
            let mut r = rec.clone(); r.book = ident;
            let k = bt_key(&r);
            let lk = bt_legacy_key(&r);
            // book_tag 无条目时间：存在同 kind+key 远端墓碑时视为已删除，丢弃远端关联。
            if tomb_exists_guarded(&tx, &local_tombs, "book_tag", &k, &lk, &r.book) { continue; }
            if tomb_exists_guarded(&tx, &remote_tombs, "book_tag", &k, &lk, &r.book) { continue; }
            if seen_bt.contains(&k) { continue; }
            tx.execute("INSERT OR IGNORE INTO book_tags(book_id, tag_id) VALUES(?1,?2)", params![id, tag_id]).map_err(|e| e.to_string())?;
            seen_bt.insert(k);
            bt_out.push(r);
            summary.added += 1;
        }
    } else {
        bt_out = remote.book_tags.clone();
    }

    // ---- shelves：shelves_local 写本机 DB，shelves_upload 写上传文件 ----
    let mut shelves_local = shelf_array(&local.shelves);
    let mut shelves_upload: Vec<serde_json::Value>;
    if opts.sync_shelves {
        // B9：远端书架墓碑 > 本地 shelf 有效时间 → 删本地；相等保留本机并记冲突。
        let mut changed = false;
        shelves_local.retain(|s| {
            let name = shelf_name(s);
            if name.is_empty() { return true; }
            let key = norm(&name);
            match remote_tombs.get(&("shelf".to_string(), key)) {
                Some(&at) if at > shelf_time(s) && at != 0 => { summary.updated += 1; changed = true; false }
                Some(&at) if at == shelf_time(s) && at != 0 => {
                    push_conflict(&mut summary, policy, &format!("shelves:{name}"), "shelves", &name, "书架墓碑同时间", shelf_time(s), at);
                    true
                }
                _ => true,
            }
        });
        let _ = changed;
        shelves_upload = shelves_local.clone();
        let remote_sh = shelf_array(&remote.shelves);
        let mut by_name: HashMap<String, usize> = shelves_upload.iter().enumerate().map(|(i, s)| (norm(&shelf_name(s)), i)).collect();
        for r in remote_sh {
            let name = shelf_name(&r);
            if name.is_empty() { continue; }
            if tomb_should_delete(&local_tombs, "shelf", &norm(&name), shelf_time(&r)) { continue; }
            if tomb_should_delete(&remote_tombs, "shelf", &norm(&name), shelf_time(&r)) { continue; }
            match by_name.get(&norm(&name)).copied() {
                Some(i) => {
                    // 同 name 且展示/筛选字段全部相同 = 无变化（忽略 id）。
                    if shelf_same_content(&shelves_local[i], &r) { continue; }
                    let (lt, rt) = (shelf_time(&shelves_local[i]), shelf_time(&r));
                    if policy == Policy::Remote || (policy == Policy::Unresolved && rt > lt && rt != 0) {
                        shelves_local[i] = r.clone();
                        shelves_upload[i] = r;
                        summary.updated += 1;
                    } else if let Some(choices) = choices {
                        let ck = format!("shelves:{}", norm(&name));
                        if choices.get(&ck).map(|c| c == "remote").unwrap_or(false) {
                            shelves_local[i] = r.clone();
                            shelves_upload[i] = r;
                            summary.updated += 1;
                        }
                    } else if policy == Policy::Unresolved && (rt == lt || rt == 0 || lt == 0) {
                        let ck = format!("shelves:{}", norm(&name));
                        push_conflict(&mut summary, policy, &ck, "shelves", &name, "书架同名", lt, rt);
                        shelves_upload[i] = r; // 本机保留，上传保留云端
                    }
                }
                None => {
                    let idx_new = shelves_upload.len();
                    by_name.insert(norm(&name), idx_new);
                    shelves_local.push(r.clone());
                    shelves_upload.push(r);
                    summary.added += 1;
                }
            }
        }
    } else {
        // 关闭书架同步：输出原样保留远端书架，不用本机覆盖。
        shelves_upload = shelf_array(&remote.shelves);
    }
    if opts.sync_shelves {
        db::set_setting(&tx, SHELVES_KEY, &serde_json::json!({"v":1,"shelves":shelves_local}).to_string());
    }

    // B9：墓碑输出 = 本地 ∪ 远端，保留 90 天；若条目已以不早于墓碑的时间重新出现则视为复活，不再传播墓碑。
    let cutoff = now_ms() - TOMBSTONE_TTL_MS;
    let mut tmap: HashMap<(String, String), i64> = HashMap::new();
    for t in local.tombstones.iter().chain(remote.tombstones.iter()) {
        if t.deleted_at >= cutoff {
            tmap.entry((t.kind.clone(), t.key.clone()))
                .and_modify(|v| *v = (*v).max(t.deleted_at))
                .or_insert(t.deleted_at);
        }
    }
    // H20：条目复活/墓碑滞留判定同时用新身份 key 与 legacy 路径 key 建表（两种 key 都识别）。
    let bm_times_new: HashMap<String, i64> = bm_out.iter().map(|b| (bm_key(b), eff(b.updated_at, b.created_at))).collect();
    let bm_times_legacy: HashMap<String, i64> = bm_out.iter().map(|b| (bm_legacy_key(b), eff(b.updated_at, b.created_at))).collect();
    let hl_times_new: HashMap<String, i64> = hl_out.iter().map(|h| (hl_key(h), eff(h.updated_at, h.created_at))).collect();
    let hl_times_legacy: HashMap<String, i64> = hl_out.iter().map(|h| (hl_legacy_key(h), eff(h.updated_at, h.created_at))).collect();
    let tag_times: HashMap<String, i64> = tag_out.iter().map(|t| (norm(&t.name), eff(t.updated_at, t.created_at))).collect();
    let bt_keys_new: HashSet<String> = bt_out.iter().map(bt_key).collect();
    let bt_keys_legacy: HashSet<String> = bt_out.iter().map(bt_legacy_key).collect();
    let shelf_times: HashMap<String, i64> = shelf_array(&Some(serde_json::json!(shelves_upload.clone()))).iter().map(|s| (norm(&shelf_name(s)), shelf_time(s))).collect();
    tmap.retain(|(kind, key), at| {
        let newer = match kind.as_str() {
            "bookmark" => bm_times_new
                .get(key)
                .or_else(|| bm_times_legacy.get(key))
                .map(|&t| t >= *at)
                .unwrap_or(false),
            "highlight" => hl_times_new
                .get(key)
                .or_else(|| hl_times_legacy.get(key))
                .map(|&t| t >= *at)
                .unwrap_or(false),
            "tag" => tag_times.get(key).map(|&t| t >= *at).unwrap_or(false),
            "book_tag" => bt_keys_new.contains(key) || bt_keys_legacy.contains(key),
            "shelf" => shelf_times.get(key).map(|&t| t >= *at).unwrap_or(false),
            _ => false,
        };
        !newer
    });
    let tombstones: Vec<SyncTombstone> = tmap.into_iter().map(|((kind, key), deleted_at)| SyncTombstone { kind, key, deleted_at }).collect();
    tx.commit().map_err(|e| e.to_string())?;
    let out = SyncData {
        format: FORMAT.into(),
        version: VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at: now_ms(),
        device: remote.device.clone(),
        bookmarks: bm_out,
        highlights: hl_out,
        tags: tag_out,
        book_tags: bt_out,
        shelves: Some(serde_json::json!(shelves_upload)),
        tombstones,
    };
    let (format, version, created, device) = (out.format.clone(), out.version, out.created_at, out.device.clone());
    Ok((SyncData { format, version, created_at: created, device, ..out }, summary))
}

/* ---------------- Tauri 命令 ---------------- */
/// S7：条件上传冲突状态：If-Match 不匹配（412）或 If-None-Match 已存在（409）。
fn upload_conflict_status(status: u16) -> bool {
    matches!(status, 409 | 412)
}

/// 冲突重试决策：只在「冲突且尚未重试」时重试一次，其余一律失败（有界重试）。
#[derive(Debug, PartialEq, Eq)]
enum UploadAction {
    Retry,
    Fail,
}

fn upload_retry_decision(conflict: bool, retried: bool) -> UploadAction {
    if conflict && !retried {
        UploadAction::Retry
    } else {
        UploadAction::Fail
    }
}

/// 无 ETag 退化路径的内容比对（sha256），避免把「服务器已接受但内容被他人覆盖」当成功。
fn same_upload_content(a: &[u8], b: &[u8]) -> bool {
    let ha = Sha256::digest(a);
    let hb = Sha256::digest(b);
    ha == hb
}

#[tauri::command]
pub async fn webdav_sync_data(
    state: State<'_, Arc<crate::library::AppState>>,
    base: String,
    user: String,
    pass: String,
    options: SyncOptions,
    resolve: Option<String>,
    resolve_choices: Option<Vec<ResolveChoice>>,
    _device: Option<String>,
) -> Result<SyncSummary, String> {
    if !options.any() {
        return Err("未启用任何数据同步类型".into());
    }
    let client = crate::dav::DavClient::new(&base, &user, &pass)?;
    client.ensure_dir("TidePage").await.ok();
    client.ensure_dir("TidePage/data").await.ok();
    // B9：优先 v2；仅首次迁移（标记未设）且 v2 缺失时读 v1 一次，避免 v2 被删后把旧 v1 数据复活。
    let v2_migrated = {
        let conn = state.db.0.lock().unwrap();
        db::get_setting(&conn, "webdav.sync.v2_migrated").is_some()
    };
    const REMOTE_MAX: u64 = 16 * 1024 * 1024;
    // S7：v2 读同时拿 ETag 用于条件上传；v1 仍是普通 get。
    let initial_v2 = client.get_with_etag(REMOTE_PATH_V2, REMOTE_MAX).await?;
    let (mut raw_v2, mut etag_v2) = match initial_v2 {
        Some((bytes, tag)) => (Some(bytes), tag),
        None => (None, None),
    };
    let raw_v1 = if should_read_v1(raw_v2.is_some(), v2_migrated) { client.get(REMOTE_PATH).await? } else { None };
    let mut remote_bytes: Option<Vec<u8>> = raw_v2.clone().or(raw_v1.clone());
    let st = state.inner().clone();
    let policy = parse_policy(resolve.as_deref());
    let choices: Option<HashMap<String, String>> = resolve_choices.map(|v| {
        v.into_iter().map(|c| (c.key, c.choice)).collect::<HashMap<_, _>>()
    });
    let mut retried = false;
    loop {
        let remote_opt = remote_bytes.clone();
        let st2 = st.clone();
        let options_c = options.clone();
        let choices_c = choices.clone();
        let (json, mut summary) = tauri::async_runtime::spawn_blocking(move || {
            let mut conn = st2.db.0.lock().unwrap();
            let local = collect_local(&conn, &options_c)?;
            let remote = match remote_opt.as_deref() {
                Some(bytes) => parse_sync(bytes)?,
                None => SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() },
            };
            let (merged, mut summary) =
                merge_local_remote(&mut conn, local, &remote, &options_c, policy, choices_c.as_ref())?;
            summary.remote_found = remote_opt.is_some();
            summary.path = REMOTE_PATH_V2.to_string();
            let json = serde_json::to_vec(&merged).map_err(|e| format!("序列化同步文件失败：{e}"))?;
            Ok::<_, String>((json, summary))
        })
        .await
        .map_err(|e| e.to_string())??;
        let existing_v2 = raw_v2.is_some();
        let status = client
            .put_if_match(
                REMOTE_PATH_V2,
                Arc::new(json.clone()),
                if existing_v2 { etag_v2.as_deref() } else { None },
                !existing_v2,
            )
            .await?;
        if upload_conflict_status(status) {
            match upload_retry_decision(true, retried) {
                UploadAction::Retry => {
                    retried = true;
                    let fetched = client.get_with_etag(REMOTE_PATH_V2, REMOTE_MAX).await?;
                    let (b, e) = match fetched {
                        Some((bytes, tag)) => (Some(bytes), tag),
                        None => (None, None),
                    };
                    raw_v2 = b;
                    etag_v2 = e;
                    remote_bytes = raw_v2.clone().or(raw_v1.clone());
                    continue;
                }
                UploadAction::Fail => {
                    return Err("云端同步数据在同步期间被修改，请重试".into());
                }
            }
        }
        if !(200..=299).contains(&status) {
            return Err(format!("上传同步数据失败：HTTP {status}"));
        }
        // S7：服务器不回 ETag（或本次是 create）时，PUT 成功后回读比对内容；
        // 不同说明期间被他人覆盖，按冲突走一次重试（提交前再合并一次）。
        if etag_v2.is_none() || !existing_v2 {
            if let Some((body, _)) = client.get_with_etag(REMOTE_PATH_V2, REMOTE_MAX).await? {
                if !same_upload_content(&body, &json) {
                    match upload_retry_decision(true, retried) {
                        UploadAction::Retry => {
                            retried = true;
                            let fetched = client.get_with_etag(REMOTE_PATH_V2, REMOTE_MAX).await?;
                            let (b, e) = match fetched {
                                Some((bytes, tag)) => (Some(bytes), tag),
                                None => (None, None),
                            };
                            raw_v2 = b;
                            etag_v2 = e;
                            remote_bytes = raw_v2.clone().or(raw_v1.clone());
                            continue;
                        }
                        UploadAction::Fail => {
                            return Err("云端同步数据在同步期间被修改，请重试".into());
                        }
                    }
                }
            }
        }
        if !v2_migrated {
            let conn = state.db.0.lock().unwrap();
            let _ = db::set_setting(&conn, "webdav.sync.v2_migrated", "1");
        }
        summary.uploaded = true;
        use tauri::Emitter;
        let app = state.app.clone();
        let _ = app.emit("notes-changed", ());
        let _ = app.emit("library-tags-changed", ());
        let _ = app.emit("library-shelves-changed", ());
        return Ok(summary);
    }
}

/* ---------------- 测试 ---------------- */
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn tmp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp-d5-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn setup(root: &Path) -> (Connection, i64, String) {
        let conn = db::init(&root.join("t.db")).unwrap();
        let p = root.join("a.epub");
        std::fs::write(&p, b"x").unwrap();
        let sp = p.to_string_lossy().into_owned();
        let id = db::upsert_book(&conn, &sp, "书", "epub", 10, 1).unwrap();
        (conn, id, sp)
    }

    fn opts() -> SyncOptions { SyncOptions { sync_notes: true, sync_tags: true, sync_shelves: true } }

    fn ident(sp: &str) -> BookIdent {
        BookIdent { book_path: sp.into(), book_title: "书".into(), book_author: "".into(), book_size: 10 }
    }


    #[test]
    fn v1_read_only_on_first_migration() {
        assert!(should_read_v1(false, false));
        assert!(!should_read_v1(true, false));
        assert!(!should_read_v1(false, true));
        assert!(!should_read_v1(true, true));
    }

    #[test]
    fn parse_rejects_version_and_format() {
        assert!(parse_sync(br#"{"format":"tidepage-data-sync","version":9}"#).is_err());
        assert!(parse_sync(br#"{"format":"other","version":1}"#).is_err());
        assert!(parse_sync(b"not json").is_err());
        assert!(parse_sync(br#"{"format":"tidepage-data-sync","version":1}"#).is_ok());
    }





    #[test]
    fn older_remote_tombstone_does_not_kill_newer_remote_item() {
        let root = tmp_root("tomb-resurrect");
        let (mut conn, _id, sp) = setup(&root);
        let k = db::bookmark_tomb_key(&sp, 0, "t", "e");
        db::record_tombstone(&conn, "bookmark", &k, 100).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: 200, updated_at: 0 });
        remote.tombstones.push(SyncTombstone { kind: "bookmark".into(), key: k, deleted_at: 100 });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(merged.bookmarks.len(), 1, "远端条目比墓碑新应复活保留");
        assert!(!merged.tombstones.iter().any(|t| t.kind == "bookmark"), "复活后不应传播旧墓碑");
    }


    #[test]
    fn updated_at_drives_bookmark_highlight_and_tag_merge() {
        let root = tmp_root("updated");
        let (mut conn, id, sp) = setup(&root);
        // 同 created_at、远端 updated_at 新 → 远端内容赢。
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at,updated_at) VALUES(?1,0,'ch',1,2,'hl','underline','', 'local',100,50)", [id]).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "hl".into(), style: "underline".into(), color: "".into(), note: "remote".into(), created_at: 100, updated_at: 200 });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(merged.highlights[0].note, "remote");
        let (note, up): (String, i64) = conn.query_row("SELECT note, updated_at FROM highlights LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((note.as_str(), up), ("remote", 200), "接受远端应双写 updated_at");
        // 本地 updated_at 新 → 不被旧远端覆盖。
        conn.execute("UPDATE highlights SET note='local-new', updated_at=400", []).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote2 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote2.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "hl".into(), style: "underline".into(), color: "".into(), note: "remote-old".into(), created_at: 100, updated_at: 200 });
        let (merged2, _) = merge_local_remote(&mut conn, local, &remote2, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(merged2.highlights[0].note, "local-new");
        // 同内容同有效时间 → 无变化/不冲突。
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote3 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote3.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "hl".into(), style: "underline".into(), color: "".into(), note: "local-new".into(), created_at: 100, updated_at: 400 });
        let (_, summary3) = merge_local_remote(&mut conn, local, &remote3, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(summary3.conflicts, 0);
        // tag：远端 updated_at 新 → 本地 updated_at 刷新；旧则不覆盖。
        let (tag_id, _) = db::create_tag(&conn, "标签").unwrap();
        conn.execute("UPDATE tags SET updated_at=100 WHERE id=?1", [tag_id]).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote4 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote4.tags.push(SyncTag { name: "标签".into(), created_at: 1, updated_at: 200 });
        let (merged4, _) = merge_local_remote(&mut conn, local, &remote4, &opts(), Policy::Unresolved, None).unwrap();
        assert!(merged4.tags.iter().any(|t| norm(&t.name) == "标签"));
        let up: i64 = conn.query_row("SELECT updated_at FROM tags WHERE id=?1", [tag_id], |r| r.get(0)).unwrap();
        assert_eq!(up, 200, "远端 tag 更新应刷新本地 updated_at");
    }

    #[test]
    fn remote_tombstone_filters_same_key_remote_rows() {
        let root = tmp_root("remote-tomb");
        let (mut conn, id, sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        let created: i64 = conn.query_row("SELECT created_at FROM bookmarks LIMIT 1", [], |r| r.get(0)).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: created, updated_at: 0 });
        remote.tombstones.push(SyncTombstone { kind: "bookmark".into(), key: db::bookmark_tomb_key(&sp, 0, "t", "e"), deleted_at: created + 1000 });
        let (merged, summary) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert!(merged.bookmarks.is_empty(), "远端墓碑应过滤同 key 旧条目");
        assert_eq!(summary.conflicts, 0, "不应误报冲突");
        // tag：本地标签 + 远端旧标签 + 远端较新墓碑 → 输出无该标签。
        let (tag_id, _) = db::create_tag(&conn, "标签").unwrap();
        let _ = tag_id;
        let local = collect_local(&conn, &opts()).unwrap();
        let tag_time = local.tags[0].updated_at;
        let mut remote2 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote2.tags.push(SyncTag { name: "标签".into(), created_at: 1, updated_at: 0 });
        remote2.tombstones.push(SyncTombstone { kind: "tag".into(), key: "标签".into(), deleted_at: tag_time + 1000 });
        let (merged2, _) = merge_local_remote(&mut conn, local, &remote2, &opts(), Policy::Unresolved, None).unwrap();
        assert!(!merged2.tags.iter().any(|t| t.name == "标签"), "远端 tag 墓碑应过滤旧标签");
        // book_tag：远端旧关联 + 远端墓碑 → 不进入输出。
        let mut remote3 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        let bt = SyncBookTag { book: ident(&sp), tag_name: "标签".into() };
        let bt_key = db::book_tag_tomb_key(&bt.book.book_path, &bt.tag_name);
        remote3.book_tags.push(bt);
        remote3.tags.push(SyncTag { name: "标签".into(), created_at: 1, updated_at: 0 });
        remote3.tombstones.push(SyncTombstone { kind: "book_tag".into(), key: bt_key, deleted_at: 1 });
        let local = collect_local(&conn, &opts()).unwrap();
        let (merged3, _) = merge_local_remote(&mut conn, local, &remote3, &opts(), Policy::Unresolved, None).unwrap();
        assert!(merged3.book_tags.is_empty(), "远端 book_tag 墓碑应过滤远端关联");
        // shelf：远端旧书架 + 远端墓碑 → 输出无该书架。
        db::set_setting(&conn, SHELVES_KEY, &serde_json::json!({"v":1,"shelves":[{"id":"s1","name":"科幻","createdAt":10,"updatedAt":10}]}).to_string());
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote4 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote4.shelves = Some(serde_json::json!([{"id":"s1","name":"科幻","createdAt":1,"updatedAt":1}]));
        remote4.tombstones.push(SyncTombstone { kind: "shelf".into(), key: "科幻".into(), deleted_at: 1000 });
        let (merged4, _) = merge_local_remote(&mut conn, local, &remote4, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(shelf_array(&merged4.shelves).len(), 0, "远端 shelf 墓碑应过滤旧书架");
    }

    #[test]
    fn shelf_tombstone_deletes_local_and_resurrection_keeps_remote() {
        let root = tmp_root("shelf-tomb");
        let (mut conn, _id, _sp) = setup(&root);
        let shelf = serde_json::json!({"v":1,"shelves":[{"id":"s1","name":"科幻","createdAt":100,"updatedAt":100}]});
        db::set_setting(&conn, SHELVES_KEY, &shelf.to_string());
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tombstones.push(SyncTombstone { kind: "shelf".into(), key: "科幻".into(), deleted_at: 200 });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(shelf_array(&merged.shelves).len(), 0, "墓碑应删本地书架");
        // 复活：远端 shelf 比本地墓碑新 → 保留远端并移除墓碑。
        db::record_tombstone(&conn, "shelf", "科幻", 300).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote2 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote2.shelves = Some(serde_json::json!([{"id":"s1","name":"科幻","createdAt":100,"updatedAt":400}]));
        let (merged2, _) = merge_local_remote(&mut conn, local, &remote2, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(shelf_array(&merged2.shelves).len(), 1, "远端更新应复活书架");
        assert!(!merged2.tombstones.iter().any(|t| t.kind == "shelf" && t.key == "科幻"), "复活后不应传播墓碑");
    }

    #[test]
    fn remote_tombstone_deletes_local_bookmark_and_outputs_tombstone() {
        let root = tmp_root("tomb");
        let (mut conn, id, sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let created = local.bookmarks[0].created_at;
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tombstones.push(SyncTombstone { kind: "bookmark".into(), key: db::bookmark_tomb_key(&sp, 0, "t", "e"), deleted_at: created + 1000 });
        let (merged, summary) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert!(merged.bookmarks.is_empty(), "远端墓碑应删除本地书签");
        assert!(summary.updated >= 1 || summary.conflicts == 0);
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(merged.tombstones.iter().any(|t| t.kind == "bookmark"));
    }

    #[test]
    fn merge_newer_wins_equal_conflicts_and_unmatched() {
        let root = tmp_root("merge");
        let (mut conn, _id, sp) = setup(&root);
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        // 远端较新 + 远端独有 + 未匹配书。
        remote.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: 200, updated_at: 0 });
        remote.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "hl".into(), style: "underline".into(), color: "#f00".into(), note: "n".into(), created_at: 300, updated_at: 0 });
        remote.tags.push(SyncTag { name: "标签".into(), created_at: 1, updated_at: 0 });
        remote.book_tags.push(SyncBookTag { book: ident(&sp), tag_name: "标签".into() });
        remote.bookmarks.push(SyncBookmark { book: BookIdent { book_path: "C:/nope.epub".into(), book_title: "无".into(), ..Default::default() }, chapter_index: 0, title: "x".into(), excerpt: "y".into(), created_at: 5, updated_at: 0 });
        let (out, s) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(s.added, 4, "bookmark + highlight + tag + booktag: {s:?}");
        assert_eq!(s.skipped, 1, "未匹配书跳过");
        assert!(out.bookmarks.iter().any(|b| b.book.book_title == "无"), "未匹配远端保留在上传文件");
        assert_eq!(out.tags.len(), 1);

        // 本地与远端同键：较新覆盖；完全相同（同时间）再次同步应无冲突、无更新。
        let mut remote2 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote2.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: 999, updated_at: 0 });
        let local2 = collect_local(&conn, &opts()).unwrap();
        let (_out2, s2) = merge_local_remote(&mut conn, local2, &remote2, &opts(), Policy::Unresolved, None).unwrap();
        assert!(s2.updated >= 1 || s2.added >= 1, "远端较新应更新/新增: {s2:?}");
        let local3 = collect_local(&conn, &opts()).unwrap();
        let mut remote3 = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote3.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: 999, updated_at: 0 });
        let (_o3, s3) = merge_local_remote(&mut conn, local3, &remote3, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!((s3.conflicts, s3.updated, s3.added), (0, 0, 0), "内容完全相同应跳过: {s3:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn identical_resync_and_content_conflicts() {
        let root = tmp_root("identical");
        let (mut conn, id, sp) = setup(&root);
        conn.execute("INSERT INTO bookmarks(book_id,chapter_index,title,excerpt,created_at) VALUES(?1,0,'t','e',100)", [id]).unwrap();
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,0,'ch',1,2,'hl','underline','', 'local',100)", [id]).unwrap();
        db::set_setting(&conn, SHELVES_KEY, r#"{"v":1,"shelves":[{"id":"a","name":"A","createdAt":100,"formats":[],"states":[],"tags":[],"folder":null,"query":""}]}"#);
        let mk_remote = |note: &str, formats: Value| {
            let mut d = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
            d.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: 100, updated_at: 0 });
            d.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "hl".into(), style: "underline".into(), color: "".into(), note: note.into(), created_at: 100, updated_at: 0 });
            d.shelves = Some(serde_json::json!([{"id":"a","name":"A","createdAt":100,"formats":formats,"states":[],"tags":[],"folder":null,"query":""}]));
            d
        };
        // 第一次/第二次：远端与本地内容完全一致 → 0 冲突、0 更新、0 新增。
        for round in 1..=2 {
            let local = collect_local(&conn, &opts()).unwrap();
            let (_o, s) = merge_local_remote(&mut conn, local, &mk_remote("local", serde_json::json!([])), &opts(), Policy::Unresolved, None).unwrap();
            assert_eq!((s.conflicts, s.updated, s.added), (0, 0, 0), "第 {round} 次一致同步不应有变化/冲突: {s:?}");
        }
        // 同 key/同时间但内容不同：highlight note 与 shelf formats → 冲突。
        let local2 = collect_local(&conn, &opts()).unwrap();
        let (_o2, s2) = merge_local_remote(&mut conn, local2, &mk_remote("remote", serde_json::json!(["epub"])), &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(s2.conflicts, 2, "note 与 formats 不同应各计冲突: {s2:?}");
        assert_eq!(s2.conflicts_preview.len(), 2);
        // 逐条选择：highlight remote、shelf local。
        let mut choices: HashMap<String, String> = HashMap::new();
        choices.insert("highlight:".to_string() + &hl_key(&mk_remote("remote", serde_json::json!([])).highlights[0]), "remote".into());
        choices.insert("shelves:a".into(), "local".into());
        let local3 = collect_local(&conn, &opts()).unwrap();
        merge_local_remote(&mut conn, local3, &mk_remote("remote", serde_json::json!(["epub"])), &opts(), Policy::Unresolved, Some(&choices)).unwrap();
        let note: String = conn.query_row("SELECT note FROM highlights WHERE text='hl'", [], |r| r.get(0)).unwrap();
        assert_eq!(note, "remote", "highlight 选择云端应生效");
        let raw = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["shelves"][0]["formats"], serde_json::json!([]), "shelf 选择本机应保持");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_remote_and_transaction_rollback() {
        let root = tmp_root("rollback");
        let (mut conn, _id, _sp) = setup(&root);
        let local = collect_local(&conn, &opts()).unwrap();
        let empty = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        let (out, s) = merge_local_remote(&mut conn, local, &empty, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!((s.added, s.skipped), (0, 0));
        assert!(out.bookmarks.is_empty());
        // 只读连接触发事务写失败 → 不应有半写。
        conn.execute_batch("PRAGMA query_only=ON").unwrap();
        let local2 = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tags.push(SyncTag { name: "新标签".into(), created_at: 1, updated_at: 0 });
        let r = merge_local_remote(&mut conn, local2, &remote, &opts(), Policy::Unresolved, None);
        assert!(r.is_err(), "只读 DB 下写入应失败");
        conn.execute_batch("PRAGMA query_only=OFF").unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "失败后回滚，标签未写入");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shelves_write_back_and_per_item_conflict_choices() {
        let root = tmp_root("choices");
        let (mut conn, id, sp) = setup(&root);
        // 本机：两条同时间冲突的划线 + 一个旧书架 A。
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,0,'ch',1,2,'h1','underline','','local1',100)", [id]).unwrap();
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,0,'ch',3,4,'h2','underline','','local2',100)", [id]).unwrap();
        db::set_setting(&conn, SHELVES_KEY, "{\"v\":1,\"shelves\":[{\"id\":\"a\",\"name\":\"A\",\"createdAt\":100}]}");
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "h1".into(), style: "underline".into(), color: "".into(), note: "remote1".into(), created_at: 100, updated_at: 0 });
        remote.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 3, pos_end: 4, text: "h2".into(), style: "underline".into(), color: "".into(), note: "remote2".into(), created_at: 100, updated_at: 0 });
        remote.shelves = Some(serde_json::json!([
            {"id":"a","name":"A","createdAt":200},
            {"id":"b","name":"B","createdAt":300}
        ]));
        let k1 = format!("highlight:{}", hl_key(&remote.highlights[0]));
        let k2 = format!("highlight:{}", hl_key(&remote.highlights[1]));
        let choices: HashMap<String, String> = HashMap::from([(k1, "remote".into()), (k2, "local".into())]);
        let local = collect_local(&conn, &opts()).unwrap();
        let (_out, _s) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, Some(&choices)).unwrap();
        let n1: String = conn.query_row("SELECT note FROM highlights WHERE text='h1'", [], |r| r.get(0)).unwrap();
        let n2: String = conn.query_row("SELECT note FROM highlights WHERE text='h2'", [], |r| r.get(0)).unwrap();
        assert_eq!(n1, "remote1", "选择采用云端的条目应应用远端");
        assert_eq!(n2, "local2", "选择保留本机的条目不变");
        // 书架：远端较新 A 与远端独有 B 都必须写回本机 settings。
        let raw = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        let names: Vec<String> = v["shelves"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or("").to_string()).collect();
        assert!(names.contains(&"A".to_string()) && names.contains(&"B".to_string()), "书架应写回本机: {names:?}");
        let a = v["shelves"].as_array().unwrap().iter().find(|s| s["name"] == "A").unwrap();
        assert_eq!(a["createdAt"], 200, "远端较新的 A 应覆盖本机");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shelves_object_shape_merge_and_choices() {
        let root = tmp_root("shelves-object");
        let (mut conn, _id, _sp) = setup(&root);
        db::set_setting(
            &conn,
            SHELVES_KEY,
            r#"{"v":1,"shelves":[{"id":"a","name":"A","createdAt":100,"formats":[],"states":[],"tags":[],"folder":null,"query":""}]}"#,
        );
        // (a) 远端空 → 本机对象书架保留，上传仍含该书架（上传为数组形状）。
        let local = collect_local(&conn, &opts()).unwrap();
        let empty = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        let (out, _s) = merge_local_remote(&mut conn, local, &empty, &opts(), Policy::Unresolved, None).unwrap();
        let raw = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        let names: Vec<String> = v["shelves"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or("").to_string()).collect();
        assert!(names.contains(&"A".to_string()), "远端空时本机对象书架不能丢: {names:?}");
        assert!(out.shelves.as_ref().and_then(|s| s.as_array()).map(|a| !a.is_empty()).unwrap_or(false), "上传应含书架");

        // (b) 本机独有 A + 远端独有 B（远端数组形状）→ 两者都写本机并上传。
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.shelves = Some(serde_json::json!([{"id":"b","name":"B","createdAt":300,"formats":["epub"],"states":[],"tags":[],"folder":null,"query":""}]));
        let local2 = collect_local(&conn, &opts()).unwrap();
        let (out2, _s2) = merge_local_remote(&mut conn, local2, &remote, &opts(), Policy::Unresolved, None).unwrap();
        let raw2 = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let v2: Value = serde_json::from_str(&raw2).unwrap();
        let names2: Vec<String> = v2["shelves"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or("").to_string()).collect();
        assert!(names2.contains(&"A".to_string()) && names2.contains(&"B".to_string()), "并集应保留两者: {names2:?}");
        let up: Vec<Value> = out2.shelves.as_ref().and_then(|s| s.as_array()).cloned().unwrap_or_default();
        let up_names: Vec<String> = up.iter().map(|s| s["name"].as_str().unwrap_or("").to_string()).collect();
        assert!(up_names.contains(&"A".to_string()) && up_names.contains(&"B".to_string()), "上传应含两者: {up_names:?}");

        // (c)(d) 冲突逐条选择：A 采用云端、B 保留本机，验证真实对象形状回写。
        let mut conflict = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        conflict.shelves = Some(serde_json::json!([
            {"id":"a","name":"A","createdAt":100,"formats":["epub"],"states":[],"tags":[],"folder":null,"query":""},
            {"id":"b","name":"B","createdAt":300,"formats":[],"states":[],"tags":[],"folder":null,"query":""}
        ]));
        let mut choices: HashMap<String, String> = HashMap::new();
        choices.insert("shelves:a".into(), "remote".into());
        choices.insert("shelves:b".into(), "local".into());
        let local3 = collect_local(&conn, &opts()).unwrap();
        merge_local_remote(&mut conn, local3, &conflict, &opts(), Policy::Unresolved, Some(&choices)).unwrap();
        let raw3 = db::get_setting(&conn, SHELVES_KEY).unwrap();
        let v3: Value = serde_json::from_str(&raw3).unwrap();
        let a = v3["shelves"].as_array().unwrap().iter().find(|s| s["name"] == "A").unwrap();
        let b = v3["shelves"].as_array().unwrap().iter().find(|s| s["name"] == "B").unwrap();
        assert_eq!(a["formats"], serde_json::json!(["epub"]), "A 选择云端应写回");
        assert_eq!(b["formats"], serde_json::json!(["epub"]), "B 选择本机应保持（步骤 b 已带 epub）");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ocr_disabled_categories_preserve_remote_output() {
        let root = tmp_root("ocr-disabled");
        let (mut conn, id, sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "local-bm", "le").unwrap();
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at) VALUES(?1,0,'ch',1,2,'local-hl','underline','','',10)", [id]).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 1, title: "remote-bm".into(), excerpt: "re".into(), created_at: 20, updated_at: 0 });
        remote.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 1, chapter_title: "rh".into(), pos_start: 3, pos_end: 4, text: "remote-hl".into(), style: "underline".into(), color: "".into(), note: "".into(), created_at: 20, updated_at: 0 });
        remote.tags.push(SyncTag { name: "远端标签".into(), created_at: 20, updated_at: 0 });
        remote.book_tags.push(SyncBookTag { book: ident(&sp), tag_name: "远端标签".into() });
        let mut o = SyncOptions { sync_notes: false, sync_tags: false, sync_shelves: true };
        let (out, _s) = merge_local_remote(&mut conn, local, &remote, &o, Policy::Unresolved, None).unwrap();
        assert_eq!(out.bookmarks.len(), 1);
        assert_eq!(out.bookmarks[0].title, "remote-bm");
        assert_eq!(out.highlights.len(), 1);
        assert_eq!(out.highlights[0].text, "remote-hl");
        assert_eq!(out.tags.len(), 1);
        assert_eq!(out.book_tags.len(), 1);
        // 本地内容不因关闭类别被删。
        let bm: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        let hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights", [], |r| r.get(0)).unwrap();
        assert_eq!((bm, hl), (1, 1));
        // 再关 shelves：输出保留远端 shelves，不用本机覆盖。
        o.sync_shelves = false;
        let local2 = collect_local(&conn, &opts()).unwrap();
        let mut remote2 = remote.clone();
        remote2.shelves = Some(serde_json::json!([{"id":"r","name":"REMOTE","createdAt":1,"formats":[],"states":[],"tags":[],"folder":null,"query":""}]));
        let (out2, _) = merge_local_remote(&mut conn, local2, &remote2, &o, Policy::Unresolved, None).unwrap();
        let names: Vec<String> = out2.shelves.as_ref().and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|s| s["name"].as_str().map(|x| x.to_string())).collect()).unwrap_or_default();
        assert_eq!(names, vec!["REMOTE".to_string()], "关闭 shelves 时必须保留远端: {names:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ocr_book_tag_tombstone_deletes_local_association() {
        let root = tmp_root("ocr-bt-tomb");
        let (mut conn, id, sp) = setup(&root);
        let (tag_id, _) = db::create_tag(&conn, "标签").unwrap();
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", [id, tag_id]).unwrap();
        let bt = SyncBookTag { book: ident(&sp), tag_name: "标签".into() };
        let key = bt_key(&bt);
        db::record_tombstone(&conn, "book_tag", &key, now_ms()).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let empty = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        let (out, _s) = merge_local_remote(&mut conn, local, &empty, &opts(), Policy::Unresolved, None).unwrap();
        assert!(out.book_tags.iter().all(|b| bt_key(b) != key), "墓碑命中的关联不应输出");
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM book_tags", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "本地关联必须删除");
        // 远端墓碑同样删除本地关联。
        conn.execute("INSERT INTO book_tags(book_id,tag_id) VALUES(?1,?2)", [id, tag_id]).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tags.push(SyncTag { name: "标签".into(), created_at: 1, updated_at: 0 });
        remote.tombstones.push(SyncTombstone { kind: "book_tag".into(), key, deleted_at: now_ms() });
        let (out, _s) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert!(out.book_tags.is_empty());
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM book_tags", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "远端墓碑也应删除本地关联");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ocr_same_highlight_refreshes_updated_at_only() {
        let root = tmp_root("ocr-hl-time");
        let (mut conn, id, sp) = setup(&root);
        conn.execute("INSERT INTO highlights(book_id,chapter_index,chapter_title,pos_start,pos_end,text,style,color,note,created_at,updated_at) VALUES(?1,0,'ch',1,2,'same','underline','','n',100,100)", [id]).unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.highlights.push(SyncHighlight { book: ident(&sp), chapter_index: 0, chapter_title: "ch".into(), pos_start: 1, pos_end: 2, text: "same".into(), style: "underline".into(), color: "".into(), note: "n".into(), created_at: 100, updated_at: 200 });
        let (out, s) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(out.highlights.len(), 1);
        let up: i64 = conn.query_row("SELECT updated_at FROM highlights LIMIT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(up, 200, "内容相同但远端 updated_at 更新应刷新本地时间");
        assert!(s.updated >= 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H19 衔接：恢复的笔记写了新 updated_at，下次同步不能被更旧的远端墓碑再删一次。
    #[test]
    fn restored_notes_survive_sync_with_old_remote_tombstone() {
        let root = tmp_root("h19-sync");
        let (mut conn, id, sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        let bk = root.join("bk.json").to_string_lossy().into_owned();
        crate::databackup::export_backup_to_file(&conn, &bk, None).unwrap();
        let created: i64 = conn
            .query_row("SELECT created_at FROM bookmarks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        conn.execute("DELETE FROM bookmarks", []).unwrap();
        let ident_key = db::book_sync_identity("书", "", 10, &sp);
        let tomb_at = created + 1000;
        db::record_tombstone(&conn, "bookmark", &db::bookmark_tomb_key(&sp, 0, "t", "e"), tomb_at).unwrap();
        db::record_tombstone(&conn, "bookmark", &db::bookmark_sync_key(&ident_key, 0, "t", "e"), tomb_at).unwrap();
        let restored = crate::databackup::restore_backup_file(&mut conn, &bk, false).unwrap();
        assert_eq!(restored.bookmarks, 1);
        // 墓碑时间取「恢复后实际 updated_at」：旧实现只写 created_at=0 updated_at，
        // eff=created_at < tomb → 会被删；修复后 eff=updated_at==tomb（严格大于才删）→ 保留。
        let restored_updated: i64 = conn
            .query_row("SELECT updated_at FROM bookmarks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(restored_updated > created);
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.bookmarks.push(SyncBookmark { book: ident(&sp), chapter_index: 0, title: "t".into(), excerpt: "e".into(), created_at: created, updated_at: 0 });
        remote.tombstones.push(SyncTombstone {
            kind: "bookmark".into(),
            key: db::bookmark_tomb_key(&sp, 0, "t", "e"),
            deleted_at: restored_updated,
        });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(merged.bookmarks.len(), 1, "恢复后的笔记不得被旧墓碑再删");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H20①：远端墓碑用新身份 key（路径不同）也能删除本地书签/划线，并在输出里继续传播。
    #[test]
    fn remote_identity_tombstone_deletes_across_paths() {
        let root = tmp_root("h20-cross");
        let (mut conn, id, _sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        db::add_highlight(&conn, id, 1, "ch", 1, 2, "hl", "underline", "", "n").unwrap();
        let bm_created: i64 = conn
            .query_row("SELECT created_at FROM bookmarks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let hl_created: i64 = conn
            .query_row("SELECT created_at FROM highlights LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        // 另一台设备上的路径（不同），身份相同。
        let remote_ident = db::book_sync_identity("书", "", 10, "D:/另一台/书.epub");
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tombstones.push(SyncTombstone {
            kind: "bookmark".into(),
            key: db::bookmark_sync_key(&remote_ident, 0, "t", "e"),
            deleted_at: bm_created + 1000,
        });
        remote.tombstones.push(SyncTombstone {
            kind: "highlight".into(),
            key: db::highlight_sync_key(&remote_ident, 1, 1, 2, "hl"),
            deleted_at: hl_created + 1000,
        });
        let (merged, _s) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert!(merged.bookmarks.is_empty(), "跨路径身份墓碑应删除本地书签");
        assert!(merged.highlights.is_empty(), "跨路径身份墓碑应删除本地划线");
        assert!(merged.tombstones.iter().any(|t| t.kind == "bookmark"), "墓碑应继续输出");
        let bm: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        let hl: i64 = conn.query_row("SELECT COUNT(*) FROM highlights", [], |r| r.get(0)).unwrap();
        assert_eq!((bm, hl), (0, 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H20②：本地与远端同书不同路径的记录按身份合并为一条，不重复 added。
    #[test]
    fn same_book_different_paths_merge_into_one_row() {
        let root = tmp_root("h20-dedupe");
        let (mut conn, id, _sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        let created: i64 = conn
            .query_row("SELECT created_at FROM bookmarks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.bookmarks.push(SyncBookmark {
            book: BookIdent { book_path: "D:/另一台/书.epub".into(), book_title: "书".into(), book_author: "".into(), book_size: 10 },
            chapter_index: 0,
            title: "t".into(),
            excerpt: "e".into(),
            created_at: created,
            updated_at: 0,
        });
        let (out, s) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(out.bookmarks.len(), 1, "同书不同路径应合并为一条: {s:?}");
        assert_eq!(s.added, 0);
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H20③：legacy 路径墓碑同路径仍生效，且输出里的 key 逐字未变。
    #[test]
    fn legacy_tombstone_key_deletes_and_is_preserved_verbatim() {
        let root = tmp_root("h20-legacy");
        let (mut conn, id, sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        let created: i64 = conn
            .query_row("SELECT created_at FROM bookmarks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let legacy_key = db::bookmark_tomb_key(&sp, 0, "t", "e");
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tombstones.push(SyncTombstone { kind: "bookmark".into(), key: legacy_key.clone(), deleted_at: created + 1000 });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert!(merged.bookmarks.is_empty(), "legacy 同路径墓碑仍应删除");
        let out_key = merged
            .tombstones
            .iter()
            .find(|t| t.kind == "bookmark")
            .map(|t| t.key.clone())
            .expect("墓碑应继续输出");
        assert_eq!(out_key, legacy_key, "legacy 墓碑 key 必须逐字保留");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H20④：身份不同的书不误删。
    #[test]
    fn identity_tombstone_does_not_delete_different_book() {
        let root = tmp_root("h20-other");
        let (mut conn, id, _sp) = setup(&root);
        db::add_bookmark(&conn, id, 0, "t", "e").unwrap();
        let created: i64 = conn
            .query_row("SELECT created_at FROM bookmarks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let other = db::book_sync_identity("另一本书", "", 99, "D:/x.epub");
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tombstones.push(SyncTombstone {
            kind: "bookmark".into(),
            key: db::bookmark_sync_key(&other, 0, "t", "e"),
            deleted_at: created + 1000,
        });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(merged.bookmarks.len(), 1, "身份不同的书不得被误删");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H20⑤：身份碰撞（本机两本同 title+author+size）无法唯一确定 → 身份墓碑不得删除任何一本的笔记。
    #[test]
    fn identity_collision_keeps_notes_conservative() {
        let root = tmp_root("h20-collision");
        let (conn, id1, sp1) = setup(&root);
        let p2 = root.join("b.epub");
        std::fs::write(&p2, b"y").unwrap();
        let sp2 = p2.to_string_lossy().into_owned();
        let id2 = db::upsert_book(&conn, &sp2, "书", "epub", 10, 2).unwrap();
        db::add_bookmark(&conn, id1, 0, "t", "e1").unwrap();
        db::add_bookmark(&conn, id2, 0, "t", "e2").unwrap();
        let created: i64 = conn
            .query_row("SELECT created_at FROM bookmarks WHERE book_id=?1", [id1], |r| r.get(0))
            .unwrap();
        let mut conn = conn;
        let ident_remote = db::book_sync_identity("书", "", 10, "D:/另一台/书.epub");
        let local = collect_local(&conn, &opts()).unwrap();
        let mut remote = SyncData { format: FORMAT.into(), version: VERSION, ..Default::default() };
        remote.tombstones.push(SyncTombstone {
            kind: "bookmark".into(),
            key: db::bookmark_sync_key(&ident_remote, 0, "t", "e1"),
            deleted_at: created + 1000,
        });
        let (merged, _) = merge_local_remote(&mut conn, local, &remote, &opts(), Policy::Unresolved, None).unwrap();
        assert_eq!(merged.bookmarks.len(), 2, "身份碰撞时必须保守保留全部条目");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "不得删除任何一本的笔记");
        let _ = sp1;
        let _ = std::fs::remove_dir_all(&root);
    }
    /// S7：条件上传冲突判定与重试上限（纯函数）。
    #[test]
    fn upload_conflict_status_and_retry_decision() {
        assert!(upload_conflict_status(409));
        assert!(upload_conflict_status(412));
        assert!(!upload_conflict_status(200));
        assert!(!upload_conflict_status(403));
        assert_eq!(upload_retry_decision(true, false), UploadAction::Retry, "首次冲突重试一次");
        assert_eq!(upload_retry_decision(true, true), UploadAction::Fail, "重试后仍冲突必须失败");
        assert_eq!(upload_retry_decision(false, false), UploadAction::Fail);
        assert_eq!(upload_retry_decision(false, true), UploadAction::Fail);
        // 无 ETag 退化：内容一致才算成功。
        assert!(same_upload_content(b"abc", b"abc"));
        assert!(!same_upload_content(b"abc", b"abd"));
    }

    /// G18：三类解析错误都带稳定码前缀（前端 errText 映射）。
    #[test]
    fn parse_sync_errors_have_stable_codes() {
        let e = parse_sync(b"not json").unwrap_err();
        assert!(e.starts_with("dataSync.jsonBroken"), "{e}");
        let e = parse_sync(br#"{"format":"other","version":1}"#).unwrap_err();
        assert!(e.starts_with("dataSync.notFile"), "{e}");
        let e = parse_sync(br#"{"format":"tidepage-data-sync","version":9}"#).unwrap_err();
        assert!(e.starts_with("dataSync.versionUnsupported"), "{e}");
    }

}
