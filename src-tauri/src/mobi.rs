//! MOBI/AZW/AZW3 解析：严格按 PalmDOC(PDB) 数据库规范处理。
//!
//! 文件布局（本实现使用的真实字节偏移）：
//!   0x00..0x1F  数据库名（32B，"\xeaDRMION\xee" 前缀 = KFX DRM 壳）
//!   0x20..      attributes/version/创建-修改-备份时间/modnum/appInfo/sortInfo
//!   0x3C..0x3F  type    —— 必须为 "BOOK"
//!   0x40..0x43  creator —— 必须为 "MOBI"（0x3C..0x43 联合即 "BOOKMOBI" 魔数）
//!   0x4C        num_records (u16 BE)
//!   0x4E        record info list 起始，每条 8 字节：offset(u32 BE) + attrs(u8) + uid(u16) + gap(u8)
//!   record[0]   PalmDOC 头（16B）：
//!                 +0x00 compression (u16: 1=无, 2=PalmDoc LZ77, 0x4438=HUFF/CDIC)
//!                 +0x04 text_length (u32)
//!                 +0x08 record_count (u16)
//!                 +0x0A record_size (u16)
//!                 +0x0C **encryption** (u16: 0=明文, 1=旧 MOBIDRM, 2=PalmDOC DRM, >2=未知/新式)
//!   record[0]+0x10  MOBI 头：+0x00 "MOBI"、+0x40 first-non-text 记录索引、
//!                 +0x5C first-image 记录索引、+0x98 DRM 记录偏移（0xFFFFFFFF=无）等
//!   record[0]+0x10+MOBI头长度  可选 EXTH 头（exth_flags & 0x40）
//!   record[first_content..first_non_text)  LZ77/HUFF 压缩的 HTML 分块
//!   record[first_image..EOF)               图片/媒体记录
//!
//! 注：需求单里写的 0x1C/0x54/0x68 与规范有出入——Encryption Type 实际位于
//! record0(PalmDOC 头) 偏移 0x0C；first-non-text 实际在 MOBI 头 +0x40；
//! first-image 实际在 MOBI 头 +0x5C。本文件按规范真实偏移处理。

use crate::models::TocEntry;
use mobi::headers::{Compression, Encryption, TextEncoding};
use mobi::Mobi;
use regex::{Captures, Regex};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default)]
pub struct MobiDoc {
    pub title: String,
    pub author: String,
    pub html: String,
    pub toc: Vec<TocEntry>,
    pub parts: Vec<(usize, usize)>,
    /// 已写盘的图片数量
    pub image_count: usize,
}

fn u16b(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|x| u16::from_be_bytes([x[0], x[1]])).unwrap_or(0)
}
fn u32b(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
        .unwrap_or(0)
}

/// 文件级快速 DRM 嗅探：只 seek 读 PDB 头 + record0 头共 <100 字节，
/// 不整读文件——加密大书也在 1ms 内拒绝，杜绝"一直卡加载"。
pub fn sniff_drm_file(path: &str) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| format!("打开失败: {e}"))?;
    let fsize = f.metadata().map_err(|e| e.to_string())?.len() as usize;
    if fsize < 0x56 {
        return Err("MOBIError: 文件过短，不是有效的 PalmDOC 数据库".to_string());
    }
    let mut head = [0u8; 0x56];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    if head[0..32].starts_with(b"\xeaDRMION\xee") {
        return Err("AWZ3EncryptedError: 检测到 KFX/DRM 外壳，该书已加密，请先用 DeDRM 等工具解密后再导入".to_string());
    }
    if &head[0x40..0x44] != b"MOBI" {
        return Err("PalmDocFormatError: 文件头 0x40 处 creator 非 \"MOBI\"（缺 BOOKMOBI 魔数）".to_string());
    }
    let rec0 = u32b(&head, 0x4E) as usize;
    if rec0 == 0 || rec0 + 0x10 > fsize {
        return Err("PalmDocFormatError: rec0 偏移越界".to_string());
    }
    f.seek(SeekFrom::Start(rec0 as u64)).map_err(|e| e.to_string())?;
    let mut pd = [0u8; 16];
    f.read_exact(&mut pd).map_err(|e| e.to_string())?;
    let compression = u16b(&pd, 0x00);
    if !matches!(compression, 1 | 2 | 17480) {
        return Err(format!("PalmDocFormatError: 未知压缩算法 {compression}"));
    }
    let encryption = u16b(&pd, 0x0C);
    if encryption != 0 {
        return Err(format!(
            "AWZ3EncryptedError: 检测到 DRM 加密（record0 偏移 0x0C 处 Encryption Type = {encryption}），\
             无法读取正文，请先解密（如 DeDRM）后重新导入"
        ));
    }
    Ok(())
}

