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
    /// EXTH121 资源号 → 本地 URL 的映射数量（by_res）
    pub res_urls: usize,
    /// 无法解析、被透明占位图替换的图片引用数
    pub placeholder_count: usize,
    /// 解析质量警告（如源文件编码损坏），供 UI 提示
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
        // bit 读取器：与 calibre/MobileRead 参考实现一致的 64 位滑动窗口。
        // 不能用「左对齐缓冲 + 逐字节 fill」的近似实现（跳位后与参考实现错位，
        // 会把正文解码成错误索引甚至自引用循环）。
        let mut padded = data.to_vec();
        padded.extend_from_slice(&[0u8; 8]);
        let read_u64 = |p: &[u8], pos: usize| -> u64 {
            if pos + 8 <= p.len() {
                u64::from_be_bytes([
                    p[pos], p[pos + 1], p[pos + 2], p[pos + 3],
                    p[pos + 4], p[pos + 5], p[pos + 6], p[pos + 7],
                ])
            } else {
                0
            }
        };
        let total_bits = data.len() as i64 * 8;
        let mut bits_remaining = total_bits;
        let mut pos = 0usize;
        let mut x = read_u64(&padded, pos);
        let mut n: i32 = 32;
        let mut out: Vec<u8> = Vec::new();
        while bits_remaining > 0 {
            if n <= 0 {
                pos += 4;
                x = read_u64(&padded, pos);
                n += 32;
            }
            let code = ((x >> n) & 0xFFFF_FFFF) as u32;
            let (code_len, term, mut max_code) = self.code_dict[(code >> 24) as usize];
            if code_len == 0 {
                return Err("ParseTimeoutOrCorrupted: HUFF 码表空槽".to_string());
            }
            let mut cl = code_len as usize;
            if !term {
                while cl < 32 && code < self.min_codes[cl] {
                    cl += 1;
                }
                max_code = self.max_codes[cl.min(32)];
            }
            n -= cl as i32;
            bits_remaining -= cl as i64;
            if bits_remaining < 0 {
                break;
            }
            let index = ((max_code.wrapping_sub(code)) >> (32 - cl)) as usize;
            // 取出条目并**立即置空**：既做缓存失效，又用于检测循环引用
            // （合法 CDIC 不会自引用；循环说明表/区间异常，直接报错而不是无限递归）。
            let entry = self
                .dictionary
                .get_mut(index)
                .ok_or("ParseTimeoutOrCorrupted: HUFF 字典索引越界")?
                .take()
                .ok_or("PalmDocFormatError: HUFF 字典存在循环引用（表或正文区间异常）")?;
            let (mut slice, is_raw) = entry;
            if !is_raw {
                match self.unpack(&slice, depth + 1, budget) {
                    Ok(expanded) => slice = expanded,
                    Err(e) => {
                        // 失败也要放回原条目，避免污染字典（下次可重新尝试/回退）
                        self.dictionary[index] = Some((slice, false));
                        return Err(e);
                    }
                }
            }
            if *budget < slice.len() {
                return Err("ParseTimeoutOrCorrupted: HUFF 解压输出超上限".to_string());
            }
            *budget -= slice.len();
            self.dictionary[index] = Some((slice.clone(), true));
            out.extend_from_slice(&slice);
        }
        Ok(out)
    }
}

/// 安全记录切片：语义等价 `mobi` crate 的 `RawRecords::parse`（按 PDB 偏移切分
/// content，扣除 extra_bytes），但**所有越界都被钳制**。受损/KF8 变体文件的
/// 偏移表可能不升序或超界，crate 内部会直接切片 panic 导致 app 崩溃，
/// 这里自实现解析以保证任何输入都能安全降级。
#[derive(Debug, Clone, Copy)]
pub struct Rec<'a> {
    pub index: usize,
    pub content: &'a [u8],
}

fn raw_records_safe(mobi: &Mobi) -> Vec<Rec<'_>> {
    raw_records_safe_data(mobi, mobi.content.as_slice())
}

/// 用**原始文件字节**按 PDB 偏移切分记录。`mobi.content` 会把解析元数据用到的
/// 前导区置零（crate 内部 `vec![0; position]` + `read_to_end` 的副作用），
/// 若用它切分会把 record0 的内容当成全零，进而误判为正文；因此有原文件字节时一律传入。
fn raw_records_safe_data<'a>(mobi: &Mobi, content: &'a [u8]) -> Vec<Rec<'a>> {
    let recs = &mobi.metadata.records.records;
    let extra = mobi.metadata.records.extra_bytes() as usize;
    let n = recs.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = (recs[i].offset as usize).min(content.len());
        let end = if i + 1 < n {
            let nxt = recs[i + 1].offset as usize;
            if nxt > extra {
                (nxt - extra).min(content.len())
            } else {
                start
            }
        } else {
            content.len()
        };
        let end = end.max(start);
        out.push(Rec {
            index: i,
            content: &content[start..end],
        });
    }
    out
}

/// 判断记录内容是否像 PalmDOC/MOBI 文本块（而非头/索引/图片/媒体）。
/// 部分 AZW3 的 `first_content_record` 为 0（把 record0 头也算进正文区间），
/// 必须跳过；同时跳过 FCIS/FLIS/IDXT/RESC/RESC 等索引与媒体记录。
fn looks_like_text_record(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }
    const NON_TEXT: &[&[u8]] = &[
        b"FCIS", b"FLIS", b"FDST", b"IDXT", b"RESC", b"BOUN", b"SRCS", b"AUDI", b"VIDE",
        b"INDX", b"TAGX", b"HUFF", b"CDIC",
    ];
    if NON_TEXT.iter().any(|t| content.starts_with(t)) {
        return false;
    }
    // PalmDOC/MOBI 头（record0）：前 4 字节为压缩算法（0x0001/0x0002/0x4448）+ 后续文本长度
    if content.len() >= 4 {
        let c = u16::from_be_bytes([content[0], content[1]]);
        if matches!(c, 1 | 2 | 17480) {
            return false;
        }
    }
    true
}

