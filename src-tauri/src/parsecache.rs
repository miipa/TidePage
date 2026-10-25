//! 解析结果磁盘缓存（P10）：把 TXT/MOBI/AZW3 的「解码 + 切分 + 图片改写」结果落盘，
//! 重启后首次打开直接复用，避免整本重解析（实测 AZW3 609MB 冷开约 20s、52MB TXT 约 2s）。
//!
//! 为什么自定义二进制而不 JSON：正文是几十 MB 的字符串，JSON 的转义与中间字符串会放大内存；
//! 这里用定长头 + 长度前缀 + 可选 zlib(fast) 压缩，读时只多一份解压后的 Vec。
//! 压缩策略按类型区分：MOBI/AZW3 同步压缩写（几十 MB 以内，压缩收益明确）；TXT 解码后正文
//! 60~80MB，同步压缩写会拖慢首次打开约 5s 且命中后要解压，实测负收益 —— TXT 改为
//! **后台线程异步 + 不压缩 + 流式写**（见 spawn_write_txt_cache）。
//! 为什么文件名带 rules_ver + size + mtime：规则版本或文件内容一变，旧文件自然不命中；
//! 旧文件由「写后清同书旧文件」与 LRU 预算淘汰回收。
//!
//! 已知限制：只持久化 TXT/MOBI（EPUB 解析已够快）；不清理进程内 L1 缓存（txt_cache 仍容量 3）。
//! 缓存内容一律视为不可信：任何读取/解压/解析错误都删除文件并按未命中重解析，绝不 panic。

use crate::models::TocEntry;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// 缓存类型：TXT。
pub const KIND_TXT: u8 = 1;
/// 缓存类型：MOBI/AZW3。
pub const KIND_MOBI: u8 = 2;

const MAGIC: &[u8; 4] = b"TPPC";
const FORMAT_VERSION: u8 = 1;
const FLAG_ZLIB: u8 = 1;
const CACHE_EXT: &str = ".tpc";
/// 解析缓存总预算（2GB）：单条就可能几百 MB，预算只防无限增长，不做精细配额。
/// 写后扫描目录按 mtime 从旧到新淘汰；刚写入的那条不删（即使单条已超预算，避免写后即删的抖动）。
pub const PARSE_CACHE_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 一条已落盘的解析缓存文件（LRU 淘汰用）。
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// 缓存键：类型 + 书 id + 规则版本 + 源文件指纹。文件名就是这四项的唯一编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheKey {
    pub kind: u8,
    pub book_id: i64,
    pub rules_ver: u64,
    pub size: i64,
    pub mtime: i64,
}

impl CacheKey {
    /// 文件名：{kind}-{book_id}-{rules_ver}-{size}-{mtime}.tpc（book_id 可含前导负号，解析用 rsplitn）。
    pub fn file_name(self) -> String {
        format!(
            "{}-{}-{}-{}-{}{}",
            self.kind, self.book_id, self.rules_ver, self.size, self.mtime, CACHE_EXT
        )
    }
}

/// 解析缓存内容：与 library.rs 的 Arc 三元组同构，额外带编码告警（MOBI）。
pub struct ParsedDoc {
    pub text: String,
    pub ranges: Vec<(usize, usize)>,
    pub toc: Vec<TocEntry>,
    pub warn: Option<String>,
}

/// 解析缓存目录：data_dir/parse_cache。
pub fn cache_dir(app: &tauri::AppHandle) -> PathBuf {
    crate::data_dir(app).join("parse_cache")
}

/// B3：解析缓存占用字节数（只统计合法缓存文件，忽略其它临时文件）。
pub fn cache_bytes(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.ends_with(CACHE_EXT) {
                return None;
            }
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(meta.len())
        })
        .sum()
}

/// 清理进度（供前端维护卡片展示）：done/total 为文件数，bytes 为已释放字节。
pub struct CacheClearProgress {
    pub done: u64,
    pub total: u64,
    pub bytes: u64,
}