/// 第一步：DRM 嗅探拦截。纯字节操作（读头部 <100 字节即返回），
/// 保证加密书在毫秒级被拒绝，绝不进入后续乱码解析路径。
pub fn detect_drm(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 0x56 {
        return Err("MOBIError: 文件过短，不是有效的 PalmDOC 数据库".to_string());
    }
    if bytes[0..32].starts_with(b"\xeaDRMION\xee") {
        return Err(
            "AWZ3EncryptedError: 检测到 KFX/DRM 外壳（记录名以 \\xeaDRMION\\xee 开头），\
             该书已加密，请先用 DeDRM 等工具解密后再导入"
                .to_string(),
        );
    }
    // 偏移 60 验证 "BOOKMOBI" 魔数（type@0x3C + creator@0x40）
    let magic_type = &bytes[0x3C..0x40];
    let magic_creator = &bytes[0x40..0x44];
    if magic_creator != b"MOBI" && magic_type != b"BOOK" {
        return Err(format!(
            "PalmDocFormatError: 文件头 0x3C 处未找到 \"BOOKMOBI\" 魔数（type={:?} creator={:?}）",
            String::from_utf8_lossy(magic_type),
            String::from_utf8_lossy(magic_creator)
        ));
    }
    let num_records = u16b(bytes, 0x4C) as usize;
    if num_records == 0 {
        return Err("PalmDocFormatError: num_records(0x4C) 为 0".to_string());
    }
    let rec0 = u32b(bytes, 0x4E) as usize; // record info list @0x4E，第一条即 rec0 偏移
    if rec0 + 0x10 > bytes.len() {
        return Err("PalmDocFormatError: rec0 偏移越界".to_string());
    }
    let compression = u16b(bytes, rec0);
    if !matches!(compression, 1 | 2 | 17480) {
        return Err(format!("PalmDocFormatError: 未知压缩算法 {compression}"));
    }
    // PalmDOC 头偏移 0x0C：Encryption Type（本库三个真实无 DRM 书实测此处=0，
    // 而 rec0+78+0x1C 处读到 6/8/8——若用后者会把正常书误判为加密）。
    // MOBI 签名实测位于 rec0+0x10（非 rec0+78）。越界防护：需能读到 rec0+0x0D。
    if rec0 + 0x0E > bytes.len() {
        return Err("PalmDocFormatError: record0 头越界".to_string());
    }
    let encryption = u16b(bytes, rec0 + 0x0C);
    if encryption > 1 {
        return Err(format!(
            "AWZ3EncryptedError: 检测到 DRM 加密（PalmDOC 头偏移 0x0C 处 Encryption Type = {encryption}，\
             MOBI 头位于 rec0+0x10）。加密的 MOBI/AZW/AZW3 无法读取正文，请先解密（如 DeDRM）后重新导入"
        ));
    }
    if encryption == 1 {
        return Err(
            "AWZ3EncryptedError: 检测到旧式 Mobipocket DRM（Encryption Type = 1），无法读取正文，请先解密"
                .to_string(),
        );
    }
    Ok(())
}

/// PalmDOC LZ77（compression=2）单记录解压。
/// 与规范一致：0x00/0x09..0x7F 字面量；0x01..0x08 复制其后 N 字节字面量；
/// 0x80..0xBF 与下一字节合成 14bit (距离<<3|长度-3) 回引（4KB 窗口）；
/// 0xC0..0xFF = 空格 + (字节^0x80)。
pub fn decompress_palmdoc_lz77(data: &[u8]) -> Vec<u8> {
    let mut text: Vec<u8> = Vec::with_capacity(data.len() * 2);
    let mut prev: Option<u8> = None;
    let mut pos = 0usize;
    let length = data.len();
    while pos < length {
        let byte = data[pos];
        pos += 1;
        if let Some(old) = prev.take() {
            let mut pair = u16::from_be_bytes([old, byte]) & 0x3FFF; // 高 2bit 为标志位
            let offset = (pair >> 3) as usize;
            let len = ((pair & 0x07) + 3) as usize;
            let text_pos = text.len();
            let start = if offset > text_pos { text_pos } else { text_pos - offset };
            let end = (start + len).min(text.len().max(text_pos + len));
            for i in 0..len {
                let src_i = start + i;
                if src_i >= text.len() {
                    break;
                }
                let ch = text[src_i];
                text.push(ch);
            }
            let _ = end;
        } else if byte == 0x00 || (0x09..=0x7F).contains(&byte) {
            text.push(byte);
        } else if (0x01..=0x08).contains(&byte) {
            let n = byte as usize;
            if pos + n <= length {
                text.extend_from_slice(&data[pos..pos + n]);
                pos += n;
            }
        } else if (0x80..=0xBF).contains(&byte) {
            prev = Some(byte);
        } else {
            text.push(b' ');
            text.push(byte ^ 0x80);
        }
    }
    text
}

/// 拼接并解压可读记录区间 [first_content_record .. first_non_text)，
/// 返回**解码前**的原始字节。HUFF/CDIC(compression=0x4438) 使用
/// mobi crate 内置的 HUFF 解码器（其输出已是文本），其余两种在本函数内
/// 按 PDB 记录手工解压，以便后续做编码纠偏。
pub fn extract_records_and_decompress(mobi: &Mobi) -> Result<Vec<u8>, String> {
    extract_records_and_decompress_limited(mobi, MAX_RECORDS, MAX_HTML_BYTES)
}

/// 防卡死熔断上限：畸形/超大文件在解析中途即中止，不拖死 UI。
const MAX_RECORDS: usize = 50_000;
const MAX_HTML_BYTES: usize = 50 * 1024 * 1024;