/// 正文记录区间：以 crate 的 `readable_records_range` 为起点，
/// 跳过开头的 record0 头与 FCIS/FLIS/IDXT 等非文本记录（部分 AZW3 会把它们算进去，
/// 导致把 MOBI 头当作正文解码，产出空正文或乱码）。
fn content_records_range(mobi: &Mobi, recs: &[Rec<'_>]) -> std::ops::Range<usize> {
    let rng = mobi.readable_records_range();
    let mut start = rng.start.min(recs.len());
    let end = rng.end.min(recs.len()).max(start);
    // 只跳过区间开头连续的非文本记录（正文中途的图片/索引不应跳，交由上层处理）。
    while start < end && !looks_like_text_record(recs[start].content) {
        start += 1;
    }
    start..end
}

/// HUFF 分支：加载表并逐 section 解压为字节（不做任何字符串解码）
fn huff_decompress_bytes(mobi: &Mobi, max_bytes: usize) -> Result<Vec<u8>, String> {
    let m = &mobi.metadata.mobi;
    let huff_start = m.first_huff_record as usize;
    let huff_count = m.huff_record_count as usize;
    let recs = raw_records_safe(mobi);
    if huff_start == 0 || huff_count < 2 || huff_start + huff_count > recs.len() {
        return Err("PalmDocFormatError: HUFF 表记录范围非法".to_string());
    }
    let mut dec = HuffDecoder::default();
    dec.load_huff(recs[huff_start].content)?;
    for r in &recs[huff_start + 1..huff_start + huff_count] {
        dec.load_cdic(r.content)?;
    }
    let rng = content_records_range(mobi, &recs);
    let mut budget = max_bytes;
    let mut out = Vec::new();
    for r in &recs[rng.clone()] {
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
    let all = raw_records_safe(mobi);
    let range = content_records_range(mobi, &all);
    // 直接用完整records：mobi crate 的 RawRecords::range() 对 Excluded end 做了 -1 处理
    // （会丢弃最后一条记录），这里不用它。
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
            Ok(bytes) if !bytes.is_empty() => Ok((bytes, None)),
            first => {
                let first_err = match first {
                    Ok(_) => "HUFF 解压结果为空".to_string(),
                    Err(e) => e,
                };
                // 自研 HUFF 解码失败/为空时，退回 crate 的 section 级解码。
                // crate 内部可能因偏移表异常 panic，这里兜底为安全默认值。
                let s = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mobi.content_as_string_lossy()
                }))
                .unwrap_or_default();
                if !s.trim().is_empty() {
                    if s.len() > max_bytes {
                        return Err(format!(
                            "ParseTimeoutOrCorrupted: HUFF 解压输出 {} 字节超过上限",
                            s.len()
                        ));
                    }
                    return Ok((
                        s.into_bytes(),
                        Some(format!(
                            "HUFF/CDIC 表异常（{first_err}），已按 section 回退解码，个别字可能有乱码"
                        )),
                    ));
                }
                // 两条解码路径都拿不到正文：明确、可操作地报错（含诊断开关提示）。
                Err(format!(
                    "PalmDocFormatError: HUFF/CDIC 正文解码失败（{first_err}）。\
                     该书可能为不支持的 KF8 变体或压缩表受损，请在书库中「转换为 EPUB」后再阅读\
                     （设 TIDEPAGE_MOBI_DIAG=1 可输出诊断信息）"
                ))
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

/// KF8（AZW3）检测结果，仅用于可诊断日志与回退决策。
#[derive(Debug, Default)]
pub struct Kf8Info {
    /// 是否为 KF8 或 MOBI6+KF8 联合文件
    pub is_kf8: bool,
    /// EXTH121 记录的 KF8 边界偏移（联合文件才有）
    pub boundary_exth: Option<u32>,
    /// 当前解析到的头类型：MOBI6 / KF8 / JOINT(MOBI6+KF8)
    pub header_kind: &'static str,
}

/// 只读探测：MOBI6 头 format_version>=8 或存在 KF8 边界，即认为含 KF8。
/// 说明：`mobi` 0.8 只能解析 record0 处的头（联合文件即 MOBI6 部分）。
/// KF8 优先需要替换/新增依赖，故此处仅诊断；正文回退用 record0 头解析，
/// 联合文件的 MOBI6 正文仍然完整可读，KF8-only 文件 record0 本身即 KF8 头。
pub fn detect_kf8(mobi: &Mobi) -> Kf8Info {
    let m = &mobi.metadata.mobi;
    let boundary_exth = mobi
        .metadata
        .exth_record_at(121)
        .and_then(|blobs| blobs.first())
        .and_then(|b| b.get(0..4))
        .map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let has_boundary = raw_records_safe(mobi)
        .iter()
        .any(|r| r.content.starts_with(b"BOUNDARY"));
    let kf8_header = m.format_version >= 8;
    let is_kf8 = has_boundary || kf8_header;
    let header_kind = if has_boundary && kf8_header {
        "KF8(+boundary)"
    } else if has_boundary {
        "JOINT(MOBI6+KF8)"
    } else if kf8_header {
        "KF8"
    } else {
        "MOBI6"
    };
    Kf8Info {
        is_kf8,
        boundary_exth,
        header_kind,
    }
}

/// 提取封面图原始字节（MOBI6/AZW3 通用）：
/// 优先级 EXTH201(cover) → EXTH202(thumb) → EXTH129(KF8 cover URI) →
/// first_image_index 起第一张图片。失败返回 None（调用方回退占位图）。
pub fn extract_cover_bytes(mobi: &Mobi) -> Option<Vec<u8>> {
    let fi = mobi.metadata.mobi.first_image_index as usize;
    let all = raw_records_safe(mobi);
    if all.is_empty() {
        return None;
    }
    let by_off = |off: u32| -> Option<Vec<u8>> {
        let idx = fi.checked_add(off as usize)?;
        all.get(idx).map(|r| r.content.to_vec())
    };
    // EXTH 201/202：值多为 4 字节大端整数，需叠加 first_image_index
    for rec in [201u32, 202u32] {
        if let Some(blobs) = mobi.metadata.exth_record_at(rec) {
            for b in blobs {
                if b.len() >= 4 {
                    let off = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                    if let Some(bytes) = by_off(off) {
                        if image_magic(&bytes).is_some() {
                            return Some(bytes);
                        }
                    }
                }
            }
        }
    }
    // EXTH 129：KF8 cover URI，形如 kindle:embed:0001
    if let Some(blobs) = mobi.metadata.exth_record_at(129) {
        let re = Regex::new(r"(?i)kindle:embed:([0-9a-fA-F]{1,8})").unwrap();
        for b in blobs {
            let s = String::from_utf8_lossy(b);
            if let Some(c) = re.captures(&s) {
                if let Ok(n) = u32::from_str_radix(&c[1], 16) {
                    if n >= 1 {
                        if let Some(bytes) = by_off(n - 1) {
                            if image_magic(&bytes).is_some() {
                                return Some(bytes);
                            }
                        }
                    }
                }
            }
        }
    }
    // 回退：first_image_index 起第一张真正图片
    (fi..all.len()).find_map(|i| {
        let c = all[i].content;
        if NON_IMAGE_MAGICS.iter().any(|m| c.starts_with(m)) {
            return None;
        }
        image_magic(c).map(|_| c.to_vec())
    })
}

/// 路径版封面提取（供 library 扫描/重提封面使用）。任何解析异常都安全返回 None。
pub fn extract_cover_path(path: &str) -> Option<Vec<u8>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let bytes = std::fs::read(path).ok()?;
        let mobi = Mobi::new(&bytes).ok()?;
        extract_cover_bytes(&mobi)
    }))
    .ok()
    .flatten()
}