/// B3：带进度的解析缓存清理：先枚举合法缓存文件，再逐个删除并回调（删除失败也计入 done）。
pub fn clear_cache_with_progress<F: FnMut(CacheClearProgress)>(
    dir: &Path,
    mut on_progress: F,
) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.ends_with(CACHE_EXT) {
            continue;
        }
        files.push((p, meta.len()));
    }
    let total = files.len() as u64;
    let total_bytes: u64 = files.iter().map(|(_, n)| *n).sum();
    on_progress(CacheClearProgress { done: 0, total, bytes: 0 });
    let mut freed = 0u64;
    for (i, (p, size)) in files.iter().enumerate() {
        if std::fs::remove_file(p).is_ok() {
            freed += size;
        }
        let done = i as u64 + 1;
        if done == 1 || done % 50 == 0 || done == total {
            let _ = total_bytes;
            on_progress(CacheClearProgress { done, total, bytes: freed });
        }
    }
    if total == 0 {
        on_progress(CacheClearProgress { done: 0, total: 0, bytes: 0 });
    }
    freed
}

/// 校验并拆解缓存文件名；格式不对、类型非法、数值为负一律返回 None（调用方忽略该文件）。
pub fn parse_file_name(name: &str) -> Option<CacheKey> {
    let stem = name.strip_suffix(CACHE_EXT)?;
    let mut it = stem.rsplitn(4, '-');
    let mtime: i64 = it.next()?.parse().ok()?;
    let size: i64 = it.next()?.parse().ok()?;
    let rules_ver: u64 = it.next()?.parse().ok()?;
    let head = it.next()?; // "{kind}-{book_id}"
    let (kind_s, book_s) = head.split_once('-')?;
    let kind: u8 = kind_s.parse().ok()?;
    let book_id: i64 = book_s.parse().ok()?;
    if kind != KIND_TXT && kind != KIND_MOBI {
        return None;
    }
    if size < 0 || mtime < 0 {
        return None;
    }
    Some(CacheKey { kind, book_id, rules_ver, size, mtime })
}

fn encode_payload(
    text: &str,
    ranges: &[(usize, usize)],
    toc: &[TocEntry],
    warn: Option<&str>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 128 + toc.len() * 64);
    out.extend_from_slice(&(text.len() as u64).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(&(ranges.len() as u64).to_le_bytes());
    for (s, e) in ranges {
        out.extend_from_slice(&(*s as u64).to_le_bytes());
        out.extend_from_slice(&(*e as u64).to_le_bytes());
    }
    out.extend_from_slice(&(toc.len() as u64).to_le_bytes());
    for t in toc {
        out.extend_from_slice(&(t.index as i64).to_le_bytes());
        out.extend_from_slice(&t.chars.to_le_bytes());
        out.extend_from_slice(&(t.title.len() as u32).to_le_bytes());
        out.extend_from_slice(t.title.as_bytes());
    }
    match warn {
        Some(w) => {
            out.push(1);
            out.extend_from_slice(&(w.len() as u32).to_le_bytes());
            out.extend_from_slice(w.as_bytes());
        }
        None => out.push(0),
    }
    out
}

/// 带边界检查的只读游标：任何越界都返回 None（损坏缓存按未命中处理，绝不 panic）。
struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        let s = &self.b[self.pos..end];
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).and_then(|s| s.try_into().ok()).map(u64::from_le_bytes)
    }
    fn i64(&mut self) -> Option<i64> {
        self.u64().map(|v| v as i64)
    }
}