pub fn extract_records_and_decompress_limited(
    mobi: &Mobi,
    max_records: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let comp = mobi.compression();
    let recs = mobi.raw_records();
    let range = mobi.readable_records_range();
    // 注意：mobi crate 的 RawRecords::range() 对 Excluded end 做了 -1 处理
    // （会丢弃最后一条记录），因此这里直接用完整 records() 切片。
    let all = recs.records();
    let start = range.start.min(all.len());
    let end = range.end.min(all.len()).max(start);
    if end - start > max_records {
        return Err(format!(
            "ParseTimeoutOrCorrupted: 可读记录数 {} 超过上限 {max_records}，文件畸形",
            end - start
        ));
    }
    match comp {
        Compression::Huff => {
            let s = mobi.content_as_string_lossy();
            if s.len() > max_bytes {
                return Err(format!(
                    "ParseTimeoutOrCorrupted: HUFF 解压输出 {} 字节超过上限",
                    s.len()
                ));
            }
            Ok(s.into_bytes())
        }
        Compression::PalmDoc => {
            let mut out = Vec::new();
            for r in &all[start..end] {
                out.extend_from_slice(&decompress_palmdoc_lz77(r.content));
                if out.len() > max_bytes {
                    return Err(format!(
                        "ParseTimeoutOrCorrupted: 解压输出超过 {max_bytes} 字节上限，文件畸形"
                    ));
                }
            }
            Ok(out)
        }
        Compression::No => {
            let mut out = Vec::new();
            for r in &all[start..end] {
                out.extend_from_slice(r.content);
                if out.len() > max_bytes {
                    return Err(format!(
                        "ParseTimeoutOrCorrupted: 正文超过 {max_bytes} 字节上限，文件畸形"
                    ));
                }
            }
            Ok(out)
        }
    }
}

