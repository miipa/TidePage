//! D7：重复书检测（按大小 + 采样 SHA-256；只读，不写 DB）。
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::Emitter;

pub static DUP_SCAN_RUNNING: AtomicBool = AtomicBool::new(false);
pub static DUP_SCAN_CANCEL: AtomicBool = AtomicBool::new(false);

const SAMPLE: u64 = 8 * 1024;
const SMALL_LIMIT: u64 = 24 * 1024;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateBook {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub author: String,
    pub format: String,
    pub size: i64,
    pub added_at: i64,
    pub last_read: i64,
    pub read_pct: f64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub key: String,
    pub size: i64,
    pub suspected: bool,
    /// C14：exact / title-author-equal / title-prefix-volume / title-equal-author-diff
    pub reason: String,
    /// C14：0–1；精确组恒 1.0，疑似组取该组最低置信度（最保守）。
    pub confidence: f64,
    pub books: Vec<DuplicateBook>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScanResult {
    pub cancelled: bool,
    pub groups: Vec<DuplicateGroup>,
    pub scanned: i64,
    pub skipped: i64,
    pub elapsed_ms: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct DuplicateProgress {
    running: bool,
    phase: String,
    done: i64,
    total: i64,
    groups: i64,
    skipped: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}



/// A5：首/尾摘要（>24KB 保留 head/tail digest 供第二阶段只读 mid；≤24KB 记 full hash）。
#[derive(Clone, Debug)]
pub struct EdgeDigest {
    pub head: Vec<u8>,
    pub tail: Vec<u8>,
    pub full: Option<String>,
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().to_vec()
}

fn prehash_edges(size: i64, head: &[u8], tail: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(size.max(0).to_le_bytes());
    h.update(head);
    h.update(tail);
    format!("p:{:x}", h.finalize())
}

/// 第一阶段：≤24KB 全读（final 直接可用）；>24KB 读 head8K+tail8K 并各自摘要。
pub fn edge_digests(path: &Path, size: i64) -> Result<EdgeDigest, String> {
    let size = size.max(0) as u64;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if size <= SMALL_LIMIT {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let mut h = Sha256::new();
        h.update(size.to_le_bytes());
        h.update(&buf);
        return Ok(EdgeDigest { head: vec![], tail: vec![], full: Some(format!("f:{:x}", h.finalize())) });
    }
    let mut head = vec![0u8; SAMPLE as usize];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(size - SAMPLE)).map_err(|e| e.to_string())?;
    let mut tail = vec![0u8; SAMPLE as usize];
    f.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let hd = sha256(&head);
    let td = sha256(&tail);
    Ok(EdgeDigest { head: hd, tail: td, full: None })
}

pub fn prehash_of(ed: &EdgeDigest, size: i64) -> String {
    if let Some(f) = &ed.full { f.clone() } else { prehash_edges(size, &ed.head, &ed.tail) }
}

/// 第二阶段：只读 mid 8KB 一次。最终公式 SHA256(size_le || head_digest || mid_digest || tail_digest)。
pub fn final_hash_with_mid(size: i64, head: &[u8], mid: &[u8], tail: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(size.max(0).to_le_bytes());
    h.update(head);
    h.update(mid);
    h.update(tail);
    format!("{:x}", h.finalize())
}

pub fn read_mid_digest(path: &Path, size: i64) -> Result<Vec<u8>, String> {
    let size = size.max(0) as u64;
    if size <= SMALL_LIMIT { return Ok(vec![]); }
    let mid_off = size / 2 - SAMPLE / 4;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(mid_off)).map_err(|e| e.to_string())?;
    let mut mid = vec![0u8; SAMPLE as usize];
    f.read_exact(&mut mid).map_err(|e| e.to_string())?;
    Ok(sha256(&mid))
}
/// A5 预筛（旧 helper，保留给测试/兼容）：≤24KB 全读；>24KB 只读首+尾 8KB。
#[cfg_attr(not(test), allow(dead_code))]
pub fn sampled_prehash(path: &Path, size: i64) -> Result<String, String> {
    let size = size.max(0) as u64;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(size.to_le_bytes());
    if size <= SMALL_LIMIT {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        hasher.update(&buf);
        return Ok(format!("f:{:x}", hasher.finalize()));
    }
    let mut buf = vec![0u8; SAMPLE as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    hasher.update(&buf);
    f.seek(SeekFrom::Start(size - SAMPLE)).map_err(|e| e.to_string())?;
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    hasher.update(&buf);
    Ok(format!("p:{:x}", hasher.finalize()))
}
/// 文件采样：>24KB 取首/中/尾各 8KB，≤24KB 全读；返回 SHA-256（包含 size）。
#[cfg_attr(not(test), allow(dead_code))]
pub fn sampled_sha256(path: &Path, size: i64, sample: u64, small_limit: u64) -> Result<String, String> {
    let size = size.max(0) as u64;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(size.to_le_bytes());
    if size <= small_limit {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        hasher.update(&buf);
    } else {
        let mut buf = vec![0u8; sample as usize];
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        hasher.update(&buf);
        let mid = size / 2 - sample / 4;
        f.seek(SeekFrom::Start(mid)).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        hasher.update(&buf);
        f.seek(SeekFrom::Start(size - sample)).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        hasher.update(&buf);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_meta_matches(path: &Path, size: i64, mtime_ms: i64) -> bool {
    let Ok(md) = std::fs::metadata(path) else { return false };
    if md.len() as i64 != size {
        return false;
    }
    if mtime_ms > 0 {
        let actual = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if (actual - mtime_ms).abs() > 2000 {
            return false;
        }
    }
    true
}

fn total_candidates(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM books b WHERE b.size>0 AND b.size IN (
            SELECT size FROM books WHERE size>0 GROUP BY size HAVING COUNT(*)>1)",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// C14：读取全库书目（不读文件）；用于疑似组候选，按 id 排序保证稳定。
fn read_all_books(conn: &Connection) -> Vec<DuplicateBook> {
    let mut stmt = match conn.prepare(
        "SELECT b.id, b.path, b.title, COALESCE(b.author,''), b.format, b.size, b.added_at,
                COALESCE(b.last_read,0), COALESCE(b.read_pct,0)
         FROM books b WHERE b.size>0 ORDER BY b.size, b.id",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |r| Ok(DuplicateBook {
        id: r.get(0)?, path: r.get(1)?, title: r.get(2)?, author: r.get(3)?,
        format: r.get(4)?, size: r.get(5)?, added_at: r.get(6)?,
        last_read: r.get(7)?, read_pct: r.get(8)?,
    })).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

fn read_chunk(conn: &Connection, last_size: i64, last_id: i64, limit: i64) -> rusqlite::Result<Vec<(DuplicateBook, i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.path, b.title, COALESCE(b.author,''), b.format, b.size, b.added_at,
                COALESCE(b.last_read,0), COALESCE(b.read_pct,0), COALESCE(b.mtime,0)
         FROM books b
         WHERE b.size>0 AND b.size IN (
             SELECT size FROM books WHERE size>0 GROUP BY size HAVING COUNT(*)>1)
           AND (b.size > ?1 OR (b.size = ?1 AND b.id > ?2))
         ORDER BY b.size, b.id LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![last_size, last_id, limit], |r| {
        Ok((
            DuplicateBook {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get(2)?,
                author: r.get(3)?,
                format: r.get(4)?,
                size: r.get(5)?,
                added_at: r.get(6)?,
                last_read: r.get(7)?,
                read_pct: r.get(8)?,
            },
            r.get::<_, i64>(9)?,
            r.get::<_, String>(1)?,
        ))
    })?;
    Ok(rows.flatten().collect())
}


/* ---------------- C14：疑似相似组（标题/作者归一化 + 大小相近；不读文件） ---------------- */
fn norm_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        // 全角 ASCII → 半角；其余小写、去标点/空白。
        let ch = if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
            char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
        } else { c };
        if ch.is_alphanumeric() { out.extend(ch.to_lowercase()); }
    }
    out
}

