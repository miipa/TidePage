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
    /// 解码质量警告（如源文件编码损坏），供 UI 提示
    pub warning: Option<String>,
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
/// 防卡死熔断上限：畸形/超大文件在解析中途即中止，不拖死 UI。
const MAX_RECORDS: usize = 50_000;
const MAX_HTML_BYTES: usize = 50 * 1024 * 1024;

// ============================ HUFF/CDIC 字节级解码器 ============================
// 规范来源：MobileRead "MOBI#HUFF/CDIC"（同 mobi crate 实现，但此处逐 section
// 输出**原始字节**，不在 Record 内做字符串解码——解码统一在全局字节流上一次性完成）

struct HuffDecoder {
    dictionary: Vec<Option<(Vec<u8>, bool)>>, // (slice, is_raw)
    code_dict: [(u8, bool, u32); 256],       // (code_len, term, max_code)
    min_codes: [u32; 33],
    max_codes: [u32; 33],
}

impl HuffDecoder {
    fn default() -> Self {
        Self {
            dictionary: Vec::new(),
            code_dict: [(0, false, 0); 256],
            min_codes: [0; 33],
            max_codes: [u32::MAX; 33],
        }
    }

    fn load_huff(&mut self, huff: &[u8]) -> Result<(), String> {
        if huff.len() < 16 || &huff[0..4] != b"HUFF" || u32b(huff, 4) != 0x18 {
            return Err("PalmDocFormatError: HUFF 表头无效".to_string());
        }
        let cache_off = u32b(huff, 8) as usize;
        let base_off = u32b(huff, 12) as usize;
        if cache_off + 256 * 4 > huff.len() || base_off + 32 * 8 > huff.len() {
            return Err("PalmDocFormatError: HUFF 表偏移越界".to_string());
        }
        for (i, code) in self.code_dict.iter_mut().enumerate() {
            let v = u32b(huff, cache_off + i * 4);
            let code_len = (v & 0x1F) as u8;
            let term = (v & 0x80) == 0x80;
            if code_len == 0 || code_len > 31 {
                return Err("PalmDocFormatError: HUFF 码长非法".to_string());
            }
            if code_len <= 8 && !term {
                return Err("PalmDocFormatError: HUFF term 位非法".to_string());
            }
            let max_code = ((v >> 8) + 1)
                .checked_shl(32u32.saturating_sub(code_len as u32))
                .unwrap_or(u32::MAX)
                .saturating_sub(1);
            *code = (code_len, term, max_code);
        }
        for idx in 1..=32usize {
            let lo = base_off + (idx - 1) * 8;
            self.min_codes[idx] = u32b(huff, lo).checked_shl(32 - idx as u32).unwrap_or(0);
            self.max_codes[idx] = (u32b(huff, lo + 4) + 1)
                .checked_shl(32 - idx as u32)
                .unwrap_or(u32::MAX)
                .saturating_sub(1);
        }
        Ok(())
    }

    fn load_cdic(&mut self, cdic: &[u8]) -> Result<(), String> {
        if cdic.len() < 16 || &cdic[0..4] != b"CDIC" || u32b(cdic, 4) != 0x10 {
            return Err("PalmDocFormatError: CDIC 表头无效".to_string());
        }
        let num_phrases = u32b(cdic, 8);
        let bits = u32b(cdic, 12);
        if bits > 16 {
            return Err("PalmDocFormatError: CDIC phrase 位宽非法".to_string());
        }
        let existing = self.dictionary.len() as u32;
        if num_phrases < existing {
            return Err("PalmDocFormatError: CDIC phrase 数下溢".to_string());
        }
        let n = (1u32 << bits).min(num_phrases - existing);
        if 16 + n as usize * 2 > cdic.len() {
            return Err("PalmDocFormatError: CDIC 偏移表越界".to_string());
        }
        for i in 0..n as usize {
            let off = u16b(cdic, 16 + i * 2) as usize;
            let hdr_pos = 16 + off;
            if hdr_pos + 2 > cdic.len() {
                return Err("PalmDocFormatError: CDIC phrase 头越界".to_string());
            }
            let hdr = u16b(cdic, hdr_pos);
            let len = (hdr & 0x7FFF) as usize;
            let start = hdr_pos + 2;
            if start + len > cdic.len() {
                return Err("PalmDocFormatError: CDIC phrase 数据越界".to_string());
            }
            self.dictionary.push(Some((cdic[start..start + len].to_vec(), (hdr & 0x8000) == 0x8000)));
        }
        Ok(())
    }