fn decode_payload(raw: &[u8]) -> Option<ParsedDoc> {
    let mut r = Reader { b: raw, pos: 0 };
    let text_len = usize::try_from(r.u64()?).ok()?;
    let text = String::from_utf8(r.take(text_len)?.to_vec()).ok()?;
    let n_ranges = usize::try_from(r.u64()?).ok()?;
    // 损坏数据里的超大计数不能直接 Vec::with_capacity（可能 TB 级分配）：按剩余字节数夹取。
    if n_ranges > raw.len() / 16 + 1 {
        return None;
    }
    let mut ranges = Vec::with_capacity(n_ranges);
    for _ in 0..n_ranges {
        let s = usize::try_from(r.u64()?).ok()?;
        let e = usize::try_from(r.u64()?).ok()?;
        if s > text.len() || e < s || e > text.len() {
            return None;
        }
        ranges.push((s, e));
    }
    let n_toc = usize::try_from(r.u64()?).ok()?;
    if n_toc > raw.len() / 20 + 1 {
        return None;
    }
    let mut toc = Vec::with_capacity(n_toc);
    for _ in 0..n_toc {
        let index = usize::try_from(r.i64()?).ok()?;
        let chars = r.i64()?;
        let title_len = r.u32()?;
        let title = String::from_utf8(r.bytes(title_len as usize)?.to_vec()).ok()?;
        toc.push(TocEntry { title, index, chars });
    }
    let warn = match r.u8()? {
        0 => None,
        1 => {
            let n = r.u32()?;
            Some(String::from_utf8(r.bytes(n as usize)?.to_vec()).ok()?)
        }
        _ => return None,
    };
    // 严格：payload 必须刚好读完，多余字节视为损坏。
    if r.pos != raw.len() {
        return None;
    }
    Some(ParsedDoc { text, ranges, toc, warn })
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        self.take(n)
    }
}

fn decode_cache_bytes(bytes: &[u8], key: CacheKey) -> Option<ParsedDoc> {
    const HEADER: usize = 40; // magic4 + ver1 + kind1 + flags1 + reserved1 + ver8 + size8 + mtime8 + len8
    if bytes.len() < HEADER || &bytes[0..4] != MAGIC {
        return None;
    }
    let format_ver = bytes[4];
    let kind = bytes[5];
    let flags = bytes[6];
    if format_ver != FORMAT_VERSION || kind != key.kind || flags & !FLAG_ZLIB != 0 {
        return None;
    }
    let rules_ver = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let size = i64::from_le_bytes(bytes[16..24].try_into().ok()?);
    let mtime = i64::from_le_bytes(bytes[24..32].try_into().ok()?);
    let payload_len = u64::from_le_bytes(bytes[32..40].try_into().ok()?);
    if (rules_ver, size, mtime) != (key.rules_ver, key.size, key.mtime) {
        return None;
    }
    let payload = &bytes[HEADER..];
    if payload.len() as u64 != payload_len {
        return None;
    }
    let raw: Vec<u8> = if flags & FLAG_ZLIB != 0 {
        let mut out = Vec::new();
        ZlibDecoder::new(payload).read_to_end(&mut out).ok()?;
        out
    } else {
        payload.to_vec()
    };
    decode_payload(&raw)
}

/// 写解析缓存（先写 .tmp 再 rename 原子替换）；成功后清理同书旧文件并做预算淘汰。
/// 失败由调用方忽略（缓存是加速手段，不能影响打开书籍）。
/// 同步 + 压缩写（保留给单测/兼容路径；生产 TXT/MOBI 打开已走 spawn_write_cache 异步不压缩）。
#[allow(dead_code)]
pub fn write_cache(
    dir: &Path,
    key: CacheKey,
    text: &str,
    ranges: &[(usize, usize)],
    toc: &[TocEntry],
    warn: Option<&str>,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let raw = encode_payload(text, ranges, toc, warn);
    let compressed = {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&raw).and_then(|_| enc.finish())
    };
    // 压缩后不比原始小（短文本/高熵）就原样存，清 flags 位。
    let (flags, payload): (u8, Vec<u8>) = match compressed {
        Ok(z) if z.len() < raw.len() => (FLAG_ZLIB, z),
        _ => (0, raw),
    };
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.push(key.kind);
    out.push(flags);
    out.push(0); // 保留字节，当前恒 0
    out.extend_from_slice(&key.rules_ver.to_le_bytes());
    out.extend_from_slice(&key.size.to_le_bytes());
    out.extend_from_slice(&key.mtime.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&payload);

    let fname = key.file_name();
    let path = dir.join(&fname);
    let tmp = dir.join(format!("{fname}.tmp"));
    std::fs::write(&tmp, &out)?;
    // Windows 上 rename 不覆盖已存在目标：先删目标再重试（同 key 重复写只发生在重解析后）。
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&tmp, &path)?;
    }
    prune_stale(dir, key);
    enforce_budget(dir, &path);
    Ok(path)
}