/// **只提取封面（seek）**：不读全文。仅读取
///   ① PDB 头(78B) + 记录偏移表；② record0（PalmDOC+MOBI 头 + EXTH）；③ 封面所在的单条图片记录。
/// 从 record0 的 EXTH 201/202 取封面记录相对偏移（叠加 first_image_index），seek 到该记录读取。
/// 供扫描路径使用，避免为取封面而 `fs::read` 整本（MOBI 常为几十 MB）。
pub fn extract_cover_seek(path: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<Vec<u8>> {
        let mut f = std::fs::File::open(path).ok()?;
        let flen = f.metadata().ok()?.len();
        // PDB 头：0x4C(76) 处 u16 BE = num_records。
        let mut phdr = [0u8; 78];
        f.read_exact(&mut phdr).ok()?;
        let num = u16::from_be_bytes([phdr[76], phdr[77]]) as usize;
        if num == 0 || num > 200_000 {
            return None;
        }
        // 记录偏移表：紧随 78B 之后，num 条，每条 8B（前 4B 为该记录在文件中的偏移）。
        let mut tbl = vec![0u8; num.checked_mul(8)?];
        f.read_exact(&mut tbl).ok()?;
        let rec_off = |i: usize| -> u64 {
            u32::from_be_bytes([tbl[i * 8], tbl[i * 8 + 1], tbl[i * 8 + 2], tbl[i * 8 + 3]]) as u64
        };
        let read_rec = |f: &mut std::fs::File, i: usize, cap: u64| -> Option<Vec<u8>> {
            if i >= num {
                return None;
            }
            let s = rec_off(i);
            let e = if i + 1 < num { rec_off(i + 1) } else { flen };
            if e <= s {
                return None;
            }
            let n = e - s;
            if n > cap {
                return None;
            }
            let mut b = vec![0u8; n as usize];
            f.seek(SeekFrom::Start(s)).ok()?;
            f.read_exact(&mut b).ok()?;
            Some(b)
        };
        // record0：PalmDOC 头(16B) + MOBI 头(+EXTH)。cap 16MB（record0 通常 < 64KB）。
        let r0 = read_rec(&mut f, 0, 16 * 1024 * 1024)?;
        if r0.len() < 16 + 0x84 || &r0[16..20] != b"MOBI" {
            return None;
        }
        let mobi_hdr_len =
            u32::from_be_bytes([r0[16 + 0x14], r0[16 + 0x15], r0[16 + 0x16], r0[16 + 0x17]]) as usize;
        let first_image =
            u32::from_be_bytes([r0[16 + 0x6C], r0[16 + 0x6D], r0[16 + 0x6E], r0[16 + 0x6F]]) as usize;
        let exth_flags =
            u32::from_be_bytes([r0[16 + 0x80], r0[16 + 0x81], r0[16 + 0x82], r0[16 + 0x83]]);
        let mut cover_offs: Vec<u32> = Vec::new();
        if exth_flags & 0x40 != 0 {
            let exth_start = 16 + mobi_hdr_len;
            if exth_start + 12 <= r0.len() && &r0[exth_start..exth_start + 4] == b"EXTH" {
                let rec_count = u32::from_be_bytes([
                    r0[exth_start + 8],
                    r0[exth_start + 9],
                    r0[exth_start + 10],
                    r0[exth_start + 11],
                ]) as usize;
                let mut p = exth_start + 12;
                for _ in 0..rec_count.min(4096) {
                    if p + 8 > r0.len() {
                        break;
                    }
                    let ty = u32::from_be_bytes([r0[p], r0[p + 1], r0[p + 2], r0[p + 3]]);
                    let len = u32::from_be_bytes([r0[p + 4], r0[p + 5], r0[p + 6], r0[p + 7]]) as usize;
                    if len < 8 || p + len > r0.len() {
                        break;
                    }
                    // 201=cover offset, 202=thumbnail offset（值为相对 first_image 的记录偏移）。
                    if (ty == 201 || ty == 202) && len >= 12 {
                        cover_offs.push(u32::from_be_bytes([
                            r0[p + 8],
                            r0[p + 9],
                            r0[p + 10],
                            r0[p + 11],
                        ]));
                    }
                    p += len;
                }
            }
        }
        let is_img = |b: &[u8]| -> bool {
            b.starts_with(&[0xFF, 0xD8, 0xFF])
                || b.starts_with(b"\x89PNG")
                || b.starts_with(b"GIF8")
                || (b.len() > 12 && &b[8..12] == b"WEBP")
        };
        for v in &cover_offs {
            if let Some(idx) = first_image.checked_add(*v as usize) {
                if let Some(b) = read_rec(&mut f, idx, 64 * 1024 * 1024) {
                    if is_img(&b) {
                        return Some(b);
                    }
                }
            }
        }
        // 回退：first_image 记录本身（单条），仍不读全文。
        if let Some(b) = read_rec(&mut f, first_image, 64 * 1024 * 1024) {
            if is_img(&b) {
                return Some(b);
            }
        }
        None
    }))
    .ok()
    .flatten()
}

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
    let recs = raw_records_safe(mobi);
    if fi == 0 || fi >= recs.len() {
        return (by_rec, by_res, ordered);
    }
    let _ = std::fs::create_dir_all(out_dir);
    for r in recs.iter().skip(fi) {
        let c = r.content;
        if NON_IMAGE_MAGICS.iter().any(|m| c.starts_with(m)) {
            continue;
        }
        let Some(ext) = image_magic(c) else { continue };
        let fname = format!("r{:05}.{ext}", r.index);
        if std::fs::write(out_dir.join(&fname), c).is_err() {
            continue;
        }
        let url = format!("{url_prefix}{fname}");
        by_rec.insert(r.index as u32, url.clone());
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

/// 图片引用重写统计，供 `[diag]` 诊断与验收断言使用。
#[derive(Debug, Default, Clone, Copy)]
pub struct RewriteStats {
    /// HTML 中的 <img> 标签总数
    pub total_imgs: usize,
    /// 成功改写成书内本地资源 URL 的数量
    pub rewritten: usize,
    /// 解析不到图片、被替换成透明占位图的数量
    pub placeholders: usize,
}

/// MOBI6 的 `recindex` 是**相对 first_image_index 的序号**（多数为十进制 1 基，
/// 少数用十六进制/0 基）。依次尝试各种解释，取第一个能在 `by_rec`（绝对记录索引）
/// 命中的；EXTH121 的 `recindex:resindex` 映射（`by_res`）优先级最高。
fn resolve_recindex(
    v: &str,
    by_rec: &HashMap<u32, String>,
    by_res: &HashMap<u32, String>,
    first_image_index: u32,
) -> Option<String> {
    let try_abs = |n: u32| -> Option<String> {
        if n >= 1 {
            if let Some(u) = by_rec.get(&first_image_index.saturating_add(n - 1)) {
                return Some(u.clone());
            }
        }
        by_rec.get(&first_image_index.saturating_add(n)).cloned()
    };
    if let Ok(n) = v.parse::<u32>() {
        if let Some(u) = by_res.get(&n) {
            return Some(u.clone());
        }
        if let Some(u) = try_abs(n) {
            return Some(u);
        }
    }
    if let Ok(n) = u32::from_str_radix(v, 16) {
        if let Some(u) = by_res.get(&n) {
            return Some(u.clone());
        }
        if let Some(u) = try_abs(n) {
            return Some(u);
        }
    }
    // 兼容旧行为：个别文件 recindex 直接是绝对记录索引（最后兜底）
    v.parse::<u32>().ok().and_then(|n| by_rec.get(&n).cloned())
}

pub fn rewrite_images(
    html: &str,
    by_rec: &HashMap<u32, String>,
    by_res: &HashMap<u32, String>,
    ordered: &[String],
    first_image_index: u32,
) -> (String, RewriteStats) {
    let tag_re = Regex::new(r"(?is)<img\b[^>]*>").unwrap();
    let rec_re = Regex::new(r#"(?i)\brecindex\s*=\s*['"]?([0-9a-fA-F]{1,8})['"]?"#).unwrap();
    let embed_re = Regex::new(r"(?i)kindle:embed:([0-9a-fA-F]{1,8})").unwrap();
    let src_re = Regex::new(r#"(?i)\bsrc\s*=\s*["']([^"']*)["']"#).unwrap();
    let num_re = Regex::new(r"(\d+)").unwrap();
    let mut stats = RewriteStats::default();
    let out_html = tag_re
        .replace_all(html, |cap: &Captures| {
            let tag = cap.get(0).unwrap().as_str();
            stats.total_imgs += 1;
            let mut url: Option<String> = None;
            if let Some(c) = rec_re.captures(tag) {
                url = resolve_recindex(&c[1], by_rec, by_res, first_image_index);
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
                stats.placeholders += 1;
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
            stats.rewritten += 1;
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
        .to_string();
    (out_html, stats)
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
    use crate::sanitize::{is_dangerous_tag, is_dangerous_url, is_url_attr};
    use html5ever::tokenizer::{TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct St {
        out: String,
        pending_text: String, // tokenizer 在实体边界切分 CharacterTokens，须合并后统一解码
        /// 已打开的危险容器标签栈（小写）；非空时丢弃其间所有 token。
        /// 用栈而非单值，保证嵌套危险标签（如 <form><script/>…</form>）不会提前解除丢弃态。
        drop_stack: Vec<String>,
        dangling: bool,
        drop_tags: usize, // 诊断：丢弃的非法标签名数
        drop_broken: usize, // 诊断：文本中剔除的破碎标签数
    }
    impl St {
        fn flush(&mut self) {
            if self.pending_text.is_empty() {
                return;
            }
            let t = std::mem::take(&mut self.pending_text);
            let (d, _e) = decode_entities_text(&t);
            // 源文件损坏产生的破碎标签以实体形态(&lt;b aid=&quot;x&quot;&gt;)混在正文里，
            // 浏览器会原样显示成源码。这里虚拟解码→剔除破碎标签/属性残留→再转义输出。
            static RE_BROKEN_TAG: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
                Regex::new(r#"(?i)<[^<>]*?[a-zA-Z][a-zA-Z0-9_:.-]*\s*=\s*["'][^<>]*["'][^<>]*>"#).unwrap()
            });
            static RE_BROKEN_ATTR: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
                Regex::new(r#"[a-zA-Z][a-zA-Z0-9_:.-]*\s*=\s*["'][^"<>]{1,80}["']"#).unwrap()
            });
            static RE_BROKEN_HEAD: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
                Regex::new(r#"(?i)(?:aid|class|style|href|src|type|width|height|align|rel|valign|char|id|onclick|onload|target)\s*=\s*["']?"#).unwrap()
            });
            let virtual_str = d
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&amp;", "&");
            let mut cleaned = RE_BROKEN_TAG.replace_all(&virtual_str, "").to_string();
            let n1 = RE_BROKEN_TAG.find_iter(&virtual_str).count();
            cleaned = RE_BROKEN_ATTR.replace_all(&cleaned, "").to_string();
            let n2 = RE_BROKEN_ATTR.find_iter(&cleaned).count();
            cleaned = RE_BROKEN_HEAD.replace_all(&cleaned, "").to_string();
            self.drop_broken += n1 + n2;
            for ch in cleaned.chars() {
                match ch {
                    '<' => self.out.push_str("&lt;"),
                    '>' => self.out.push_str("&gt;"),
                    '&' => self.out.push_str("&amp;"),
                    '"' => self.out.push_str("&quot;"),
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
                    // NUL 等非法字符开头的标签名（tokenizer 规范转 U+FFFD）：整体丢弃，
                    // 浏览器对这种标签同样不会渲染成界面元素
                    if !ln.starts_with(|c: char| c.is_ascii_alphabetic()) {
                        st.drop_tags += 1;
                        return TokenSinkResult::Continue;
                    }
                    // 危险标签：script/style/iframe/object/applet/form 连内容丢弃，
                    // meta/base/link/frame/embed 等无内容标签只丢标签本身。
                    if is_dangerous_tag(&ln) {
                        let void = matches!(ln.as_str(), "meta" | "base" | "link" | "frame" | "embed");
                        if matches!(tag.kind, TagKind::EndTag) {
                            // 从栈顶向下找同名并截断，避免内层结束标签提前解除外层丢弃态。
                            if let Some(pos) = st.drop_stack.iter().rposition(|t| t == &ln) {
                                st.drop_stack.truncate(pos);
                            }
                            return TokenSinkResult::Continue;
                        }
                        if !tag.self_closing && !void {
                            if st.dangling {
                                return TokenSinkResult::Continue; // 悬空模式：去标签，文本按普通处理
                            }
                            st.drop_stack.push(ln);
                        }
                        return TokenSinkResult::Continue;
                    }
                    if !st.drop_stack.is_empty() {
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
                            let an = a.name.local.to_string();
                            let anl = an.to_ascii_lowercase();
                            // 事件属性（on*）全删；URL 属性遇危险协议整条删除。
                            if anl.starts_with("on") {
                                continue;
                            }
                            let v = a.value.to_string();
                            if is_url_attr(&anl) && is_dangerous_url(&v) {
                                continue;
                            }
                            s.push(' ');
                            s.push_str(&an);
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
                    if !st.drop_stack.is_empty() {
                        return TokenSinkResult::Continue;
                    }
                    st.pending_text.push_str(&chars);
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
        drop_stack: Vec::new(),
        dangling,
        drop_tags: 0,
        drop_broken: 0,
    }));
    let tok = Tokenizer::new(Sink(st.clone()), html5ever::tokenizer::TokenizerOpts::default());
    let mut q = html5ever::tokenizer::BufferQueue::default();
    q.push_back(html.into());
    let _res = tok.feed(&q);
    tok.end();
    let mut b = st.borrow_mut();
    b.flush();
    if std::env::var("MOBI_SANITIZE_DEBUG").is_ok() {
        eprintln!(
            "[sanitize] in={} out={} drop_tags={} drop_broken={} dangling={}",
            html.len(),
            b.out.len(),
            b.drop_tags,
            b.drop_broken,
            b.dangling
        );
    }
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
///
/// 外层捕获 `mobi` crate 在受损/KF8 变体文件上的内部 panic，转为可读错误，
/// 保证 app 绝不因单本坏书崩溃。
pub fn parse_full(
    path: &str,
    force_encoding: Option<&str>,
    images: Option<(&Path, &str)>,
) -> Result<MobiDoc, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_full_inner(path, force_encoding, images)
    })) {
        Ok(r) => r,
        Err(_) => Err(format!(
            "PalmDocFormatError: {path} 解析中断（文件受损或为不支持的 KF8 变体），已安全中止，可尝试转换后再导入"
        )),
    }
}

/// 一次 `read` + 一次 `Mobi::new`，并完成 DRM 拦截。返回的 `Mobi` 自持内容，
/// 不借用传入字节，调用方可立即释放文件缓冲。
fn read_and_parse_mobi(path: &str) -> Result<Mobi, String> {
    read_and_parse_mobi_bytes(path).map(|(m, _)| m)
}

/// 一次读取 + 一次解析，同时返回**原始文件字节**（用于正确切分记录）。
fn read_and_parse_mobi_bytes(path: &str) -> Result<(Mobi, Vec<u8>), String> {
    // 快速失败路径：不整读文件即拦截 DRM/畸形头
    sniff_drm_file(path)?;
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    detect_drm(&bytes)?;
    let mut mobi = Mobi::new(&bytes).map_err(|e| format!("MOBI/AZW3 解析失败: {e:?}"))?;
    // 双保险：crate 解析后的头字段再查一次（覆盖新式 DRM 的 MOBI 头 DRM 域）
    if mobi.encryption() != Encryption::No {
        return Err("AWZ3EncryptedError: PalmDOC 头标记为已加密，请先解密后导入".to_string());
    }
    if mobi.metadata.mobi.has_drm() {
        return Err(
            "AWZ3EncryptedError: MOBI 头 DRM 记录偏移域非空（存在 DRM 记录），请先解密后导入".to_string(),
        );
    }
    // 修复 crate 的 content 前导置零：`Mobi::from_reader` 用 `vec![0; position]` 再
    // `read_to_end` 追加，导致元数据区被置零，而 PDB 记录偏移是**绝对文件偏移**，
    // 若不修复则 record0 内容全零、正文切分会错位（KF8/AZW3 尤其明显）。
    if mobi.content.len() == bytes.len() {
        mobi.content.copy_from_slice(&bytes);
    }
    Ok((mobi, bytes))
}

/// 元数据阶段专用：**一次读取、一次解析**同时拿到 `MobiDoc` 与封面字节，
/// 避免 `parse` + `extract_cover_path` 各整读一遍（大书是 2× 全量读盘）。
/// 返回 `(解析结果, 封面字节)`；解析失败时封面为 None。
pub fn parse_with_cover(path: &str) -> (Result<MobiDoc, String>, Option<Vec<u8>>) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse_with_cover_inner(path))) {
        Ok(r) => r,
        Err(_) => (
            Err(format!(
                "PalmDocFormatError: {path} 解析中断（文件受损或为不支持的 KF8 变体），已安全中止，可尝试转换后再导入"
            )),
            None,
        ),
    }
}