    fn unpack(&mut self, data: &[u8], depth: u8, budget: &mut usize) -> Result<Vec<u8>, String> {
        if depth > 32 {
            return Err("ParseTimeoutOrCorrupted: HUFF 递归过深（畸形文件）".to_string());
        }
        struct BitR<'a> {
            d: &'a [u8],
            byte_pos: usize,
            buf: u64,
            bits: u32,
        }
        impl<'a> BitR<'a> {
            fn fill(&mut self) {
                while self.bits <= 56 {
                    let b = if self.byte_pos < self.d.len() {
                        let v = self.d[self.byte_pos];
                        self.byte_pos += 1;
                        v
                    } else {
                        0
                    };
                    self.buf = (self.buf << 8) | b as u64;
                    self.bits += 8;
                }
            }
            fn peek32(&mut self) -> u32 {
                self.fill();
                (self.buf >> (self.bits - 32)) as u32
            }
            fn skip(&mut self, n: u32) {
                self.buf <<= n;
                self.bits -= n;
            }
            fn left(&self) -> usize {
                self.byte_pos.saturating_mul(8).saturating_sub(self.bits as usize)
            }
        }
        let mut br = BitR { d: data, byte_pos: 0, buf: 0, bits: 0 };
        let total_bits = data.len() * 8;
        let mut out: Vec<u8> = Vec::new();
        let mut consumed = 0usize;
        loop {
            if consumed >= total_bits {
                break;
            }
            let code = br.peek32();
            let (code_len, term, mut max_code) = self.code_dict[(code >> 24) as usize];
            if code_len == 0 {
                return Err("ParseTimeoutOrCorrupted: HUFF 码表空槽".to_string());
            }
            let mut cl = code_len as usize;
            if !term {
                match self.min_codes[cl..33].iter().position(|&m| code >= m) {
                    Some(off) => cl += off,
                    None => return Err("ParseTimeoutOrCorrupted: HUFF 码定位失败".to_string()),
                }
                max_code = self.max_codes[cl.min(32)];
            }
            let index = ((max_code.wrapping_sub(code)) >> (32 - cl)) as usize;
            let entry = self
                .dictionary
                .get(index)
                .and_then(|e| e.clone())
                .ok_or("ParseTimeoutOrCorrupted: HUFF 字典索引越界")?;
            let (mut slice, is_raw) = entry;
            if !is_raw {
                slice = self.unpack(&slice, depth + 1, budget)?;
                self.dictionary[index] = Some((slice.clone(), true));
            }
            if *budget < slice.len() {
                return Err("ParseTimeoutOrCorrupted: HUFF 解压输出超上限".to_string());
            }
            *budget -= slice.len();
            out.extend_from_slice(&slice);
            br.skip(cl as u32);
            consumed += cl;
        }
        let _ = br.left();
        Ok(out)
    }
}

/// HUFF 分支：加载表并逐 section 解压为字节（不做任何字符串解码）
fn huff_decompress_bytes(mobi: &Mobi, max_bytes: usize) -> Result<Vec<u8>, String> {
    let m = &mobi.metadata.mobi;
    let huff_start = m.first_huff_record as usize;
    let huff_count = m.huff_record_count as usize;
    let all = mobi.raw_records();
    let recs = all.records();
    if huff_start == 0 || huff_count < 2 || huff_start + huff_count > recs.len() {
        return Err("PalmDocFormatError: HUFF 表记录范围非法".to_string());
    }
    let mut dec = HuffDecoder::default();
    dec.load_huff(recs[huff_start].content)?;
    for r in &recs[huff_start + 1..huff_start + huff_count] {
        dec.load_cdic(r.content)?;
    }
    let rng = mobi.readable_records_range();
    let s0 = rng.start.min(recs.len());
    let e0 = rng.end.min(recs.len()).max(s0);
    let mut budget = max_bytes;
    let mut out = Vec::new();
    for r in &recs[s0..e0] {
        out.extend(dec.unpack(r.content, 0, &mut budget)?);
    }
    Ok(out)
}