fn sniff_meta_charset(raw: &[u8]) -> Option<String> {
    // 前 4KB 以 Latin-1 无损视图扫描 <meta ... charset=...>（ASCII 标签不受编码影响）
    let head: String = raw.iter().take(4096).map(|b| *b as char).collect();
    let re = Regex::new(r#"(?i)c(?:harset|oding)\s*=\s*["']?\s*([a-z0-9][a-z0-9_\-]{1,20})"#).unwrap();
    re.captures(&head).map(|c| c[1].to_ascii_lowercase())
}

fn decode_label(label: &str, raw: &[u8]) -> Option<String> {
    let enc = encoding_rs::Encoding::for_label(label.as_bytes())?;
    let (cow, _actual, _had_err) = enc.decode(raw);
    Some(cow.into_owned())
}

/// GBK/Big5 严格猜测：要求零坏字节且 CJK 达到占比门槛
fn strict_cjk_guess(raw: &[u8], lang_cjk: bool) -> Option<String> {
    for label in ["gbk", "gb18030", "big5"] {
        let enc = encoding_rs::Encoding::for_label(label.as_bytes())?;
        let (cow, _actual, had_err) = enc.decode(raw);
        if had_err {
            continue;
        }
        let s = cow.into_owned();
        let total = s.chars().count().max(1);
        let cjk = s
            .chars()
            .filter(|c| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'))
            .count();
        let ok = if lang_cjk { cjk * 20 > total } else { cjk * 2 > total };
        if ok {
            return Some(s);
        }
    }
    None
}

/// 编码纠偏：优先用户强制指定，其次内嵌 meta 声明，最后 MOBI 头 text encoding；
/// 全部失败时按 UTF-8 严格、再 CP1252、再 lossy UTF-8 兜底。
/// 分块 UTF-8 解码：返回 (文本, 非法字节段数)。个别损坏字节不应毁掉整本书。
fn utf8_chunked(raw: &[u8]) -> (String, usize) {
    let mut s = String::with_capacity(raw.len());
    let mut rest = raw;
    let mut bad = 0usize;
    loop {
        match std::str::from_utf8(rest) {
            Ok(v) => {
                s.push_str(v);
                return (s, bad);
            }
            Err(e) => {
                let upto = e.valid_up_to();
                s.push_str(&String::from_utf8_lossy(&rest[..upto]));
                let elen = e.error_len().unwrap_or(1);
                s.push('\u{FFFD}');
                bad += 1;
                rest = &rest[(upto + elen).min(rest.len())..];
            }
        }
    }
}

pub fn decode_html_bytes(raw: &[u8], header_enc: TextEncoding, forced: Option<&str>, lang_cjk: bool) -> String {
    let forced = forced
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty() && s != "auto");
    let meta = sniff_meta_charset(raw);
    for cand in forced.iter().map(|s| s.as_str()).chain(meta.iter().map(|s| s.as_str())) {
        if let Some(s) = decode_label(cand, raw) {
            return s;
        }
    }
    // 无 meta/强制：旧中文书普遍存在"文件头谎报编码 / 记录边界截断产生少量非法序列"的情况。
    // UTF-8 探测容忍 ≤1% 的损坏字节段；头报 CP1252 的西文书仅在"中文语言 + 大量 CJK"
    // 时才允许带损 UTF-8，避免把 é/è 等高位字节当损坏。
    let (utf8_s, bad) = utf8_chunked(raw);
    let utf8_trusted = match header_enc {
        TextEncoding::CP1252 if !lang_cjk => bad == 0,
        _ => bad * 100 <= raw.len().max(1),
    };
    if utf8_trusted {
        let total = utf8_s.chars().count().max(1);
        let cjk = utf8_s
            .chars()
            .filter(|c| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'))
            .count();
        let looks_cjk = cjk * 10 > total;
        if bad == 0 || looks_cjk || matches!(header_enc, TextEncoding::UTF8 | TextEncoding::Unknown(_)) {
            return utf8_s;
        }
    }
    drop(utf8_s);
    match header_enc {
        TextEncoding::UTF8 | TextEncoding::Unknown(_) => {
            // 头报 UTF-8 但损坏率过高（常见于实为 GBK 的书）：严格试探 GBK/Big5
            strict_cjk_guess(raw, true).unwrap_or_else(|| {
                decode_label("windows-1252", raw)
                    .unwrap_or_else(|| String::from_utf8_lossy(raw).into_owned())
            })
        }
        TextEncoding::CP1252 => {
            // 头声称 CP1252 但 UTF-8 探测失败：试 GBK/Big5 严格解码（不允许坏字节），
            // 文件头语言为中文时 CJK 占比门槛降为 5%，否则要求过半 CJK
            if let Some(s) = strict_cjk_guess(raw, lang_cjk) {
                return s;
            }
            decode_label("windows-1252", raw).unwrap_or_default()
        }
    }
}

fn image_magic(c: &[u8]) -> Option<&'static str> {
    if c.len() < 16 {
        return None;
    }
    match c {
        _ if c.starts_with(&[0xFF, 0xD8, 0xFF]) => Some("jpg"),
        _ if c.starts_with(&[0x89, b'P', b'N', b'G']) => Some("png"),
        _ if c.starts_with(b"GIF87a") || c.starts_with(b"GIF89a") => Some("gif"),
        _ if c.len() > 12 && &c[0..4] == b"RIFF" && &c[8..12] == b"WEBP" => Some("webp"),
        _ if c.starts_with(b"BM") => Some("bmp"),
        _ => None,
    }
}

const NON_IMAGE_MAGICS: &[&[u8]] = &[
    b"FLIS", b"FCIS", b"SRCS", b"RESC", b"BOUN", b"FDST", b"AUDI", b"VIDE",
];

/// 从 first_image 索引到 EOF 提取图片记录：嗅探魔数、写盘并建立
/// (记录索引→URL)、(EXTH121 资源号→URL)、按序列表 三种映射。
pub fn extract_images(
    mobi: &Mobi,
    out_dir: &Path,
    url_prefix: &str,
) -> (HashMap<u32, String>, HashMap<u32, String>, Vec<String>) {
    let mut by_rec: HashMap<u32, String> = HashMap::new();
    let mut by_res: HashMap<u32, String> = HashMap::new();
    let mut ordered: Vec<String> = Vec::new();
    let fi = mobi.metadata.mobi.first_image_index as usize;
    let all = mobi.raw_records();
    let recs = all.records();
    if fi == 0 || fi >= recs.len() {
        return (by_rec, by_res, ordered);
    }
    let _ = std::fs::create_dir_all(out_dir);
    for (i, rec) in recs.iter().enumerate().skip(fi) {
        let c = rec.content;
        if NON_IMAGE_MAGICS.iter().any(|m| c.starts_with(m)) {
            continue;
        }
        let Some(ext) = image_magic(c) else { continue };
        let fname = format!("r{i:05}.{ext}");
        if std::fs::write(out_dir.join(&fname), c).is_err() {
            continue;
        }
        let url = format!("{url_prefix}{fname}");
        by_rec.insert(i as u32, url.clone());
        ordered.push(url);
    }
    if by_rec.is_empty() {
        return (by_rec, by_res, ordered);
    }
    // EXTH 121: "recidx:residx" 逗号分隔（十六进制），recidx 为绝对记录索引
    if let Some(list) = mobi.metadata.exth_record_at(121) {
        for blob in list {
            let s = String::from_utf8_lossy(blob).to_string();
            for pair in s.split(|ch: char| ch == ',' || ch == ';' || ch == '\0' || ch == '\n' || ch.is_whitespace()) {
                let Some((rec_s, res_s)) = pair.split_once(':') else { continue };
                let Ok(rec_i) = u32::from_str_radix(rec_s.trim(), 16) else { continue };
                let Ok(res_i) = u32::from_str_radix(res_s.trim(), 16) else { continue };
                if let Some(u) = by_rec.get(&rec_i).cloned() {
                    by_res.insert(res_i, u);
                } else if let Some(u) = by_rec.get(&((fi + rec_i as usize) as u32)).cloned() {
                    by_res.insert(res_i, u);
                }
            }
        }
    }
    (by_rec, by_res, ordered)
}

/// 重写 HTML 中的图片引用 → 本地资源 URL：
///   `<img recindex="0002">`（MOBI6）、`src="kindle:embed:0001"`（KF8）、
///   `<img src="1.jpg">`（按图片出现顺序的数字文件名）
/// 1x1 全透明 GIF，用于无法解析的 MOBI 图片引用的安全占位
const TRANSPARENT_PLACEHOLDER: &str =
    "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

pub fn rewrite_images(
    html: &str,
    by_rec: &HashMap<u32, String>,
    by_res: &HashMap<u32, String>,
    ordered: &[String],
) -> String {
    let tag_re = Regex::new(r"(?is)<img\b[^>]*>").unwrap();
    let rec_re = Regex::new(r#"(?i)\brecindex\s*=\s*['"]?([0-9a-fA-F]{1,8})['"]?"#).unwrap();
    let embed_re = Regex::new(r"(?i)kindle:embed:([0-9a-fA-F]{1,8})").unwrap();
    let src_re = Regex::new(r#"(?i)\bsrc\s*=\s*["']([^"']*)["']"#).unwrap();
    let num_re = Regex::new(r"(\d+)").unwrap();
    tag_re
        .replace_all(html, |cap: &Captures| {
            let tag = cap.get(0).unwrap().as_str();
            let mut url: Option<String> = None;
            if let Some(c) = rec_re.captures(tag) {
                if let Ok(r) = u32::from_str_radix(&c[1], 16) {
                    url = by_rec.get(&r).cloned();
                }
            }
            if url.is_none() {
                if let Some(c) = embed_re.captures(tag) {
                    if let Ok(r) = u32::from_str_radix(&c[1], 16) {
                        url = by_res.get(&r).cloned().or_else(|| {
                            if r >= 1 {
                                ordered.get((r - 1) as usize).cloned()
                            } else {
                                None
                            }
                        });
                    }
                }
            }
            if url.is_none() {
                if let Some(c) = src_re.captures(tag) {
                    let v = c[1].trim();
                    if !v.contains("kindle:embed")
                        && !v.starts_with("http")
                        && !v.starts_with("data:")
                        && !v.starts_with('/')
                    {
                        let base = v.rsplit(['/', '\\']).next().unwrap_or("");
                        if let Some(n) = num_re.captures(base).and_then(|d| d[1].parse::<usize>().ok()) {
                            // 数字文件名：0 基（0000.jpg）与 1 基（1.jpg）都尝试
                            url = ordered.get(n).cloned().or_else(|| ordered.get(n.saturating_sub(1)).cloned());
                        }
                    }
                }
            }
            if url.is_none() {
                // 外部 http/data:/绝对路径 src 是合法引用，保持原样
                let external_ok = src_re
                    .captures(tag)
                    .map(|c| {
                        let v = c[1].trim();
                        v.starts_with("http") || v.starts_with("data:") || v.starts_with('/')
                    })
                    .unwrap_or(false);
                if external_ok {
                    return tag.to_string();
                }
                // 含 MOBI 图片引用（recindex / kindle:embed / 本地文件名 src）但解析不到图片：
                // 换成透明占位图，绝不留下浏览器无法加载的非法 src
                let is_img_ref = rec_re.is_match(tag) || embed_re.is_match(tag) || src_re.is_match(tag);
                if !is_img_ref {
                    return tag.to_string();
                }
                let mut out = rec_re.replace_all(tag, "").to_string();
                out = if src_re.is_match(&out) {
                    src_re
                        .replace_all(&out, format!("src=\"{TRANSPARENT_PLACEHOLDER}\""))
                        .to_string()
                } else {
                    out.replacen("<img", &format!("<img src=\"{TRANSPARENT_PLACEHOLDER}\""), 1)
                };
                return out;
            }
            let u = url.unwrap();
            // 去掉 recindex 属性、重写 src 为本地 URL
            let mut out = rec_re.replace_all(tag, "").to_string();
            if src_re.is_match(&out) {
                out = src_re.replace_all(&out, format!("src=\"{u}\"")).to_string();
            } else {
                out = out.replacen("<img", &format!("<img src=\"{u}\""), 1);
            }
            out
        })
        .to_string()
}

fn clean_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let code = ch as u32;
        let is_kf8_marker = (0xE000..=0xF8FF).contains(&code);
        let is_bad_ctrl = code < 0x20 && !matches!(ch, '\n' | '\r' | '\t');
        if is_kf8_marker || is_bad_ctrl {
            continue;
        }
        out.push(ch);
    }
    out
}

/// 兼容旧调用：自动编码、不提取图片。
pub fn parse(path: &str) -> Result<MobiDoc, String> {
    parse_full(path, None, None)
}

/// 完整解析。images=Some((写盘目录, URL 前缀)) 时提取并重写图片引用；
/// force_encoding=Some(label) 时跳过嗅探按该编码重解码正文。
pub fn parse_full(
    path: &str,
    force_encoding: Option<&str>,
    images: Option<(&Path, &str)>,
) -> Result<MobiDoc, String> {
    // 快速失败路径：不整读文件即拦截 DRM/畸形头
    sniff_drm_file(path)?;
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    detect_drm(&bytes)?;
    let mobi = Mobi::new(&bytes).map_err(|e| format!("MOBI/AZW3 解析失败: {e:?}"))?;
    // 双保险：crate 解析后的头字段再查一次（覆盖新式 DRM 的 MOBI 头 DRM 域）
    if mobi.encryption() != Encryption::No {
        return Err("AWZ3EncryptedError: PalmDOC 头标记为已加密，请先解密后导入".to_string());
    }
    if mobi.metadata.mobi.has_drm() {
        return Err(
            "AWZ3EncryptedError: MOBI 头 DRM 记录偏移域非空（存在 DRM 记录），请先解密后导入".to_string(),
        );
    }
    let raw = extract_records_and_decompress(&mobi)?;
    let is_huff = mobi.compression() == Compression::Huff;
    let lang_cjk = matches!(
        mobi.language(),
        mobi::headers::Language::Chinese | mobi::headers::Language::Unknown
    );
    let decoded = if is_huff {
        String::from_utf8_lossy(&raw).into_owned()
    } else {
        decode_html_bytes(&raw, mobi.text_encoding(), force_encoding, lang_cjk)
    };
    let mut html = clean_html(&decoded);
    let (toc, parts) = split_mobi_chapters(&html);
    let mut image_count = 0usize;
    if let Some((dir, prefix)) = images {
        let (by_rec, by_res, ordered) = extract_images(&mobi, dir, prefix);
        image_count = ordered.len();
        if image_count > 0 {
            html = rewrite_images(&html, &by_rec, &by_res, &ordered);
        }
    }
    let title = mobi.title().trim().to_string();
    let author = mobi.author().unwrap_or_default().trim().to_string();
    Ok(MobiDoc {
        title: if title.is_empty() {
            std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            title
        },
        author,
        html,
        toc,
        parts,
        image_count,
    })
}

fn split_mobi_chapters(html: &str) -> (Vec<TocEntry>, Vec<(usize, usize)>) {
    let h_re = Regex::new(r"(?is)<h[1-6][^>]*>(.*?)</h[1-6]>").unwrap();
    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    let mut starts: Vec<(usize, String)> = Vec::new();
    for c in h_re.captures_iter(html) {
        let m = c.get(0).unwrap();
        let text: String = tag_re.replace_all(&c[1], "").trim().chars().take(60).collect();
        if text.is_empty() {
            continue;
        }
        if starts.last().map(|(p, _)| *p) == Some(m.start()) {
            continue;
        }
        starts.push((m.start(), text));
    }

    if starts.len() < 2 {
        starts.clear();
        let pb_re = Regex::new(r"(?i)<mbp:pagebreak\s*/?>").unwrap();
        let mut n = 0;
        for m in pb_re.find_iter(html) {
            n += 1;
            starts.push((m.start(), format!("第 {n} 章")));
        }
    }

    let mut toc = Vec::new();
    let mut parts = Vec::new();
    if starts.is_empty() {
        toc.push(TocEntry {
            title: "全文".into(),
            index: 0,
            chars: html.chars().count() as i64,
        });
        parts.push((0, html.len()));
        return (toc, parts);
    }
    if starts[0].0 > 0 {
        toc.push(TocEntry {
            title: "前言".into(),
            index: 0,
            chars: html[..starts[0].0].chars().count() as i64,
        });
        parts.push((0, starts[0].0));
    }
    for (i, (start, title)) in starts.iter().enumerate() {
        let end = if i + 1 < starts.len() {
            starts[i + 1].0
        } else {
            html.len()
        };
        let chars = html.get(*start..end).map(|s| s.chars().count() as i64).unwrap_or(0);
        toc.push(TocEntry {
            title: title.clone(),
            index: toc.len(),
            chars,
        });
        parts.push((*start, end));
    }
    (toc, parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }
    fn push_u32(v: &mut Vec<u8>, x: u32) { v.extend_from_slice(&x.to_be_bytes()); }

    struct Builder {
        compression: u16,
        encryption: u16,
        text: Vec<u8>,
        images: Vec<Vec<u8>>,
        res121: Vec<Vec<u8>>,
        text_encoding: u32,
    }

    impl Builder {
        fn build(&self) -> Vec<u8> {
            // 记录: rec0=PalmDOC(16)+MOBI(232)+EXTH, rec1..=text chunks, then images
            let exth_body = if self.res121.is_empty() {
                12usize
            } else {
                12 + self.res121.iter().map(|d| 8 + d.len()).sum::<usize>()
            };
            let name_rel = 16 + 232 + exth_body + 1; // 名称放最后 +1
            let mut rec0: Vec<u8> = Vec::new();
            // PalmDOC header (16)
            push_u16(&mut rec0, self.compression);
            push_u16(&mut rec0, 0); // unused
            push_u32(&mut rec0, self.text.len() as u32); // text_length
            push_u16(&mut rec0, 1 + self.images.len() as u16); // record_count
            push_u16(&mut rec0, 4096); // record_size
            push_u16(&mut rec0, self.encryption);
            push_u16(&mut rec0, 0);
            // MOBI header (232)
            let mobi_start = rec0.len();
            rec0.extend_from_slice(b"MOBI");
            push_u32(&mut rec0, 232); // header_length (自 identifier 起)
            push_u32(&mut rec0, 2); // mobi_type = MOBI
            push_u32(&mut rec0, self.text_encoding); // 65001/1252
            push_u32(&mut rec0, 0); // id
            push_u32(&mut rec0, 6); // gen_version
            push_u32(&mut rec0, 0); push_u32(&mut rec0, 0); // ortho, inflect
            push_u32(&mut rec0, 0); push_u32(&mut rec0, 0); // index names/keys
            for _ in 0..6 { push_u32(&mut rec0, 0); } // extra indices
            push_u32(&mut rec0, 2); // first_non_book_index (text 记录为 rec1..rec2)
            push_u32(&mut rec0, name_rel as u32); // name offset
            let title = "测试之书";
            push_u32(&mut rec0, title.len() as u32); // name length(按字节)
            push_u16(&mut rec0, 0); // unused
            rec0.push(0); // locale
            rec0.push(0); // language code
            push_u32(&mut rec0, 4); push_u32(&mut rec0, 4); // in/out lang
            push_u32(&mut rec0, 6); // format_version
            push_u32(&mut rec0, if self.images.is_empty() { 0xFFFFFFFF } else { 2u32 }); // first_image_index
            push_u32(&mut rec0, 0xFFFFFFFF); // first_huff_record
            push_u32(&mut rec0, 0); // huff count
            push_u32(&mut rec0, 0); push_u32(&mut rec0, 0); // huff table off/len
            push_u32(&mut rec0, 0x40); // exth_flags: EXTH 存在
            rec0.extend_from_slice(&[0u8; 32]); // unused_0
            push_u32(&mut rec0, 0); // unused_1
            push_u32(&mut rec0, 0xFFFFFFFF); // drm_offset (无 DRM)
            push_u32(&mut rec0, 0); push_u32(&mut rec0, 0); push_u32(&mut rec0, 0); // drm count/size/flags
            rec0.extend_from_slice(&[0u8; 8]); // unused_2
            push_u16(&mut rec0, 1); // first_content_record
            push_u16(&mut rec0, 1); // last_content_record
            push_u32(&mut rec0, 1); // unused_3
            push_u32(&mut rec0, 0xFFFFFFFF); // fcis
            push_u32(&mut rec0, 1);
            push_u32(&mut rec0, 0xFFFFFFFF); // flis
            push_u32(&mut rec0, 1);
            push_u64(&mut rec0, 0);
            push_u32(&mut rec0, 0xFFFFFFFF); // first_compilation_data_section_count? default F
            push_u32(&mut rec0, 0xFFFFFFFF); // data_section_count
            push_u32(&mut rec0, 0xFFFFFFFF); // unused_8
            push_u32(&mut rec0, 0); // extra_record_data_flags
            push_u32(&mut rec0, 0xFFFFFFFF); // first_index_record
            push_u32(&mut rec0, 0); // 填充至 232
            let filled = rec0.len() - mobi_start;
            assert_eq!(filled, 232, "MOBI 头长度应为 232，实际 {filled}");
            // EXTH (12 + records)
            rec0.extend_from_slice(b"EXTH");
            push_u32(&mut rec0, exth_body as u32);
            push_u32(&mut rec0, self.res121.len() as u32);
            for (i, d) in self.res121.iter().enumerate() {
                push_u32(&mut rec0, 121 + i as u32);
                push_u32(&mut rec0, (8 + d.len()) as u32);
                rec0.extend_from_slice(d);
            }
            rec0.push(0); // 填充
            rec0.extend_from_slice(title.as_bytes());

            let mut chunks: Vec<Vec<u8>> = vec![rec0];
            if self.text.is_empty() {
                chunks.push(vec![]);
            } else {
                for part in self.text.chunks(4096) {
                    chunks.push(part.to_vec());
                }
            }
            for im in &self.images {
                chunks.push(im.clone());
            }
            // 组装 PDB
            let n = chunks.len() as u16;
            let mut out: Vec<u8> = Vec::new();
            out.extend_from_slice(&[0u8; 32]); // name
            push_u16(&mut out, 0); // attrs
            push_u16(&mut out, 0); // version
            for _ in 0..6 { push_u32(&mut out, 0); } // dates + appinfo + sortinfo(占4)
            // 上面 6×u32 = created,modified,backup,modnum,appInfo,sortInfo ✓ 到 60
            out.extend_from_slice(b"BOOK");
            out.extend_from_slice(b"MOBI");
            push_u32(&mut out, 0); // unique id seed
            push_u32(&mut out, 0); // next record list
            push_u16(&mut out, n); // num_records  @76
            let mut data_off = 80 + 8 * n as usize;
            for c in &chunks {
                let idv = (out.len() as u16) & 0x7FFF;
                push_u32(&mut out, data_off as u32);
                out.push(0); // attrs
                push_u16(&mut out, idv); // id low
                out.push(0);
                data_off += c.len();
            }
            // 记录列表尾随 2 字节 extra_bytes：rec0 数据自 78 + 8n + 2 = 80 + 8n 开始
            out.push(0);
            out.push(0);
            for c in &chunks {
                out.extend_from_slice(c);
            }
            out
        }
    }

    fn push_u64(v: &mut Vec<u8>, x: u64) { v.extend_from_slice(&x.to_be_bytes()); }

    #[test]
    fn detect_drm_rejects_awz3_encrypted() {
        let b = Builder {
            compression: 2,
            encryption: 2,
            text: b"<p>x</p>".to_vec(),
            images: vec![],
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        let e = detect_drm(&b).unwrap_err();
        assert!(e.contains("AWZ3EncryptedError"), "{e}");
    }

    #[test]
    fn sniff_drm_file_fast_path() {
        let dir = std::env::temp_dir().join(format!("flowread-mobi-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 明文：嗅探应放行
        let ok_file = dir.join("plain.mobi");
        let b_ok = Builder {
            compression: 2,
            encryption: 0,
            text: b"<p>hi</p>".to_vec(),
            images: vec![],
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        std::fs::write(&ok_file, &b_ok).unwrap();
        assert!(sniff_drm_file(ok_file.to_str().unwrap()).is_ok());
        // 加密：仅凭头部即拒绝（不整读/不进解压）
        let bad_file = dir.join("enc.mobi");
        let mut b_bad = b_ok.clone();
        let rec0 = u32b(&b_bad, 0x4E) as usize;
        let off = rec0 + 0x0C;
        b_bad[off] = 0;
        b_bad[off + 1] = 2; // Encryption Type = 2
        std::fs::write(&bad_file, &b_bad).unwrap();
        let e = sniff_drm_file(bad_file.to_str().unwrap()).unwrap_err();
        assert!(e.contains("AWZ3EncryptedError"), "{e}");
        // 解析入口同样快速失败
        let e2 = parse_full(bad_file.to_str().unwrap(), None, None).unwrap_err();
        assert!(e2.contains("AWZ3EncryptedError"), "{e2}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_drm_rejects_bad_magic() {        let mut b = vec![0u8; 512];
        b[0x3C..0x40].copy_from_slice(b"JUNK");
        b[0x40..0x44].copy_from_slice(b"XXXX");
        assert!(detect_drm(&b).unwrap_err().contains("PalmDocFormatError"));
    }

    #[test]
    fn parse_plain_utf8_and_paginate() {
        let text = format!(
            "<h1>第一章</h1><p>{}</p><h1>第二章</h1><p>{}</p>",
            "甲".repeat(200),
            "乙".repeat(200)
        );
        let b = Builder {
            compression: 1,
            encryption: 0,
            text: text.into_bytes(),
            images: vec![],
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        detect_drm(&b).unwrap();
        let mobi = Mobi::new(&b).unwrap();
        assert_eq!(mobi.compression(), Compression::No);
        // 熔断：记录数/输出大小超限时立即报错，不死循环
        let e = extract_records_and_decompress_limited(&mobi, 0, usize::MAX).unwrap_err();
        assert!(e.contains("ParseTimeoutOrCorrupted"), "{e}");
        let e = extract_records_and_decompress_limited(&mobi, 50_000, 8).unwrap_err();
        assert!(e.contains("ParseTimeoutOrCorrupted"), "{e}");
        let raw = extract_records_and_decompress(&mobi).unwrap();
        let html = decode_html_bytes(&raw, mobi.text_encoding(), None, false);
        let (toc, parts) = split_mobi_chapters(&html);
        assert!(html.contains("第一章") && html.contains("乙"), "{html}");
        assert_eq!(toc.len(), 2, "应识别出两个章节 {toc:?}");
        assert_eq!(parts.len(), 2);
        assert_eq!(mobi.title(), "测试之书");
    }

    #[test]
    fn palmdoc_lz77_roundtrip() {
        // "ABABAB" = "AB" + 回引(距离2,长度4)
        let src = vec![b'A', b'B', 0x80, 0x11];
        assert_eq!(decompress_palmdoc_lz77(&src), b"ABABAB".to_vec());
        // 0xC1 => " A"
        assert_eq!(decompress_palmdoc_lz77(&[0xC1]), b" A".to_vec());
        // 0x03 + 3 literal bytes
        assert_eq!(decompress_palmdoc_lz77(&[0x03, b'x', b'y', b'z']), b"xyz".to_vec());
    }

    #[test]
    fn big5_forced_and_meta_sniff() {
        // 繁体中文：编码实际为 Big5，MOBI 头却谎报 UTF-8；meta 也没声明 → 强制 Big5
        let (big5, _, _) = encoding_rs::BIG5.encode("這裡是繁體中文測試");
        let mut raw_head = b"<html><body>".to_vec();
        raw_head.extend_from_slice(&big5);
        let html = decode_html_bytes(&raw_head, TextEncoding::UTF8, Some("big5"), false);
        assert!(html.contains("繁體中文"), "{html}");
        // 带 meta 声明时自动纠偏
        let (gbk, _, _) = encoding_rs::GBK.encode("简体中文测试内容");
        let mut raw2 = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\">".to_vec();
        raw2.extend_from_slice(&gbk);
        let html2 = decode_html_bytes(&raw2, TextEncoding::UTF8, None, false);
        assert!(html2.contains("简体中文"), "{html2}");
    }

    #[test]
    fn image_extraction_and_rewrite() {
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpg.extend_from_slice(&[7u8; 128]);
        let mut png = vec![0x89, b'P', b'N', b'G'];
        png.extend_from_slice(&[9u8; 64]);
        let text = b"<p>a</p><img recindex=\"0002\"/><img src=\"kindle:embed:0001\"/><img src=\"1.jpg\"/><img src=\"kindle:embed:00FF\"/>".to_vec();
        let b = Builder {
            compression: 1,
            encryption: 0,
            text: text.clone(),
            images: vec![jpg, png],
            res121: vec![b"0002:0001".to_vec()],
            text_encoding: 65001,
        }
        .build();
        detect_drm(&b).unwrap();
        let mobi = Mobi::new(&b).unwrap();
        let dir = std::env::temp_dir().join(format!("flowread-mobi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (by_rec, by_res, ordered) = extract_images(&mobi, &dir, "http://flowread.localhost/res/1/");
        assert_eq!(ordered.len(), 2, "应提取 2 张图");
        std::fs::remove_dir_all(&dir).ok();
        let html = String::from_utf8_lossy(&text).to_string();
        let out = rewrite_images(&html, &by_rec, &by_res, &ordered);
        assert!(out.contains("http://flowread.localhost/res/1/r00002.jpg"), "{out}");
        assert!(out.contains("http://flowread.localhost/res/1/r00003.png"), "{out}");
        assert!(!out.contains("recindex"), "{out}");
        assert!(!out.contains("kindle:embed"), "{out}");
        assert!(!out.contains("\"1.jpg\""), "{out}");
        // 解析不到的 kindle:embed:00FF → 透明占位图，不留非法 src
        assert!(out.contains("data:image/gif;base64,R0lGODlhAQAB"), "{out}");
        assert!(!out.contains("00FF"), "{out}");
        assert_eq!(out.matches("<img").count(), 4, "{out}");
    }

    /// 端到端真实文件测试：设置 FLOWREAD_TEST_MOBI="路径1;路径2" 时运行
    #[test]
    fn real_world_samples_end_to_end() {
        let env = match std::env::var("FLOWREAD_TEST_MOBI") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => {
                eprintln!("skip: FLOWREAD_TEST_MOBI not set");
                return;
            }
        };
        for path in env.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let t0 = std::time::Instant::now();
            let doc = parse_full(path, None, None).unwrap_or_else(|e| panic!("{path}: {e}"));
            let dt = t0.elapsed();
            let plain: String = doc
                .html
                .chars()
                .filter(|c| matches!(c, '\u{4e00}'..='\u{9fff}'))
                .take(8)
                .collect();
            eprintln!("{} -> {}ms, toc={}, html={}B, 中文样例={:?}", std::path::Path::new(path).file_name().unwrap().to_string_lossy(), dt.as_millis(), doc.toc.len(), doc.html.len(), plain);
            assert!(!doc.html.is_empty(), "{path}: 正文为空");
            assert!(doc.toc.len() > 0, "{path}: 无章节");
        }
    }
}