/// 文件头长度：magic4 + ver1 + kind1 + flags1 + reserved1 + rules_ver8 + size8 + mtime8 + payload_len8。
const HEADER_LEN: usize = 40;

/// 未压缩 payload 的精确字节数：流式写时先算长度写进 40 字节头，正文不必先拼成一个完整 Vec。
fn payload_len_uncompressed(
    text: &str,
    ranges: &[(usize, usize)],
    toc: &[TocEntry],
    warn: Option<&str>,
) -> u64 {
    let mut n: u64 = 8 + text.len() as u64 + 8 + (ranges.len() as u64) * 16 + 8;
    for t in toc {
        n += 8 + 8 + 4 + t.title.len() as u64;
    }
    n += 1;
    if let Some(w) = warn {
        n += 4 + w.len() as u64;
    }
    n
}

/// 唯一的临时文件名（纳秒 + 线程号）：并发两次打开同一本书各自写 tmp，绝不互相写坏。
fn unique_tmp_path(dir: &Path, fname: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tid = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    dir.join(format!("{fname}.{nanos}.{tid}.tmp"))
}

/// 写「不压缩（flags=0）+ 流式」解析缓存：先算 payload 长度写头，再逐段写正文/ranges/toc/warn，
/// 不构造完整 payload 大 Vec（TXT 正文 60~80MB，额外一份复制没有意义）。
/// 同名目标已存在则直接跳过（同一 key 的解析结果是确定性的，重复写纯浪费）。
/// 成功后清理同书旧文件并做预算淘汰，语义与 write_cache 一致。
pub fn write_cache_uncompressed(
    dir: &Path,
    key: CacheKey,
    text: &str,
    ranges: &[(usize, usize)],
    toc: &[TocEntry],
    warn: Option<&str>,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let fname = key.file_name();
    let path = dir.join(&fname);
    if path.exists() {
        return Ok(path);
    }
    let tmp = unique_tmp_path(dir, &fname);
    let payload_len = payload_len_uncompressed(text, ranges, toc, warn);
    {
        let f = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::new(f);
        // 40 字节头：flags=0 表示不压缩。
        w.write_all(MAGIC)?;
        w.write_all(&[FORMAT_VERSION, key.kind, 0u8, 0u8])?;
        w.write_all(&key.rules_ver.to_le_bytes())?;
        w.write_all(&key.size.to_le_bytes())?;
        w.write_all(&key.mtime.to_le_bytes())?;
        w.write_all(&payload_len.to_le_bytes())?;
        // payload：逐段写，正文直接写字节切片。
        w.write_all(&(text.len() as u64).to_le_bytes())?;
        w.write_all(text.as_bytes())?;
        w.write_all(&(ranges.len() as u64).to_le_bytes())?;
        let mut buf = [0u8; 16];
        for (s, e) in ranges {
            buf[0..8].copy_from_slice(&(*s as u64).to_le_bytes());
            buf[8..16].copy_from_slice(&(*e as u64).to_le_bytes());
            w.write_all(&buf)?;
        }
        w.write_all(&(toc.len() as u64).to_le_bytes())?;
        for t in toc {
            w.write_all(&(t.index as i64).to_le_bytes())?;
            w.write_all(&t.chars.to_le_bytes())?;
            w.write_all(&(t.title.len() as u32).to_le_bytes())?;
            w.write_all(t.title.as_bytes())?;
        }
        match warn {
            Some(wr) => {
                w.write_all(&[1u8])?;
                w.write_all(&(wr.len() as u32).to_le_bytes())?;
                w.write_all(wr.as_bytes())?;
            }
            None => w.write_all(&[0u8])?,
        }
        w.flush()?;
    }
    // 目标可能在「存在性检查」之后被别的线程/进程写入：视为已完成，删 tmp 返回。
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        if path.exists() {
            return Ok(path);
        }
        return Err(e);
    }
    prune_stale(dir, key);
    enforce_budget(dir, &path);
    Ok(path)
}