pub fn extract_records_and_decompress(mobi: &Mobi) -> Result<Vec<u8>, String> {
    extract_records_and_decompress_limited(mobi, MAX_RECORDS, MAX_HTML_BYTES).map(|(b, _)| b)
}

/// 只做"解压 + 拼接"，输出**原始字节流**；字符串解码统一由上层对整个字节流一次完成，
/// 从根上避免 Record/section 边界截断汉字产生乱码。
pub fn extract_records_and_decompress_limited(
    mobi: &Mobi,
    max_records: usize,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<String>), String> {
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
        Compression::Huff => match huff_decompress_bytes(mobi, max_bytes) {
            Ok(bytes) => Ok((bytes, None)),
            // 自研 HUFF 解码失败时退回 crate 的 section 级解码（边界可能有瑕疵但不致读不了）
            Err(_e) => {
                let s = mobi.content_as_string_lossy();
                if s.len() > max_bytes {
                    return Err(format!(
                        "ParseTimeoutOrCorrupted: HUFF 解压输出 {} 字节超过上限",
                        s.len()
                    ));
                }
                Ok((s.into_bytes(), Some("HUFF/CDIC 表异常，已按 section 回退解码，个别字可能有乱码".to_string())))
            }
        },
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
            Ok((out, None))
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
            Ok((out, None))
        }
    }
}

fn sniff_meta_charset(raw: &[u8]) -> Option<String> {
    // 前 4KB 以 Latin-1 无损视图扫描 <meta ... charset=...>（ASCII 标签不受编码影响）
    let head: String = raw.iter().take(4096).map(|b| *b as char).collect();
    let re = Regex::new(r#"(?i)c(?:harset|oding)\s*=\s*["']?\s*([a-z0-9][a-z0-9_\-]{1,20})"#).unwrap();
    re.captures(&head).map(|c| c[1].to_ascii_lowercase())
}

pub struct DecodedText {
    pub text: String,
    pub warning: Option<String>,
}

/// 全字节流一次性解码（严禁在 Record 循环内做任何字符串解码）。
/// 顺序：用户强制 → 严格 UTF-8 → GBK → Big5 → CP1252 →
/// 全部失败才进入"修复式解码"（剔除非法字节，不产生 U+FFFD 方块）并回报警告。
pub fn decode_html_bytes(raw: &[u8], header_enc: TextEncoding, forced: Option<&str>) -> DecodedText {
    let forced = forced
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty() && s != "auto");
    if let Some(label) = &forced {
        // 用户指定编码：直接用对应 Decoder 解码整个字节流，有未映射字节则警告
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (cow, _actual, had_err) = enc.decode(raw);
            return DecodedText {
                text: cow.into_owned(),
                warning: if had_err {
                    Some(format!("按指定编码 {label} 解码时存在无法映射的字节，可能有个别乱码"))
                } else {
                    None
                },
            };
        }
    }
    // meta 声明（Latin-1 视图嗅探）。注意：GBK 能"合法地"吞下 UTF-8 字节流，
    // 所以仅当 UTF-8 整体不可读时才允许 meta 直接接管，防止谎报声明劫持正常书。
    if utf8_try(raw).is_none() {
        if let Some(label) = sniff_meta_charset(raw) {
            if let Some(text) = strict_decode(&label, raw) {
                return DecodedText { text, warning: None };
            }
        }
    }
    // 自动链：按文件头声明分层决策。
    // UTF-8 中文序列被 GBK 两两配对后几乎全合法（错误率高），"GBK 严格成功"不能压过
    // "UTF-8 头 + 低损坏率"的联合证据；仅当 UTF-8 几乎全线崩（>20% 坏段）才让位给中文编码。
    match header_enc {
        TextEncoding::UTF8 | TextEncoding::Unknown(_) => {
            if let Some((text, dropped)) = utf8_try(raw) {
                return DecodedText { text, warning: dropped.then_some("源文件含少量损坏字节序列，已自动剔除，可能有极个别缺字".to_string()) };
            }
            // UTF-8 损坏率 >20%：头可能谎报，给中文编码机会
            for label in ["gbk", "big5"] {
                if let Some(text) = strict_decode(label, raw) {
                    if cjk_heavy(&text) {
                        return DecodedText { text, warning: None };
                    }
                }
            }
            let (text, _n) = utf8_strip_invalid(raw);
            DecodedText {
                text,
                warning: Some("检测到编码异常（源文件存在大量损坏字节序列，已按 UTF-8 尽量修复），可能存在少量乱码".to_string()),
            }
        }
        TextEncoding::CP1252 => {
            // 头声称 CP1252：西文书多数可严格解析；中文书先试 CJK 占主体的 GBK/Big5
            for label in ["gbk", "big5"] {
                if let Some(text) = strict_decode(label, raw) {
                    if cjk_heavy(&text) {
                        return DecodedText { text, warning: None };
                    }
                }
            }
            if let Some((text, dropped)) = utf8_try(raw) {
                return DecodedText { text, warning: dropped.then_some("源文件含少量损坏字节序列，已自动剔除".to_string()) };
            }
            if let Some(text) = strict_decode("windows-1252", raw) {
                return DecodedText { text, warning: None };
            }
            let (text, _n) = utf8_strip_invalid(raw);
            DecodedText {
                text,
                warning: Some("检测到编码异常，可能存在少量乱码".to_string()),
            }
        }
    }
}