#[cfg_attr(not(test), allow(dead_code))]
fn strip_volume_suffix(title: &str) -> String {
    static VOL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:第\s*[0-9一二三四五六七八九十百千]+\s*[卷册部集]|vol\.?\s*\d+|v\d+|套装共\d+册|全集|修订版|新版|第\d+版).*$").unwrap()
    });
    let t = norm_text(title);
    VOL.replace(&t, "").to_string()
}

fn sizes_close(a: i64, b: i64, tol_pct: f64) -> bool {
    let max = a.max(b).max(1) as f64;
    (a - b).unsigned_abs() as f64 <= max * tol_pct.clamp(0.0, 1.0) || (a - b).unsigned_abs() <= 2 * 1024 * 1024
}

/// 从全库书籍里构造疑似组：只从未进入精确组的书选；按去卷册标题 key 分桶，
/// 桶内按大小接近 + 作者归一化相等/空 聚类。桶规模有上限，避免 O(n²)。
#[cfg_attr(not(test), allow(dead_code))]
pub fn suspected_groups_legacy(
    books: &[DuplicateBook],
    exact_ids: &HashSet<i64>,
    size_tolerance_pct: f64,
    max_candidates: usize,
    title_threshold: f64,
) -> Vec<DuplicateGroup> {
    let max_candidates = max_candidates.max(2);
    let mut buckets: HashMap<String, Vec<&DuplicateBook>> = HashMap::new();
    for b in books {
        if exact_ids.contains(&b.id) || b.size <= 0 { continue; }
        let key = strip_volume_suffix(&b.title);
        if key.is_empty() { continue; }
        let entry = buckets.entry(key).or_default();
        if entry.len() < max_candidates { entry.push(b); }
    }
    let mut out = Vec::new();
    for (key, mut rows) in buckets {
        if rows.len() < 2 { continue; }
        rows.sort_by_key(|b| b.size);
        let mut used = vec![false; rows.len()];
        for i in 0..rows.len() {
            if used[i] { continue; }
            let mut cluster = vec![rows[i]];
            used[i] = true;
            for j in (i + 1)..rows.len() {
                if used[j] { continue; }
                let a = rows[i];
                let b = rows[j];
                if !sizes_close(a.size, b.size, size_tolerance_pct) { continue; }
                let (an, bn) = (norm_text(&a.author), norm_text(&b.author));
                if !(an == bn || an.is_empty() || bn.is_empty()) { continue; }
                cluster.push(b);
                used[j] = true;
            }
            if cluster.len() < 2 { continue; }
            let title_same = norm_text(&cluster[0].title) == norm_text(&cluster[1].title);
            let confidence = if title_same { 0.92 } else { 0.78 };
            let reason = if title_same { "title-author-equal" } else { "title-prefix-volume" };
            out.push(DuplicateGroup {
                key: format!("suspected:{key}:{}", cluster[0].id),
                size: cluster.iter().map(|b| b.size).max().unwrap_or(0),
                suspected: true,
                reason: reason.into(),
                confidence,
                books: cluster.into_iter().cloned().collect(),
            });
        }
    }
    // C14：标题不完全相等的相似候选。先按「首字符 + base 长度桶」分桶（每桶有上限），
    // 桶内只在大小接近、作者兼容时算 Levenshtein 相似度，达到 titleThreshold 才成组。
    let threshold = title_threshold.clamp(0.5, 1.0);
    let mut approx: HashMap<String, Vec<&DuplicateBook>> = HashMap::new();
    for b in books {
        if exact_ids.contains(&b.id) || b.size <= 0 { continue; }
        let base = strip_volume_suffix(&b.title);
        if base.is_empty() { continue; }
        let first = base.chars().next().unwrap();
        let key = format!("{first}:{}", base.chars().count() / 4);
        let entry = approx.entry(key).or_default();
        if entry.len() < max_candidates { entry.push(b); }
    }
    for (_, mut rows) in approx {
        if rows.len() < 2 { continue; }
        rows.sort_by_key(|b| b.size);
        let mut used = vec![false; rows.len()];
        for i in 0..rows.len() {
            if used[i] { continue; }
            let mut cluster = vec![rows[i]];
            used[i] = true;
            let ti = strip_volume_suffix(&rows[i].title);
            for j in (i + 1)..rows.len() {
                if used[j] { continue; }
                let a = rows[i];
                let b = rows[j];
                if !sizes_close(a.size, b.size, size_tolerance_pct) { continue; }
                let tj = strip_volume_suffix(&b.title);
                if ti == tj { continue; } // 同 key 已由精确桶处理
                let (an, bn) = (norm_text(&a.author), norm_text(&b.author));
                if !(an == bn || an.is_empty() || bn.is_empty()) { continue; }
                let sim = levenshtein_similarity(&ti, &tj);
                if sim + 1e-9 < threshold { continue; }
                cluster.push(b);
                used[j] = true;
            }
            if cluster.len() < 2 { continue; }
            let sim = cluster.windows(2).map(|w| levenshtein_similarity(&strip_volume_suffix(&w[0].title), &strip_volume_suffix(&w[1].title))).fold(1.0f64, f64::min);
            out.push(DuplicateGroup {
                key: format!("suspected-sim:{}:{}", strip_volume_suffix(&cluster[0].title), cluster[0].id),
                size: cluster.iter().map(|b| b.size).max().unwrap_or(0),
                suspected: true,
                reason: "title-similar".into(),
                confidence: (0.55 + 0.25 * sim).min(0.8),
                books: cluster.into_iter().cloned().collect(),
            });
        }
    }
    out.sort_by(|a, b| b.books.len().cmp(&a.books.len()).then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal)));
    out
}