/// R8-2：通用后台异步写解析缓存（不压缩 + 流式），TXT 与 MOBI 共用。
/// 线程只持有目录/键/结果 Arc，不借用 AppState，也不在持锁时写盘；失败只在诊断开关下记一行。
pub fn spawn_write_cache(
    dir: PathBuf,
    key: CacheKey,
    doc: Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>,
    warn: Option<String>,
) {
    std::thread::Builder::new()
        .name("tp-parse-cache-write".into())
        .spawn(move || {
            if let Err(e) =
                write_cache_uncompressed(&dir, key, &doc.0, &doc.1, &doc.2, warn.as_deref())
            {
                if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
                    eprintln!("[parse-cache] async write failed book={}: {e}", key.book_id);
                }
            }
        })
        .ok();
}

/// 后台异步写 TXT 解析缓存（与 spawn_write_cache 行为一致，保留原入口）。
pub fn spawn_write_txt_cache(
    dir: PathBuf,
    key: CacheKey,
    doc: Arc<(String, Vec<(usize, usize)>, Vec<TocEntry>)>,
) {
    spawn_write_cache(dir, key, doc, None);
}

/// 读解析缓存：文件名/头部/解压/解析任何一步不符都删除该文件并返回 None。
pub fn read_cache(dir: &Path, key: CacheKey) -> Option<ParsedDoc> {
    let path = dir.join(key.file_name());
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            // 存在但读不了（权限/损坏）也删掉，避免下次继续白读。
            if e.kind() != std::io::ErrorKind::NotFound {
                let _ = std::fs::remove_file(&path);
            }
            return None;
        }
    };
    match decode_cache_bytes(&bytes, key) {
        Some(doc) => {
            touch(&path);
            Some(doc)
        }
        None => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// 命中时更新文件 mtime，作为 LRU 的「最近访问」时间（尽力而为）。
fn touch(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// 扫描目录里的解析缓存条目（忽略无法解析的文件名/读不到属性的文件）。
pub fn scan_entries(dir: &Path) -> Vec<CacheEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if parse_file_name(&name).is_none() {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        out.push(CacheEntry {
            path: e.path(),
            size: md.len(),
            modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    out
}

/// LRU 淘汰计划（纯函数）：总量超预算时按 mtime 从旧到新删除，直到不超预算。
/// keep（本次刚写入的文件）永不删除；若它单条就超预算，则保留它、不删别的（明确策略：
/// 避免「写后立刻被删」的抖动，下次写入时自然按 LRU 回收）。
pub fn plan_evictions(entries: &[CacheEntry], budget: u64, keep: Option<&Path>) -> Vec<PathBuf> {
    let total: u64 = entries.iter().map(|e| e.size).sum();
    if total <= budget {
        return Vec::new();
    }
    let mut candidates: Vec<&CacheEntry> = entries
        .iter()
        .filter(|e| Some(e.path.as_path()) != keep)
        .collect();
    candidates.sort_by(|a, b| a.modified.cmp(&b.modified));
    let mut remaining = total;
    let mut out = Vec::new();
    for e in candidates {
        if remaining <= budget {
            break;
        }
        remaining = remaining.saturating_sub(e.size);
        out.push(e.path.clone());
    }
    out
}

/// 删除同一本书的其它缓存文件（kind+book_id 相同、文件名不同＝旧规则版本或旧指纹）。
fn prune_stale(dir: &Path, key: CacheKey) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let current = key.file_name();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(k) = parse_file_name(&name) {
            if k.kind == key.kind && k.book_id == key.book_id && name != current {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn enforce_budget(dir: &Path, keep: &Path) {
    let entries = scan_entries(dir);
    for p in plan_evictions(&entries, PARSE_CACHE_BUDGET_BYTES, Some(keep)) {
        let _ = std::fs::remove_file(p);
    }
}

/// 删除某本书的全部解析缓存文件（删书/清理资源时调用）。
pub fn cleanup_book(dir: &Path, book_id: i64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(k) = parse_file_name(&name) {
            if k.book_id == book_id {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp_parsecache_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_toc() -> Vec<TocEntry> {
        vec![
            TocEntry { title: "第一章 测试".into(), index: 0, chars: 12 },
            TocEntry { title: "Chapter 2".into(), index: 1, chars: 7 },
        ]
    }

    fn key(kind: u8, book_id: i64, rules_ver: u64, size: i64, mtime: i64) -> CacheKey {
        CacheKey { kind, book_id, rules_ver, size, mtime }
    }

    #[test]
    fn roundtrip_text_and_warn() {
        let dir = tmp_dir("roundtrip");
        let text = "第一章 你好世界\n第二段".to_string();
        let ranges = vec![(0, 9), (9, text.len())];
        let toc = sample_toc();
        let k = key(KIND_MOBI, 42, 3, 1234, 5678);
        let p = write_cache(&dir, k, &text, &ranges, &toc, Some("编码告警")).unwrap();
        assert!(p.exists());
        let d = read_cache(&dir, k).expect("应命中");
        assert_eq!(d.text, text);
        assert_eq!(d.ranges, ranges);
        assert_eq!(d.toc.len(), 2);
        assert_eq!(d.toc[0].title, "第一章 测试");
        assert_eq!(d.toc[1].index, 1);
        assert_eq!(d.toc[1].chars, 7);
        assert_eq!(d.warn.as_deref(), Some("编码告警"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_txt_compresses_or_stores_raw() {
        let dir = tmp_dir("compress");
        // 高度重复的正文走 zlib；短文本压缩后更大则原样存。两者都必须能读回。
        let big = "潮汐页阅读器测试正文。".repeat(2000);
        let k1 = key(KIND_TXT, 7, 1, 10, 20);
        write_cache(&dir, k1, &big, &[(0, big.len())], &sample_toc(), None).unwrap();
        assert_eq!(read_cache(&dir, k1).unwrap().text, big);
        let short = "hi";
        let k2 = key(KIND_TXT, 8, 1, 10, 20);
        write_cache(&dir, k2, short, &[(0, 2)], &[], None).unwrap();
        assert_eq!(read_cache(&dir, k2).unwrap().text, short);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_cache_is_rejected_and_removed() {
        let dir = tmp_dir("corrupt");
        let k = key(KIND_TXT, 9, 1, 10, 20);
        let p = write_cache(&dir, k, "正文", &[(0, 6)], &[], None).unwrap();
        // 坏 magic
        let mut b = std::fs::read(&p).unwrap();
        b[0] = b'X';
        std::fs::write(&p, &b).unwrap();
        assert!(read_cache(&dir, k).is_none(), "坏 magic 必须拒绝");
        assert!(!p.exists(), "损坏文件应被删除");
        // 截断
        write_cache(&dir, k, "正文", &[(0, 6)], &[], None).unwrap();
        let mut b = std::fs::read(&p).unwrap();
        b.truncate(10);
        std::fs::write(&p, &b).unwrap();
        assert!(read_cache(&dir, k).is_none(), "截断必须拒绝");
        // 坏 zlib：手工构造 flags=zlib + 垃圾 payload
        let mut bad = Vec::new();
        bad.extend_from_slice(MAGIC);
        bad.push(FORMAT_VERSION);
        bad.push(k.kind);
        bad.push(FLAG_ZLIB);
        bad.push(0);
        bad.extend_from_slice(&k.rules_ver.to_le_bytes());
        bad.extend_from_slice(&k.size.to_le_bytes());
        bad.extend_from_slice(&k.mtime.to_le_bytes());
        bad.extend_from_slice(&3u64.to_le_bytes());
        bad.extend_from_slice(b"abc");
        std::fs::write(&p, &bad).unwrap();
        assert!(read_cache(&dir, k).is_none(), "坏 zlib 必须拒绝");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_name_roundtrip_and_validation() {
        let k = key(KIND_MOBI, 123456, 7, 999, 888);
        assert_eq!(parse_file_name(&k.file_name()), Some(k));
        // book_id 为负（防御性）也能解析
        let k2 = CacheKey { book_id: -5, ..k };
        assert_eq!(parse_file_name(&k2.file_name()), Some(k2));
        for bad in [
            "1-2-3-4-5",
            "1-2-3-4-5.json",
            "3-2-3-4-5.tpc",
            "x-2-3-4-5.tpc",
            "1-a-3-4-5.tpc",
            "1-2-3-4--5.tpc",
            "1-2-3--4-5.tpc",
            "1-2-3-4-5-6.tpc",
        ] {
            assert!(parse_file_name(bad).is_none(), "非法文件名不应通过: {bad}");
        }
    }

    #[test]
    fn plan_evictions_under_budget_keeps_all() {
        let a = CacheEntry { path: PathBuf::from("a"), size: 10, modified: SystemTime::UNIX_EPOCH };
        assert!(plan_evictions(&[a], 100, None).is_empty());
        assert!(plan_evictions(&[], 0, None).is_empty());
    }

    #[test]
    fn plan_evictions_removes_oldest_until_budget() {
        let old = CacheEntry { path: PathBuf::from("old"), size: 60, modified: SystemTime::UNIX_EPOCH };
        let mid = CacheEntry {
            path: PathBuf::from("mid"),
            size: 60,
            modified: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10),
        };
        let new = CacheEntry {
            path: PathBuf::from("new"),
            size: 60,
            modified: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20),
        };
        // 总量 180、预算 100：删掉最旧的 60（余 120 仍超）→ 再删次旧的（余 60）即可。
        let ev = plan_evictions(&[new.clone(), old.clone(), mid.clone()], 100, None);
        assert_eq!(ev, vec![old.path.clone(), mid.path.clone()], "超预算时应从最旧的开始删到不超预算");
        // keep 永不删除
        let ev2 = plan_evictions(&[new.clone(), old.clone()], 10, Some(&new.path));
        assert!(!ev2.contains(&new.path), "keep 不得被淘汰");
    }

    #[test]
    fn plan_evictions_single_oversized_keep_is_retained() {
        // 明确策略：单条就超预算时，若它正是刚写入的 keep → 保留（避免写后即删的抖动）。
        let big = CacheEntry { path: PathBuf::from("big"), size: 500, modified: SystemTime::UNIX_EPOCH };
        assert!(plan_evictions(&[big.clone()], 100, Some(&big.path)).is_empty());
        // 非 keep 的单条超预算文件会被删除。
        assert_eq!(plan_evictions(&[big.clone()], 100, None), vec![big.path]);
    }

    #[test]
    fn prune_stale_and_cleanup_book_remove_old_entries() {
        let dir = tmp_dir("prune");
        let same_book_old = key(KIND_TXT, 33, 1, 10, 20);
        let same_book_new = key(KIND_TXT, 33, 1, 11, 21);
        let other_book = key(KIND_MOBI, 44, 1, 10, 20);
        write_cache(&dir, same_book_old, "旧", &[(0, 3)], &[], None).unwrap();
        write_cache(&dir, other_book, "别书", &[(0, 6)], &[], None).unwrap();
        // 写新条目应清掉同书旧指纹文件，保留别的书。
        write_cache(&dir, same_book_new, "新", &[(0, 3)], &[], None).unwrap();
        assert!(read_cache(&dir, same_book_old).is_none());
        assert!(read_cache(&dir, same_book_new).is_some());
        assert!(read_cache(&dir, other_book).is_some());
        cleanup_book(&dir, 33);
        assert!(read_cache(&dir, same_book_new).is_none(), "删书应清掉该书两种类型缓存");
        assert!(read_cache(&dir, other_book).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uncompressed_streaming_roundtrip_uses_flags_zero() {
        let dir = tmp_dir("raw");
        let text = "第一章 未压缩正文\n第二段".to_string();
        let ranges = vec![(0, 9), (9, text.len())];
        let toc = sample_toc();
        let k = key(KIND_TXT, 11, 2, 333, 444);
        let p = write_cache_uncompressed(&dir, k, &text, &ranges, &toc, None).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes[6] & 1, 0, "未压缩写必须置 flags=0");
        assert_eq!(
            bytes.len() as u64,
            HEADER_LEN as u64 + payload_len_uncompressed(&text, &ranges, &toc, None),
            "流式写的文件长度必须与头里声明的 payload 长度一致"
        );
        let d = read_cache(&dir, k).expect("未压缩缓存应可读");
        assert_eq!(d.text, text);
        assert_eq!(d.ranges, ranges);
        assert_eq!(d.toc.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uncompressed_write_skips_when_target_exists() {
        let dir = tmp_dir("skip");
        let k = key(KIND_TXT, 12, 1, 10, 20);
        let p = write_cache_uncompressed(&dir, k, "正文", &[(0, 6)], &[], None).unwrap();
        // 人为破坏目标内容：再次调用必须直接跳过（不重写）——同一 key 的解析结果是确定性的。
        std::fs::write(&p, b"tampered").unwrap();
        let p2 = write_cache_uncompressed(&dir, k, "正文", &[(0, 6)], &[], None).unwrap();
        assert_eq!(p, p2);
        assert_eq!(std::fs::read(&p).unwrap(), b"tampered", "同名目标已存在时应跳过写入");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_txt_write_reaches_read_cache() {
        let dir = tmp_dir("async");
        let text = "第一章 异步写\n第二段".to_string();
        let ranges = vec![(0, 9), (9, text.len())];
        let toc = sample_toc();
        let k = key(KIND_TXT, 13, 1, 77, 88);
        let doc = Arc::new((text.clone(), ranges.clone(), toc.clone()));
        spawn_write_txt_cache(dir.clone(), k, doc);
        // TXT 走异步写：轮询等待线程完成（最长 5s），完成后必须能命中且内容一致。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut hit = read_cache(&dir, k);
        while hit.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            hit = read_cache(&dir, k);
        }
        let d = hit.expect("异步写完成后 read_cache 应能命中");
        assert_eq!(d.text, text);
        assert_eq!(d.ranges, ranges);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn async_mobi_write_is_uncompressed_and_readable() {
        // R8-2：MOBI 解析缓存改为后台异步 + 不压缩写（与 TXT 对齐）。
        let dir = tmp_dir("mobi-async");
        let text = "潮汐页异步缓存测试正文。".repeat(500);
        let ranges = vec![(0, text.len())];
        let k = key(KIND_MOBI, 14, 1, 123, 456);
        let doc = Arc::new((text.clone(), ranges.clone(), Vec::new()));
        spawn_write_cache(dir.clone(), k, doc, Some("告警".to_string()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut hit = read_cache(&dir, k);
        while hit.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            hit = read_cache(&dir, k);
        }
        let d = hit.expect("异步写完成后 read_cache 应能命中");
        assert_eq!(d.text, text);
        assert_eq!(d.ranges, ranges);
        assert_eq!(d.warn.as_deref(), Some("告警"));
        let bytes = std::fs::read(dir.join(k.file_name())).unwrap();
        assert_eq!(bytes[6] & 1, 0, "异步写不压缩（flags bit0=0）");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