/// UTF-8 严格解码 + 损坏率 ≤1% 时按修复处理（剔除非法字节）。返回 (文本, 是否有剔除)。
fn utf8_try(raw: &[u8]) -> Option<(String, bool)> {
    if let Ok(s) = std::str::from_utf8(raw) {
        return Some((s.to_string(), false));
    }
    let (text, dropped) = utf8_strip_invalid(raw);
    // 坏段平均按 ~2 字节算：dropped*200 <= len ⇒ 损坏率 ≤1%
    if dropped * 200 <= raw.len().max(1) && !text.is_empty() {
        Some((text, dropped > 0))
    } else {
        None
    }
}

fn cjk_heavy(s: &str) -> bool {
    let total = s.chars().count().max(1);
    let cjk = s
        .chars()
        .filter(|c| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'))
        .count();
    cjk * 10 > total
}

/// 严格解码：要求整个字节流被该编码零错误解析，否则 None（继续尝试下一编码）
fn strict_decode(label: &str, raw: &[u8]) -> Option<String> {
    let enc = encoding_rs::Encoding::for_label(label.as_bytes())?;
    let (cow, _actual, had_err) = enc.decode(raw);
    if had_err {
        return None;
    }
    Some(cow.into_owned())
}

fn utf8_strip_invalid(raw: &[u8]) -> (String, usize) {
    let mut s = String::with_capacity(raw.len());
    let mut rest = raw;
    let mut dropped = 0usize;
    loop {
        match std::str::from_utf8(rest) {
            Ok(v) => {
                s.push_str(v);
                return (s, dropped);
            }
            Err(e) => {
                s.push_str(&String::from_utf8_lossy(&rest[..e.valid_up_to()]));
                // 该 valid_up_to 之前保证合法，lossy 不产生替换符
                let elen = e.error_len().unwrap_or(1);
                dropped += 1;
                rest = &rest[(e.valid_up_to() + elen).min(rest.len())..];
            }
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

/// 文本节点内的实体解码（数字 + 命名），返回 (结果, 命中实体数)
fn decode_entities_text(s: &str) -> (String, usize) {
    static ENT_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"(?i)&(?:amp;)?#([xX]?)([0-9a-fA-F]{1,7});").unwrap()
    });
    let out = ENT_RE.replace_all(s, |c: &Captures| {
        let hex = !c[1].is_empty();
        let cp = u32::from_str_radix(&c[2], if hex { 16 } else { 10 }).unwrap_or(0xFFFFFFFF);
        match cp {
            0 => c[0].to_string(),
            0x3C | 0x3E | 0x26 => c[0].to_string(),
            v if v > 0x10FFFF => c[0].to_string(),
            _ => char::from_u32(cp).map(|ch| ch.to_string()).unwrap_or_else(|| c[0].to_string()),
        }
    });
    static NAMED_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"&(?:amp;)?(nbsp|bull|middot|hellip|mdash|ndash|lsquo|rsquo|ldquo|rdquo|laquo|raquo|copy|reg|trade|deg|plusmn|times|divide|sect|para|dagger|Dagger|permil|prime|Prime|larr|rarr|uarr|darr|harr|ne|equiv|minus|micro|cedil|circ|tilde|euro|pound|yen|curren|brvbar|prod|sum|radic|infin|asymp|isin|not|and|or|cap|cup|int)();").unwrap()
    });
    let mut hits = 0usize;
    let out2 = NAMED_RE.replace_all(&out, |c: &Captures| -> String {
        hits += 1;
        let mapped: Option<char> = match &c[1] {
            "nbsp" => Some('\u{A0}'),
            "bull" => Some('\u{2022}'),
            "middot" => Some('\u{B7}'),
            "hellip" => Some('\u{2026}'),
            "mdash" => Some('\u{2014}'),
            "ndash" => Some('\u{2013}'),
            "lsquo" => Some('\u{2018}'),
            "rsquo" => Some('\u{2019}'),
            "ldquo" => Some('\u{201C}'),
            "rdquo" => Some('\u{201D}'),
            "laquo" => Some('\u{AB}'),
            "raquo" => Some('\u{BB}'),
            "copy" => Some('\u{A9}'),
            "reg" => Some('\u{AE}'),
            "trade" => Some('\u{2122}'),
            "deg" => Some('\u{B0}'),
            "plusmn" => Some('\u{B1}'),
            "times" => Some('\u{D7}'),
            "divide" => Some('\u{F7}'),
            "sect" => Some('\u{A7}'),
            "para" => Some('\u{B6}'),
            "dagger" => Some('\u{2020}'),
            "Dagger" => Some('\u{2021}'),
            "permil" => Some('\u{2030}'),
            "prime" => Some('\u{2032}'),
            "Prime" => Some('\u{2033}'),
            "larr" => Some('\u{2190}'),
            "rarr" => Some('\u{2192}'),
            "uarr" => Some('\u{2191}'),
            "darr" => Some('\u{2193}'),
            "harr" => Some('\u{2194}'),
            "ne" => Some('\u{2260}'),
            "equiv" => Some('\u{2261}'),
            "minus" => Some('\u{2212}'),
            "micro" => Some('\u{B5}'),
            "cedil" => Some('\u{B8}'),
            "circ" => Some('\u{2C6}'),
            "tilde" => Some('\u{2DC}'),
            "euro" => Some('\u{20AC}'),
            "pound" => Some('\u{A3}'),
            "yen" => Some('\u{A5}'),
            "curren" => Some('\u{A4}'),
            "brvbar" => Some('\u{A6}'),
            "prod" => Some('\u{220F}'),
            "sum" => Some('\u{2211}'),
            "radic" => Some('\u{221A}'),
            "infin" => Some('\u{221E}'),
            "asymp" => Some('\u{2248}'),
            "isin" => Some('\u{2208}'),
            "not" => Some('\u{AC}'),
            "and" => Some('\u{2227}'),
            "or" => Some('\u{2228}'),
            "cap" => Some('\u{2229}'),
            "cup" => Some('\u{222A}'),
            "int" => Some('\u{222B}'),
            _ => None,
        };
        match mapped {
            Some(ch) => ch.to_string(),
            None => c[0].to_string(),
        }
    });
    (out2.to_string(), hits)
}