/* ---------------- C14 v2：性能/卷册安全版疑似组 ---------------- */
const SUSPECT_MAX_GROUP_BOOKS: usize = 10;
const SUSPECT_MAX_GROUPS: usize = 200;
const SUSPECT_MAX_COMPARISONS: usize = 15;

fn title_tokens(title: &str) -> (String, String, String) {
    static VOL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:第\s*[0-9一二三四五六七八九十百千]+\s*[卷册部集]|(?:vol\.?|v)\s*\d+|套装\s*共?\s*\d+\s*册|全集)").unwrap()
    });
    static ED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:第\s*\d+\s*版|修订版|新版)").unwrap()
    });
    let n = norm_text(title);
    let vols: Vec<String> = VOL.find_iter(&n).map(|m| m.as_str().to_string()).collect();
    let eds: Vec<String> = ED.find_iter(&n).map(|m| m.as_str().to_string()).collect();
    let mut base = VOL.replace_all(&n, "").to_string();
    base = ED.replace_all(&base, "").to_string();
    (base, vols.join("|"), eds.join("|"))
}

/// C14 v2：按 size 排序 + 相邻窗口 + 每本最多 N 次比较；卷/册/部/版次 token 必须完全相等才允许疑似；
/// 组 ≤10 本、全库 ≤200 组。默认 includeSuspected=false，由调用方控制。
pub fn suspected_groups(
    books: &[DuplicateBook],
    exact_ids: &HashSet<i64>,
    size_tolerance_pct: f64,
    max_candidates: usize,
    title_threshold: f64,
) -> Vec<DuplicateGroup> {
    let threshold = title_threshold.clamp(0.5, 1.0);
    let tol = size_tolerance_pct.clamp(0.0, 0.5);
    let max_comparisons = max_candidates.clamp(2, SUSPECT_MAX_COMPARISONS);
    let mut rows: Vec<&DuplicateBook> = books.iter().filter(|b| !exact_ids.contains(&b.id) && b.size > 0).collect();
    rows.sort_by_key(|b| (b.size, b.id));
    let infos: Vec<(String, String, String)> = rows.iter().map(|b| title_tokens(&b.title)).collect();
    let mut parent: Vec<usize> = (0..rows.len()).collect();
    let mut pair_conf: Vec<Vec<f64>> = vec![Vec::new(); rows.len()];
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r { r = p[r]; }
        let mut c = x;
        while p[c] != c { let n = p[c]; p[c] = r; c = n; }
        r
    }
    let mut comparisons = 0usize;
    for i in 0..rows.len() {
        let base_i = infos[i].0.as_str();
        if base_i.is_empty() { continue; }
        let first_i = base_i.chars().next().unwrap();
        let mut compared = 0usize;
        let mut examined = 0usize;
        for j in (i + 1)..rows.len() {
            if compared >= max_comparisons { break; }
            examined += 1;
            if examined > max_comparisons * 6 { break; }
            if !sizes_close(rows[i].size, rows[j].size, tol) { break; }
            let (base_j, vol_j, ed_j) = (&infos[j].0, &infos[j].1, &infos[j].2);
            if base_j.is_empty() { continue; }
            if infos[i].1 != *vol_j || infos[i].2 != *ed_j { continue; } // 卷/册/版次必须完全一致
            if base_j.chars().next() != Some(first_i) { continue; }
            let (la, lb) = (base_i.chars().count(), base_j.chars().count());
            if (la.min(lb) as f64) / (la.max(lb).max(1) as f64) < 0.55 { continue; }
            let (an, bn) = (norm_text(&rows[i].author), norm_text(&rows[j].author));
            if !(an == bn || an.is_empty() || bn.is_empty()) { continue; }
            compared += 1;
            let sim = if base_i == base_j { 1.0 } else { levenshtein_similarity(base_i, base_j) };
            if sim + 1e-9 < threshold { continue; }
            let same_title = base_i == base_j;
            let conf = if same_title { 0.92 } else { (0.55 + 0.25 * sim).min(0.8) };
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj { parent[rj] = ri; }
            pair_conf[i].push(conf);
            comparisons += 1;
        }
    }
    let _ = comparisons;
    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..rows.len() {
        let r = find(&mut parent, i);
        clusters.entry(r).or_default().push(i);
    }
    let mut out: Vec<DuplicateGroup> = Vec::new();
    for (_, idxs) in clusters {
        if idxs.len() < 2 { continue; }
        let mut books_out: Vec<DuplicateBook> = idxs.iter().take(SUSPECT_MAX_GROUP_BOOKS).map(|&i| rows[i].clone()).collect();
        books_out.sort_by_key(|b| b.id);
        let same_titles = books_out.windows(2).all(|w| title_tokens(&w[0].title).0 == title_tokens(&w[1].title).0);
        let mut conf = 1.0f64;
        for &i in &idxs { for &c in &pair_conf[i] { conf = conf.min(c); } }
        let sizes: Vec<i64> = books_out.iter().map(|b| b.size).collect();
        let max_size = *sizes.iter().max().unwrap_or(&0);
        let min_size = *sizes.iter().min().unwrap_or(&0);
        out.push(DuplicateGroup {
            key: format!("suspected:{}:{}", books_out.iter().map(|b| b.id).min().unwrap_or(0), title_tokens(&books_out[0].title).0),
            size: max_size,
            suspected: true,
            reason: if same_titles { "title-author-equal".into() } else { "title-similar".into() },
            confidence: conf.max(0.5),
            books: books_out,
        });
        let _ = min_size;
    }
    out.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal)
        .then((b.size - a.size).abs().cmp(&(a.size - b.size).abs()))
        .then(b.books.len().cmp(&a.books.len())));
    out.truncate(SUSPECT_MAX_GROUPS);
    out
}