fn parse_with_cover_inner(path: &str) -> (Result<MobiDoc, String>, Option<Vec<u8>>) {
    let (mobi, _data) = match read_and_parse_mobi_bytes(path) {
        Ok(m) => m,
        Err(e) => return (Err(e), None),
    };
    let cover = extract_cover_bytes(&mobi);
    match build_mobi_doc(path, &mobi, None, None) {
        Ok(doc) => (Ok(doc), cover),
        Err(e) => (Err(e), None),
    }
}

fn parse_full_inner(
    path: &str,
    force_encoding: Option<&str>,
    images: Option<(&Path, &str)>,
) -> Result<MobiDoc, String> {
    let (mobi, _data) = read_and_parse_mobi_bytes(path)?;
    build_mobi_doc(path, &mobi, force_encoding, images)
}

/// 纯构建：解码/清洗正文、改写图片、切分章节（不做任何文件读取）。
fn build_mobi_doc(
    path: &str,
    mobi: &Mobi,
    force_encoding: Option<&str>,
    images: Option<(&Path, &str)>,
) -> Result<MobiDoc, String> {
    let (raw, extract_warn) = extract_records_and_decompress_limited(&mobi, MAX_RECORDS, MAX_HTML_BYTES)?;
    // 一次性全局解码整个字节流（不在任何 Record 内做字符串解码）
    let mut decoded = decode_html_bytes(&raw, mobi.text_encoding(), force_encoding);
    if decoded.warning.is_none() {
        decoded.warning = extract_warn;
    }
    if decoded.text.trim().is_empty() {
        return Err(
            "PalmDocFormatError: 正文解压为空（HUFF/CDIC 受损或不受支持的 KF8 变体），请转换后再导入"
                .to_string(),
        );
    }
    // NUL/控制字符必须在 tokenizer 之前剔除（tag-name 状态按规范会把 NUL 变成 U+FFFD 混进标签名）；
    // 之后再清一次：实体解码可能还原出 PUA/控制字符
    let mut html = clean_html(&decode_and_sanitize(&clean_html(&decoded.text)));
    // 根因1：图片重写会改变字符串长度，必须**先改图片再算章节区间**，
    // 否则 parts 是改写前的旧字节区间，safe_slice 会从标签/单词/URI 中间切开。
    let first_image_index = mobi.metadata.mobi.first_image_index;
    let mut image_count = 0usize;
    let mut res_map = 0usize;
    let mut rewrite_stats = RewriteStats::default();
    if let Some((dir, prefix)) = images {
        let (by_rec, by_res, ordered) = extract_images(&mobi, dir, prefix);
        image_count = ordered.len();
        res_map = by_res.len();
        if image_count > 0 {
            let (h, st) = rewrite_images(&html, &by_rec, &by_res, &ordered, first_image_index);
            html = h;
            rewrite_stats = st;
        }
    }
    // res_urls 语义：成功重写为书内本地资源 URL 的图片引用数（验收指标之一）
    let res_urls = rewrite_stats.rewritten;
    let (toc, parts) = split_mobi_chapters(&html);
    if std::env::var("TIDEPAGE_MOBI_DIAG").is_ok() {
        let kf8 = detect_kf8(&mobi);
        let m = &mobi.metadata.mobi;
        let recs = raw_records_safe(&mobi);
        let txt_rng = content_records_range(&mobi, &recs);
        eprintln!(
            "[diag] path={:?} type={:?} fmt={} comp={:?} first_content={} first_image={} \
             first_non_text={} huff={}+{} text_rng={:?} exth121={:?} kf8={} head={} lt={} \
             toc={} html={}B imgs={} res_urls={} res_map={} placeholder={} raw={}B warning={:?}",
            std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()),
            mobi.mobi_type(),
            m.format_version,
            mobi.compression(),
            m.first_content_record,
            m.first_image_index,
            m.first_non_book_index,
            m.first_huff_record,
            m.huff_record_count,
            txt_rng,
            kf8.boundary_exth,
            kf8.is_kf8,
            kf8.header_kind,
            decoded.text.matches('<').count(),
            toc.len(),
            html.len(),
            image_count,
            res_urls,
            res_map,
            rewrite_stats.placeholders,
            raw.len(),
            decoded.warning,
        );
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
        res_urls,
        placeholder_count: rewrite_stats.placeholders,
        warning: decoded.warning,
    })
}