/// 标准 tokenizer 定界 + 原始标签透传管线（结构零破坏）：
/// - 标签（属性原文、mbp:pagebreak、recindex）逐 token 从 tokenizer 重序列化；
///   属性值中 `&`/`"` 重新转义（tokenizer 已解码，浏览器还要再解一次，转义回填保证值一致且结构安全）
/// - 文本节点缓冲合并后选择性实体解码（`&amp;#N;` 双重转义、`&#N;`、命名实体），
///   并把解码出的 `<`/`>` 再转义防文本变标签
/// - `<script>`/`<style>` 连内容丢弃；检测到未闭合 RAWTEXT 时改为"去标签留文本"（正文优先）
pub fn decode_and_sanitize(html: &str) -> String {
    use html5ever::tokenizer::{TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct St {
        out: String,
        pending_text: String, // tokenizer 在实体边界切分 CharacterTokens，须合并后统一解码
        drop_raw: Option<String>,
        dangling: bool,
    }
    impl St {
        fn flush(&mut self) {
            if self.pending_text.is_empty() {
                return;
            }
            let t = std::mem::take(&mut self.pending_text);
            let (d, _e) = decode_entities_text(&t);
            // 解码可能引入裸 < >（源自 &lt; 等实体的双重语义），重新转义保证只作文本；
            // tokenizer 对畸形实体/输入按规范产 U+FFFD，同样剔除（与 strip 策略一致）
            for ch in d.chars() {
                match ch {
                    '<' => self.out.push_str("&lt;"),
                    '>' => self.out.push_str("&gt;"),
                    '\u{FFFD}' => {}
                    c => self.out.push(c),
                }
            }
        }
    }
    struct Sink(Rc<RefCell<St>>);
    impl Sink {
        fn ser_attr_value(&self, v: &str, out: &mut String) {
            for c in v.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '"' => out.push_str("&quot;"),
                    // tokenizer 对属性内畸形实体会规范产出 U+FFFD，剔除（属性值无害化）
                    '\u{FFFD}' => {}
                    c => out.push(c),
                }
            }
        }
    }
    impl TokenSink for Sink {
        type Handle = ();
        fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
            let mut st = self.0.borrow_mut();
            match token {
                Token::TagToken(tag) => {
                    st.flush();
                    let name = tag.name.to_string();
                    let ln = name.to_ascii_lowercase();
                    let is_raw = ln == "script" || ln == "style";
                    if is_raw {
                        if matches!(tag.kind, TagKind::StartTag) && !tag.self_closing {
                            if st.dangling {
                                return TokenSinkResult::Continue; // 悬空模式：去标签，文本按普通处理
                            }
                            st.drop_raw = Some(ln);
                            return TokenSinkResult::Continue;
                        }
                        if matches!(tag.kind, TagKind::EndTag) {
                            st.drop_raw = None;
                            return TokenSinkResult::Continue;
                        }
                    }
                    if st.drop_raw.is_some() {
                        return TokenSinkResult::Continue;
                    }
                    let is_end = matches!(tag.kind, TagKind::EndTag);
                    let mut s = String::with_capacity(name.len() + 8);
                    s.push('<');
                    if is_end {
                        s.push('/');
                    }
                    s.push_str(&name);
                    if !is_end {
                        for a in tag.attrs.iter() {
                            s.push(' ');
                            s.push_str(&a.name.local.to_string());
                            let v = a.value.to_string();
                            if !v.is_empty() {
                                s.push_str("=\"");
                                self.ser_attr_value(&v, &mut s);
                                s.push('"');
                            }
                        }
                        if tag.self_closing {
                            s.push_str("/>");
                            st.out.push_str(&s);
                            return TokenSinkResult::Continue;
                        }
                    }
                    s.push('>');
                    st.out.push_str(&s);
                }
                Token::CharacterTokens(chars) => {
                    if st.drop_raw.is_some() {
                        return TokenSinkResult::Continue;
                    }
                    st.pending_text.push_str(&chars.to_string());
                }
                Token::DoctypeToken(dt) => {
                    st.flush();
                    let n = dt.name.map(|x| x.to_string()).unwrap_or_default();
                    if !n.is_empty() {
                        st.out.push_str("<!DOCTYPE ");
                        st.out.push_str(&n);
                        st.out.push('>');
                    }
                }
                _ => {}
            }
            TokenSinkResult::Continue
        }
    }

    let dangling = has_dangling_rawtext(html);
    let st = Rc::new(RefCell::new(St {
        out: String::with_capacity(html.len() + 1024),
        pending_text: String::new(),
        drop_raw: None,
        dangling,
    }));
    let tok = Tokenizer::new(Sink(st.clone()), html5ever::tokenizer::TokenizerOpts::default());
    let q = html5ever::tokenizer::BufferQueue::default();
    q.push_back(html.into());
    let _res = tok.feed(&q);
    tok.end();
    let mut b = st.borrow_mut();
    b.flush();
    std::mem::take(&mut b.out)
}
/// 悬空 RAWTEXT 检测：`<style` 与 `</style` 数量不等时，tokenizer 会把未闭合 style
/// 之后的正文全吞进 CSS 文本——此时保留文本只去标签（正文优先于完美）。
fn has_dangling_rawtext(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    for tag in ["style", "script", "textarea", "title"] {
        let (mut opens, mut closes) = (0usize, 0usize);
        let mut from = 0usize;
        while let Some(i) = find_sub(&lower, tag, from) {
            if i > 0 && lower.as_bytes()[i - 1] == b'<' {
                opens += 1;
            } else if i > 1 && lower.as_bytes()[i - 1] == b'/' && lower.as_bytes()[i - 2] == b'<' {
                closes += 1;
            }
            from = i + tag.len();
        }
        if opens != closes {
            return true;
        }
    }
    false
}fn find_sub(hay: &str, needle: &str, from: usize) -> Option<usize> {
    hay.get(from..).and_then(|s| s.find(needle).map(|i| from + i))
}