fn levenshtein_similarity(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    if ac.is_empty() && bc.is_empty() { return 1.0; }
    let max = ac.len().max(bc.len()).max(1);
    // 截断：长度差本身已低于阈值时无需算完（这里直接 DP，字符串为书名级别，成本可控）。
    let mut prev: Vec<usize> = (0..=bc.len()).collect();
    let mut cur = vec![0usize; bc.len() + 1];
    for i in 1..=ac.len() {
        cur[0] = i;
        for j in 1..=bc.len() {
            let cost = if ac[i - 1] == bc[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    1.0 - prev[bc.len()] as f64 / max as f64
}

/// 纯分组：同 size+hash 且 >1 的组；suspected 暂不启用（报告说明）。
pub fn build_groups(map: &HashMap<(i64, String), Vec<(DuplicateBook, i64, String)>>) -> Vec<DuplicateGroup> {
    let mut out: Vec<DuplicateGroup> = map
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|((size, hash), rows)| DuplicateGroup {
            key: format!("exact:{size}:{hash}"),
            size: *size,
            suspected: false,
            reason: "exact".into(),
            confidence: 1.0,
            books: rows.iter().map(|(b, _, _)| b.clone()).collect(),
        })
        .collect();
    out.sort_by(|a, b| b.books.len().cmp(&a.books.len()).then(b.size.cmp(&a.size)));
    out
}

/// C14：扫描选项；后端夹取到安全范围。
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScanOptions {
    #[serde(default = "default_false")]
    pub include_suspected: bool,
    #[serde(default = "default_title_threshold")]
    pub title_threshold: f64,
    #[serde(default = "default_size_tolerance")]
    pub size_tolerance_pct: f64,
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
}
fn default_false() -> bool { false }
fn default_title_threshold() -> f64 { 0.82 }
fn default_size_tolerance() -> f64 { 0.10 }
fn default_max_candidates() -> usize { 500 }
impl Default for DuplicateScanOptions {
    fn default() -> Self {
        Self { include_suspected: false, title_threshold: 0.82, size_tolerance_pct: 0.10, max_candidates: 500 }
    }
}
impl DuplicateScanOptions {
    fn clamped(self) -> Self {
        Self {
            include_suspected: self.include_suspected,
            title_threshold: self.title_threshold.clamp(0.5, 1.0),
            size_tolerance_pct: self.size_tolerance_pct.clamp(0.0, 0.5),
            max_candidates: self.max_candidates.clamp(2, SUSPECT_MAX_COMPARISONS),
        }
    }
}

#[tauri::command]
pub async fn find_duplicate_books(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<crate::library::AppState>>,
    options: Option<DuplicateScanOptions>,
) -> Result<DuplicateScanResult, String> {
    let opts = options.unwrap_or_default().clamped();
    if DUP_SCAN_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("重复书扫描已在进行".into());
    }
    DUP_SCAN_CANCEL.store(false, Ordering::SeqCst);
    let st = state.inner().clone();
    let app2 = app.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let conn = st.db.0.lock().unwrap();
        let total = total_candidates(&conn);
        drop(conn);
        let mut done = 0i64;
        let mut skipped = 0i64;
        let mut last_emit = 0i64;
        // A5：先按首+尾（摘要保留）预筛，第二阶段只读中段算最终 hash。
        let mut pre: HashMap<(i64, String), (EdgeDigest, Vec<(DuplicateBook, i64, String)>)> = HashMap::new();
        let mut last_size = -1i64;
        let mut last_id = -1i64;
        loop {
            if DUP_SCAN_CANCEL.load(Ordering::SeqCst) {
                let _ = app2.emit("duplicate-progress", DuplicateProgress { running: false, phase: "done".into(), done, total, groups: 0, skipped });
                return Ok(DuplicateScanResult { cancelled: true, groups: vec![], scanned: done, skipped, elapsed_ms: t0.elapsed().as_millis() as i64 });
            }
            let chunk = {
                let guard = st.db.0.lock().unwrap();
                read_chunk(&guard, last_size, last_id, 200).map_err(|e| e.to_string())?
            };
            if chunk.is_empty() { break; }
            for (book, mtime, path) in chunk {
                last_size = book.size;
                last_id = book.id;
                done += 1;
                if !file_meta_matches(Path::new(&path), book.size, mtime) {
                    skipped += 1;
                } else if let Ok(ed) = edge_digests(Path::new(&path), book.size) {
                    let hash = prehash_of(&ed, book.size);
                    let entry = pre
                        .entry((book.size, hash))
                        .or_insert_with(|| (ed.clone(), Vec::new()));
                    entry.1.push((book, mtime, path));
                } else {
                    skipped += 1;
                }
                if DUP_SCAN_CANCEL.load(Ordering::SeqCst) {
                    let _ = app2.emit("duplicate-progress", DuplicateProgress { running: false, phase: "done".into(), done, total, groups: 0, skipped });
                    return Ok(DuplicateScanResult { cancelled: true, groups: vec![], scanned: done, skipped, elapsed_ms: t0.elapsed().as_millis() as i64 });
                }
                let tick = now_ms();
                if tick - last_emit >= 80 {
                    last_emit = tick;
                    let _ = app2.emit("duplicate-progress", DuplicateProgress { running: true, phase: "scan".into(), done, total, groups: 0, skipped });
                }
            }
        }
        // 第二阶段：只读 mid 8KB 一次；最终 hash = SHA256(size || head_digest || mid_digest || tail_digest)。
        // 受限并发 2（HDD 随机读，线程多会寻道放大）；取消标志共享。
        let mut map: HashMap<(i64, String), Vec<(DuplicateBook, i64, String)>> = HashMap::new();
        let mut big: Vec<(i64, DuplicateBook, i64, String, EdgeDigest)> = Vec::new();
        for ((size, prehash), (ed, rows)) in pre {
            if rows.len() < 2 { continue; }
            if prehash.starts_with("f:") {
                let key = prehash.trim_start_matches("f:").to_string();
                map.entry((size, key)).or_default().extend(rows);
                continue;
            }
            for (book, mtime, path) in rows {
                big.push((size, book, mtime, path, ed.clone()));
            }
        }
        let failed = AtomicUsize::new(0);
        let compute = || -> Vec<(i64, DuplicateBook, i64, String, Option<String>)> {
            big.par_iter()
                .map(|(size, book, mtime, path, ed)| {
                    if DUP_SCAN_CANCEL.load(Ordering::SeqCst) {
                        return (*size, book.clone(), *mtime, path.clone(), None);
                    }
                    match read_mid_digest(Path::new(path), book.size) {
                        Ok(mid) => (*size, book.clone(), *mtime, path.clone(), Some(final_hash_with_mid(*size, &ed.head, &mid, &ed.tail))),
                        Err(_) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            (*size, book.clone(), *mtime, path.clone(), None)
                        }
                    }
                })
                .collect()
        };
        let results = match rayon::ThreadPoolBuilder::new().num_threads(2).build() {
            Ok(pool) => pool.install(compute),
            Err(_) => compute(),
        };
        if DUP_SCAN_CANCEL.load(Ordering::SeqCst) {
            let _ = app2.emit("duplicate-progress", DuplicateProgress { running: false, phase: "done".into(), done, total, groups: 0, skipped });
            return Ok(DuplicateScanResult { cancelled: true, groups: vec![], scanned: done, skipped, elapsed_ms: t0.elapsed().as_millis() as i64 });
        }
        skipped += failed.load(Ordering::Relaxed) as i64;
        for (size, book, mtime, path, hash) in results {
            if let Some(hash) = hash {
                map.entry((size, hash)).or_default().push((book, mtime, path));
            }
        }
        let mut groups = build_groups(&map);
        // C14：疑似组只从未进入精确组的书里选，避免同一本书出现在两组；不读文件。
        if opts.include_suspected {
            let exact_ids: HashSet<i64> = groups.iter().flat_map(|g| g.books.iter().map(|b| b.id)).collect();
            let guard = st.db.0.lock().unwrap();
            let all = read_all_books(&guard);
            drop(guard);
            groups.extend(suspected_groups(&all, &exact_ids, opts.size_tolerance_pct, opts.max_candidates, opts.title_threshold));
            groups.sort_by(|a, b| a.suspected.cmp(&b.suspected)
                .then(b.books.len().cmp(&a.books.len()))
                .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal)));
        }
        let _ = app2.emit("duplicate-progress", DuplicateProgress { running: false, phase: "done".into(), done, total, groups: groups.len() as i64, skipped });
        Ok(DuplicateScanResult { cancelled: false, groups, scanned: done, skipped, elapsed_ms: t0.elapsed().as_millis() as i64 })
    })
    .await;
    DUP_SCAN_RUNNING.store(false, Ordering::SeqCst);
    let _ = app.emit("duplicate-progress", DuplicateProgress { running: false, phase: "done".into(), done: 0, total: 0, groups: 0, skipped: 0 });
    out.map_err(|e| e.to_string())?
}