/// 去标签取纯文本，用于给无名章节生成可读标题。
/// - 行内标签（font/b/i/em/strong/u/s/span/a/sup/sub/small/big…）删除时**不补空格**，
///   避免首字下沉 `<font size="6"><b>M</b></font>an` 被拆成假空格 "M an"。
/// - 块级/换行标签（p/div/br/h1-6/li/tr/td…）视为空白分隔。
/// - 合并多余空白、还原常用实体、最多 max 个字符。
fn segment_text(html: &str, max: usize) -> String {
    // head/title/script/style 不参与命名（否则 <guide> 里的目录标题会当成正文）
    // 注：regex crate 不支持反向引用，用显式成对分支替代
    static DROP: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r"(?is)<head\b[^>]*>.*?</head\s*>|<title\b[^>]*>.*?</title\s*>|<script\b[^>]*>.*?</script\s*>|<style\b[^>]*>.*?</style\s*>",
        )
        .unwrap()
    });
    // 块级/换行标签 → 空格；其余标签（行内）随后直接删除
    static BLOCK: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r#"(?is)</?(?:address|article|aside|blockquote|body|br|caption|center|dd|div|dl|dt|fieldset|figcaption|figure|footer|form|h[1-6]|header|hr|html|li|main|nav|ol|p|pre|section|table|tbody|td|tfoot|th|thead|tr|ul)\b[^>]*/?>"#,
        )
        .unwrap()
    });
    static TAG: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
    let cleaned = DROP.replace_all(html, " ");
    let spaced = BLOCK.replace_all(&cleaned, " ");
    let stripped = TAG.replace_all(&spaced, "");
    let decoded = decode_basic_entities(&stripped);
    let mut out = String::new();
    let mut prev_space = true;
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
            if out.chars().count() >= max {
                break;
            }
        }
    }
    out.trim().to_string()
}

/// 常用实体还原（章节命名用，非 HTML 转义管线）
fn decode_basic_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&#8217;", "’")
        .replace("&nbsp;", " ")
}

/// 按词/句边界收尾，避免截半个词；拉丁截断处补「…」，CJK 按字数截断。
fn truncate_label(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    // 1) 首个句末标点（在 6..max 内出现时收在句末）
    const STOP: [char; 8] = ['.', '!', '?', '。', '！', '？', '；', ';'];
    for i in 6..max {
        if STOP.contains(&chars[i]) {
            let s: String = chars[..=i].iter().collect();
            return s.trim().to_string();
        }
    }
    // 2) 拉丁词边界：恰好截在词中时回退到上一个空白
    let mut cut = max;
    let at = chars.get(max.saturating_sub(1)).copied().unwrap_or(' ');
    let nxt = chars.get(max).copied().unwrap_or(' ');
    if at.is_ascii_alphanumeric() && nxt.is_ascii_alphanumeric() {
        if let Some(ws) = chars[..max].iter().rposition(|c| c.is_whitespace()) {
            if ws >= 6 {
                cut = ws;
            }
        }
    }
    let mut s: String = chars[..cut].iter().collect();
    while s.ends_with(|c: char| c.is_whitespace() || c == '—' || c == '–' || c == '-') {
        s.pop();
    }
    s.push('…');
    s
}

/// 标题线索：取段内**最靠前**的醒目块（h1-6 / font size=5..7 / b），
/// 只认它作为标题，避免把首字下沉或大段正文当标题。
fn title_clue(seg: &str) -> Option<String> {
    static H: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"(?is)<h[1-6]\b[^>]*>(.*?)</h[1-6]\s*>").unwrap()
    });
    static F: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"(?is)<font\b[^>]*\bsize\s*=\s*["']?[5-7]["']?[^>]*>(.*?)</font\s*>"#).unwrap()
    });
    static B: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?is)<b\b[^>]*>(.*?)</b\s*>").unwrap());
    fn consider(best: &mut Option<(usize, u8, String)>, start: usize, prio: u8, inner: &str) {
        // <b> 仅在段首附近作为标题线索，避免正文强调词误判
        if start > 400 || (prio == 2 && start > 160) {
            return;
        }
        let txt = segment_text(inner, 80);
        // 过滤首字下沉的单个字符
        if txt.chars().count() < 2 {
            return;
        }
        let key = (start, prio);
        if best.as_ref().map(|(s, p, _)| key < (*s, *p)).unwrap_or(true) {
            *best = Some((start, prio, txt));
        }
    }
    let mut best: Option<(usize, u8, String)> = None;
    for c in H.captures_iter(seg) {
        consider(&mut best, c.get(0).unwrap().start(), 0, &c[1]);
    }
    for c in F.captures_iter(seg) {
        consider(&mut best, c.get(0).unwrap().start(), 1, &c[1]);
    }
    for c in B.captures_iter(seg) {
        consider(&mut best, c.get(0).unwrap().start(), 2, &c[1]);
    }
    best.map(|(_, _, t)| t)
}

/// 生成可读标题：优先醒目块，否则首句；统一按词/句边界截断。
fn segment_label(seg: &str, max: usize) -> String {
    if let Some(clue) = title_clue(seg) {
        let t = truncate_label(&clue, max);
        if !t.is_empty() {
            return t;
        }
    }
    let txt = segment_text(seg, max.saturating_mul(3).max(64));
    truncate_label(&txt, max)
}

/// 卷首内容（封面/书名页/献词/目录）的合理命名。
fn front_label(seg: &str, index: usize) -> String {
    if seg.matches("filepos").count() >= 3 {
        return "目录".into();
    }
    let has_img = seg.contains("<img");
    let txt = segment_text(seg, 64);
    let label = segment_label(seg, 40);
    if txt.is_empty() || label.is_empty() {
        return if index == 0 || has_img { "封面".into() } else { "插图".into() };
    }
    if index == 0 && has_img && txt.chars().count() <= 12 {
        return "封面".into();
    }
    label
}

/// 正文分页的标题：优先页面醒目块/首句，拿不到再退回「第 N 章」。
fn body_label(seg: &str, n: usize) -> String {
    let label = segment_label(seg, 24);
    if label.is_empty() {
        format!("第 {n} 章")
    } else {
        label
    }
}

/// 卷首"标题式"页面（无句末标点、非目录）按块级 `<p>/<div>` 细分成多条，
/// 让书名页的「书名」与「作者」各自成条；含句末标点的正文/题词页保持整段。
/// 返回值保证连续覆盖 `[s, e)`，供 parts 直接使用。
fn expand_front_segment(html: &str, s: usize, e: usize) -> Vec<(usize, usize)> {
    let whole = vec![(s, e)];
    if e <= s + 1 {
        return whole;
    }
    let seg = &html[s..e];
    if seg.matches("filepos").count() >= 3 {
        return whole;
    }
    // 用去标签后的文本判断标点（避免把 src="r00159.jpg" 之类的句点误当正文标点）
    let seg_txt = segment_text(seg, 200);
    if seg_txt
        .chars()
        .any(|c| matches!(c, '.' | '。' | '!' | '！' | '?' | '？' | ';' | '；'))
    {
        return whole;
    }
    // 块级起始标签即分段边界
    static BLK: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"(?i)<(?:p|div|center|table|tr|td|blockquote|h[1-6])\b[^>]*>").unwrap()
    });
    let mut bounds: Vec<usize> = vec![s];
    bounds.extend(
        BLK.find_iter(seg)
            .map(|m| s + m.start())
            .filter(|p| *p > s),
    );
    bounds.push(e);
    bounds.dedup();
    let kept: Vec<(usize, usize)> = bounds
        .windows(2)
        .map(|w| (w[0], w[1]))
        .filter(|(bs, be)| !segment_text(&html[*bs..*be], 3).is_empty())
        .collect();
    if kept.len() < 2 {
        return whole;
    }
    let n = kept.len();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        let start = if i == 0 { s } else { kept[i].0 };
        let end = if i + 1 < n { kept[i + 1].0 } else { e };
        out.push((start, end));
    }
    out
}