/// 字符级过滤（KF8 私有区标记/控制字符），在 DOM 清理与实体解码之后执行
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
    let (raw, extract_warn) = extract_records_and_decompress_limited(&mobi, MAX_RECORDS, MAX_HTML_BYTES)?;
    // 一次性全局解码整个字节流（不在任何 Record 内做字符串解码）
    let mut decoded = decode_html_bytes(&raw, mobi.text_encoding(), force_encoding);
    if decoded.warning.is_none() {
        decoded.warning = extract_warn;
    }
    // NUL/控制字符必须在 tokenizer 之前剔除（tag-name 状态按规范会把 NUL 变成 U+FFFD 混进标签名）；
    // 之后再清一次：实体解码可能还原出 PUA/控制字符
    let mut html = clean_html(&decode_and_sanitize(&clean_html(&decoded.text)));
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
        warning: decoded.warning,
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
            let n_text = if self.text.is_empty() {
                1usize
            } else {
                (self.text.len() + 4095) / 4096
            };
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
            push_u32(&mut rec0, 1 + n_text as u32); // first_non_book_index（文本记录为 rec1..rec(1+n_text)）
            push_u32(&mut rec0, name_rel as u32); // name offset
            let title = "测试之书";
            push_u32(&mut rec0, title.len() as u32); // name length(按字节)
            push_u16(&mut rec0, 0); // unused
            rec0.push(0); // locale
            rec0.push(0); // language code
            push_u32(&mut rec0, 4); push_u32(&mut rec0, 4); // in/out lang
            push_u32(&mut rec0, 6); // format_version
            push_u32(&mut rec0, if self.images.is_empty() { 0xFFFFFFFF } else { (1 + n_text) as u32 }); // first_image_index
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
        let html = decode_html_bytes(&raw, mobi.text_encoding(), None).text;
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
        let html = decode_html_bytes(&raw_head, TextEncoding::UTF8, Some("big5")).text;
        assert!(html.contains("繁體中文"), "{html}");
        // 带 meta 声明时自动纠偏
        let (gbk, _, _) = encoding_rs::GBK.encode("简体中文测试内容");
        let mut raw2 = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\">".to_vec();
        raw2.extend_from_slice(&gbk);
        let html2 = decode_html_bytes(&raw2, TextEncoding::UTF8, None).text;
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

    /// 汉字被切断在 Record 边界时，全局一次性解码必须无损还原（不得出现任何 U+FFFD）
    #[test]
    fn record_boundary_does_not_break_characters() {
        // 4500 个汉字（13500 字节）→ Builder 按 4096 切多条记录，字符大概率跨记录被劈开
        let body: String = "魃饕餮貔貅魑魅魍魎齾龘靐".repeat(340);
        let html_doc = format!("<p>{body}</p>");
        let b = Builder {
            compression: 1,
            encryption: 0,
            text: html_doc.into_bytes(),
            images: vec![],
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        let mobi = Mobi::new(&b).unwrap();
        let raw = extract_records_and_decompress(&mobi).unwrap();
        let d = decode_html_bytes(&raw, TextEncoding::UTF8, None);
        assert!(d.warning.is_none(), "干净文本不应有警告: {:?}", d.warning);
        assert!(!d.text.contains('\u{FFFD}'), "出现替换符说明仍有 Record 级解码");
        assert!(d.text.matches("魃").count() >= 340);
    }

    /// 繁体生僻字：强制 Big5 完美解码；无声明损坏时走修复路径并带警告
    #[test]
    fn big5_rare_traditional_and_repair_warning() {
        let (big5, _, _) = encoding_rs::BIG5.encode("龍馬精神富貴吉祥繁榮昌盛圖騰");
        let mut raw = b"<html><body>".to_vec();
        raw.extend_from_slice(&big5);
        let d = decode_html_bytes(&raw, TextEncoding::UTF8, Some("big5"));
        assert!(d.text.contains("龍馬精神富貴吉祥繁榮昌盛圖騰"), "{}", d.text);
        assert!(d.warning.is_none());
        // 有效 UTF-8 中文 + 注入无法整链解析的垃圾字节 → 全部严格失败 → 修复式（无 U+FFFD）+ 警告
        let mut raw2: Vec<u8> = Vec::new();
        for _ in 0..400 {
            raw2.extend_from_slice("繁體中文測試。".as_bytes());
        }
        // 每隔 ~900 字节注入 UTF-8/GBK/Big5/CP1252 全部非法的序列（0x8D 后跟 0xFF：GBK 尾字节非法）
        let mut i = 900usize;
        while i < raw2.len() {
            raw2.splice(i..i, vec![0x8D, 0xFF, 0xFF, 0xFF, 0x81]);
            i += 950;
        }
        let d2 = decode_html_bytes(&raw2, TextEncoding::UTF8, None);
        assert!(d2.text.contains("繁體中文測試"));
        assert!(!d2.text.contains('\u{FFFD}'), "修复模式不得输出替换符");
        assert!(d2.warning.is_some(), "应有编码异常警告: {:?}", d2.warning);
    }

    #[test]
    fn entity_decode_and_sanitize() {
        // 双重转义与裸实体均还原；属性内永不解码（回归根因）
        let d = decode_and_sanitize("<p>&amp;#8226; a&amp;#8226;&#8226;&#x2022;&amp;nbsp;</p>");
        assert!(d.contains("• a•••"), "{d}");
        assert!(d.contains(" "), "{d}");
        let g = decode_and_sanitize("<p style=\"text-align:&and;center\" title=\"x&amp;y\">a &amp; b &and;</p>");
        // 属性经 tokenizer 解码后按**等值**序列化（`&and;`→`∧` 不再引入裸引号破坏标签结构；
        // `&amp;` 回填转义），文本实体还原
        assert!(g.contains("text-align:∧center"), "属性值等价且结构完好: {g}");
        assert!(g.contains("title=\"x&amp;y\""), "属性 & 转义回填: {g}");
        assert!(g.contains("a & b ∧"), "文本节点实体解码: {g}");
        assert_eq!(g.matches('<').count(), g.matches('>').count(), "标签结构完好: {g}");
        // script/style 连内容丢弃；标签/属性结构保留
        let c = decode_and_sanitize("<p>x</p><script>evil()</script><p>y</p>");
        assert!(!c.contains("evil") && c.contains("<p>x</p>") && c.contains("<p>y</p>"), "{c}");
        let c2 = decode_and_sanitize("<p>ok</p><style>.a{font-family:\"N\";aid:\"x\"}</style><p>keep</p>");
        assert!(!c2.contains("aid:") && !c2.contains("font-family"), "CSS 不得泄漏: {c2}");
        assert!(c2.contains("ok") && c2.contains("keep"), "{c2}");
        // 未闭合 style：保留正文（宁可 CSS 露出也不吞正文）
        let c3 = decode_and_sanitize("<p>a</p><style>.x{y:z}<p>body-must-survive</p>");
        assert!(c3.contains("body-must-survive"), "{c3}");
        // mbp:pagebreak 与 recindex 属性原样保留
        let m = decode_and_sanitize("<p>1</p><mbp:pagebreak/><img recindex=\"0002\" alt=\"0\"/>");
        assert!(m.contains("mbp:pagebreak") && m.contains("recindex=\"0002\""), "{m}");
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
            let fffd = doc.html.matches('\u{FFFD}').count();
            eprintln!(
                "{} -> {}ms, toc={}, html={}B, FFFD={}, warning={:?}, 中文样例={:?}",
                std::path::Path::new(path).file_name().unwrap().to_string_lossy(),
                dt.as_millis(),
                doc.toc.len(),
                doc.html.len(),
                fffd,
                doc.warning,
                plain
            );
            assert!(!doc.html.is_empty(), "{path}: 正文为空");
            assert!(doc.toc.len() > 0, "{path}: 无章节");
        }
    }
}