#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DupDisplayGroup {
    pub rep_id: i64,
    pub copies: i64,
    pub member_ids: Vec<i64>,
}

#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DupDisplay {
    pub hidden_ids: Vec<i64>,
    pub groups: Vec<DupDisplayGroup>,
}

/// 主书库默认视图：每组（title/author/size/format）只保留代表书，返回应隐藏的 id。
/// 代表书排序：read_pct DESC, last_read DESC, length(path) ASC, id ASC；空 title 不参与去重。
#[tauri::command]
pub async fn duplicate_display_hidden(
    state: tauri::State<'_, Arc<crate::library::AppState>>,
) -> Result<DupDisplay, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 独立只读连接：WAL 下不占主连接锁，不参与 order_index/library_ready 的全局 DB 锁竞争。
        let path = crate::data_dir(&st.app).join("tidepage.db");
        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("打开只读数据库失败: {e}"))?;
        let _ = conn.busy_timeout(std::time::Duration::from_secs(2));
        display_hidden_data(&conn)
    })
    .await
    .map_err(|e| format!("去重任务失败: {e}"))?
}

/// 默认视图需要隐藏的 id（纯 SQL 窗口函数，不物化全表到 Rust）。
pub(crate) fn display_hidden_data(conn: &Connection) -> Result<DupDisplay, String> {
    const PART: &str = "PARTITION BY lower(title), lower(COALESCE(author,'')), size, format";
    const ORD: &str = "ORDER BY COALESCE(read_pct,0) DESC, COALESCE(last_read,0) DESC, length(COALESCE(path,'')) ASC, id ASC";
    let sql = format!(
        "{}{}{}{}{}{}{}",
        "SELECT id, rep_id, rn FROM ( ",
        "SELECT id, ",
        format!("FIRST_VALUE(id) OVER ({PART} {ORD}) AS rep_id, "),
        format!("ROW_NUMBER() OVER ({PART} {ORD}) AS rn, "),
        format!("COUNT(*) OVER ({PART}) AS cnt "),
        "FROM books WHERE size>0 AND COALESCE(title,'')<>'' ",
        ") WHERE cnt > 1 ORDER BY rep_id, rn"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("准备去重查询失败: {e}"))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
        .map_err(|e| format!("执行去重查询失败: {e}"))?;
    let mut groups: std::collections::BTreeMap<i64, Vec<i64>> = std::collections::BTreeMap::new();
    let mut hidden = Vec::new();
    for row in rows {
        let (id, rep_id, rn) = row.map_err(|e| format!("读取去重结果失败: {e}"))?;
        groups.entry(rep_id).or_default().push(id);
        if rn > 1 { hidden.push(id); }
    }
    let groups = groups.into_iter().map(|(rep_id, ids)| DupDisplayGroup {
        rep_id,
        copies: ids.len() as i64,
        member_ids: ids,
    }).collect();
    hidden.sort_unstable();
    Ok(DupDisplay { hidden_ids: hidden, groups })
}
#[tauri::command]
pub fn cancel_duplicate_scan() {
    DUP_SCAN_CANCEL.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tp-d7-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sample_boundaries_small_and_large() {
        let root = tmp("sample");
        let small = root.join("s.bin");
        std::fs::write(&small, b"hello world").unwrap();
        let h1 = sampled_sha256(&small, 11, SAMPLE, SMALL_LIMIT).unwrap();
        let h2 = sampled_sha256(&small, 11, SAMPLE, SMALL_LIMIT).unwrap();
        assert_eq!(h1, h2);
        let big = root.join("b.bin");
        let mut data = vec![0u8; 100_000];
        for (i, b) in data.iter_mut().enumerate() { *b = (i % 251) as u8; }
        std::fs::write(&big, &data).unwrap();
        let big1 = sampled_sha256(&big, 100_000, SAMPLE, SMALL_LIMIT).unwrap();
        let mut data2 = data.clone();
        data2[50_000] ^= 0xFF; // 改动中段：采样包含中段，必须变化
        println!("hash {}", big1);
        let other = root.join("b2.bin");
        std::fs::write(&other, &data2).unwrap();
        let big2 = sampled_sha256(&other, 100_000, SAMPLE, SMALL_LIMIT).unwrap();
        assert_ne!(big1, big2);
        let _ = std::fs::remove_dir_all(&root);
    }



    #[test]
    fn edge_and_mid_hash_formula() {
        let root = tmp("edges");
        let p = root.join("x.bin");
        let mut data = vec![0u8; 100_000];
        for (i, b) in data.iter_mut().enumerate() { *b = (i % 251) as u8; }
        std::fs::write(&p, &data).unwrap();
        let ed = edge_digests(&p, 100_000).unwrap();
        let pre = prehash_of(&ed, 100_000);
        assert!(pre.starts_with("p:"));
        let mid = read_mid_digest(&p, 100_000).unwrap();
        let f1 = final_hash_with_mid(100_000, &ed.head, &mid, &ed.tail);
        let f2 = final_hash_with_mid(100_000, &ed.head, &mid, &ed.tail);
        assert_eq!(f1, f2);
        let mut data2 = data.clone(); data2[50_000] ^= 0xFF;
        let p2 = root.join("y.bin"); std::fs::write(&p2, &data2).unwrap();
        let ed2 = edge_digests(&p2, 100_000).unwrap();
        assert_eq!(prehash_of(&ed2, 100_000), pre, "首尾同预筛同组");
        let mid2 = read_mid_digest(&p2, 100_000).unwrap();
        assert_ne!(final_hash_with_mid(100_000, &ed2.head, &mid2, &ed2.tail), f1, "中段不同最终 hash 必不同");
        let small = root.join("s.bin"); std::fs::write(&small, b"same").unwrap();
        let ed3 = edge_digests(&small, 4).unwrap();
        assert!(ed3.full.as_deref().unwrap_or("").starts_with("f:"));
        let _ = std::fs::remove_dir_all(&root);
    }




    #[test]
    fn display_hidden_same_group_and_representative_order() {
        let root = tmp("display-hidden");
        let conn = crate::db::init(&root.join("t.db")).unwrap();
        let ins = |id: i64, path: &str, title: &str, author: &str, size: i64, format: &str, pct: f64, last_read: i64| {
            conn.execute(
                "INSERT INTO books(id,path,title,author,size,format,read_pct,last_read) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![id, path, title, author, size, format, pct, last_read],
            ).unwrap();
        };
        ins(1, "C:/a.epub", "同名", "作者", 100, "epub", 90.0, 0);
        ins(2, "C:/bb.epub", "同名", "作者", 100, "epub", 50.0, 0);
        ins(3, "C:/ccc.epub", "同名", "作者", 100, "epub", 10.0, 0);
        ins(4, "C:/d.epub", "同名", "作者", 100, "pdf", 0.0, 0);   // 不同 format 不隐藏
        ins(5, "C:/e.epub", "同名", "作者", 200, "epub", 0.0, 0);  // 不同 size 不隐藏
        ins(6, "C:/f.epub", "", "作者", 100, "epub", 0.0, 0);      // 空 title 不参与
        ins(7, "C:/g-long-name.epub", "并列", "作者", 10, "epub", 0.0, 0);
        ins(8, "C:/h.epub", "并列", "作者", 10, "epub", 0.0, 0);    // 同 pct/last_read，路径短者代表
        ins(9, "C:/i-long-name.epub", "同名", "作者", 100, "epub", 0.0, 5);
        ins(10, "C:/j.epub", "同名", "作者", 100, "epub", 0.0, 0); // 同 pct 时 last_read 大者代表
        let data = display_hidden_data(&conn).unwrap();
        assert_eq!(data.hidden_ids, vec![2, 3, 7, 9, 10]);
        let same = data.groups.iter().find(|g| g.rep_id == 1).expect("同名组");
        assert_eq!(same.copies, 5);
        assert_eq!(same.member_ids, vec![1, 2, 3, 9, 10]);
        let tie = data.groups.iter().find(|g| g.rep_id == 8).expect("并列组");
        assert_eq!((tie.copies, tie.member_ids.clone()), (2, vec![8, 7]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn c14_suspected_large_synthetic_is_linear_enough() {
        let books: Vec<DuplicateBook> = (0..20_000).map(|i| DuplicateBook {
            id: i, path: format!("C:/{i}.epub"), title: format!("书{i}"), author: "作者".into(),
            format: "epub".into(), size: 1_000_000 + (i % 500) as i64, added_at: 0, last_read: 0, read_pct: 0.0,
        }).collect();
        let start = std::time::Instant::now();
        let gs = suspected_groups(&books, &HashSet::new(), 0.10, 15, 0.82);
        assert!(gs.len() <= 200);
        assert!(start.elapsed().as_secs() < 5, "20k 纯函数应远快于全流程预算: {:?}", start.elapsed());
    }

    #[test]
    fn c14_suspected_groups_normalize_volume_and_exclude_exact() {
        let b = |id: i64, title: &str, author: &str, size: i64| DuplicateBook {
            id, path: format!("C:/{id}.epub"), title: title.into(), author: author.into(),
            format: "epub".into(), size, added_at: 0, last_read: 0, read_pct: 0.0,
        };
        // 同卷/同版可组；不同卷、不同版、一方有卷一方空都不得组。
        let same = vec![
            b(1, "三体 第1卷", "刘慈欣", 1_000_000),
            b(2, "三体 第1卷", "刘慈欣", 1_050_000),
        ];
        let gs = suspected_groups(&same, &HashSet::new(), 0.10, 15, 0.82);
        assert_eq!(gs.len(), 1);
        assert_eq!(gs[0].reason, "title-author-equal");
        assert!(gs[0].books.iter().all(|b| b.id == 1 || b.id == 2));
        let diff_vol = vec![
            b(1, "三体 第1卷", "刘慈欣", 1_000_000),
            b(3, "三体 第2卷", "刘慈欣", 1_000_000),
        ];
        assert!(suspected_groups(&diff_vol, &HashSet::new(), 0.10, 15, 0.82).is_empty(), "不同卷不得组");
        let mixed_vol = vec![
            b(1, "三体 第1卷", "刘慈欣", 1_000_000),
            b(4, "三体", "刘慈欣", 1_000_000),
        ];
        assert!(suspected_groups(&mixed_vol, &HashSet::new(), 0.10, 15, 0.82).is_empty(), "一方有卷一方空不得组");
        let diff_edition = vec![
            b(6, "三体 第1版", "刘慈欣", 1_000_000),
            b(7, "三体 第2版", "刘慈欣", 1_000_000),
        ];
        assert!(suspected_groups(&diff_edition, &HashSet::new(), 0.10, 15, 0.82).is_empty(), "不同版次不得组");
        // 精确组书排除。
        let mut exact = HashSet::new();
        exact.insert(2);
        let only_one = suspected_groups(&same, &exact, 0.10, 15, 0.82);
        assert!(only_one.iter().all(|g| g.books.iter().all(|b| b.id != 2)));
        // 标题不完全相等：多字/副标题命中 title-similar；高阈值不命中低相似。
        let sim_books = vec![b(10, "人类简史", "尤瓦尔", 2_000_000), b(11, "人类简史绘本版", "尤瓦尔", 2_050_000)];
        assert!(suspected_groups(&sim_books, &HashSet::new(), 0.10, 15, 0.5).iter().any(|g| g.reason == "title-similar"));
        let low = vec![b(20, "abcde", "a", 1_000), b(21, "abxyz", "a", 1_000)];
        assert!(suspected_groups(&low, &HashSet::new(), 0.10, 15, 0.95).iter().all(|g| g.reason != "title-similar"));
        // 每组最多 10 本。
        let many: Vec<DuplicateBook> = (0..15).map(|i| b(100 + i, "同一本书 第1卷", "a", 1_000_000 + i as i64)).collect();
        let many_groups = suspected_groups(&many, &HashSet::new(), 0.10, 15, 0.82);
        assert!(many_groups.iter().all(|g| g.books.len() <= 10));
        // 大小远隔不比较（不组）。
        let far = vec![b(30, "同一本书 第1卷", "a", 1_000_000), b(31, "同一本书 第1卷", "a", 9_000_000)];
        assert!(suspected_groups(&far, &HashSet::new(), 0.01, 15, 0.82).is_empty());
        // 疑似 key 稳定性：库中增删无关书后同组 key 不变。
        let base: Vec<DuplicateBook> = vec![b(1, "稳定书 第1卷", "a", 1_000_000), b(2, "稳定书 第1卷", "a", 1_050_000)];
        let k1 = suspected_groups(&base, &HashSet::new(), 0.10, 15, 0.82)[0].key.clone();
        let mut more = base.clone();
        more.push(b(3, "完全无关", "b", 50_000_000));
        let k2 = suspected_groups(&more, &HashSet::new(), 0.10, 15, 0.82).iter().find(|g| g.books.iter().any(|x| x.id == 1)).unwrap().key.clone();
        assert_eq!(k1, k2, "疑似 key 不应随无关书增删变化");
        // 选项夹取。
        let c = DuplicateScanOptions { include_suspected: true, title_threshold: 9.0, size_tolerance_pct: -1.0, max_candidates: 1 }.clamped();
        assert_eq!(c.title_threshold, 1.0);
        assert_eq!(c.size_tolerance_pct, 0.0);
        assert_eq!(c.max_candidates, 2);
    }

    #[test]
    fn prefilter_same_edges_different_middle_not_grouped() {
        let root = tmp("prefilter");
        let mut base = vec![0u8; 100_000];
        for (i, b) in base.iter_mut().enumerate() { *b = (i % 251) as u8; }
        let a = root.join("a.bin"); std::fs::write(&a, &base).unwrap();
        let mut mid = base.clone(); mid[50_000] ^= 0xFF;
        let b = root.join("b.bin"); std::fs::write(&b, &mid).unwrap();
        let c = root.join("c.bin"); std::fs::write(&c, &base).unwrap();
        let pa = sampled_prehash(&a, 100_000).unwrap();
        let pb = sampled_prehash(&b, 100_000).unwrap();
        let pc = sampled_prehash(&c, 100_000).unwrap();
        assert_eq!(pa, pb, "首尾相同预筛应同组");
        assert_eq!(pa, pc);
        let fa = sampled_sha256(&a, 100_000, SAMPLE, SMALL_LIMIT).unwrap();
        let fb = sampled_sha256(&b, 100_000, SAMPLE, SMALL_LIMIT).unwrap();
        let fc = sampled_sha256(&c, 100_000, SAMPLE, SMALL_LIMIT).unwrap();
        assert_ne!(fa, fb, "中段不同最终 hash 必须不同");
        assert_eq!(fa, fc);
        let small1 = root.join("s1.bin"); std::fs::write(&small1, b"same").unwrap();
        let small2 = root.join("s2.bin"); std::fs::write(&small2, b"same").unwrap();
        assert_eq!(sampled_prehash(&small1, 4).unwrap(), sampled_prehash(&small2, 4).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn grouping_and_skip_unreadable() {
        let root = tmp("group");
        let a = root.join("a.bin"); std::fs::write(&a, b"same").unwrap();
        let b = root.join("b.bin"); std::fs::write(&b, b"same").unwrap();
        let c = root.join("c.bin"); std::fs::write(&c, b"diff").unwrap();
        let mk = |id: i64, p: &std::path::Path, size: i64| DuplicateBook { id, path: p.to_string_lossy().into_owned(), title: format!("t{id}"), author: String::new(), format: "txt".into(), size, added_at: 0, last_read: 0, read_pct: 0.0 };
        let mut map: HashMap<(i64, String), Vec<(DuplicateBook, i64, String)>> = HashMap::new();
        map.entry((4, "h1".into())).or_default().push((mk(1, &a, 4), 0, a.to_string_lossy().into_owned()));
        map.entry((4, "h1".into())).or_default().push((mk(2, &b, 4), 0, b.to_string_lossy().into_owned()));
        map.entry((4, "h2".into())).or_default().push((mk(3, &c, 4), 0, c.to_string_lossy().into_owned()));
        let groups = build_groups(&map);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].books.len(), 2);
        assert!(!file_meta_matches(&root.join("missing.bin"), 4, 0));
        let _ = std::fs::remove_dir_all(&root);
    }
}