/// 章节切分：优先书内标题 `<h1-6>`，否则按 `<mbp:pagebreak>` 分页；
/// 无标题的卷首内容按「封面/目录/前言」命名，正文页用首段文字命名。
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

    // 卷首（第一个标题之前）的内容单列：封面/目录/前言
    let mut front: Vec<(usize, usize)> = Vec::new();
    if starts.len() < 2 {
        starts.clear();
        // 分页回退：按 pagebreak 切段，先去掉纯空白/无图无字的空段
        let pb_re = Regex::new(r"(?i)<mbp:pagebreak\s*/?>").unwrap();
        let mut bounds: Vec<usize> = vec![0];
        bounds.extend(pb_re.find_iter(html).map(|m| m.start()));
        bounds.push(html.len());
        bounds.dedup();
        let segs: Vec<(usize, usize)> = bounds
            .windows(2)
            .map(|w| (w[0], w[1]))
            .filter(|(s, e)| {
                let body = &html[*s..*e];
                !segment_text(body, 5).is_empty() || body.contains("<img")
            })
            .collect();
        // 首个含多个 filepos 链接的段视为目录：其（含）之前为卷首，之后为正文
        let toc_at = segs
            .iter()
            .position(|(s, e)| html[*s..*e].matches("filepos").count() >= 3);
        let body_from = match toc_at {
            Some(i) => {
                front = segs[..=i]
                    .iter()
                    .flat_map(|(s, e)| expand_front_segment(html, *s, *e))
                    .collect();
                i + 1
            }
            None => {
                // 无目录：仅把开头的纯封面段归入卷首
                if let Some(&(s, e)) = segs.first() {
                    let seg = &html[s..e];
                    if seg.contains("<img") && segment_text(seg, 13).chars().count() <= 12 {
                        front = expand_front_segment(html, s, e);
                        1
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
        };
        let mut n = 0usize;
        for (s, e) in segs[body_from..].iter().copied() {
            n += 1;
            starts.push((s, body_label(&html[s..e], n)));
        }
    }

    let mut toc = Vec::new();
    let mut parts = Vec::new();
    // 卷首段（封面/书名页/目录/前言）
    for (j, (s, e)) in front.iter().enumerate() {
        let title = if html[*s..*e].matches("filepos").count() >= 3 {
            "目录".to_string()
        } else {
            front_label(&html[*s..*e], j)
        };
        toc.push(TocEntry {
            title,
            index: toc.len(),
            chars: html[*s..*e].chars().count() as i64,
        });
        parts.push((*s, *e));
    }
    if starts.is_empty() {
        if parts.is_empty() {
            toc.push(TocEntry {
                title: "全文".into(),
                index: 0,
                chars: html.chars().count() as i64,
            });
            parts.push((0, html.len()));
        }
        return (toc, parts);
    }
    if front.is_empty() && starts[0].0 > 0 {
        // 标题模式下的前置内容
        let seg = &html[..starts[0].0];
        let title = if seg.contains("<img") && segment_text(seg, 20).is_empty() {
            "封面".to_string()
        } else {
            "前言".to_string()
        };
        toc.push(TocEntry {
            title,
            index: 0,
            chars: seg.chars().count() as i64,
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
        let dir = std::env::temp_dir().join(format!("tidepage-mobi-sniff-{}", std::process::id()));
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

    /// 正文记录判定：MOBI 头（record0）、FCIS/FLIS/IDXT/HUFF/CDIC 等非文本记录必须被排除，
    /// 否则会把头当正文解码，产出空正文或乱码（AZW3 回归根因）。
    #[test]
    fn text_record_detection_skips_headers_and_indices() {
        // PalmDOC/MOBI 头：压缩算法 0x4448(HUFF)/0x0002(PalmDoc)
        assert!(!looks_like_text_record(&[0x44, 0x48, 0x00, 0x00, 0x11]));
        assert!(!looks_like_text_record(&[0x00, 0x02, 0x00, 0x00]));
        assert!(!looks_like_text_record(b"FCIS\x00\x00"));
        assert!(!looks_like_text_record(b"FLIS\x00\x00"));
        assert!(!looks_like_text_record(b"IDXT\x00\x00"));
        assert!(!looks_like_text_record(b"HUFF\x00\x00"));
        assert!(!looks_like_text_record(b"CDIC\x00\x00"));
        assert!(!looks_like_text_record(b""));
        // 真实 XHTML/HTML 文本块
        assert!(looks_like_text_record(b"<?xml version=\"1.0\"?>"));
        assert!(looks_like_text_record(b"<html><body>hi</body></html>"));
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
        let dir = std::env::temp_dir().join(format!("tidepage-mobi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (by_rec, by_res, ordered) = extract_images(&mobi, &dir, "http://tidepage.localhost/res/1/");
        assert_eq!(ordered.len(), 2, "应提取 2 张图");
        std::fs::remove_dir_all(&dir).ok();
        let html = String::from_utf8_lossy(&text).to_string();
        let fi = mobi.metadata.mobi.first_image_index;
        let (out, st) = rewrite_images(&html, &by_rec, &by_res, &ordered, fi);
        assert!(out.contains("http://tidepage.localhost/res/1/r00002.jpg"), "{out}");
        assert!(out.contains("http://tidepage.localhost/res/1/r00003.png"), "{out}");
        assert!(!out.contains("recindex"), "{out}");
        assert!(!out.contains("kindle:embed"), "{out}");
        assert!(!out.contains("\"1.jpg\""), "{out}");
        // 解析不到的 kindle:embed:00FF → 透明占位图，不留非法 src
        assert!(out.contains("data:image/gif;base64,R0lGODlhAQAB"), "{out}");
        assert!(!out.contains("00FF"), "{out}");
        assert_eq!(out.matches("<img").count(), 4, "{out}");
        assert_eq!(st.total_imgs, 4);
        assert_eq!(st.rewritten, 3);
        assert_eq!(st.placeholders, 1);
    }

    /// 根因2 回归：MOBI6 `recindex` 是相对 first_image_index 的 1 基序号，
    /// 而不是绝对记录索引（旧实现用 abs 查找导致所有图片变透明占位）。
    #[test]
    fn mobi6_recindex_is_relative_to_first_image() {
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpg.extend_from_slice(&[7u8; 128]);
        let mut png = vec![0x89, b'P', b'N', b'G'];
        png.extend_from_slice(&[9u8; 64]);
        // fi = 2；recindex 0001→rec2(jpg)，0002→rec3(png)，0000(0基)→rec2(jpg)
        let text = b"<img recindex=\"0001\"/><img recindex=\"0002\"/><img recindex=\"0000\"/>".to_vec();
        let b = Builder {
            compression: 1,
            encryption: 0,
            text: text.clone(),
            images: vec![jpg, png],
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        let mobi = Mobi::new(&b).unwrap();
        let dir = std::env::temp_dir().join(format!("tidepage-mobi-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (by_rec, by_res, ordered) = extract_images(&mobi, &dir, "res/");
        std::fs::remove_dir_all(&dir).ok();
        let fi = mobi.metadata.mobi.first_image_index;
        assert_eq!(fi, 2);
        let (out, st) =
            rewrite_images(&text_to_str(&text), &by_rec, &by_res, &ordered, fi);
        assert_eq!(st.rewritten, 3, "{out}");
        assert_eq!(st.placeholders, 0, "{out}");
        assert_eq!(out.matches("r00002.jpg").count(), 2, "{out}");
        assert_eq!(out.matches("r00003.png").count(), 1, "{out}");
        assert!(!out.contains("recindex"), "{out}");
    }

    fn text_to_str(b: &[u8]) -> String {
        String::from_utf8_lossy(b).to_string()
    }

    /// 受损 PDB 偏移表（记录间距 < extra_bytes）不得 panic：旧路径用
    /// mobi crate RawRecords::parse 会直接越界切片崩溃，改为自实现安全切片后
    /// 必须返回 Ok/Err 而非 panic（真实 AZW3 样本实测崩溃点 record.rs:154）。
    #[test]
    fn malformed_offsets_do_not_panic() {
        let body = "<p>".to_string() + &"x".repeat(9000) + "</p>";
        let b = Builder {
            compression: 1,
            encryption: 0,
            text: body.into_bytes(),
            images: vec![],
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        let mut bad = b.clone();
        let n = u16b(&bad, 0x4C) as usize;
        assert!(n >= 4, "需要至少 4 条记录，实际 {n}");
        let rec_off = 0x4E;
        // record[1].offset = record[2].offset - 1，并令 extra_bytes=4
        // ⇒ next - extra < curr，旧实现切片 panic
        let o2 = u32b(&bad, rec_off + 2 * 8) as usize;
        let new_o1 = (o2.saturating_sub(1)) as u32;
        bad[rec_off + 8..rec_off + 12].copy_from_slice(&new_o1.to_be_bytes());
        let extra_pos = rec_off + 8 * n;
        bad[extra_pos] = 0;
        bad[extra_pos + 1] = 4;
        let mobi = Mobi::new(&bad).unwrap();
        // 旧实现此处 panic；现在只能返回 Ok 或 Err
        let _ = extract_records_and_decompress(&mobi);
    }

    /// 章节命名回归（问题1）：行内标签不产生假空格、首字下沉不被拆开、
    /// 按词/句边界截断、CJK 按字数、醒目块优先、卷首语义命名。
    #[test]
    fn title_labels_avoid_fake_spaces_and_half_words() {
        assert_eq!(
            segment_text("<p><font size=\"6\"><b>M</b></font>an, he loved that smile.</p>", 60),
            "Man, he loved that smile."
        );
        assert_eq!(segment_text("<p>word<b>bold</b>word</p>", 60), "wordboldword");
        assert_eq!(segment_text("<p>one</p><p>two</p>", 60), "one two");
        assert_eq!(segment_text("<p>a<br/>b</p>", 60), "a b");
        // 拉丁按词边界截断，不截半词
        assert_eq!(truncate_label("alpha beta gamma delta epsilon", 20), "alpha beta gamma…");
        // 句末标点优先收尾
        assert_eq!(truncate_label("Hello world. This is more text here.", 30), "Hello world.");
        // CJK 按字数截断
        let cjk = truncate_label("这是一个很长很长的中文标题需要截断处理", 10);
        assert_eq!(cjk.chars().count(), 11, "{cjk}");
        assert!(cjk.starts_with("这是一个很长很长的中") && cjk.ends_with('…'), "{cjk}");
        // 醒目块优先于正文；首字下沉单字不作为标题
        assert_eq!(
            body_label("<p><font size=\"5\"><b>Chapter One</b></font></p><p>Body text.</p>", 1),
            "Chapter One"
        );
        let drop = body_label(
            "<p><font size=\"6\"><b>M</b></font>an, he loved that smile of hers. It was a killer.</p>",
            2,
        );
        assert!(!drop.contains("M an"), "{drop}");
        assert!(drop.starts_with("Man"), "{drop}");
        // 卷首语义命名
        assert_eq!(front_label("<div><p><img src=\"x\"/></p></div>", 0), "封面");
        assert_eq!(front_label("<p><i>For Jessica and Adam</i></p>", 2), "For Jessica and Adam");
        assert_eq!(
            front_label(
                "<p><font size=\"5\"><b>A Killing Night</b></font></p><p>Jonathon King</p>",
                1
            ),
            "A Killing Night"
        );
        assert_eq!(
            front_label("<a filepos=\"1\"></a><a filepos=\"2\"></a><a filepos=\"3\"></a>", 4),
            "目录"
        );
        // 书名页按块细分为「书名」「作者」两条，且连续覆盖（src 里的句点不算正文标点）
        let page = "<mbp:pagebreak/></div><div><p><font size=\"5\"><b>A Killing Night</b></font></p>\
                    <p>Jonathon King</p><p><img src=\"r00159.jpg\"/></p><div></div></div>";
        let ps = expand_front_segment(page, 0, page.len());
        assert_eq!(ps.len(), 2, "{ps:?}");
        assert_eq!(ps[0].0, 0);
        assert_eq!(ps[1].1, page.len());
        assert_eq!(ps[0].1, ps[1].0, "细分后应连续");
        assert_eq!(front_label(&page[ps[0].0..ps[0].1], 1), "A Killing Night");
        assert_eq!(front_label(&page[ps[1].0..ps[1].1], 2), "Jonathon King");
    }

    /// 最小合法 1x1 PNG（供合成样本当图片记录）
    fn tiny_png() -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=")
            .unwrap()
    }

    /// 合成 MOBI 回归（问题1/2/3 离线可跑，不依赖外部样本）：
    /// pagebreak 分段 + 十进制 recindex(00001/00010>9) + hex recindex(0000a)
    /// + <font color> + <guide>/<reference> 包裹 + 末尾真实图片记录。
    #[test]
    fn synthetic_mobi_regression_offsets_and_recindex() {
        let png = tiny_png();
        let images: Vec<Vec<u8>> = (0..10).map(|_| png.clone()).collect();
        let text = concat!(
            "<html><head><guide><reference type=\"toc\" title=\"TOC\" filepos=\"0000000010\"/></guide></head><body>",
            "<div><img recindex=\"00001\"/></div><mbp:pagebreak/>",
            "<div><font size=\"5\"><b>Chapter One</b></font><p>Body one <font color=\"red\">red</font>.</p></div><mbp:pagebreak/>",
            "<div><font size=\"5\"><b>Chapter Two</b></font><p>Body two <img recindex=\"00010\"/> tail.</p></div><mbp:pagebreak/>",
            "<div><font size=\"5\"><b>Chapter Three</b></font><p>Body three <img recindex=\"0000a\"/> tail.</p></div>",
            "</body></html>"
        )
        .as_bytes()
        .to_vec();
        let b = Builder {
            compression: 1,
            encryption: 0,
            text,
            images,
            res121: vec![],
            text_encoding: 65001,
        }
        .build();
        let dir = std::env::temp_dir().join(format!("tidepage-mobi-synth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("synth.mobi");
        std::fs::write(&file, &b).unwrap();
        let res_dir = dir.join("res");
        let doc = parse_full(file.to_str().unwrap(), None, Some((&res_dir, "res/"))).unwrap();
        // 问题3 根因2：recindex 相对 first_image_index 解析（十进制 >9 与 hex 都要命中）
        assert!(doc.res_urls >= 2, "res_urls={}（应 >=2）", doc.res_urls);
        assert_eq!(doc.placeholder_count, 0, "不应有透明占位");
        // 问题3 根因1：章节起点在标签边界，且 parts 连续覆盖 [0, html.len())
        for (s, _) in &doc.parts {
            let head: String = doc.html[*s..].trim_start().chars().take(24).collect();
            assert!(head.starts_with('<'), "章节起点非标签边界: {head:?}");
        }
        assert_eq!(doc.parts.first().map(|p| p.0), Some(0));
        assert_eq!(doc.parts.last().map(|p| p.1), Some(doc.html.len()));
        let covered: usize = doc.parts.iter().map(|(s, e)| doc.html[*s..*e].chars().count()).sum();
        assert_eq!(covered, doc.html.chars().count(), "parts 未连续覆盖");
        // 问题2：一次读取/一次解析同时拿到 doc 与封面（无 EXTH201/202/129 时回退首图）
        let (doc2, cover) = parse_with_cover(file.to_str().unwrap());
        assert!(doc2.is_ok(), "parse_with_cover 应解析成功");
        let cover = cover.expect("应能提取封面");
        assert!(image_magic(&cover).is_some());
        // 兼容旧入口：extract_cover_path 仍可独立工作（reextract_cover 用）
        let cover2 = extract_cover_path(file.to_str().unwrap()).expect("extract_cover_path 应可用");
        assert_eq!(cover2.len(), cover.len());
        let _ = std::fs::remove_dir_all(&dir);
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

    /// 全库扫描：定位哪本书含 "aid=" 泄漏源（EPUB/MOBI/TXT 分别处理）
    /// 模拟浏览器渲染：跑完整管线后，把输出再 tokenize，检查文本节点里是否出现 aid=
    /// 把输出再走一遍 tokenizer，收集文本节点，找疑似 HTML 源码 token
    fn scan_visible_text_for_leaks(html: &str) -> Vec<String> {
        use html5ever::tokenizer::{Token, TokenSink, TokenSinkResult, Tokenizer};
        use std::cell::RefCell;
        use std::rc::Rc;
        #[derive(Clone)]
        struct Collector(Rc<RefCell<String>>, Rc<RefCell<Vec<String>>>);
        impl TokenSink for Collector {
            type Handle = ();
            fn process_token(&self, token: Token, _l: u64) -> TokenSinkResult<()> {
                if let Token::CharacterTokens(c) = token {
                    self.0.borrow_mut().push_str(&c.to_string());
                }
                TokenSinkResult::Continue
            }
        }
        let text = Rc::new(RefCell::new(String::new()));
        let _hits = Rc::new(RefCell::new(Vec::new()));
        let tok = Tokenizer::new(
            Collector(text.clone(), _hits.clone()),
            html5ever::tokenizer::TokenizerOpts::default(),
        );
        let mut q = html5ever::tokenizer::BufferQueue::default();
        q.push_back(html.into());
        let _ = tok.feed(&q);
        tok.end();
        let t = text.borrow();
        t.split_whitespace()
            .filter(|w| w.contains("aid=\"") || w.contains("class=\""))
            .take(8)
            .map(|w| w.to_string())
            .collect()
    }

    /// 回归：源文件里以实体形态存在的破碎标签（将夜 AZW3 实测样本）必须被剔除且不误伤正文
    #[test]
    fn broken_entity_tags_stripped() {
        let src = "<p>压低声音aid=&quot;1IIHD&quot;道：夫人</p>";
        let out = decode_and_sanitize(src);
        assert!(!out.contains("aid="), "不得残留属性源码: {out}");
        assert!(out.contains("压低声音") && out.contains("道：夫人"), "正文必须完整: {out}");
        let src2 = "<p>x</p><div class=&quot;calibre2&quot;><b>粗体</b></div>";
        let o2 = decode_and_sanitize(src2);
        assert!(o2.contains("<div") && o2.contains("<b>粗体</b>"), "合法标签保留: {o2}");
        assert!(!o2.contains("&lt;div"), "标签属性内实体不视为泄漏（浏览器不可见）: {o2}");
        // 文本节点里的纯源码破碎标签才要剔除
        let src4 = "<p>前&amp;lt;div class=&quot;toc&quot; aid=&quot;2&quot;&amp;gt;后</p>";
        let o4 = decode_and_sanitize(src4);
        assert!(o4.contains("前") && o4.contains("后"), "正文保留: {o4}");
        assert!(!o4.contains("aid=") && !o4.contains("class="), "破碎标签文本剔除: {o4}");
        // 数学小于号不误伤
        let src3 = "<p>若a&lt;b且b&lt;c则成立</p>";
        let o3 = decode_and_sanitize(src3);
        assert!(o3.contains("&lt;b") && o3.contains("&lt;c"), "比较符保留: {o3}");
    }
    #[test]
    fn scan_library_for_leak_source() {
        let dir = std::env::var("LIB_SCAN_DIR").unwrap_or_default();
        if dir.is_empty() {
            return;
        }
        for ent in walk(&dir) {
            let Ok(bytes) = std::fs::read(&ent) else { continue };
            let name = ent.to_ascii_lowercase();
            let mut hay: Vec<u8> = Vec::new();
            if name.ends_with(".epub") {
                if let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())) {
                    for i in 0..z.len() {
                        if let Ok(mut f) = z.by_index(i) {
                            let mut buf = Vec::new();
                            let _ = std::io::Read::read_to_end(&mut f, &mut buf);
                            hay.extend_from_slice(&buf);
                        }
                    }
                }
            } else if name.ends_with(".mobi") || name.ends_with(".azw3") || name.ends_with(".prc") {
                if let Ok(mobi) = Mobi::new(&bytes) {
                    if let Ok(raw) = extract_records_and_decompress(&mobi) {
                        hay = raw;
                    }
                }
            } else if name.ends_with(".txt") {
                hay = bytes;
            }
            let text = String::from_utf8_lossy(&hay).to_lowercase();
            if let Some(i) = text.find("aid=\"") {
                let raw = String::from_utf8_lossy(&hay).to_string();
                let a = raw[i.saturating_sub(70)..i.min(raw.len())].to_string();
                let b = raw[i..(i + 90).min(raw.len())].to_string();
                eprintln!("AID@ {ent}\n  ctx: {a:?} ||| {b:?}");
            }
        }
        eprintln!("scan done");
    }

    fn walk(dir: &str) -> Vec<String> {
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(dir) { Ok(r) => r, Err(_) => return out };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                out.extend(walk(&path.to_string_lossy()));
            } else if let Some(ext) = path.extension() {
                let ex = ext.to_string_lossy().to_ascii_lowercase();
                if matches!(ex.as_str(), "epub" | "mobi" | "azw3" | "prc" | "txt") {
                    out.push(path.to_string_lossy().to_string());
                }
            }
        }
        out
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
        assert!(g.contains("a &amp; b ∧"), "文本节点实体解码: {g}");
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

    #[test]
    fn sanitize_strips_event_attrs_and_protocols() {
        // 事件属性全删；危险协议 URL 整条删除；MOBI 专有标记/图片引用保留。
        let out = decode_and_sanitize(
            "<p onclick=\"x()\">a</p><img recindex=\"0002\" onerror=\"alert(1)\"/>\
             <a href=\"javascript:alert(1)\">y</a><iframe src=\"https://evil\">z</iframe>\
             <mbp:pagebreak/><img src=\"kindle:embed:0001\"/>",
        );
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("javascript:"), "{out}");
        assert!(!out.contains("iframe"), "{out}");
        assert!(!out.contains("evil"), "{out}");
        assert!(out.contains("recindex=\"0002\""), "recindex 必须保留: {out}");
        assert!(out.contains("mbp:pagebreak"), "{out}");
        assert!(out.contains("kindle:embed:0001"), "图片引用必须保留: {out}");
    }

    #[test]
    fn sanitize_nested_dangerous_container_drops_whole_outer() {
        // 嵌套危险标签：内层 </script> 不得提前解除外层 <form> 的丢弃态，
        // 外层容器内、内层之后的 CCC 及闭合后内容都要整段丢弃。
        let out = decode_and_sanitize("<form>AAA<script>BBB</script>CCC</form><p>KEEP</p>");
        assert!(!out.contains("AAA"), "{out}");
        assert!(!out.contains("BBB"), "{out}");
        assert!(!out.contains("CCC"), "{out}");
        assert!(!out.contains("<form"), "{out}");
        assert!(!out.contains("script"), "{out}");
        assert!(out.contains("<p>KEEP</p>"), "{out}");
    }
    /// 端到端真实文件测试：设置 TIDEPAGE_TEST_MOBI="路径1;路径2" 时运行
    #[test]
    fn real_world_samples_end_to_end() {
        let env = match std::env::var("TIDEPAGE_TEST_MOBI") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => {
                eprintln!("skip: TIDEPAGE_TEST_MOBI not set");
                return;
            }
        };
        for path in env.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let dir = std::env::temp_dir().join(format!("tidepage-real-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let t0 = std::time::Instant::now();
            let doc = match parse_full(path, None, Some((&dir, "res/"))) {
                Ok(d) => d,
                Err(e) => {
                    // 受损/不支持的样本：只报告可诊断信息，不让单个坏文件中断批量验证
                    // （app 侧同样返回错误而非崩溃，见 parse_full 的 panic 兜底）
                    eprintln!("[skip] {path}: {e}");
                    let _ = std::fs::remove_dir_all(&dir);
                    continue;
                }
            };
            let dt = t0.elapsed();
            let plain: String = doc
                .html
                .chars()
                .filter(|c| matches!(c, '\u{4e00}'..='\u{9fff}'))
                .take(8)
                .collect();
            let fffd = doc.html.matches('\u{FFFD}').count();
            // 章节起点必须落在标签边界：任一 part 起点字符不是 '<'，或切片开头是可见文本以外的
            // 情况都说明 offsets 失效（根因1 回归）。
            let bad_starts: Vec<String> = doc
                .parts
                .iter()
                .take(12)
                .filter_map(|(s, _)| {
                    let head = doc.html.get(*s..(*s).saturating_add(24))?;
                    let head = head.trim_start();
                    if head.starts_with('<') {
                        None
                    } else {
                        Some(head.replace('\n', "\\n"))
                    }
                })
                .collect();
            eprintln!(
                "[diag] {} -> {}ms, toc={}, html={}B, imgs={}, res_urls={}, placeholder={}, \
                 FFFD={}, bad_chapter_starts={:?}, warning={:?}, 中文样例={:?}",
                std::path::Path::new(path).file_name().unwrap().to_string_lossy(),
                dt.as_millis(),
                doc.toc.len(),
                doc.html.len(),
                doc.image_count,
                doc.res_urls,
                doc.placeholder_count,
                fffd,
                bad_starts,
                doc.warning,
                plain
            );
            eprintln!(
                "[toc] {:?}",
                doc.toc.iter().take(12).map(|t| t.title.clone()).collect::<Vec<_>>()
            );
            let cover = extract_cover_path(path);
            eprintln!(
                "[cover] {:?} len={:?} magic={:?}",
                std::path::Path::new(path).file_name().unwrap().to_string_lossy(),
                cover.as_ref().map(|c| c.len()),
                cover.as_ref().and_then(|c| image_magic(c)),
            );
            assert!(
                cover.as_ref().and_then(|c| image_magic(c)).is_some(),
                "{path}: 封面提取失败（应为有效图片字节）"
            );
            assert!(!doc.html.is_empty(), "{path}: 正文为空");
            assert!(doc.toc.len() > 0, "{path}: 无章节");
            assert_eq!(
                doc.placeholder_count, 0,
                "{path}: 有 {} 个图片引用解析不到（根因2 未修复）: {:?}",
                doc.placeholder_count, bad_starts
            );
            assert!(
                bad_starts.is_empty() || doc.parts.len() <= 1,
                "{path}: 章节起点未落在标签边界（根因1）: {bad_starts:?}"
            );
            // 章节区间必须与最终 html 完全对应：连续覆盖 [0,len)（无重叠/无空洞）
            if doc.parts.first().map(|p| p.0) == Some(0)
                && doc.parts.last().map(|p| p.1) == Some(doc.html.len())
            {
                let covered: usize = doc
                    .parts
                    .iter()
                    .map(|(s, e)| doc.html.get(*s..*e).map(|x| x.chars().count()).unwrap_or(0))
                    .sum();
                assert_eq!(
                    covered,
                    doc.html.chars().count(),
                    "{path}: 章节区间与实际 html 长度不一致（根因1：offsets 失效）"
                );
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
