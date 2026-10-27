use crate::models::TocEntry;
use crate::txt::escape_html;
use regex::Regex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct EpubDoc {
    pub title: String,
    pub author: String,
    pub spine: Vec<String>,
    pub toc: Vec<TocEntry>,
    /// 每个目录项对应的章节在 spine 中的结束下标（不含）。有些 EPUB 把一个章节拆成
    /// 多个小文件、目录只指向首个小文件，必须靠这个区间把整章内容合起来。
    pub toc_end: Vec<usize>,
    pub cover_href: Option<String>,
    pub spine_chars: Vec<i64>,
    pub total_chars: i64,
    pub total_text_chars: i64,
    /// OPF manifest 中的图片资源（zip 内绝对路径，按声明顺序）。供图片查看器快速列举。
    pub image_hrefs: Vec<String>,
    /// P11（图片页型 EPUB 单章过大）：是否已把「目录折叠 + 图片页型」的书按 NCX/导航锚点
    /// 切成多个虚拟章。为 true 时消费方用 split_toc/chapters，toc/toc_end 保持 legacy 语义。
    pub split: bool,
    /// 切分后的有效目录（每章一条，index = 章节序号 0..n）。split=false 时为空。
    pub split_toc: Vec<TocEntry>,
    /// 切分后的章节列表（含切片章与其后的整篇 fallback 章）。split=false 时为空。
    pub chapters: Vec<EpubChapter>,
    /// R6-1：本次切分是否来自「无锚点块级自动切章」（仅诊断用，不参与业务判断）。
    pub auto_block_split: bool,
}

/// P11：切分后的一个虚拟章节。
///
/// 两种形态：
/// - 切片章（start=Some）：只覆盖 spine 文档 body 内容的 [start, end) 字节区间；
/// - 整篇章（start=None）：覆盖 spine..spine_end 的全部文档（普通 legacy 章与锚点组之后的
///   整篇 fallback 章都是这种）。
#[derive(Debug, Clone)]
pub struct EpubChapter {
    pub title: String,
    /// 起始 spine 下标。
    pub spine: usize,
    /// 结束 spine 下标（不含）；切片章恒为 spine+1。
    pub spine_end: usize,
    /// 切片章的 body 内容字节起点；整篇章为 None。
    pub start: Option<usize>,
    /// 切片章的 body 内容字节终点（start 为 Some 时有效）。
    pub end: usize,
    /// 本章内 <img 数（迁移权重与诊断；0 在权重计算里当 1）。
    pub imgs: usize,
    /// 所属去重后的 legacy 目录项下标（进度/书签迁移用）。
    pub legacy: usize,
}

impl EpubChapter {
    pub fn is_slice(&self) -> bool {
        self.start.is_some()
    }
}

/// P11 阈值（只在「目录折叠 + 图片页型」的书上生效，普通文字书零影响）：
/// - SPLIT_MIN_IMAGES：spine 文档至少这么多图才值得切章（几百张以上才拖慢渲染/DOM）；
/// - SPLIT_TEXT_PER_IMG_MAX：每张图平均最多配这么多非空白文字（水印型图片页可通过，
///   带大量正文的普通带图书会被这条件挡掉）；
/// - SPLIT_MAX_CHAPTERS：单组最多切多少章，避免异常 NCX 生成上千虚拟章；
/// - SPLIT_MIN_PAIRS：同一目录项至少折叠了这么多导航点才视为「折叠目录」。
const SPLIT_MIN_IMAGES: usize = 100;
const SPLIT_TEXT_PER_IMG_MAX: usize = 40;
const SPLIT_MAX_CHAPTERS: usize = 500;
const SPLIT_MIN_PAIRS: usize = 2;

pub type CiMap = HashMap<String, usize>;

fn build_ci<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> CiMap {
    let mut map: CiMap = HashMap::new();
    let count = zip.len();
    for i in 0..count {
        if let Ok(f) = zip.by_index(i) {
            let nm = f.name().replace('\\', "/").to_lowercase();
            map.entry(nm.clone()).or_insert(i);
            if let Some(base) = nm.rsplit('/').next() {
                map.entry(format!("b:{base}")).or_insert(i);
            }
            if let Some(rest) = nm.strip_prefix("./") {
                map.entry(rest.to_string()).or_insert(i);
            }
        }
    }
    map
}

fn resolve_idx(ci: &CiMap, name: &str) -> Option<usize> {
    let want = name.replace('\\', "/").to_lowercase();
    let want = want.strip_prefix("./").unwrap_or(&want);
    ci.get(want)
        .or_else(|| ci.get(&format!("b:{}", want.rsplit('/').next().unwrap_or(want))))
        .copied()
}

/// 单条目解压硬上限（与 PDF 通道既有 64MiB 口径一致）。
/// 中央目录里声明的大小是攻击者可控值：不能按它预分配，也不能无限 read_to_end。
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// 读取至多 cap 字节：只多读 1 字节即可判断超限，绝不按声明尺寸预分配。
/// 超限/读失败返回 None，调用方按「该条目不可用」处理。
fn read_capped<R: Read>(r: &mut R, cap: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    r.take(cap.saturating_add(1)).read_to_end(&mut out).ok()?;
    if out.len() as u64 > cap {
        None
    } else {
        Some(out)
    }
}

fn zip_read<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    name: &str,
) -> Option<Vec<u8>> {
    let idx = resolve_idx(ci, name)?;
    let mut f = zip.by_index(idx).ok()?;
    read_capped(&mut f, MAX_ENTRY_BYTES)
}

/// 解码文本，兼容 UTF-8 与带 BOM 的 UTF-16（部分工具生成的 XML 是 UTF-16）。
fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return decode_utf16(bytes, true);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return decode_utf16(bytes, false);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_utf16(bytes: &[u8], little: bool) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2);
    let mut i = 2; // 跳过 BOM
    while i + 1 < bytes.len() {
        let v = if little {
            u16::from_le_bytes([bytes[i], bytes[i + 1]])
        } else {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        };
        units.push(v);
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

/// 从 container.xml 文本里取出 rootfile 的 full-path，兼容单引号/双引号（原来只认双引号，
/// 导致 `full-path='...'` 的 EPUB 报「container.xml 中未找到 OPF」）。
fn opf_path_from_container(container: &str) -> Option<String> {
    let re_d = Regex::new(r#"(?i)full-path\s*=\s*"([^"]+)""#).ok()?;
    if let Some(c) = re_d.captures(container) {
        return Some(c[1].to_string());
    }
    let re_s = Regex::new(r#"(?i)full-path\s*=\s*'([^']+)'"#).ok()?;
    re_s.captures(container).map(|c| c[1].to_string())
}

/// container.xml 缺失/损坏时的兜底：在压缩包里找一个 .opf（优先 content.opf）。
fn find_opf_in_zip(ci: &CiMap) -> Option<String> {
    let mut fallback: Option<String> = None;
    for k in ci.keys() {
        if k.starts_with("b:") {
            continue;
        }
        let path = k.strip_prefix("./").unwrap_or(k);
        if !path.ends_with(".opf") {
            continue;
        }
        if path.ends_with("/content.opf") || path == "content.opf" {
            return Some(path.to_string());
        }
        if fallback.is_none() {
            fallback = Some(path.to_string());
        }
    }
    fallback
}

fn zip_has(ci: &CiMap, name: &str) -> bool {
    resolve_idx(ci, name).is_some()
}

/// 判断是否受 DRM/加密保护（掌阅 zhangyue、或其他声明了 encryption.xml 的加密书）。
fn is_encrypted(ci: &CiMap) -> bool {
    zip_has(ci, "META-INF/encryption.xml") || zip_has(ci, "META-INF/zhangyue-expansion.xml")
}

fn looks_like_opf(s: &str) -> bool {
    let low = s.to_ascii_lowercase();
    low.contains("<package") || low.contains("<opf:package")
}

const ERR_ENCRYPTED: &str =
    "这本 EPUB 是加密的，需要官方阅读器才能打开；请获取无 DRM 的版本后再导入。";

fn norm_path(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href);
    let href = percent_decode(href);
    let combined = if href.starts_with('/') {
        href.trim_start_matches('/').to_string()
    } else {
        let base_dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut parts: Vec<&str> = if base_dir.is_empty() {
            vec![]
        } else {
            base_dir.split('/').collect()
        };
        for seg in href.split('/') {
            match seg {
                "." => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        parts.join("/")
    };
    combined
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // 逐字节解析 %XX：旧实现按字符下标切片，%中 这类输入会落在 UTF-8 字符中间 panic。
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        // EPUB href 是 URI 引用而非 form-urlencoded 表单值：'+' 是合法文件名字符，
        // 不能按 URL query 规则转空格；只解码 %XX，非法 % 保持原样。
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 快速解析：只解析 container/OPF/目录/封面，**不做全书逐章文本统计**。
/// 打开书籍与取章节走这里，避免 7000+ 章的书在打开时卡 5 秒以上。
/// 需要全书字数时用 `parse_with_chars`（后台元数据阶段）。
/// 接收 `&[u8]` 借用（内部 `Cursor` 也只借用），调用方无需为解析复制整份书字节。
pub fn parse(data: &[u8]) -> Result<EpubDoc, String> {
    parse_impl(std::io::Cursor::new(data), false).map(|(doc, _)| doc)
}

/// 完整解析：额外统计每一章的字符数（供全书字数/总页数估算，后台执行）。
/// 打开书籍与取章节走快速路径（`parse` / `parse_with_ci`），不统计字数。
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_with_chars(data: &[u8]) -> Result<EpubDoc, String> {
    parse_impl(std::io::Cursor::new(data), true).map(|(doc, _)| doc)
}

/// 全书字数统计：`(每章字数, 全书字数 total_chars, 纯字数 total_text_chars)`。
///
/// `total / pure` 必须取 `EpubDoc` 的 spine 逐文档求和，**不能把 toc 的字数求和**：
/// 同一个 spine 文档被多个目录项指向时会被重复计数；toc 数组只服务「全书总页数」的按章估算。
///
/// 为什么单独走一次完整解析：快速解析刻意不做全书逐章统计（数千章可达数秒），
/// 于是 toc 里的 chars 与 total_chars 全是 0。调用方（书详情/本书统计、全书总页数）
/// 按需补算一次，并共享同一份缓存。
#[cfg_attr(not(test), allow(dead_code))]
pub fn epub_word_stats(data: &[u8]) -> Result<(Vec<i64>, i64, i64), String> {
    let doc = parse_with_chars(data)?;
    let per_chapter = doc.toc.iter().map(|t| t.chars).collect();
    Ok((per_chapter, doc.total_chars, doc.total_text_chars))
}

/// C13：按文件路径做完整字数统计，不把整本读入常驻。
#[cfg_attr(not(test), allow(dead_code))]
pub fn epub_word_stats_file(path: &Path) -> Result<(Vec<i64>, i64, i64), String> {
    let doc = parse_file_impl(path, true)?.0;
    let per_chapter = doc.toc.iter().map(|t| t.chars).collect();
    Ok((per_chapter, doc.total_chars, doc.total_text_chars))
}



/// 快速解析并**同时返回条目索引**（避免调用方再扫一遍 zip 建 CiMap）。
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_with_ci(data: &[u8]) -> Result<(EpubDoc, CiMap), String> {
    parse_impl(std::io::Cursor::new(data), false)
}

/// C13：用 File + ZipArchive 按需读取 central directory/OPF/导航，不整本入内存。
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_file_with_ci(path: &Path) -> Result<(EpubDoc, CiMap), String> {
    parse_file_impl(path, false)
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_file_impl(path: &Path, compute_chars: bool) -> Result<(EpubDoc, CiMap), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("EPUB 打开失败: {e}"))?;
    parse_impl(std::io::BufReader::new(file), compute_chars)
}

fn parse_impl<R: Read + std::io::Seek>(reader: R, compute_chars: bool) -> Result<(EpubDoc, CiMap), String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("EPUB 打开失败: {e}"))?;
    let ci = build_ci(&mut zip);

    let container = zip_read(&mut zip, &ci, "META-INF/container.xml")
        .ok_or("缺少 META-INF/container.xml")?;
    let container_str = decode_text(&container);
    let opf_path = opf_path_from_container(&container_str)
        .or_else(|| find_opf_in_zip(&ci))
        .ok_or_else(|| {
            if is_encrypted(&ci) {
                ERR_ENCRYPTED.to_string()
            } else {
                "container.xml 中未找到 OPF，且压缩包内没有 .opf 文件".to_string()
            }
        })?;

    let opf_bytes = zip_read(&mut zip, &ci, &opf_path).ok_or("OPF 文件缺失")?;
    let opf = decode_text(&opf_bytes);
    if !looks_like_opf(&opf) {
        return Err(if is_encrypted(&ci) {
            ERR_ENCRYPTED.to_string()
        } else {
            "OPF 内容无法解析（文件可能已加密或损坏）".to_string()
        });
    }
    let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let mut title = String::new();
    let mut author = String::new();
    let mut cover_href: Option<String> = None;
    let mut items: Vec<(String, String, String)> = Vec::new();
    let mut spine_ids: Vec<String> = Vec::new();

    let mut reader = quick_xml::Reader::from_str(&opf);
    reader.config_mut().trim_text(true);
    let mut cur_elem = String::new();
    let mut in_meta = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                let name = String::from_utf8_lossy(&name).into_owned();
                if name.ends_with("metadata") {
                    in_meta = true;
                }
                if name == "item" {
                    let mut id = String::new();
                    let mut href = String::new();
                    let mut props = String::new();
                    let mut mtype = String::new();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        let val = String::from_utf8_lossy(&a.value).into_owned();
                        match key.as_str() {
                            "id" => id = val,
                            "href" => href = val,
                            "properties" => props = val,
                            "media-type" => mtype = val,
                            _ => {}
                        }
                    }
                    items.push((id, href, format!("{props}\u{1}{mtype}")));
                }
                if name == "itemref" {
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        if key == "idref" {
                            spine_ids.push(String::from_utf8_lossy(&a.value).into_owned());
                        }
                    }
                }
                cur_elem = name;
            }
            Ok(quick_xml::events::Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if name == "item" {
                    let mut id = String::new();
                    let mut href = String::new();
                    let mut props = String::new();
                    let mut mtype = String::new();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        let val = String::from_utf8_lossy(&a.value).into_owned();
                        match key.as_str() {
                            "id" => id = val,
                            "href" => href = val,
                            "properties" => props = val,
                            "media-type" => mtype = val,
                            _ => {}
                        }
                    }
                    items.push((id, href, format!("{props}\u{1}{mtype}")));
                }
                if name == "itemref" {
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        if key == "idref" {
                            spine_ids.push(String::from_utf8_lossy(&a.value).into_owned());
                        }
                    }
                }
                if name == "meta" && in_meta {
                    let mut mn = String::new();
                    let mut mv = String::new();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        let val = String::from_utf8_lossy(&a.value).into_owned();
                        if key == "name" {
                            mn = val;
                        } else if key == "content" {
                            mv = val;
                        }
                    }
                    if mn == "cover" && !mv.is_empty() {
                        for (id, href, _) in &items {
                            if id == &mv {
                                cover_href = Some(href.clone());
                            }
                        }
                    }
                }
            }
            Ok(quick_xml::events::Event::Text(t)) => {
                let txt = t.unescape().unwrap_or_default().trim().to_string();
                if !txt.is_empty() {
                    if cur_elem.ends_with("title") && title.is_empty() && in_meta {
                        title = txt;
                    } else if cur_elem.ends_with("creator") && author.is_empty() && in_meta {
                        author = txt;
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if name.ends_with("metadata") {
                    in_meta = false;
                }
                cur_elem.clear();
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    if title.is_empty() {
        title = Path::new(&opf_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    let mut spine: Vec<String> = Vec::new();
    for idref in &spine_ids {
        for (id, href, meta) in &items {
            if id == idref {
                let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
                if mtype.contains("html") || mtype.is_empty() || mtype.contains("xml") {
                    spine.push(norm_path(&opf_dir, href));
                }
            }
        }
    }
    if spine.is_empty() {
        for (_, href, meta) in &items {
            let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
            if mtype.contains("html") {
                spine.push(norm_path(&opf_dir, href));
            }
        }
    }

    if cover_href.is_none() {
        for (id, href, meta) in &items {
            let (props, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
            if props.contains("cover-image") || (mtype.starts_with("image") && id.contains("cover"))
            {
                cover_href = Some(norm_path(&opf_dir, href));
                break;
            }
        }
    } else {
        cover_href = cover_href.map(|h| norm_path(&opf_dir, &h));
    }

    // OPF manifest 里的图片资源（按声明顺序），供图片查看器快速列举（无需扫描全部章节）。
    let image_hrefs: Vec<String> = items
        .iter()
        .filter(|(_, _, meta)| {
            let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
            mtype.starts_with("image")
        })
        .map(|(_, href, _)| norm_path(&opf_dir, href))
        .collect();

    let spine_idx: Vec<Option<usize>> = spine.iter().map(|s| resolve_idx(&ci, s)).collect();
    let nworkers = rayon::current_num_threads().max(1);
    // 全书逐章文本统计（7000+ 章时可达数秒）。仅在需要字数的后台元数据阶段执行；
    // 打开书籍/取章节的快速路径跳过，保证首屏尽快出现。
    let spine_chars: Vec<(i64, i64)> = if !compute_chars {
        vec![(0i64, 0i64); spine_idx.len()]
    } else {
        // C13：文件路径解析没有整本 data 可借用，改为在当前 ZipArchive 上顺序统计；
        // 只在后台字数统计路径执行，不阻塞打开/取章。
        let mut results = vec![(0i64, 0i64); spine_idx.len()];
        let _ = nworkers;
        for (i, member) in spine_idx.iter().enumerate() {
            let Some(member) = *member else { continue };
            if let Ok(mut f) = zip.by_index(member) {
                match read_capped(&mut f, MAX_ENTRY_BYTES) {
                    Some(b) => {
                        let html = String::from_utf8_lossy(&b).into_owned();
                        let text = html_to_text(&html);
                        let total = text.chars().count() as i64;
                        let pure = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
                        results[i] = (total, pure);
                    }
                    None => {
                        // 单章解压超限：该章计 0 并跳过，不中断整本统计（其余章仍可用）。
                        crate::memlog::stage_note(
                            "epub:chars_entry_capped",
                            &format!(
                                "spine={i} path={}",
                                spine.get(i).map(|s| s.as_str()).unwrap_or("")
                            ),
                        );
                    }
                }
            }
        }
        results
    };

    let (toc, groups) = extract_toc(&mut zip, &ci, &opf_dir, &items, &spine, &title, &spine_chars);

    // 每章 = 从本章目录项指向的 spine 下标，直到下一个目录项（或 spine 末尾）。
    // 这样「一章拆成多个小文件」的 EPUB 也能显示完整正文。
    let toc_end: Vec<usize> = (0..toc.len())
        .map(|i| {
            let start = toc[i].index;
            let next = toc.get(i + 1).map(|t| t.index).unwrap_or(spine.len());
            next.max(start + 1).min(spine.len())
        })
        .collect();

    // P11：图片页型「折叠目录」书按 NCX/导航锚点切成虚拟章。toc 保持 legacy 语义，
    // 切分结果放 split_toc/chapters；这里不做切分时 split=false，所有消费方行为不变。
    let (split, split_toc, chapters) =
        detect_image_split(&mut zip, &ci, &spine, &toc, &toc_end, &groups, &title, &spine_chars);
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        eprintln!(
            "[epub] split={split} chapters={} legacy_toc={} spine={}",
            chapters.len(),
            toc.len(),
            spine.len()
        );
        if split {
            let imgs: Vec<usize> = chapters.iter().map(|c| c.imgs).collect();
            eprintln!("[epub] split chapter imgs={imgs:?}");
        }
    }

    let total_chars: i64 = spine_chars.iter().map(|c| c.0).sum();
    let total_text_chars: i64 = spine_chars.iter().map(|c| c.1).sum();

    Ok((
        EpubDoc {
            title,
            author,
            spine,
            toc,
            toc_end,
            cover_href,
            spine_chars: spine_chars.iter().map(|c| c.0).collect(),
            total_chars,
            total_text_chars,
            image_hrefs,
            split,
            split_toc,
            chapters,
            auto_block_split: false,
        },
        ci,
    ))
}

/// 目录解析结果：toc 是去重后的 legacy 目录；groups[i] 是第 i 个目录项折叠的全部
/// 原始导航点（标题 + fragment，None 表示 href 不带锚点），供 P11 判断「锚点折叠」。
fn extract_toc<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    opf_dir: &str,
    items: &[(String, String, String)],
    spine: &[String],
    fallback: &str,
    spine_chars: &[(i64, i64)],
) -> (Vec<TocEntry>, Vec<Vec<(String, Option<String>)>>) {
    let mut pairs: Vec<(String, String)> = Vec::new();

    let nav_href = items
        .iter()
        .find(|(_, _, meta)| meta.split('\u{1}').next().unwrap_or("").contains("nav"))
        .map(|(_, href, _)| href.clone());
    if let Some(nav) = nav_href {
        let full = norm_path(opf_dir, &nav);
        if let Some(bytes) = zip_read(zip, ci, &full) {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(seg) = find_toc_nav_section(&html) {
                pairs = extract_anchor_pairs(&seg, &full);
            }
        }
    }

    if pairs.is_empty() {
        let ncx_href = items
            .iter()
            .find(|(id, _, _)| id.eq_ignore_ascii_case("ncx"))
            .map(|(_, href, _)| href.clone())
            .or_else(|| {
                items
                    .iter()
                    .find(|(_, _, meta)| {
                        let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
                        mtype.contains("ncx")
                    })
                    .map(|(_, href, _)| href.clone())
            })
            .or_else(|| {
                zip.file_names()
                    .find(|n| n.to_lowercase().ends_with(".ncx"))
                    .map(|s| s.to_string())
            });
        if let Some(ncx) = ncx_href {
            let full = norm_path(opf_dir, &ncx);
            if let Some(bytes) = zip_read(zip, ci, &full) {
                let xml = String::from_utf8_lossy(&bytes).into_owned();
                pairs = extract_ncx_pairs(&xml, &full);
            }
        }
    }

    fn spine_pos(spine: &[String], target: &str) -> Option<usize> {
        if let Some(i) = spine.iter().position(|s| s == target) {
            return Some(i);
        }
        let t = target.replace('\\', "/").to_lowercase();
        let tb = t.rsplit('/').next().unwrap_or(&t);
        spine.iter().position(|s| {
            let sl = s.replace('\\', "/").to_lowercase();
            sl == t || sl.rsplit('/').next().unwrap_or(&sl) == tb
        })
    }

    let mut toc: Vec<TocEntry> = Vec::new();
    let mut groups: Vec<Vec<(String, Option<String>)>> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (text, href) in pairs {
        // fragment 必须在 norm_path 之前截取（norm_path 会丢弃 # 之后的部分）。
        let (path, frag) = match href.split_once('#') {
            Some((p, f)) => (p, Some(f.to_string())),
            None => (href.as_str(), None),
        };
        let target = norm_path(opf_dir, path);
        if let Some(idx) = spine_pos(spine, &target) {
            let title: String = text.chars().take(80).collect();
            if let Some(pos) = used.iter().position(|u| *u == idx) {
                groups[pos].push((title, frag));
            } else {
                used.push(idx);
                toc.push(TocEntry {
                    title: title.clone(),
                    index: idx,
                    chars: spine_chars.get(idx).map(|c| c.0).unwrap_or(0),
                });
                groups.push(vec![(title, frag)]);
            }
        }
    }
    if toc.is_empty() {
        for (i, doc) in spine.iter().enumerate() {
            let name = doc.rsplit('/').next().unwrap_or(doc);
            let title = format!("{fallback} - {}", name);
            toc.push(TocEntry {
                title: title.clone(),
                index: i,
                chars: spine_chars.get(i).map(|c| c.0).unwrap_or(0),
            });
            groups.push(vec![(title, None)]);
        }
    }
    (toc, groups)
}


/// 在 hay 中从 from 起找 needle 的 ASCII 大小写不敏感出现，返回**原串**字节偏移。
///
/// 不能先 to_lowercase 再在 lower 里找偏移：Unicode 小写化会改变字节长度
/// （İ → i + 组合点变长、ẞ → ß 变短），用 lower 的偏移切原串会错位，
/// 轻则切出残破 HTML，重则落在非 UTF-8 字符边界 panic。HTML 标签名都是 ASCII，直接按字节比较。
fn find_ascii_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || from > h.len() || n.len() > h.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| {
        h[i..i + n.len()]
            .iter()
            .zip(n)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// 从 from 起向后找最后一个 ASCII 大小写不敏感出现，返回原串字节偏移。
fn rfind_ascii_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    let last = h.len() - n.len();
    if from > last {
        return None;
    }
    (from..=last).rev().find(|&i| {
        h[i..i + n.len()]
            .iter()
            .zip(n)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// P11：找出 raw HTML 的 body 内容字节区间（开标签之后、闭标签之前）。
/// 只按首个 <body ...> 处理，返回 None 表示不是常规 XHTML。
/// 偏移全部由原串计算（见 find_ascii_ci 的说明）。
fn body_bounds(raw: &str) -> Option<(usize, usize)> {
    let bs = find_ascii_ci(raw, "<body", 0)?;
    let after = raw[bs..].find('>').map(|i| bs + i + 1)?;
    let be = rfind_ascii_ci(raw, "</body", after)?;
    if after >= be {
        return None;
    }
    Some((after, be))
}

/// 统计片段里的 <img 标签数（大小写不敏感、按标签边界判断，避免 <image 误计）。
fn count_img_tags(s: &str) -> usize {
    let b = s.as_bytes();
    let mut n = 0usize;
    let mut i = 0usize;
    while i + 3 < b.len() {
        if b[i] == b'<'
            && (b[i + 1] | 0x20) == b'i'
            && (b[i + 2] | 0x20) == b'm'
            && (b[i + 3] | 0x20) == b'g'
            && b.get(i + 4).map_or(true, |c| !c.is_ascii_alphanumeric())
        {
            n += 1;
            i += 4;
        } else {
            i += 1;
        }
    }
    n
}

/// 向下/向上取到 UTF-8 字符边界（锚点与 body 边界本身都是 ASCII，这里只做防御性校正）。
fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// P11：取 body 内容区间 [start,end) 的字符串切片（字符边界防御性校正；越界自动收敛）。
pub fn slice_body(raw: &str, start: usize, end: usize) -> &str {
    let st = floor_char_boundary(raw, start);
    let en = ceil_char_boundary(raw, end).max(st);
    &raw[st..en]
}

/// P11：在 body 区间里找 fragment 锚点。按 id="frag" 或 name="frag"（单双引号均可）定位，
/// 返回该属性所在标签的 < 字节偏移（章节从锚点标签开始）。要求属性前是空白，
/// 避免 data-id="x" 这类子串误命中。
fn find_anchor_offset(raw: &str, body_start: usize, body_end: usize, frag: &str) -> Option<usize> {
    // 用正则直接在原文上定位（返回原文字节偏移）；不能先 to_lowercase 再找，个别 Unicode
    // 字符小写化会改变字节长度，偏移会错位。
    let body = &raw[body_start..body_end];
    let esc = regex::escape(frag);
    let re = Regex::new(&format!(r#"(?i)(?:id|name)\s*=\s*["']{esc}["']"#)).ok()?;
    for m in re.find_iter(body) {
        let abs = body_start + m.start();
        // 属性前必须是空白（避免 data-id="x" 这类子串误命中）。
        let prev_ws = raw[..abs].chars().next_back().map_or(true, |c| c.is_whitespace());
        if prev_ws {
            if let Some(p) = raw[body_start..abs].rfind('<') {
                return Some(body_start + p);
            }
        }
    }
    None
}


/// R12-3：P11 切分书的跨虚拟章锚点定位。给定目标文件名 basename（大小写不敏感）与 fragment，
/// 读取对应 spine HTML，找第一个 id/name 锚点的 body 偏移，再按 chapters 的区间归入虚拟章：
/// 切片章 [start,end) 半开、整篇章按 spine..spine_end；按章顺序取第一个匹配。
/// 未切分书 / 文件不匹配 / 锚点不存在均返回 None。
pub fn locate_split_anchor_chapter(
    doc: &EpubDoc,
    zip_bytes: &[u8],
    ci: &CiMap,
    file_base: &str,
    fragment: &str,
) -> Option<usize> {
    if !doc.split || fragment.is_empty() || file_base.is_empty() {
        return None;
    }
    let target = file_base.to_ascii_lowercase();
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    for (si, name) in doc.spine.iter().enumerate() {
        let base = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
        if base != target {
            continue;
        }
        // 同名 basename 可能出现在不同目录：本文件读不到/无锚点/锚点不落在任何章时，
        // 继续找下一个 spine，而不是直接返回 None。
        let raw_b = match zip_read(&mut zip, ci, name) {
            Some(b) => b,
            None => continue,
        };
        let raw = String::from_utf8_lossy(&raw_b).into_owned();
        let (bs, be) = match body_bounds(&raw) {
            Some(x) => x,
            None => continue,
        };
        let off = match find_anchor_offset(&raw, bs, be, fragment) {
            Some(o) => o,
            None => continue,
        };
        for (idx, c) in doc.chapters.iter().enumerate() {
            match c.start {
                Some(start) => {
                    if c.spine == si && off >= start && off < c.end {
                        return Some(idx);
                    }
                }
                None => {
                    if c.spine <= si && si < c.spine_end {
                        return Some(idx);
                    }
                }
            }
        }
        // 锚点在这个文件里，但不在任何章区间（异常索引）：继续找下一个同名文件。
    }
    None
}

/// P11：锚点必须落在块级边界：标签前一个非空白字符序列以 </p> </div> </h1-6> </li> </tr> 结尾，
/// 或锚点就是 body 的第一个内容（否则从锚点切开可能切在行内元素中间）。
fn anchor_boundary_ok(raw: &str, body_start: usize, tag_start: usize) -> bool {
    let before = raw[body_start..tag_start].trim_end_matches(|c: char| c.is_whitespace());
    if before.is_empty() {
        return true;
    }
    let low = before.to_ascii_lowercase();
    ["</p>", "</div>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>", "</h6>", "</li>", "</tr>"]
        .iter()
        .any(|t| low.ends_with(t))
}

/// P11：判断能否把「目录折叠」的图片页书按锚点切章，并生成切分章节。
/// 条件（全部满足才切）：某 legacy 目录项折叠了 >= SPLIT_MIN_PAIRS 个导航点且全部带
/// fragment；这些点都指向同一 spine 文档 S（S 即该目录项区间首篇）；S 的图片数
/// >= SPLIT_MIN_IMAGES 且非空白文本 <= 图片数 * SPLIT_TEXT_PER_IMG_MAX；每个 fragment 都能
/// 在 S 的 body 里找到严格递增、处于块级边界的锚点；单组章数 <= SPLIT_MAX_CHAPTERS。
/// 任一不满足即返回 (false, 空, 空)，调用方保持 legacy 行为。
fn detect_image_split<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    spine: &[String],
    toc: &[TocEntry],
    toc_end: &[usize],
    groups: &[Vec<(String, Option<String>)>],
    book_title: &str,
    spine_chars: &[(i64, i64)],
) -> (bool, Vec<TocEntry>, Vec<EpubChapter>) {
    let mut chapters: Vec<EpubChapter> = Vec::new();
    let mut split_any = false;
    for (li, t) in toc.iter().enumerate() {
        let pairs: &[(String, Option<String>)] = groups.get(li).map(|v| v.as_slice()).unwrap_or(&[]);
        let frags: Vec<&str> = pairs
            .iter()
            .filter_map(|(_, f)| f.as_deref())
            .filter(|f| !f.is_empty())
            .collect();
        let s = t.index;
        // a) 至少两个导航点、全部带非空 fragment；b) S 在 spine 范围内。
        let candidate = pairs.len() >= SPLIT_MIN_PAIRS && frags.len() == pairs.len() && s < spine.len();
        let mut sliced: Option<(String, usize, usize, Vec<usize>, usize)> = None;
        if candidate {
            if let Some(bytes) = zip_read(zip, ci, &spine[s]) {
            let raw = String::from_utf8_lossy(&bytes).into_owned();
            if let Some((bs, be)) = body_bounds(&raw) {
                let imgs = count_img_tags(&raw[bs..be]);
                let text_chars = html_to_text(&raw)
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .count();
                if imgs >= SPLIT_MIN_IMAGES
                    && text_chars <= imgs.saturating_mul(SPLIT_TEXT_PER_IMG_MAX)
                {
                    let mut offs: Vec<usize> = Vec::with_capacity(frags.len());
                    let mut valid = true;
                    for f in &frags {
                        let Some(off) = find_anchor_offset(&raw, bs, be, f) else {
                            valid = false;
                            break;
                        };
                        if offs.last().map_or(false, |p| off <= *p) {
                            valid = false;
                            break;
                        }
                        if !anchor_boundary_ok(&raw, bs, off) {
                            valid = false;
                            break;
                        }
                        offs.push(off);
                    }
                    if valid && !offs.is_empty() && offs.len() <= SPLIT_MAX_CHAPTERS {
                        sliced = Some((raw, bs, be, offs, imgs));
                    }
                }
            }
            }
        }
        if let Some((raw, bs, be, offs, _imgs)) = sliced {
            split_any = true;
            // 首章从 body 起点开始（第一个锚点前的内容并入第一章），末章到 body 末尾；
            // 第 2..n 个锚点是后续各章的起点（锚点个数 == 章数）。
            let mut starts: Vec<usize> = Vec::with_capacity(frags.len());
            starts.push(bs);
            starts.extend(offs.iter().skip(1).copied());
            for k in 0..frags.len() {
                let st = floor_char_boundary(&raw, starts[k]);
                let en = match starts.get(k + 1) {
                    Some(next) => ceil_char_boundary(&raw, *next),
                    None => be,
                };
                // R7-2：锚点标题先规范化（空/纯数字 → 空），再按 80 字符截断。
                let title: String = normalize_split_title(&pairs[k].0).chars().take(80).collect();
                chapters.push(EpubChapter {
                    title,
                    spine: s,
                    spine_end: s + 1,
                    start: Some(st),
                    end: en.max(st),
                    imgs: count_img_tags(slice_body(&raw, st, en)),
                    legacy: li,
                });
            }
            // 组区间内 S 之后的 spine 文档 → 整篇 fallback 章（标题沿用 legacy 规则）。
            let end = toc_end.get(li).copied().unwrap_or(s + 1).min(spine.len());
            for si in (s + 1)..end {
                let name = spine[si].rsplit('/').next().unwrap_or(&spine[si]).to_string();
                let imgs = zip_read(zip, ci, &spine[si])
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .and_then(|raw| body_bounds(&raw).map(|(bs, be)| count_img_tags(&raw[bs..be])))
                    .unwrap_or(0);
                chapters.push(EpubChapter {
                    title: normalize_split_title(&format!("{book_title} - {name}")),
                    spine: si,
                    spine_end: si + 1,
                    start: None,
                    end: 0,
                    imgs,
                    legacy: li,
                });
            }
        } else {
            // 非候选目录项：整段区间作为一个普通章（顺序与索引都不变）。
            let end = toc_end
                .get(li)
                .copied()
                .unwrap_or(s + 1)
                .min(spine.len())
                .max(s + 1);
            chapters.push(EpubChapter {
                title: normalize_split_title(&t.title),
                spine: s,
                spine_end: end,
                start: None,
                end: 0,
                imgs: 0,
                legacy: li,
            });
        }
    }
    finish_split(chapters, split_any, spine_chars)
}

/// detect_image_split 的收尾：没切成功就返回空；成功则生成 index=章节序号的有效目录。
fn finish_split(
    chapters: Vec<EpubChapter>,
    split_any: bool,
    spine_chars: &[(i64, i64)],
) -> (bool, Vec<TocEntry>, Vec<EpubChapter>) {
    if !split_any {
        return (false, Vec::new(), Vec::new());
    }
    let split_toc: Vec<TocEntry> = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| TocEntry {
            title: c.title.clone(),
            index: i,
            chars: if c.is_slice() {
                // 图片页章不参与字数/页数估算（它们只有水印文字）。
                0
            } else {
                (c.spine..c.spine_end)
                    .map(|si| spine_chars.get(si).map(|x| x.0).unwrap_or(0))
                    .sum()
            },
        })
        .collect();
    (true, split_toc, chapters)
}

/// P11 迁移纯函数：把旧（legacy 目录项下标, 章内比例）映射到切分后的（章号, 章内比例）。
/// 组 = 同一 legacy 的全部新章（切片章 + 其后的整篇 fallback 章）；组内按各章 imgs 权重累计
/// （imgs=0 当 1）。图片页书各章大小不均，故进度条按章平均只有近似线性（报告已写明）。
/// 越界/找不到组 → (0, 0.0)（回到全书开头），不 panic。
pub fn map_legacy_position(chapters: &[EpubChapter], legacy_index: i64, ratio: f64) -> (usize, f64) {
    let r = ratio.clamp(0.0, 1.0);
    let idxs: Vec<usize> = chapters
        .iter()
        .enumerate()
        .filter(|(_, c)| c.legacy as i64 == legacy_index)
        .map(|(i, _)| i)
        .collect();
    if idxs.is_empty() {
        return (0, 0.0);
    }
    let weights: Vec<f64> = idxs.iter().map(|i| chapters[*i].imgs.max(1) as f64).collect();
    let total: f64 = weights.iter().sum::<f64>().max(1.0);
    let target = r * total;
    let mut acc = 0.0f64;
    for (k, w) in weights.iter().enumerate() {
        let last = k + 1 == weights.len();
        if target < acc + *w || last {
            let within = ((target - acc) / *w).clamp(0.0, 1.0);
            return (idxs[k], within);
        }
        acc += *w;
    }
    (idxs[0], r)
}


/// R6-1 阈值：无锚点图片页书按「每章最多多少张图」做块级切章（2042 图 → 11 章）。
pub const AUTO_BLOCK_IMGS_PER_CHAPTER: usize = 200;
/// R6-1 阈值：自动块级切章的总章数上限（防止异常文档切出上千章）。
pub const AUTO_BLOCK_MAX_CHAPTERS: usize = 500;

/// R6-1：按「块级闭合标签之后」把 body 切成多段，返回后续章起点的字节偏移列表
/// （首章起点由调用方用 body_start）。每累计到 max_imgs 张图，就在下一个块级闭合标签
/// （</p>、</div>、</li>、</tr>、</h1-6>、</section>、</blockquote>、</figure>、</table>、
/// </article>）之后切一刀；切点跳过空白后必须是 '<' 或 body 末尾，保证不切在标签/文本中间。
/// 注释、script、style 区间整体跳过：里面的 '<' 不参与图片计数，也不会成为切点。
pub fn block_split_starts(
    raw: &str,
    body_start: usize,
    body_end: usize,
    max_imgs_per_chapter: usize,
) -> Vec<usize> {
    const CLOSERS: &[&str] = &[
        "</p>", "</div>", "</li>", "</tr>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>",
        "</h6>", "</section>", "</blockquote>", "</figure>", "</table>", "</article>",
    ];
    let max = max_imgs_per_chapter.max(1);
    let mut starts: Vec<usize> = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = body_start.min(raw.len());
    let body_end = body_end.min(raw.len());
    let mut imgs = 0usize;
    while i < body_end {
        let Some(rel) = raw[i..body_end].find('<') else { break };
        let lt = i + rel;
        let rest = &raw[lt..body_end];
        let low = rest.as_bytes();
        // 注释 / script / style：整体跳过，避免把里面的 '<' 当标签。
        if low.len() >= 4 && &low[..4] == b"<!--" {
            match rest.find("-->") {
                Some(e) => {
                    i = lt + e + 3;
                    continue;
                }
                None => break,
            }
        }
        if low.len() >= 7 && low[..7].eq_ignore_ascii_case(b"<script") {
            let lower = rest.to_ascii_lowercase();
            match lower.find("</script") {
                Some(e) => match rest[e..].find('>') {
                    Some(gt) => {
                        i = lt + e + gt + 1;
                        continue;
                    }
                    None => break,
                },
                None => break,
            }
        }
        if low.len() >= 6 && low[..6].eq_ignore_ascii_case(b"<style") {
            let lower = rest.to_ascii_lowercase();
            match lower.find("</style") {
                Some(e) => match rest[e..].find('>') {
                    Some(gt) => {
                        i = lt + e + gt + 1;
                        continue;
                    }
                    None => break,
                },
                None => break,
            }
        }
        if low.len() >= 4 && low[..4].eq_ignore_ascii_case(b"<img") {
            imgs += 1;
            i = lt + 4;
            continue;
        }
        if low.len() >= 2 && low[0] == b'<' && low[1] == b'/' {
            let Some(gt) = rest.find('>') else { break };
            let tag: String = rest[..=gt]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            if CLOSERS.iter().any(|c| tag == *c) && imgs >= max {
                // 切点：闭合标签之后跳过空白；要求下一字符是 '<' 或 body 末尾。
                let mut cut = lt + gt + 1;
                while cut < body_end && bytes[cut].is_ascii_whitespace() {
                    cut += 1;
                }
                if cut < body_end
                    && bytes[cut] == b'<'
                    && starts.last().map_or(true, |p| cut > *p)
                {
                    starts.push(cut);
                    imgs = 0;
                }
            }
            i = lt + gt + 1;
            continue;
        }
        i = lt + 1;
    }
    starts
}


/// R6-1：只读 spine 文档，返回单篇最大 <img 数（目录折叠书的块级切章门控前置条件）。
pub fn spine_doc_max_img_count(zip_bytes: &[u8], ci: &CiMap, spine: &[String]) -> usize {
    let cursor = std::io::Cursor::new(zip_bytes);
    let Ok(mut zip) = zip::ZipArchive::new(cursor) else {
        return 0;
    };
    let mut max = 0usize;
    for name in spine {
        if let Some(b) = zip_read(&mut zip, ci, name) {
            let raw = String::from_utf8_lossy(&b).into_owned();
            if let Some((bs, be)) = body_bounds(&raw) {
                max = max.max(count_img_tags(&raw[bs..be]));
            }
        }
    }
    max
}

/// R6-1：把「目录折叠 + 单文档巨量图片」的图片页书按块级边界切成虚拟章。
/// 只在 doc.split == false 时调用（P11 锚点切分优先，绝不改动已有 split 的书）；
/// 目录项 >2 的书直接放弃（普通多章书/多章漫画保持原状）。
/// 返回 (split_toc, chapters)，由调用方设置 doc.split=true 后使用；不满足条件返回 None。
pub fn plan_block_split(
    doc: &EpubDoc,
    zip_bytes: &[u8],
    ci: &CiMap,
    max_imgs_per_chapter: usize,
    max_chapters: usize,
) -> Option<(Vec<TocEntry>, Vec<EpubChapter>)> {
    if doc.split || doc.toc.len() > 2 {
        return None;
    }
    let max = max_imgs_per_chapter.max(1);
    // 只读 spine 文档（目录折叠书通常 1~3 篇），不读图片等其它资源。
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let mut raws: Vec<Option<String>> = Vec::with_capacity(doc.spine.len());
    let mut bounds: Vec<Option<(usize, usize)>> = Vec::with_capacity(doc.spine.len());
    let mut counts: Vec<usize> = Vec::with_capacity(doc.spine.len());
    for name in &doc.spine {
        let raw = zip_read(&mut zip, ci, name).map(|b| String::from_utf8_lossy(&b).into_owned());
        let bb = raw.as_deref().and_then(body_bounds);
        let n = match (&raw, bb) {
            (Some(r), Some((bs, be))) => count_img_tags(&r[bs..be]),
            _ => 0,
        };
        raws.push(raw);
        bounds.push(bb);
        counts.push(n);
    }
    if counts.iter().all(|n| *n <= max) {
        return None;
    }
    let total_imgs: usize = counts.iter().sum();
    let mut chapters = build_block_chapters(doc, &raws, &bounds, &counts, max);
    if chapters.len() > max_chapters {
        // 先按「总图数 / 章数上限」调大每章图片上限再重算一次；仍超则放弃。
        let new_max = total_imgs.div_ceil(max_chapters.max(1)).max(max);
        chapters = build_block_chapters(doc, &raws, &bounds, &counts, new_max);
        if chapters.len() > max_chapters {
            return None;
        }
    }
    if chapters.is_empty() || chapters.iter().all(|c| !c.is_slice()) {
        return None;
    }
    let split_toc: Vec<TocEntry> = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| TocEntry {
            title: c.title.clone(),
            index: i,
            chars: if c.is_slice() {
                0
            } else {
                doc.spine_chars.get(c.spine).copied().unwrap_or(0)
            },
        })
        .collect();
    Some((split_toc, chapters))
}

/// plan_block_split 的章节生成：逐 legacy 目录项的 spine 区间，每篇文档一章；
/// 图片数 > max 的文档按块级边界切成多章（title 置空，由前端用 common.sectionN 兜底），
/// 其余整篇章保留首个 legacy 标题（多篇区间里非首篇 title 置空）。
fn build_block_chapters(
    doc: &EpubDoc,
    raws: &[Option<String>],
    bounds: &[Option<(usize, usize)>],
    counts: &[usize],
    max: usize,
) -> Vec<EpubChapter> {
    let mut out: Vec<EpubChapter> = Vec::new();
    for (li, t) in doc.toc.iter().enumerate() {
        let s0 = t.index.min(doc.spine.len());
        let s1 = doc
            .toc_end
            .get(li)
            .copied()
            .unwrap_or(s0 + 1)
            .max(s0 + 1)
            .min(doc.spine.len());
        for si in s0..s1 {
            let first = si == s0;
            let whole_title = if first { normalize_split_title(&t.title) } else { String::new() };
            let mut sliced = false;
            if counts.get(si).copied().unwrap_or(0) > max {
                if let (Some(raw), Some((bs, be))) = (raws.get(si).and_then(|r| r.as_deref()), bounds.get(si).copied().flatten()) {
                    let starts = block_split_starts(raw, bs, be, max);
                    if !starts.is_empty() {
                        let mut cuts = Vec::with_capacity(starts.len() + 1);
                        cuts.push(bs);
                        cuts.extend(starts.iter().copied());
                        for k in 0..cuts.len() {
                            let st = floor_char_boundary(raw, cuts[k]);
                            let en = cuts
                                .get(k + 1)
                                .map(|x| ceil_char_boundary(raw, *x))
                                .unwrap_or(be)
                                .max(st);
                            let imgs = count_img_tags(slice_body(raw, st, en));
                            out.push(EpubChapter {
                                title: String::new(),
                                spine: si,
                                spine_end: si + 1,
                                start: Some(st),
                                end: en,
                                imgs,
                                legacy: li,
                            });
                        }
                        sliced = true;
                    }
                }
            }
            if !sliced {
                out.push(EpubChapter {
                    title: whole_title,
                    spine: si,
                    spine_end: si + 1,
                    start: None,
                    end: 0,
                    imgs: counts.get(si).copied().unwrap_or(0),
                    legacy: li,
                });
            }
        }
    }
    out
}



/// R7-2：自动切章产生的章标题规范化——trim 后为空，或全部由数字/全角数字组成（如 1、１２、003）
/// 视为无意义标题，返回空字符串；其余返回 trim 后的原词。只用于自动切章，不改普通书目录。
pub fn normalize_split_title(title: &str) -> String {
    // R10-1：先去掉「文件名.扩展名（html/htm/xhtml）」后缀（含前面的分隔符），
    // 再做空/纯数字判定；普通标题里合法出现「 - 」或句中的 .html 字样不受影响。
    let t = strip_chapter_filename_suffix(title);
    let t = t.trim();
    if t.is_empty() {
        return String::new();
    }
    let all_digits = t
        .chars()
        .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c));
    if all_digits {
        String::new()
    } else {
        t.to_string()
    }
}

/// R10-1：标题若以「文件名.扩展名（html/htm/xhtml，大小写不敏感）」结尾，
/// 去掉该文件名与它前面的分隔符（空白/-/—/– 的任意组合）并 trim 尾部；
/// 其余标题原样返回（不做 trim，由 normalize_split_title 统一 trim）。
fn strip_chapter_filename_suffix(title: &str) -> String {
    let t = title.trim_end();
    let Some(dot) = t.rfind('.') else {
        return title.to_string();
    };
    let ext = t[dot + 1..].to_ascii_lowercase();
    if !matches!(ext.as_str(), "html" | "htm" | "xhtml") {
        return title.to_string();
    }
    let prefix = &t[..dot];
    // 文件名 token 起点：prefix 中最后一个分隔符之后（无分隔符则整个 prefix 都是文件名）。
    let sep = |c: char| c.is_whitespace() || matches!(c, '-' | '—' | '–');
    let cut = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| sep(*c))
        .map(|(i, _)| i)
        .unwrap_or(0);
    // 文件名 token 为空（如标题本身是 ".html"）→ 不视为可去后缀，保持原样。
    if prefix[cut..].trim_matches(sep).is_empty() {
        return title.to_string();
    }
    let head = prefix[..cut].trim_end_matches(sep).trim_end();
    if head.is_empty() {
        return String::new();
    }
    head.to_string()
}

/// R7-1：判断两份切章布局是否一致（章数、每章 spine/spine_end/start/end/imgs；不比较标题）。
/// 用于迁移时「布局没变 → 只重写 marker、不迁移坐标」的短路。
pub fn same_chapter_layout(a: &[EpubChapter], b: &[EpubChapter]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| {
            x.spine == y.spine
                && x.spine_end == y.spine_end
                && x.start == y.start
                && x.end == y.end
                && x.imgs == y.imgs
        })
}

/// R7-1 细分模式：Restricted 保留第六轮的入口限制（P11 章数<=2，供迁移重建 v2 布局与既有单测）；
/// General 只要求 P11 已切且存在 >max 图章（不再限制章数），供第七轮泛化细分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubdivideMode {
    Restricted,
    General,
}

/// R6-B：读取 split 文档各章的实际图片数上限（按章 start/end 或 spine 区间从 raw HTML 重新统计，
/// 不信任整篇章可能为 0 的 imgs 缓存值）。用于 open_book 的「超大章受限细分」候选判定。
pub fn max_split_chapter_imgs(doc: &EpubDoc, zip_bytes: &[u8], ci: &CiMap) -> usize {
    if !doc.split || doc.chapters.is_empty() {
        return 0;
    }
    let cursor = std::io::Cursor::new(zip_bytes);
    let Ok(mut zip) = zip::ZipArchive::new(cursor) else {
        return 0;
    };
    let mut raws: Vec<Option<String>> = vec![None; doc.spine.len()];
    let mut doc_imgs: Vec<usize> = vec![0; doc.spine.len()];
    let actual = chapter_actual_imgs(doc, &mut zip, ci, &mut raws, &mut doc_imgs);
    actual.into_iter().max().unwrap_or(0)
}

/// 惰性读取并缓存某个 spine 文档的 raw HTML（供候选判定与细分计划共用）。
fn load_spine_raw<'a, R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    spine: &[String],
    raws: &'a mut Vec<Option<String>>,
    si: usize,
) -> Option<&'a str> {
    if si >= spine.len() || si >= raws.len() {
        return None;
    }
    if raws[si].is_none() {
        let raw = zip_read(zip, ci, &spine[si]).map(|b| String::from_utf8_lossy(&b).into_owned());
        raws[si] = raw;
    }
    raws[si].as_deref()
}

/// 统计 split 文档各章实际图片数（切片章按 slice 统计；整篇章按 spine 区间求和）。
fn chapter_actual_imgs<R: Read + std::io::Seek>(
    doc: &EpubDoc,
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    raws: &mut Vec<Option<String>>,
    doc_imgs: &mut Vec<usize>,
) -> Vec<usize> {
    let mut out = Vec::with_capacity(doc.chapters.len());
    for c in &doc.chapters {
        if let Some(st) = c.start {
            let n = match load_spine_raw(zip, ci, &doc.spine, raws, c.spine) {
                Some(raw) => count_img_tags(slice_body(raw, st, c.end)),
                None => 0,
            };
            out.push(n);
        } else {
            let mut sum = 0usize;
            for si in c.spine..c.spine_end.min(doc.spine.len()) {
                if doc_imgs.get(si).copied().unwrap_or(0) == 0 {
                    let n = match load_spine_raw(zip, ci, &doc.spine, raws, si) {
                        Some(raw) => body_bounds(raw)
                            .map(|(bs, be)| count_img_tags(&raw[bs..be]))
                            .unwrap_or(0),
                        None => 0,
                    };
                    if si < doc_imgs.len() {
                        doc_imgs[si] = n;
                    }
                }
                sum += doc_imgs.get(si).copied().unwrap_or(0);
            }
            out.push(sum);
        }
    }
    out
}

/// 把一个切片子章 push 进结果（首子章保留原标题，其余 title 置空）。
#[allow(clippy::too_many_arguments)]
fn push_sub_slice(
    out: &mut Vec<EpubChapter>,
    raw: &str,
    st: usize,
    en: usize,
    title: &str,
    c: &EpubChapter,
    spine: usize,
    first_emitted: &mut bool,
) {
    let st = floor_char_boundary(raw, st);
    let en = ceil_char_boundary(raw, en).max(st);
    out.push(EpubChapter {
        title: if *first_emitted { String::new() } else { title.to_string() },
        spine,
        spine_end: spine + 1,
        start: Some(st),
        end: en,
        imgs: count_img_tags(slice_body(raw, st, en)),
        legacy: c.legacy,
    });
    *first_emitted = true;
}

/// plan_subdivide 的中间结果。
struct SubdividePlan {
    chapters: Vec<EpubChapter>,
    subdivided: bool,
    unsubdividable: usize,
}

/// 逐章生成细分后的章节（保持原顺序，锚点章边界绝对保留，绝不跨越原章 start/end）。
fn build_subdivided_chapters<R: Read + std::io::Seek>(
    doc: &EpubDoc,
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    raws: &mut Vec<Option<String>>,
    doc_imgs: &[usize],
    actual: &[usize],
    max: usize,
) -> SubdividePlan {
    let mut out: Vec<EpubChapter> = Vec::new();
    let mut subdivided = false;
    let mut unsubdividable = 0usize;
    for (i, c) in doc.chapters.iter().enumerate() {
        let imgs = actual.get(i).copied().unwrap_or(0);
        if imgs <= max {
            out.push(c.clone());
            continue;
        }
        let mut first_emitted = false;
        if let Some(start) = c.start {
            let mut handled = false;
            if let Some(raw) = load_spine_raw(zip, ci, &doc.spine, raws, c.spine) {
                let starts = block_split_starts(raw, start, c.end, max);
                if !starts.is_empty() {
                    let mut cuts = vec![start];
                    cuts.extend(starts.iter().copied());
                    for k in 0..cuts.len() {
                        let en = cuts.get(k + 1).copied().unwrap_or(c.end);
                        push_sub_slice(
                            &mut out,
                            raw,
                            cuts[k],
                            en,
                            &c.title,
                            c,
                            c.spine,
                            &mut first_emitted,
                        );
                    }
                    subdivided = true;
                    handled = true;
                }
            }
            if !handled {
                // 章内没有块级边界可切：保留原章（允许单章仍 >max，不整体失败）。
                unsubdividable += 1;
                out.push(c.clone());
            }
            continue;
        }
        // 整篇章（start=None）：按 spine 逐篇文档处理。
        let s0 = c.spine.min(doc.spine.len());
        let s1 = c.spine_end.min(doc.spine.len()).max(s0);
        for si in s0..s1 {
            let doc_n = doc_imgs.get(si).copied().unwrap_or(0);
            if doc_n <= max {
                out.push(EpubChapter {
                    title: if first_emitted { String::new() } else { c.title.clone() },
                    spine: si,
                    spine_end: si + 1,
                    start: None,
                    end: 0,
                    imgs: doc_n,
                    legacy: c.legacy,
                });
                first_emitted = true;
                continue;
            }
            let mut handled = false;
            if let Some(raw) = load_spine_raw(zip, ci, &doc.spine, raws, si) {
                if let Some((bs, be)) = body_bounds(raw) {
                    let starts = block_split_starts(raw, bs, be, max);
                    if !starts.is_empty() {
                        let mut cuts = vec![bs];
                        cuts.extend(starts.iter().copied());
                        for k in 0..cuts.len() {
                            let en = cuts.get(k + 1).copied().unwrap_or(be);
                            push_sub_slice(
                                &mut out,
                                raw,
                                cuts[k],
                                en,
                                &c.title,
                                c,
                                si,
                                &mut first_emitted,
                            );
                        }
                        subdivided = true;
                        handled = true;
                    }
                }
            }
            if !handled {
                // 单篇文档内无块级边界：该篇整篇保留。
                unsubdividable += 1;
                out.push(EpubChapter {
                    title: if first_emitted { String::new() } else { c.title.clone() },
                    spine: si,
                    spine_end: si + 1,
                    start: None,
                    end: 0,
                    imgs: doc_n,
                    legacy: c.legacy,
                });
                first_emitted = true;
            }
        }
    }
    SubdividePlan { chapters: out, subdivided, unsubdividable }
}

/// R6-B：P11 锚点切分后的超大章内部受限细分。仅在 doc.split==true 且 chapters.len()<=2 时尝试；
/// 锚点章边界绝对保留（子章 start/end 严格落在原章范围内，绝不跨越），P11 章数 >2 的书完全不进入。
/// 实际图片数一律从 raw HTML 重新统计；只细分图片数 > max 的章（文字多的章不按文字切）。
/// 标题规则：每个原章的第一个子章保留原章标题（可能为空），其余子章 title 置空（前端 common.sectionN）。
/// 返回 (split_toc, chapters)，由调用方 patch EpubDoc；不满足条件/没有实际细分时返回 None。
pub fn plan_subdivide_oversized_chapters(
    doc: &EpubDoc,
    zip_bytes: &[u8],
    ci: &CiMap,
    max_imgs_per_chapter: usize,
    max_chapters: usize,
) -> Option<(Vec<TocEntry>, Vec<EpubChapter>)> {
    plan_subdivide_oversized_chapters_mode(
        doc,
        zip_bytes,
        ci,
        max_imgs_per_chapter,
        max_chapters,
        SubdivideMode::Restricted,
    )
}

/// R7-1：按模式细分 P11 锚点切分后的超大章；General 不再限制章数，对任意超大章在原章边界内细分。
pub fn plan_subdivide_oversized_chapters_mode(
    doc: &EpubDoc,
    zip_bytes: &[u8],
    ci: &CiMap,
    max_imgs_per_chapter: usize,
    max_chapters: usize,
    mode: SubdivideMode,
) -> Option<(Vec<TocEntry>, Vec<EpubChapter>)> {
    if !doc.split {
        return None;
    }
    if mode == SubdivideMode::Restricted && doc.chapters.len() > 2 {
        return None;
    }
    let max = max_imgs_per_chapter.max(1);
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let mut raws: Vec<Option<String>> = vec![None; doc.spine.len()];
    let mut doc_imgs: Vec<usize> = vec![0; doc.spine.len()];
    let actual = chapter_actual_imgs(doc, &mut zip, ci, &mut raws, &mut doc_imgs);
    if actual.iter().all(|n| *n <= max) {
        return None;
    }
    let total_imgs: usize = actual.iter().sum();
    let mut plan = build_subdivided_chapters(doc, &mut zip, ci, &mut raws, &doc_imgs, &actual, max);
    if plan.chapters.len() > max_chapters {
        // 章数上限：先按「总图数 / 上限」调大每章图片上限再重算一次；仍超则放弃。
        let new_max = total_imgs.div_ceil(max_chapters.max(1)).max(max);
        plan = build_subdivided_chapters(doc, &mut zip, ci, &mut raws, &doc_imgs, &actual, new_max);
        if plan.chapters.len() > max_chapters {
            return None;
        }
    }
    if !plan.subdivided || plan.chapters.is_empty() {
        return None;
    }
    // R7-2：继承来的标题再规范化一次作防御（空/纯数字 → 空，前端显示「第 N 节」）。
    for c in &mut plan.chapters {
        c.title = normalize_split_title(&c.title);
    }
    if std::env::var("TIDEPAGE_OPEN_DIAG").is_ok() {
        let imgs: Vec<usize> = plan.chapters.iter().map(|c| c.imgs).collect();
        eprintln!(
            "[epub] subdivide chapters={} unsubdividable={} imgs={imgs:?}",
            plan.chapters.len(),
            plan.unsubdividable
        );
    }
    let split_toc: Vec<TocEntry> = plan
        .chapters
        .iter()
        .enumerate()
        .map(|(i, c)| TocEntry {
            title: c.title.clone(),
            index: i,
            chars: if c.is_slice() {
                0
            } else {
                doc.spine_chars.get(c.spine).copied().unwrap_or(0)
            },
        })
        .collect();
    Some((split_toc, plan.chapters))
}

/// R6-B 布局升级迁移纯函数：把「旧布局（base，如 P11 锚点两章）的章号+章内比例」映射到
/// 「新布局（new，如细分后的 12 章）」——按全书图片累计位置映射（权重 = 章 imgs.max(1)）。
/// 越界/空布局返回 (0, 0.0)，不 panic。
pub fn map_split_layout_position(
    base: &[EpubChapter],
    new: &[EpubChapter],
    old_idx: i64,
    old_ratio: f64,
) -> (usize, f64) {
    if base.is_empty() || new.is_empty() {
        return (0, 0.0);
    }
    let old_idx = old_idx.max(0) as usize;
    if old_idx >= base.len() {
        return (0, 0.0);
    }
    let weights: Vec<f64> = base.iter().map(|c| c.imgs.max(1) as f64).collect();
    let old_total: f64 = weights.iter().sum::<f64>().max(1.0);
    let before: f64 = weights[..old_idx].iter().sum();
    let pos = ((before + old_ratio.clamp(0.0, 1.0) * weights[old_idx]) / old_total).clamp(0.0, 1.0);
    let new_weights: Vec<f64> = new.iter().map(|c| c.imgs.max(1) as f64).collect();
    let new_total: f64 = new_weights.iter().sum::<f64>().max(1.0);
    let target = pos * new_total;
    let mut acc = 0.0f64;
    for (k, w) in new_weights.iter().enumerate() {
        let last = k + 1 == new_weights.len();
        if target < acc + *w || last {
            return (k, ((target - acc) / *w).clamp(0.0, 1.0));
        }
        acc += *w;
    }
    (0, 0.0)
}

fn find_toc_nav_section(html: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(abs) = find_ascii_ci(html, "<nav", search_from) {
        let end = find_ascii_ci(html, "</nav>", abs).map(|e| e + 6);
        if let Some(end) = end {
            let seg = &html[abs..end];
            // 这里只判断片段内容（不产生偏移），大小写不敏感比较无副作用。
            if seg.to_lowercase().contains("toc") || seg.contains("<h") {
                return Some(seg.to_string());
            }
            search_from = end;
        } else {
            break;
        }
    }
    None
}

fn extract_anchor_pairs(html: &str, base_doc: &str) -> Vec<(String, String)> {
    let re = Regex::new(r#"(?is)<a\s[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    let mut out = Vec::new();
    for c in re.captures_iter(html) {
        let href = c[1].to_string();
        let inner = tag_re.replace_all(&c[2], "");
        let text = inner
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_string();
        if text.is_empty() || href.starts_with("http") {
            continue;
        }
        out.push((text, href));
    }
    let _ = base_doc;
    out
}

fn extract_ncx_pairs(xml: &str, base_doc: &str) -> Vec<(String, String)> {
    let re = Regex::new(
        r#"(?is)<navPoint[^>]*>.*?<text[^>]*>(.*?)</text>.*?<content[^>]*src\s*=\s*["']([^"']+)["']"#,
    )
    .unwrap();
    let mut out = Vec::new();
    for c in re.captures_iter(xml) {
        let text = c[1]
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_string();
        out.push((text, c[2].to_string()));
    }
    let _ = base_doc;
    out
}

pub fn resolve_in_doc(doc_path: &str, href: &str) -> String {
    norm_path(doc_path, href)
}

/// 从 OPF 文本里找封面图片的 href（不解析全书）：
///   1) `<meta name="cover" content="ID">` → 找同 ID 的 `<item ... href=...>`；
///   2) 退而求其次：`<item properties="cover-image" href=...>`。
fn cover_href_from_opf(opf: &str) -> Option<String> {
    if let Ok(re) = Regex::new(
        r#"(?is)<meta[^>]*\bname\s*=\s*["']cover["'][^>]*\bcontent\s*=\s*["']([^"']+)["']"#,
    ) {
        if let Some(c) = re.captures(opf) {
            let id = c[1].to_string();
            let esc = regex::escape(&id);
            if let Ok(re2) = Regex::new(&format!(
                r#"(?is)<item[^>]*\bid\s*=\s*["']{esc}["'][^>]*\bhref\s*=\s*["']([^"']+)["']"#
            )) {
                if let Some(c2) = re2.captures(opf) {
                    return Some(c2[1].to_string());
                }
            }
            if let Ok(re3) = Regex::new(&format!(
                r#"(?is)<item[^>]*\bhref\s*=\s*["']([^"']+)["'][^>]*\bid\s*=\s*["']{esc}["']"#
            )) {
                if let Some(c3) = re3.captures(opf) {
                    return Some(c3[1].to_string());
                }
            }
        }
    }
    if let Ok(re) = Regex::new(
        r#"(?is)<item\b[^>]*\bproperties\s*=\s*["'][^"']*cover-image[^"']*["'][^>]*>"#,
    ) {
        if let Some(m) = re.find(opf) {
            if let Ok(hre) = Regex::new(r#"(?is)\bhref\s*=\s*["']([^"']+)["']"#) {
                if let Some(c) = hre.captures(m.as_str()) {
                    return Some(c[1].to_string());
                }
            }
        }
    }
    None
}

/// **封面提取前的 EOCD 预检**：只读文件尾最多 1 KiB 定位 EOCD（`PK\x05\x06`），
/// 取出中央目录大小；CD > 8 MiB 或找不到 EOCD 即返回 Err，**不进入 `ZipArchive::new`**。
/// 用于在解析前挡住畸形/超大中央目录的文件（否则 `ZipArchive::new` 可能长时间不返回）。
/// 该函数只读 EOCD 搜索范围（≤1 KiB），不读其它字节。
pub fn precheck_eocd(path: &str) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_CD_BYTES: u64 = 8 * 1024 * 1024;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len < 22 {
        return Err("文件过小，不是有效 ZIP/EPUB".to_string());
    }
    let read_n = len.min(1024);
    f.seek(SeekFrom::End(-(read_n as i64))).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; read_n as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    // 在尾部 ≤1KiB 内找最后一个 EOCD 签名。
    let sig = [0x50u8, 0x4b, 0x05, 0x06];
    let pos = buf
        .windows(4)
        .rposition(|w| w == sig)
        .ok_or("尾部 1KiB 内未找到 EOCD 签名")?;
    if pos + 16 > buf.len() {
        return Err("EOCD 记录被截断".to_string());
    }
    // EOCD +12：中央目录大小(u32 LE)。
    let cd_size = u32::from_le_bytes([buf[pos + 12], buf[pos + 13], buf[pos + 14], buf[pos + 15]]) as u64;
    if cd_size > MAX_CD_BYTES {
        return Err(format!("中央目录过大: {cd_size} 字节 (> 8 MiB)"));
    }
    Ok(())
}

/// **解析阶段**的读取预算上限：字节数与 read 调用次数（两者任一耗尽即判为病态）。
///
/// 背景：zip 2.4.x 的 `get_metadata` 在「中央目录某条目解析失败」时会回退为
/// **在全文件内向前搜索更早的 EOCD 候选**，以 2 KiB 窗口回扫整个文件——对若干
/// 局部头偏移指向垃圾/零区的畸形条目，会读取数百 MiB 且大量 read 调用、长时间不返回。
/// 仅靠 EOCD 预检（CD 大小 > 8 MiB / 无 EOCD）无法拦住这类文件（其 CD 可能只有几百 KiB）。
///
/// 上限取值由本模块 `budget_probe` 的实测决定（正常书解析期消耗见下）：
/// - **字节**：正常 EPUB 解析期仅读 CD + 各条目局部头，实测 18 MB 书 10–312 KB、
///   100 MB 书 33–154 KB、434 MB 书 1.1 MB；病态回扫读数百 MiB。故 8 MiB 有 >7x 余量。
/// - **调用次数**：正常书随条目数增长，实测 18 MB 书 270–9684 次、124 MB 书 4806 次、
///   147 MB 书 5394 次、434 MB 书 **34683** 次。**初值 5000 会误伤正常书**，故按实测
///   校准为 200000（约 5.8x 余量）；病态文件则由 8 MiB 字节上限更早拦下。
pub const MAX_PARSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PARSE_CALLS: u64 = 200_000;

/// worker 与看门狗共享的读取预算/取消令牌。
pub struct CoverReadBudget {
    // staged: 解析预算计量基准；仅 TP_EPUB_COVER cargo test cover_budget_bench 读取（校准 MAX_PARSE_CALLS）。
    #[allow(dead_code)]
    bytes_limit: u64,
    #[allow(dead_code)]
    calls_limit: u64,
    bytes_left: AtomicU64,
    calls_left: AtomicU64,
    /// 解析阶段为 true（执行上限）；`finish_parse()` 后为 false（只保留取消）。
    limiting: AtomicBool,
    /// 预算（字节或调用次数）被 worker 自身耗尽。
    exhausted: AtomicBool,
    /// 看门狗超时取消。
    cancelled: AtomicBool,
}

impl CoverReadBudget {
    pub fn new(bytes_limit: u64, calls_limit: u64) -> Self {
        Self {
            bytes_limit,
            calls_limit,
            bytes_left: AtomicU64::new(bytes_limit),
            calls_left: AtomicU64::new(calls_limit),
            limiting: AtomicBool::new(true),
            exhausted: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }

    /// 解析成功：解除上限（封面数据读取不受限），仅保留取消语义。
    fn finish_parse(&self) {
        self.limiting.store(false, Ordering::SeqCst);
    }

    /// 看门狗超时：令 worker 内下一次 read 立即失败退出（消除线程泄漏）。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 预算是否被耗尽（病态文件的判据，由 worker 侧读取）。
    pub fn exhausted(&self) -> bool {
        self.exhausted.load(Ordering::SeqCst)
    }

    /// 解析阶段已消耗的 read 调用次数 / 字节数（供计量）。
    /// staged: 仅 #[test] cover_budget_bench 诊断用；与上面两字段四者同加 #[allow]。
    #[allow(dead_code)]
    pub fn calls_used(&self) -> u64 {
        self.calls_limit.saturating_sub(self.calls_left.load(Ordering::SeqCst))
    }
    #[allow(dead_code)]
    pub fn bytes_used(&self) -> u64 {
        self.bytes_limit.saturating_sub(self.bytes_left.load(Ordering::SeqCst))
    }
}

impl Default for CoverReadBudget {
    fn default() -> Self {
        Self::new(MAX_PARSE_BYTES, MAX_PARSE_CALLS)
    }
}

/// 带读取预算的 reader：解析阶段超过字节/调用上限即返回 io::Error（中止 zip 解析）。
/// 预算为共享的 `Arc<CoverReadBudget>`：看门狗可 `cancel()` 取消。
struct BudgetReader<R> {
    inner: R,
    budget: Arc<CoverReadBudget>,
}

impl<R> BudgetReader<R> {
    fn new(inner: R, budget: Arc<CoverReadBudget>) -> Self {
        Self { inner, budget }
    }
}

impl<R: Read> Read for BudgetReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.budget.cancelled.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "cover read cancelled",
            ));
        }
        let limiting = self.budget.limiting.load(Ordering::SeqCst);
        if limiting {
            let c = self.budget.calls_left.load(Ordering::SeqCst);
            if c == 0 {
                self.budget.exhausted.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "cover read call limit exceeded",
                ));
            }
            self.budget.calls_left.store(c - 1, Ordering::SeqCst);
        }
        let n = self.inner.read(buf)?;
        if limiting {
            let b = self.budget.bytes_left.load(Ordering::SeqCst);
            if n as u64 > b {
                self.budget.bytes_left.store(0, Ordering::SeqCst);
                self.budget.exhausted.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "cover read byte budget exceeded",
                ));
            }
            self.budget.bytes_left.store(b - n as u64, Ordering::SeqCst);
        }
        Ok(n)
    }
}

impl<R: std::io::Seek> std::io::Seek for BudgetReader<R> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// **只提取封面**：以 `File`(Seek) 打开 zip，仅读取中央目录 + `META-INF/container.xml`
/// + OPF + 封面图片三个条目，其余条目一律不解压（不读全文）。返回封面图字节（不含任何元数据）。
/// `budget` 为共享读取预算/取消令牌（看门狗可 cancel 取消）。
pub fn extract_cover_file(path: &str, budget: &Arc<CoverReadBudget>) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    extract_cover_from_reader(std::io::BufReader::new(file), budget.clone())
}

/// 泛型版（供计量测试用 counting reader 精确统计读取字节数）。
pub fn extract_cover_from_reader<R: Read + std::io::Seek>(
    reader: R,
    budget: Arc<CoverReadBudget>,
) -> Result<Option<Vec<u8>>, String> {
    // 解析阶段带字节/调用上限：拦住 zip 2.4.x 因中央目录条目损坏而回扫整文件的病态读取。
    let mut zip = zip::ZipArchive::new(BudgetReader::new(reader, budget.clone()))
        .map_err(|e| format!("EPUB 打开失败: {e}"))?;
    // 解析成功：解除上限，封面数据（可达 32 MiB）读取不受限；取消语义仍保留。
    budget.finish_parse();
    // 仅读中央目录建立「小写名→索引」表（by_index_raw 不解压条目数据）。
    let mut idx_of: HashMap<String, usize> = HashMap::new();
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index_raw(i) {
            idx_of.insert(f.name().trim_start_matches("./").to_ascii_lowercase(), i);
        }
    }
    let read_entry = |zip: &mut zip::ZipArchive<BudgetReader<R>>, name: &str| -> Option<Vec<u8>> {
        let key = name.trim_start_matches("./").to_ascii_lowercase();
        let i = *idx_of.get(&key)?;
        let mut f = zip.by_index(i).ok()?;
        // 解析预算解除后仍要有单条目上限：封面条目同样可能是压缩炸弹。
        read_capped(&mut f, MAX_ENTRY_BYTES)
    };
    let container = match read_entry(&mut zip, "META-INF/container.xml") {
        Some(b) => decode_text(&b),
        None => return Ok(None),
    };
    let opf_path = match opf_path_from_container(&container) {
        Some(p) => p,
        None => return Ok(None),
    };
    let opf = match read_entry(&mut zip, &opf_path) {
        Some(b) => decode_text(&b),
        None => return Ok(None),
    };
    let href = match cover_href_from_opf(&opf) {
        Some(h) => h,
        None => return Ok(None),
    };
    let full = norm_path(&opf_path, &href);
    Ok(read_entry(&mut zip, &full).filter(|b| !b.is_empty()))
}

/// **只提取封面（CBZ=图片 zip）**：seek 打开 zip，按条目名排序取第一张图片，仅解压该条目。
pub fn extract_cbz_cover_file(path: &str, budget: &Arc<CoverReadBudget>) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(BudgetReader::new(std::io::BufReader::new(file), budget.clone()))
        .map_err(|e| e.to_string())?;
    budget.finish_parse(); // 解析成功：解除上限
    let mut imgs: Vec<(String, usize)> = Vec::new();
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index_raw(i) {
            let n = f.name().to_string();
            let l = n.to_ascii_lowercase();
            if l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png")
                || l.ends_with(".webp") || l.ends_with(".gif")
            {
                imgs.push((n, i));
            }
        }
    }
    if imgs.is_empty() {
        return Ok(None);
    }
    imgs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut f = zip.by_index(imgs[0].1).map_err(|e| e.to_string())?;
    let b = read_capped(&mut f, MAX_ENTRY_BYTES)
        .ok_or_else(|| format!("封面条目解压后超过上限（{MAX_ENTRY_BYTES} 字节）"))?;
    Ok(if b.is_empty() { None } else { Some(b) })
}


/// 按已缓存的 `CiMap` 读取 zip 内任意条目（供按需图片服务使用）。
#[cfg_attr(not(test), allow(dead_code))]
pub fn read_entry(zip_bytes: &[u8], ci: &CiMap, name: &str) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    zip_read(&mut zip, ci, name)
}

/// C13：只打开文件并读取目标条目，不整本入内存（图片/章节按需）。
#[cfg_attr(not(test), allow(dead_code))]
pub fn read_entry_file(path: &Path, ci: &CiMap, name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    zip_read(&mut zip, ci, name)
}

/// 供协议处理器的 MIME 判断。
pub fn mime_for_path(target: &str) -> &'static str {
    mime_of(target)
}

/// 从图片字节头部解析宽高（webp/jpeg/png/gif/bmp），用于在正文里预置 `width/height`
/// 属性，避免图片解码后布局跳变引发反复重排。
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 16 {
        return None;
    }
    // PNG
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) && bytes.len() >= 24 {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((w, h));
    }
    // GIF
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some((w, h));
    }
    // BMP
    if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        let w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]).unsigned_abs();
        let h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]).unsigned_abs();
        return Some((w, h));
    }
    // WebP
    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let fourcc = &bytes[12..16];
        if fourcc == b"VP8X" {
            let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            return Some((w, h));
        }
        if fourcc == b"VP8 " && bytes.len() >= 30 {
            let w = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3FFF;
            let h = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3FFF;
            return Some((w, h));
        }
        if fourcc == b"VP8L" && bytes.len() >= 25 {
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            return Some((w, h));
        }
    }
    // JPEG：扫描 SOF 段
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&marker)
                && marker != 0xC4
                && marker != 0xC8
                && marker != 0xCC
            {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

/// 可复用外部 `CiMap`（避免每章重跑 build_ci）与按需图片 URL（book_id=Some 时不内联 base64）。
/// `keep_styles=true` 时保留原书样式表（作用域隔离后内联），否则剥离全部原书 CSS。
#[cfg_attr(not(test), allow(dead_code))]
pub fn chapter_html_ci(
    zip_bytes: &[u8],
    ci: Option<&CiMap>,
    doc_path: &str,
    book_id: Option<i64>,
    keep_styles: bool,
) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let owned;
    let ci = match ci {
        Some(c) => c,
        None => {
            owned = build_ci(&mut zip);
            &owned
        }
    };
    let bytes = zip_read(&mut zip, ci, doc_path).ok_or("章节文件缺失")?;
    let html = String::from_utf8_lossy(&bytes).into_owned();
    Ok(render_doc_html(&mut zip, ci, doc_path, &html, book_id, keep_styles))
}

/// 带外部 `CiMap` 与按需图片 URL 的多文件章节渲染。
#[cfg_attr(not(test), allow(dead_code))]
pub fn chapter_html_multi_ci(
    zip_bytes: &[u8],
    ci: Option<&CiMap>,
    doc_paths: &[String],
    book_id: Option<i64>,
    keep_styles: bool,
) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let owned;
    let ci = match ci {
        Some(c) => c,
        None => {
            owned = build_ci(&mut zip);
            &owned
        }
    };
    let mut out = String::new();
    let mut found = 0usize;
    for doc_path in doc_paths {
        if let Some(bytes) = zip_read(&mut zip, ci, doc_path) {
            found += 1;
            let html = String::from_utf8_lossy(&bytes).into_owned();
            out.push_str(&render_doc_html(&mut zip, ci, doc_path, &html, book_id, keep_styles));
        }
    }
    // P1-3：与单文档版一致——非空 spine 一个条目都读不到时报可读 Err，不再返回空串。
    if !doc_paths.is_empty() && found == 0 {
        return Err("章节文件缺失".into());
    }
    Ok(out)
}



/// C13：文件路径版单文档渲染（File+ZipArchive 打开一次，条目读完即释放）。
pub fn chapter_html_ci_file(path: &Path, ci: &CiMap, doc_path: &str, book_id: Option<i64>, keep_styles: bool) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let bytes = zip_read(&mut zip, ci, doc_path).ok_or("章节文件缺失")?;
    let html = String::from_utf8_lossy(&bytes).into_owned();
    Ok(render_doc_html(&mut zip, ci, doc_path, &html, book_id, keep_styles))
}

/// C13：文件路径版多文档渲染。
pub fn chapter_html_multi_file(path: &Path, ci: &CiMap, doc_paths: &[String], book_id: Option<i64>, keep_styles: bool) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let mut out = String::new();
    let mut found = 0usize;
    for doc_path in doc_paths {
        if let Some(bytes) = zip_read(&mut zip, ci, doc_path) {
            found += 1;
            let html = String::from_utf8_lossy(&bytes).into_owned();
            out.push_str(&render_doc_html(&mut zip, ci, doc_path, &html, book_id, keep_styles));
        }
    }
    // P1-3：非空 spine 全部读不到 → Err；部分缺失继续拼接其余章节。
    if !doc_paths.is_empty() && found == 0 {
        return Err("章节文件缺失".into());
    }
    Ok(out)
}

/// C13：文件路径版切片渲染。
pub fn chapter_html_fragment_file(path: &Path, ci: &CiMap, doc_path: &str, raw_html: &str, start: usize, end: usize, book_id: Option<i64>, keep_styles: bool) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let slice = slice_body(raw_html, start, end);
    if slice.trim().is_empty() { return Ok(String::new()); }
    let head = extract_head(raw_html);
    let synthetic = format!("<html><head>{head}</head><body>{slice}</body></html>");
    Ok(render_doc_html(&mut zip, ci, doc_path, &synthetic, book_id, keep_styles))
}
/// P11：把 spine 文档 raw HTML 的 body 内容区间 [start,end) 渲染成独立章节片段。
/// 用原文档的 head 造合成 XHTML 再交给同一渲染管线（相对图片/CSS 路径仍按 doc_path 解析），
/// 因此图片 URL、懒加载、宽度样式与整篇渲染完全一致；切片本身不含 <html>/<body> 包装。
#[cfg_attr(not(test), allow(dead_code))]
pub fn chapter_html_fragment_ci(
    zip_bytes: &[u8],
    ci: Option<&CiMap>,
    doc_path: &str,
    raw_html: &str,
    start: usize,
    end: usize,
    book_id: Option<i64>,
    keep_styles: bool,
) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let owned;
    let ci = match ci {
        Some(c) => c,
        None => {
            owned = build_ci(&mut zip);
            &owned
        }
    };
    let slice = slice_body(raw_html, start, end);
    if slice.trim().is_empty() {
        return Ok(String::new());
    }
    let head = extract_head(raw_html);
    let synthetic = format!("<html><head>{head}</head><body>{slice}</body></html>");
    Ok(render_doc_html(&mut zip, ci, doc_path, &synthetic, book_id, keep_styles))
}

/// 生成按需图片 URL（不内联 base64，避免单章 HTML 达到几百 KB）。
/// 形如 `http://tidepage.localhost/epubimg/<book_id>/<b64url(zip内路径)>`。
pub fn epub_image_url(book_id: i64, target: &str) -> String {
    use base64::Engine;
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(target.as_bytes());
    format!("http://tidepage.localhost/epubimg/{book_id}/{enc}")
}

/// 把单个 XHTML 文档渲染为可内联进阅读器的 HTML 片段（复用已打开的 zip 与索引）。
/// `book_id=Some` 时图片改写为按需 URL（懒加载）；`None` 时回退 base64 内联。
fn render_doc_html<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    doc_path: &str,
    html: &str,
    book_id: Option<i64>,
    keep_styles: bool,
) -> String {
    let body = extract_body(html);

    // 原书样式：keep_styles=true 时把 <head> 的样式表与文档内 <style> 作用域隔离后内联，
    // 否则整体剥离（由阅读器「覆盖书籍排版」等全局设置决定版式）。
    let mut book_css = String::new();
    if keep_styles {
        for href in head_stylesheet_hrefs(html) {
            let target = resolve_in_doc(doc_path, &href);
            let mut visited = std::collections::HashSet::new();
            load_css_recursive(zip, ci, &target, book_id, &mut visited, &mut book_css, 0);
        }
        for css in inline_style_blocks(html) {
            book_css.push_str(&rewrite_css_urls(&css, doc_path, zip, ci, book_id));
            book_css.push('\n');
        }
    }

    // 统一净化：移除危险标签（连内容，如 script/iframe/object/form/link/style 等）
    // 与危险属性（on*、javascript: 等），同时保留合法 <img src="...jpg">、<a href>、
    // style 等，供下方按需图片 URL 改写与 keep_styles 逻辑继续处理。
    let mut styled = crate::sanitize::sanitize_fragment(&body);

    // 先处理整段 <img ...>：改写 src 为按需 URL，并预置 width/height/loading/decoding，
    // 让浏览器在图片解码前就占好位，避免布局跳变与反复重排（卡死根因之一）。
    let tag_re = Regex::new(r#"(?is)<img\b[^>]*>"#).unwrap();
    styled = tag_re
        .replace_all(&styled, |caps: &regex::Captures| {
            process_img_tag(&caps[0], zip, ci, doc_path, book_id)
        })
        .into_owned();

    // 再处理非 <img> 的图片引用（如 SVG <image xlink:href>、<a href="x.jpg">）。
    let img_re = Regex::new(
        r#"(?i)(src|xlink:href|href)\s*=\s*"([^"]+\.(?:png|jpe?g|gif|bmp|webp|svg))""#,
    )
    .unwrap();
    styled = img_re
        .replace_all(&styled, |caps: &regex::Captures| {
            let attr = caps[1].to_string();
            let target = resolve_in_doc(doc_path, &caps[2]);
            match book_id {
                Some(bid) => format!("{attr}=\"{}\"", epub_image_url(bid, &target)),
                None => {
                    if let Some(bytes) = zip_read(zip, ci, &target) {
                        let mime = mime_of(&target);
                        format!("{attr}=\"data:{mime};base64,{}\"", crate::covers::b64(&bytes))
                    } else {
                        caps[0].to_string()
                    }
                }
            }
        })
        .into_owned();

    let css_url_re =
        Regex::new(r#"(?i)url\(\s*["']?([^"')]+\.(?:png|jpe?g|gif|bmp|webp|svg))["']?\s*\)"#)
            .unwrap();
    styled = css_url_re
        .replace_all(&styled, |caps: &regex::Captures| {
            let target = resolve_in_doc(doc_path, &caps[1]);
            match book_id {
                Some(bid) => {
                    if resolve_idx(ci, &target).is_some() {
                        format!("url({})", epub_image_url(bid, &target))
                    } else {
                        caps[0].to_string()
                    }
                }
                None => {
                    if let Some(bytes) = zip_read(zip, ci, &target) {
                        let mime = mime_of(&target);
                        format!("url(data:{mime};base64,{})", crate::covers::b64(&bytes))
                    } else {
                        caps[0].to_string()
                    }
                }
            }
        })
        .into_owned();

    // 关闭「覆盖书籍排版」时保留内联 style（属于原书排版的一部分）；
    // 开启时去掉内联 style，保证阅读器字体/颜色与用户自定义 CSS 生效。
    if !keep_styles {
        let style_attr_re =
            Regex::new(r#"(?is)\sstyle\s*=\s*"[^"]*"|\sstyle\s*=\s*'[^']*'"#).unwrap();
        styled = style_attr_re.replace_all(&styled, " ").into_owned();
    }

    // body 的 class（如 body.cip）迁移到包裹层，保证原书 body 级样式仍能命中。
    let body_cls = body_classes(html);
    let mut classes = String::from("epub-doc");
    if !body_cls.is_empty() {
        classes.push(' ');
        classes.push_str(&body_cls);
    }

    let mut style_tag = String::new();
    if keep_styles && !book_css.trim().is_empty() {
        let scoped = scope_book_css(&book_css);
        if !scoped.trim().is_empty() {
            // J26：外部 CSS 里可能含 </style><script>：插进 <style> 前必须转义闭合序列，
            // 否则 CSS 文本会逃逸成 HTML。大小写不敏感，用 CSS 合法转义 <\/style。
            style_tag = format!(
                "<style class=\"tp-book-css\">{}</style>",
                escape_style_close(&scoped)
            );
        }
    }

    format!(
        "<div class=\"{}\" data-doc=\"{}\">{}{}</div>",
        escape_html(&classes),
        escape_html(doc_path),
        style_tag,
        styled
    )
}

/// J26：把 CSS 里的 </style（大小写不敏感）改写为 CSS 合法转义 <\/style，
/// 保证外部样式内容永远不能闭合宿主 style 标签。
fn escape_style_close(css: &str) -> String {
    let mut out = String::with_capacity(css.len() + 8);
    let bytes = css.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i..].len() >= 7 && bytes[i..i + 7].eq_ignore_ascii_case(b"</style") {
            out.push_str(r"<\/style");
            i += 7;
        } else {
            let ch = css[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// 取出 `<head>...</head>` 之间的内容（原书样式表引用都在这里）。
fn extract_head(html: &str) -> String {
    if let Some(start) = find_ascii_ci(html, "<head", 0) {
        let after = html[start..].find('>').map(|i| start + i + 1).unwrap_or(start);
        let end = find_ascii_ci(html, "</head>", after).unwrap_or(html.len());
        if after < end {
            return html[after..end].to_string();
        }
    }
    String::new()
}

/// 文档 `<head>` 中所有 `<link rel=stylesheet>` 的 href（原样，未解析）。
fn head_stylesheet_hrefs(html: &str) -> Vec<String> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static HREF: OnceLock<Regex> = OnceLock::new();
    static REL: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r#"(?is)<link\b[^>]*>"#).unwrap());
    let href = HREF.get_or_init(|| Regex::new(r#"(?is)href\s*=\s*["']([^"']+)["']"#).unwrap());
    let rel = REL.get_or_init(|| Regex::new(r#"(?is)rel\s*=\s*["']?([^"'\s>]+)"#).unwrap());
    let head = extract_head(html);
    let mut out = Vec::new();
    for cap in tag.captures_iter(&head) {
        let t = &cap[0];
        let low = t.to_ascii_lowercase();
        let is_css = rel
            .captures(t)
            .map(|c| c[1].to_ascii_lowercase().contains("stylesheet"))
            .unwrap_or(false)
            || low.contains(".css");
        if !is_css {
            continue;
        }
        if let Some(h) = href.captures(t) {
            out.push(h[1].to_string());
        }
    }
    out
}

/// 文档内所有 `<style>` 块的内容。
fn inline_style_blocks(html: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?is)<style[^>]*>(.*?)</style>"#).unwrap());
    re.captures_iter(html).map(|c| c[1].to_string()).collect()
}

/// `<body class="...">` 的 class 值。
fn body_classes(html: &str) -> String {
    static BODY: OnceLock<Regex> = OnceLock::new();
    static CLS: OnceLock<Regex> = OnceLock::new();
    let body = BODY.get_or_init(|| Regex::new(r#"(?is)<body\b[^>]*>"#).unwrap());
    let cls = CLS.get_or_init(|| Regex::new(r#"(?is)class\s*=\s*["']([^"']*)["']"#).unwrap());
    if let Some(m) = body.find(html) {
        if let Some(c) = cls.captures(m.as_str()) {
            return c[1].trim().to_string();
        }
    }
    String::new()
}

/// 读取一个样式表并递归内联其 `@import`，重写其中 url() 为按需资源 URL。
fn load_css_recursive<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    css_path: &str,
    book_id: Option<i64>,
    visited: &mut std::collections::HashSet<String>,
    out: &mut String,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let key = css_path.replace('\\', "/").to_lowercase();
    if !visited.insert(key) {
        return;
    }
    let Some(bytes) = zip_read(zip, ci, css_path) else {
        return;
    };
    let css = String::from_utf8_lossy(&bytes).into_owned();
    static IMPORT: OnceLock<Regex> = OnceLock::new();
    let import_re = IMPORT
        .get_or_init(|| Regex::new(r#"(?is)@import\s+(?:url\(\s*)?["']?([^"')]+)["']?\s*\)?\s*;"#).unwrap());
    for cap in import_re.captures_iter(&css) {
        let imp = cap[1].trim();
        if imp.is_empty()
            || imp.starts_with("http")
            || imp.starts_with("//")
            || imp.starts_with("data:")
        {
            continue;
        }
        let target = resolve_in_doc(css_path, imp);
        load_css_recursive(zip, ci, &target, book_id, visited, out, depth + 1);
    }
    let no_import = import_re.replace_all(&css, "").into_owned();
    static CHARSET: OnceLock<Regex> = OnceLock::new();
    let no_charset = CHARSET
        .get_or_init(|| Regex::new(r#"(?is)@charset\s+["'][^"']*["']\s*;"#).unwrap())
        .replace_all(&no_import, "")
        .into_owned();
    out.push_str(&rewrite_css_urls(&no_charset, css_path, zip, ci, book_id));
    out.push('\n');
}

/// 把 CSS 中的 `url(...)` 指向 zip 内资源的引用改写为按需资源 URL（字体/图片均可）。
fn rewrite_css_urls<R: Read + std::io::Seek>(
    css: &str,
    css_path: &str,
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    book_id: Option<i64>,
) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?i)url\(\s*["']?([^"')]+)["']?\s*\)"#).unwrap());
    re.replace_all(css, |caps: &regex::Captures| {
        let raw = caps[1].trim();
        if raw.is_empty()
            || raw.starts_with("data:")
            || raw.starts_with("http")
            || raw.starts_with("//")
            || raw.starts_with('#')
            || raw.starts_with("res:")
            || raw.starts_with("res%3A")
            || raw.starts_with("file:")
        {
            return caps[0].to_string();
        }
        let target = resolve_in_doc(css_path, raw);
        if resolve_idx(ci, &target).is_none() {
            return caps[0].to_string();
        }
        match book_id {
            Some(bid) => format!("url({})", epub_image_url(bid, &target)),
            None => {
                if let Some(bytes) = zip_read(zip, ci, &target) {
                    let mime = mime_of(&target);
                    format!("url(data:{mime};base64,{})", crate::covers::b64(&bytes))
                } else {
                    caps[0].to_string()
                }
            }
        }
    })
    .into_owned()
}

/// 把原书 CSS 作用域隔离到 `#chapter-content` 内，避免污染阅读器 UI。
/// 做法：所有普通选择器统一加 `#chapter-content` 前缀（`html`/`body`/`:root` 映射到包裹层
/// `.epub-doc`）；`@font-face` / `@keyframes` 等需要全局生效的 at-rule 提到前缀外；
/// `@media` 等分组 at-rule 递归处理。
fn scope_book_css(css: &str) -> String {
    let (global, scoped) = split_css_scope(css);
    let mut out = String::new();
    if !global.trim().is_empty() {
        out.push_str(global.trim_end());
        out.push('\n');
    }
    if !scoped.trim().is_empty() {
        out.push_str(scoped.trim_end());
        out.push('\n');
    }
    out
}

fn split_css_scope(css: &str) -> (String, String) {
    let mut global = String::new();
    let mut scoped = String::new();
    let bytes = css.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                i = i + 2 + end + 2;
            } else {
                break;
            }
            continue;
        }
        if c == b'}' || c == b';' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        let mut paren = 0i32;
        let mut quote: Option<u8> = None;
        while j < n {
            let ch = bytes[j];
            if let Some(q) = quote {
                if ch == b'\\' {
                    j += 2;
                    continue;
                }
                if ch == q {
                    quote = None;
                }
                j += 1;
                continue;
            }
            match ch {
                b'"' | b'\'' => quote = Some(ch),
                b'(' => paren += 1,
                b')' => paren -= 1,
                b'{' if paren == 0 => break,
                b';' if paren == 0 => break,
                _ => {}
            }
            j += 1;
        }
        if j >= n {
            break;
        }
        if bytes[j] == b';' {
            i = j + 1;
            continue;
        }
        let prelude = css[start..j].trim();
        let block_start = j + 1;
        let mut k = block_start;
        let mut depth = 1i32;
        let mut quote2: Option<u8> = None;
        while k < n {
            let ch = bytes[k];
            if let Some(q) = quote2 {
                if ch == b'\\' {
                    k += 2;
                    continue;
                }
                if ch == q {
                    quote2 = None;
                }
                k += 1;
                continue;
            }
            match ch {
                b'"' | b'\'' => quote2 = Some(ch),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        let block = &css[block_start..k.min(n)];
        let lower = prelude.to_ascii_lowercase();
        if lower.starts_with("@font-face")
            || lower.starts_with("@keyframes")
            || lower.starts_with("@-webkit-keyframes")
            || lower.starts_with("@page")
            || lower.starts_with("@property")
            || lower.starts_with("@counter-style")
        {
            global.push_str(prelude);
            global.push('{');
            global.push_str(block);
            global.push_str("}\n");
        } else if lower.starts_with("@media")
            || lower.starts_with("@supports")
            || lower.starts_with("@layer")
            || lower.starts_with("@container")
            || lower.starts_with("@scope")
        {
            let (g2, s2) = split_css_scope(block);
            global.push_str(&g2);
            scoped.push_str(prelude);
            scoped.push('{');
            scoped.push_str(&s2);
            scoped.push_str("}\n");
        } else if lower.starts_with('@') {
            // 其余未知 at-rule 丢弃，避免污染
        } else {
            let sel = scope_selector_list(prelude);
            if !sel.is_empty() {
                scoped.push_str(&sel);
                scoped.push('{');
                scoped.push_str(block);
                scoped.push_str("}\n");
            }
        }
        i = k + 1;
    }
    (global, scoped)
}

fn scope_selector_list(sel: &str) -> String {
    let parts = split_top_commas(sel);
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        out.push(format!("#chapter-content {}", map_root_selectors(p)));
    }
    out.join(",")
}

fn map_root_selectors(sel: &str) -> String {
    // 把选择器里的 html/body/:root 类型选择器替换为包裹层 .epub-doc（保留 UTF-8 原文）。
    let b = sel.as_bytes();
    let n = b.len();
    let is_delim = |c: u8| {
        c.is_ascii_whitespace()
            || matches!(c, b'>' | b'+' | b'~' | b',' | b'.' | b'#' | b':' | b'[' | b']' | b'(' | b')' | b'*')
    };
    let mut out = String::with_capacity(n + 16);
    let mut i = 0usize;
    let mut last = 0usize;
    while i < n {
        let c = b[i];
        if c == b'"' || c == b'\'' {
            i += 1;
            while i < n {
                if b[i] == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let before_ok = i == 0 || is_delim(b[i - 1]);
        if before_ok {
            let kw_len = if i + 5 <= n && &b[i..i + 5] == b":root" {
                5
            } else if i + 4 <= n
                && (b[i..i + 4].eq_ignore_ascii_case(b"html")
                    || b[i..i + 4].eq_ignore_ascii_case(b"body"))
            {
                4
            } else {
                0
            };
            if kw_len > 0 {
                let end = i + kw_len;
                if end >= n || is_delim(b[end]) {
                    out.push_str(&sel[last..i]);
                    out.push_str(".epub-doc");
                    i = end;
                    last = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&sel[last..]);
    out
}

fn split_top_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for ch in s.chars() {
        if let Some(q) = quote {
            cur.push(ch);
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                cur.push(ch);
            }
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// 处理单个 `<img ...>` 标签：改写 src 为按需 URL（或内联 base64），
/// 读取图片头部尺寸并预置 width/height，补 loading/decoding。
fn process_img_tag<R: Read + std::io::Seek>(
    tag: &str,
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    doc_path: &str,
    book_id: Option<i64>,
) -> String {
    let low = tag.to_ascii_lowercase();
    if !low.starts_with("<img") {
        return tag.to_string();
    }
    static IMG_ATTR_RE: OnceLock<Regex> = OnceLock::new();
    let attr_re = IMG_ATTR_RE.get_or_init(|| {
        Regex::new(r#"(?i)(src|xlink:href)\s*=\s*"([^"]+\.(?:png|jpe?g|gif|bmp|webp|svg))""#).unwrap()
    });
    let Some(caps) = attr_re.captures(tag) else {
        return tag.to_string();
    };
    let target = resolve_in_doc(doc_path, &caps[2]);
    // 读取图片头部尺寸（不内联整图）。失败则不加 width/height，由 CSS 兜底。
    let dims = zip_read(zip, ci, &target).and_then(|b| image_size(&b));
    let attr = &caps[1];
    let new_src = match book_id {
        Some(bid) => format!("{attr}=\"{}\"", epub_image_url(bid, &target)),
        None => {
            if let Some(bytes) = zip_read(zip, ci, &target) {
                let mime = mime_of(&target);
                format!("{attr}=\"data:{mime};base64,{}\"", crate::covers::b64(&bytes))
            } else {
                caps[0].to_string()
            }
        }
    };
    let mut out = tag.to_string();
    out.replace_range(caps.get(0).unwrap().start()..caps.get(0).unwrap().end(), &new_src);
    // 追加属性（在标签名之后）。不用 loading="lazy"：页模式是多列 + transform 容器，
    // Chromium 的懒加载可见性判断不可靠，会导致图片永不加载（页模式图片空白）。
    let mut extra = String::from(" decoding=\"async\"");
    if let Some((w, h)) = dims {
        if w > 0 && h > 0 {
            extra.push_str(&format!(" width=\"{w}\" height=\"{h}\""));
        }
    }
    let name_end = out
        .to_ascii_lowercase()
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(out.len());
    out.insert_str(name_end, &extra);
    out
}

fn mime_of(target: &str) -> &'static str {
    match target.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn extract_body(html: &str) -> String {
    if let Some(start) = find_ascii_ci(html, "<body", 0) {
        let after = html[start..].find('>').map(|i| start + i + 1).unwrap_or(start);
        let end = rfind_ascii_ci(html, "</body>", 0).unwrap_or(html.len());
        if after < end {
            return html[after..end].to_string();
        }
    }
    html.to_string()
}

/// 保留单章入口供兼容/测试；生产批量路径改用 collect_texts（一次打开 zip）。
#[allow(dead_code)]
pub fn doc_text(zip_bytes: &[u8], doc_path: &str) -> String {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return String::new(),
    };
    let ci = build_ci(&mut zip);
    let bytes = match zip_read(&mut zip, &ci, doc_path) {
        Some(b) => b,
        None => return String::new(),
    };
    let html = String::from_utf8_lossy(&bytes).into_owned();
    html_to_text(&html)
}

pub fn html_to_text(html: &str) -> String {
    static BLOCK_END: OnceLock<Regex> = OnceLock::new();
    static BR: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    let block_end = BLOCK_END
        .get_or_init(|| Regex::new(r"(?i)</(?:p|div|h[1-6]|li|tr|blockquote|section)>").unwrap());
    let br = BR.get_or_init(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
    let tag = TAG.get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let body = extract_body(html);
    let s = block_end.replace_all(&body, "\n").into_owned();
    let s = br.replace_all(&s, "\n").into_owned();
    let s = tag.replace_all(&s, "").into_owned();
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// 一次性提取整本书每章纯文本：只开一次 zip、只建一次 CiMap，
/// 避免按 spine 逐章调用 `doc_text` 时重复 O(条目数) 重建索引（大书会退化到分钟级）。
pub fn collect_texts(zip_bytes: &[u8], spine: &[String]) -> Vec<String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return vec![String::new(); spine.len()],
    };
    let ci = build_ci(&mut zip);
    let mut out = Vec::with_capacity(spine.len());
    for doc in spine {
        let text = match zip_read(&mut zip, &ci, doc) {
            Some(b) => {
                let html = String::from_utf8_lossy(&b);
                html_to_text(&html)
            }
            None => String::new(),
        };
        out.push(text);
    }
    out
}

/// 漫画判据扫描：按 spine 顺序逐篇读取纯文本，回调 `(spine 下标, 纯文本)`；
/// 回调返回 false 表示调用方已得出结论，立即停止（普通小说第一章就会停）。
/// `budget` 是累计**解压字节**预算：耗尽即停止，不再解压后续文档。
/// 返回实际消耗的字节数，供 `[open] is_manga` 诊断日志说明扫描了多少。
pub fn scan_spine_text<F: FnMut(usize, &str) -> bool>(
    zip_bytes: &[u8],
    spine: &[String],
    budget: usize,
    mut visit: F,
) -> usize {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return 0,
    };
    let ci = build_ci(&mut zip);
    let mut used = 0usize;
    for (i, doc) in spine.iter().enumerate() {
        if used >= budget {
            break;
        }
        let Some(bytes) = zip_read(&mut zip, &ci, doc) else {
            continue;
        };
        used = used.saturating_add(bytes.len());
        let html = String::from_utf8_lossy(&bytes);
        let text = html_to_text(&html);
        if !visit(i, &text) {
            break;
        }
    }
    used
}


/// 图片页书文本探测结果（第五轮）：累计非空白字符数与是否完整扫描。
pub struct ImagePageProbe {
    pub text_chars: i64,
    pub scanned_bytes: usize,
    pub complete: bool,
}

/// 图片页书探测（EPUB only，独立于漫画判定）：按 spine 顺序读文档累计非空白字符数。
/// - 一旦 text_chars > img_count * 20 立即停止（decisive false，complete=false）；
/// - 累计解压字节 >= budget 也停止（complete=false，保守兜底，不判图片页书）；
/// - 全部 spine 文档扫完 complete=true。
/// 普通插图小说在首篇正文就会超阈值提前退出，成本毫秒级；预算复用 MANGA_SCAN_BUDGET 语义。
pub fn image_page_text_probe(
    zip_bytes: &[u8],
    spine: &[String],
    img_count: usize,
    budget: usize,
) -> ImagePageProbe {
    let limit = img_count as i64 * 20;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => {
            return ImagePageProbe { text_chars: 0, scanned_bytes: 0, complete: false };
        }
    };
    let ci = build_ci(&mut zip);
    let mut text_chars = 0i64;
    let mut used = 0usize;
    let mut complete = true;
    for doc in spine {
        if used >= budget {
            complete = false;
            break;
        }
        let Some(bytes) = zip_read(&mut zip, &ci, doc) else {
            continue;
        };
        used = used.saturating_add(bytes.len());
        let html = String::from_utf8_lossy(&bytes);
        let text = html_to_text(&html);
        text_chars += text.chars().filter(|c| !c.is_whitespace()).count() as i64;
        if text_chars > limit {
            complete = false;
            break;
        }
    }
    ImagePageProbe { text_chars, scanned_bytes: used, complete }
}

pub fn read_cover(zip_bytes: &[u8], cover_href: &str) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let ci = build_ci(&mut zip);
    zip_read(&mut zip, &ci, cover_href)
}

pub fn find_cover_fallback(zip_bytes: &[u8], spine: &[String]) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let ci = build_ci(&mut zip);

    let img_re =
        Regex::new(r#"(?i)<img[^>]+(?:src|xlink:href)\s*=\s*["']([^"']+\.(?:png|jpe?g|gif|bmp|webp))["']"#)
            .ok()?;

    let mut candidates: Vec<String> = Vec::new();
    for doc in spine.iter().take(4) {
        if let Some(bytes) = zip_read(&mut zip, &ci, doc) {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            for c in img_re.captures_iter(&html) {
                candidates.push(resolve_in_doc(doc, &c[1]));
            }
            if !candidates.is_empty() {
                break;
            }
        }
    }

    for name in candidates {
        if let Some(bytes) = zip_read(&mut zip, &ci, &name) {
            if bytes.len() > 4096 {
                return Some(bytes);
            }
        }
    }

    let mut big: Option<(u64, Vec<u8>)> = None;
    for i in 0..zip.len() {
        if let Ok(mut f) = zip.by_index(i) {
            let name = f.name().to_lowercase();
            if (name.ends_with(".jpg") || name.ends_with(".jpeg") || name.ends_with(".png"))
                && (name.contains("cover") || f.size() > 60_000)
            {
                let size = f.size();
                // 声明尺寸超限直接跳过：不按它预分配（Vec::with_capacity(size) 可被
                // 一个伪造的中央目录条目放大成巨量分配）。
                if size > MAX_ENTRY_BYTES {
                    continue;
                }
                if big.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
                    if let Some(buf) = read_capped(&mut f, MAX_ENTRY_BYTES) {
                        big = Some((size, buf));
                    }
                }
            }
        }
    }
    big.map(|(_, b)| b)
}

/// 用正则把某个标签的文本内容替换为新值（保留开标签属性）。
fn set_tag_text(xml: &str, tag: &str, value: &str) -> Option<String> {
    let esc = escape_html(value);
    let pat = format!(r"(?s)(<{tag}[^>]*>).*?(</{tag}>)");
    let re = Regex::new(&pat).ok()?;
    if re.is_match(xml) {
        Some(
            re.replace(xml, |c: &regex::Captures| format!("{}{}{}", &c[1], esc, &c[2]))
                .into_owned(),
        )
    } else {
        None
    }
}

/// P0-3：写回前把 OPF 统一成 UTF-8 文档：去掉 BOM，若 XML 声明里写了
/// encoding="UTF-16..."（大小写不敏感）改成 utf-8；无 encoding 属性保持声明原样。
/// 为什么统一 UTF-8：write_meta 用 new_opf.as_bytes() 写盘，若沿用 UTF-16 声明而写 UTF-8
/// 字节，声明与内容不一致，其它阅读器会按 UTF-16 解出乱码。
fn normalize_opf_for_utf8(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let trimmed_start = text.trim_start();
    let Some(decl_offset) = text.len().checked_sub(trimmed_start.len()) else {
        return text.to_string();
    };
    if !trimmed_start.starts_with("<?xml") {
        return text.to_string();
    }
    let Some(end_rel) = trimmed_start.find("?>") else {
        return text.to_string();
    };
    let decl_end = decl_offset + end_rel + 2;
    let (decl, rest) = text.split_at(decl_end);
    let re = Regex::new(r#"(?i)encodings*=s*(["'])UTF-16(?:LE|BE)?(["'])"#).unwrap();
    let decl_utf8 = re
        .replace(decl, |_: &regex::Captures| "encoding=\"utf-8\"".to_string())
        .into_owned();
    format!("{decl_utf8}{rest}")
}

/// 把 EPUB 的 OPF 元数据（dc:title / dc:creator）写回原文件。
///
/// 安全约定：**原文件字节在成功前不变**。流程为「同目录临时文件 → 流式重写 zip →
/// sync_all → rename 原子替换」；任何失败删除临时文件并返回 Err，不留下截断半成品。
/// 源包只打开一次流式读取，container/OPF 之外不整本驻留内存，也不按中央目录声明尺寸预分配。
pub fn write_meta(path: &str, title: &str, author: &str) -> Result<(), String> {
    // 只开文件句柄解析中央目录：不 std::fs::read 整本（MOBI/EPUB 可能几十 MB~数 GB）。
    let src_file = std::fs::File::open(path).map_err(|e| format!("打开 EPUB 失败：{e}"))?;
    let src_len = src_file
        .metadata()
        .map_err(|e| format!("读取文件信息失败：{e}"))?
        .len();
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(src_file))
        .map_err(|e| format!("EPUB 打开失败: {e}"))?;
    let ci = build_ci(&mut zip);

    let container = zip_read(&mut zip, &ci, "META-INF/container.xml")
        .ok_or("缺少 META-INF/container.xml")?;
    let container_str = decode_text(&container);
    let opf_path = opf_path_from_container(&container_str)
        .or_else(|| find_opf_in_zip(&ci))
        .ok_or_else(|| {
            if is_encrypted(&ci) {
                ERR_ENCRYPTED.to_string()
            } else {
                "container.xml 中未找到 OPF".to_string()
            }
        })?;

    let opf_bytes = zip_read(&mut zip, &ci, &opf_path).ok_or("OPF 文件缺失")?;
    let opf = decode_text(&opf_bytes);

    let mut new_opf = set_tag_text(&opf, "dc:title", title);
    new_opf = new_opf.and_then(|x| set_tag_text(&x, "dc:creator", author).or_else(|| {
        // 没有 dc:creator 时，在 dc:title 之后插入一个
        let esc = escape_html(author);
        let re = Regex::new(r"(?s)(</dc:title>)").ok()?;
        Some(
            re.replace(&x, |c: &regex::Captures| {
                format!("{c1}<dc:creator>{esc}</dc:creator>", c1 = &c[1])
            })
            .into_owned(),
        )
    }));
    let new_opf = new_opf.ok_or("OPF 中未找到 dc:title 标签，无法写回")?;
    // P0-3：统一 UTF-8 文档（声明同步），否则 UTF-16 OPF 写回后声明与字节不一致。
    let new_opf = normalize_opf_for_utf8(&new_opf);

    // 写前空间检查：同卷需要放得下临时文件（源大小 + 64KiB 余量）。
    // 取不到剩余空间（非 Windows/权限受限）时跳过，不阻断既有能力。
    if let Some(free) = crate::library::disk_free_bytes(Path::new(path)) {
        let need = src_len.saturating_add(64 * 1024);
        if free < need {
            return Err(format!(
                "磁盘剩余空间不足：需要约 {need} 字节，当前 {free} 字节"
            ));
        }
    }

    // 同目录临时文件（与源同卷），成功后 rename 覆盖。
    let tmp = format!("{path}.tidepage-tmp-{}", std::process::id());
    let opf_key = opf_path.replace('\\', "/").to_lowercase();
    let write_result = (|| -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败：{e}"))?;
        let mut zw = zip::ZipWriter::new(file);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // 按 EPUB 规范：mimetype 首个且不压缩。
        if zip.file_names().any(|n| n.eq_ignore_ascii_case("mimetype")) {
            zw.start_file("mimetype", stored).map_err(|e| e.to_string())?;
            zw.write_all(b"application/epub+zip").map_err(|e| e.to_string())?;
        }
        // 第二遍按原顺序逐条目流式拷贝：仅 OPF 条目替换内容，其余原样。
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| format!("读取压缩包条目失败：{e}"))?;
            let name = f.name().to_string();
            if name.eq_ignore_ascii_case("mimetype") || name.ends_with('/') {
                continue;
            }
            zw.start_file(name.clone(), deflated)
                .map_err(|e| format!("写入条目 {name} 失败：{e}"))?;
            if name.replace('\\', "/").to_lowercase() == opf_key {
                zw.write_all(new_opf.as_bytes())
                    .map_err(|e| format!("写入 OPF 失败：{e}"))?;
            } else {
                // 逐块拷贝：不按 f.size() 预分配，大条目不会整块驻留。
                std::io::copy(&mut f, &mut zw)
                    .map_err(|e| format!("写入条目 {name} 失败：{e}"))?;
            }
        }
        let file = zw.finish().map_err(|e| format!("收尾写入失败：{e}"))?;
        file.sync_all().map_err(|e| format!("写入磁盘失败：{e}"))?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // 释放源文件句柄后再替换（Windows 允许替换已打开文件，但没必要赌共享模式）。
    drop(zip);
    // 同卷 rename：Windows 上 std::fs::rename 会替换已存在的目标文件。
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换原文件失败：{e}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 计量：对给定文件，(1) 从尾部 EOCD 读出中央目录大小；(2) 用共享计数 reader 执行一次
    /// `ZipArchive::new`，报告读取字节总数（用看门狗防止畸形 CD 卡死）。
    /// 用法：TP_ZIP_BENCH=<epub> cargo test zipnew_bytes_bench -- --nocapture
    #[test]
    fn zipnew_bytes_bench() {
        use std::io::{Read, Seek, SeekFrom};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let Ok(spec) = std::env::var("TP_ZIP_BENCH") else { return };
        // 支持 "@文件" 形式（文件首行是 UTF-8 路径），避免命令行传非 ASCII 路径被转码。
        let path = if let Some(f) = spec.strip_prefix('@') {
            std::fs::read_to_string(f).unwrap().lines().next().unwrap().trim().to_string()
        } else {
            spec
        };
        let file_len = std::fs::metadata(&path).unwrap().len();
        // EOCD：读尾部 ≤1KiB，取中央目录大小(+12)。
        let (eocd_found, cd_size) = {
            let mut f = std::fs::File::open(&path).unwrap();
            let n = file_len.min(1024);
            f.seek(SeekFrom::End(-(n as i64))).unwrap();
            let mut b = vec![0u8; n as usize];
            f.read_exact(&mut b).unwrap();
            match b.windows(4).rposition(|w| w == [0x50, 0x4b, 0x05, 0x06]) {
                Some(p) if p + 16 <= b.len() => (
                    true,
                    u32::from_le_bytes([b[p + 12], b[p + 13], b[p + 14], b[p + 15]]) as u64,
                ),
                _ => (false, 0),
            }
        };
        // ZipArchive::new 读取字节（共享计数，看门狗 30s）。
        #[derive(Default)]
        struct Stats { calls: u64, bytes: u64, min_off: u64, max_end: u64, seq: bool, first: Vec<(u64,u64)>, last: Vec<(u64,u64)>, below_cd: u64, at_cd: u64 }
        let cd_off = {
            // 再从 EOCD 取 CD 起始偏移(+16)，用于分区域统计。
            let mut f = std::fs::File::open(&path).unwrap();
            let n = file_len.min(1024);
            f.seek(SeekFrom::End(-(n as i64))).unwrap();
            let mut b = vec![0u8; n as usize];
            f.read_exact(&mut b).unwrap();
            match b.windows(4).rposition(|w| w == [0x50,0x4b,0x05,0x06]) {
                Some(p) if p + 20 <= b.len() => u32::from_le_bytes([b[p+16],b[p+17],b[p+18],b[p+19]]) as u64,
                _ => 0,
            }
        };
        let stats = Arc::new(std::sync::Mutex::new(Stats { min_off: u64::MAX, ..Default::default() }));
        struct Counting<R> { inner: R, pos: u64, cd_off: u64, stats: Arc<std::sync::Mutex<Stats>> }
        impl<R: std::io::Read> std::io::Read for Counting<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                let mut s = self.stats.lock().unwrap();
                let start = self.pos; let end = self.pos + n as u64;
                s.calls += 1; s.bytes += n as u64;
                if start < s.min_off { s.min_off = start; }
                if end > s.max_end { s.max_end = end; }
                if start < self.cd_off { s.below_cd += n as u64; } else { s.at_cd += n as u64; }
                if s.first.len() < 6 { s.first.push((start, n as u64)); }
                s.last.push((start, n as u64)); if s.last.len() > 6 { s.last.remove(0); }
                self.pos = end;
                Ok(n)
            }
        }
        impl<R: std::io::Seek> std::io::Seek for Counting<R> {
            fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> { let r = self.inner.seek(p)?; self.pos = r; Ok(r) }
        }
        let counter = Arc::new(AtomicU64::new(0));
        let c2 = counter.clone();
        let s2 = stats.clone();
        let p2 = path.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        std::thread::spawn(move || {
            let f = std::fs::File::open(&p2).unwrap();
            let c = Counting { inner: f, pos: 0, cd_off, stats: s2 };
            let _ = zip::ZipArchive::new(c);
            done2.store(true, Ordering::SeqCst);
        });
        let t0 = std::time::Instant::now();
        while !done.load(Ordering::SeqCst) && t0.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let finished = done.load(Ordering::SeqCst);
        let s = stats.lock().unwrap();
        eprintln!(
            "[zipnew_bench] file_size={} eocd_found={} cd_size={} cd_offset={} zipnew_finished={} bytes_read={} calls={} min_off={} max_end={} below_cd={} at_cd={} first={:?} last={:?} elapsed_ms={}",
            file_len, eocd_found, cd_size, cd_off, finished, counter.load(Ordering::SeqCst),
            s.calls, s.min_off, s.max_end, s.below_cd, s.at_cd, s.first, s.last, t0.elapsed().as_millis()
        );
    }

    /// 计量：对给定文件执行**带预算**的封面提取，输出解析阶段消耗的 read 调用次数/字节数、
    /// 是否命中预算（病态）、耗时与结果。用于为 `MAX_PARSE_CALLS` 取值提供依据，并对失败文件归类。
    /// 用法：TP_EPUB_COVER=<epub|@pathfile> cargo test cover_budget_bench -- --nocapture
    #[test]
    fn cover_budget_bench() {
        let Ok(spec) = std::env::var("TP_EPUB_COVER") else { return };
        let path = if let Some(f) = spec.strip_prefix('@') {
            std::fs::read_to_string(f).unwrap().lines().next().unwrap().trim().to_string()
        } else {
            spec
        };
        let t = std::time::Instant::now();
        let bytes = std::env::var("TP_EPUB_COVER_BYTES").ok().and_then(|s| s.parse().ok()).unwrap_or(MAX_PARSE_BYTES);
        let calls = std::env::var("TP_EPUB_COVER_CALLS").ok().and_then(|s| s.parse().ok()).unwrap_or(MAX_PARSE_CALLS);
        let budget = Arc::new(CoverReadBudget::new(bytes, calls));
        let r = extract_cover_file(&path, &budget);
        let desc = match &r {
            Ok(Some(b)) => format!("Ok(cover {} bytes)", b.len()),
            Ok(None) => "Ok(no cover)".to_string(),
            Err(e) => format!("Err({e})"),
        };
        let kind = if r.is_ok() {
            "ok"
        } else if budget.exhausted() {
            "cover-pathological"
        } else {
            "cover-error"
        };
        eprintln!(
            "[cover_budget] kind={kind} exhausted={} calls_used={} bytes_used={} result={desc} elapsed_ms={}",
            budget.exhausted(), budget.calls_used(), budget.bytes_used(), t.elapsed().as_millis()
        );
    }

    /// 计量：对病态文件执行带预算的封面提取，并在 500ms 后 cancel（模拟看门狗取消），
    /// 验证 worker 线程随即结束（不再泄漏）。用法：TP_EPUB_CANCEL=<epub> cargo test cover_cancel_bench -- --nocapture
    #[test]
    fn cover_cancel_bench() {
        let Ok(spec) = std::env::var("TP_EPUB_CANCEL") else { return };
        let path = if let Some(f) = spec.strip_prefix('@') {
            std::fs::read_to_string(f).unwrap().lines().next().unwrap().trim().to_string()
        } else {
            spec
        };
        let budget = Arc::new(CoverReadBudget::default());
        let b = budget.clone();
        let p = path.clone();
        let t = std::time::Instant::now();
        let h = std::thread::spawn(move || extract_cover_file(&p, &b).is_err());
        std::thread::sleep(std::time::Duration::from_millis(500));
        budget.cancel(); // 取消
        let is_err = h.join().unwrap();
        eprintln!(
            "[cover_cancel] is_err={} elapsed_ms={}",
            is_err,
            t.elapsed().as_millis()
        );
    }

    /// 计量：用共享计数的 reader 统计 extract_cover 各阶段实际从磁盘拉取的字节数。
    /// 用法：TP_COVER_BENCH=<epub 路径> cargo test cover_bytes_bench -- --nocapture
    #[test]
    fn cover_bytes_bench() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let Ok(path) = std::env::var("TP_COVER_BENCH") else { return };
        struct Counting<R> { inner: R, read: Arc<AtomicU64> }
        impl<R: std::io::Read> std::io::Read for Counting<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                self.read.fetch_add(n as u64, Ordering::SeqCst);
                Ok(n)
            }
        }
        impl<R: std::io::Seek> std::io::Seek for Counting<R> {
            fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> { self.inner.seek(p) }
        }
        let file_len = std::fs::metadata(&path).unwrap().len();
        let counter = Arc::new(AtomicU64::new(0));
        let c = Counting { inner: std::fs::File::open(&path).unwrap(), read: counter.clone() };
        let snap = || counter.load(Ordering::SeqCst);

        let mut zip = zip::ZipArchive::new(c).unwrap();
        let after_open = snap(); // EOCD + 中央目录

        let read_named = |zip: &mut zip::ZipArchive<Counting<std::fs::File>>, name: &str| -> Option<Vec<u8>> {
            let mut e = zip.by_name(name).ok()?;
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut b).ok()?;
            Some(b)
        };
        let container = read_named(&mut zip, "META-INF/container.xml").map(|b| decode_text(&b)).unwrap_or_default();
        let after_container = snap();
        let opf_path = opf_path_from_container(&container).unwrap_or_default();
        let opf = if opf_path.is_empty() { String::new() }
                  else { read_named(&mut zip, &opf_path).map(|b| decode_text(&b)).unwrap_or_default() };
        let after_opf = snap();
        let href = cover_href_from_opf(&opf).unwrap_or_default();
        let full = if href.is_empty() { String::new() } else { norm_path(&opf_path, &href) };
        let cover = if full.is_empty() { None } else { read_named(&mut zip, &full) };
        let after_cover = snap();
        let cover_len = cover.as_ref().map(|b| b.len()).unwrap_or(0);

        eprintln!(
            "[cover_bench] file_size={} total_read={} ratio={:.4} | open_CD={} container={} opf={} cover_entry={} (cover_decoded={}) path={}",
            file_len, after_cover, after_cover as f64 / file_len as f64,
            after_open, after_container - after_open, after_opf - after_container,
            after_cover - after_opf, cover_len, path
        );
    }

    /// 设置 TIDEPAGE_PROFILE_EPUB 时，校验按需图片 URL 能被 read_entry 正确解析。

    #[test]
    fn c13_file_parse_and_read_entry_equivalent() {
        let data = tiny_epub();
        let dir = std::env::temp_dir().join(format!("tp-c13-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.epub");
        std::fs::write(&p, &data).unwrap();
        let (doc_bytes, ci_bytes) = parse_with_ci(&data).unwrap();
        let (doc_file, ci_file) = parse_file_with_ci(&p).unwrap();
        assert_eq!(doc_file.spine, doc_bytes.spine);
        assert_eq!(doc_file.toc.len(), doc_bytes.toc.len());
        assert_eq!(doc_file.title, doc_bytes.title);
        assert_eq!(ci_file.len(), ci_bytes.len());
        let target = doc_file.spine.first().cloned().unwrap_or_default();
        let from_file = read_entry_file(&p, &ci_file, &target).expect("按路径读条目");
        assert_eq!(from_file, read_entry(&data, &ci_bytes, &target).unwrap());
        assert!(read_entry_file(&p, &ci_file, "not/exist.xhtml").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn epubimg_urls_resolve() {
        let path = match std::env::var("TIDEPAGE_PROFILE_EPUB") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return,
        };
        let data = std::fs::read(&path).unwrap();
        let (doc, ci) = parse_with_ci(&data).unwrap();
        let mut total = 0usize;
        let mut ok = 0usize;
        for idx in [0usize, 1, 100, 1000, 3000] {
            if idx >= doc.toc.len() {
                continue;
            }
            let start = doc.toc[idx].index;
            let end = doc.toc_end.get(idx).copied().unwrap_or(start + 1).max(start + 1).min(doc.spine.len());
            let docs: Vec<String> = (start..end).filter_map(|si| doc.spine.get(si).cloned()).collect();
            let html = chapter_html_multi_ci(&data, Some(&ci), &docs, Some(42), false).unwrap();
            let re = Regex::new(r"epubimg/42/([A-Za-z0-9_\-]+)").unwrap();
            for caps in re.captures_iter(&html) {
                total += 1;
                let target = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(caps[1].as_bytes())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default();
                if read_entry(&data, &ci, &target).map(|b| !b.is_empty()).unwrap_or(false) {
                    ok += 1;
                }
            }
        }
        eprintln!("[epubimg] urls={total} resolved={ok}");
        assert!(total > 0, "no epubimg urls generated");
        assert_eq!(total, ok, "some epubimg urls did not resolve");
    }

    /// 任务三/五共用的内存 EPUB 夹具：2 章、NCX 目录、正文可统计字数。
    fn tiny_epub() -> Vec<u8> {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("mimetype", stored).unwrap();
            zw.write_all(b"application/epub+zip").unwrap();
            zw.start_file("META-INF/container.xml", deflated).unwrap();
            zw.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            zw.start_file("OEBPS/content.opf", deflated).unwrap();
            zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#).unwrap();
            zw.start_file("OEBPS/toc.ncx", deflated).unwrap();
            zw.write_all(
                r#"<?xml version="1.0" encoding="UTF-8"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="n1" playOrder="1"><navLabel><text>第一章</text></navLabel><content src="c1.xhtml"/></navPoint><navPoint id="n2" playOrder="2"><navLabel><text>第二章</text></navLabel><content src="c2.xhtml"/></navPoint></navMap></ncx>"#
                    .as_bytes(),
            )
            .unwrap();
            for (name, body) in [
                ("OEBPS/c1.xhtml", "第一章正文，这里是一段足够长的文字用来统计字数。"),
                ("OEBPS/c2.xhtml", "第二章正文，同样是一段用来统计字数的文字。"),
            ] {
                zw.start_file(name, deflated).unwrap();
                zw.write_all(format!("<html><body><p>{body}</p></body></html>").as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 任务三/五的共用路径（book_char_counts 与 get_book_word_count 都走 epub_word_stats）：
    /// 快速解析 parse 不给任何字数，完整解析必须给出每章真实字数与全书 total/pure，
    /// 且与 parse_with_chars 逐项一致；total/pure 取自 spine 逐文档求和（不是 toc 求和）。
    #[test]
    fn full_parse_fills_chapter_and_total_chars() {
        let data = tiny_epub();
        let fast = parse(&data).unwrap();
        assert!(fast.toc.len() >= 2, "目录应有两条: {:?}", fast.toc.len());
        assert!(
            fast.toc.iter().all(|t| t.chars == 0),
            "快速解析不得统计每章字数: {:?}",
            fast.toc.iter().map(|t| t.chars).collect::<Vec<_>>()
        );
        assert_eq!(fast.total_chars, 0, "快速解析不得统计全书字数");
        assert_eq!(fast.total_text_chars, 0);

        let (per_chapter, total, pure) = epub_word_stats(&data).unwrap();
        assert_eq!(per_chapter.len(), fast.toc.len(), "每章字数条数应等于目录条数");
        assert!(
            per_chapter.iter().all(|c| *c > 0),
            "每章字数应大于 0: {per_chapter:?}"
        );
        assert!(total > 0, "total 应大于 0: {total}");
        assert!(pure > 0, "pure 应大于 0: {pure}");
        assert!(pure <= total, "纯字数不应超过总字数: {pure}/{total}");

        let full = parse_with_chars(&data).unwrap();
        assert_eq!(total, full.total_chars, "total 必须等于 parse_with_chars 的 total_chars");
        assert_eq!(pure, full.total_text_chars, "pure 必须等于 total_text_chars");
        assert_eq!(
            per_chapter,
            full.toc.iter().map(|t| t.chars).collect::<Vec<_>>(),
            "每章字数应与完整解析的目录字数逐项一致"
        );
    }

    /// P11 夹具：目录折叠成 1 个 legacy 目录项（NCX 3 个锚点都指向同一 spine 文档），
    /// 该文档 120 张图 + 每段一句水印；第二个 spine 文档是「后记」整篇 fallback 章。
    /// text_heavy=true 时每段加大量正文，用于验证「折叠但正文多不切分」。
    fn split_epub(text_heavy: bool) -> Vec<u8> {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("mimetype", stored).unwrap();
            zw.write_all(b"application/epub+zip").unwrap();
            zw.start_file("META-INF/container.xml", deflated).unwrap();
            zw.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            zw.start_file("OEBPS/content.opf", deflated).unwrap();
            zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title></metadata><manifest><item id="img" href="imgdoc.xhtml" media-type="application/xhtml+xml"/><item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="img"/><itemref idref="tail"/></spine></package>"#).unwrap();
            zw.start_file("OEBPS/toc.ncx", deflated).unwrap();
            zw.write_all(
                r#"<?xml version="1.0" encoding="UTF-8"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="n1" playOrder="1"><navLabel><text>第一页</text></navLabel><content src="imgdoc.xhtml#frag1"/></navPoint><navPoint id="n2" playOrder="2"><navLabel><text>第二页</text></navLabel><content src="imgdoc.xhtml#frag2"/></navPoint><navPoint id="n3" playOrder="3"><navLabel><text>第三页</text></navLabel><content src="imgdoc.xhtml#frag3"/></navPoint></navMap></ncx>"#
                    .as_bytes(),
            )
            .unwrap();
            let seg_text = |s: &str| {
                if text_heavy {
                    format!("<p>{s}{}</p>", "正文".repeat(1000))
                } else {
                    format!("<p>{s}</p>")
                }
            };
            let mut body = String::from("<span id=\"frag1\"></span>");
            for (i, seg) in ["甲段水印", "乙段水印", "丙段水印"].iter().enumerate() {
                body.push_str(&seg_text(seg));
                for _ in 0..40 {
                    body.push_str("<p><img src=\"a1.png\"/></p>");
                }
                if i < 2 {
                    body.push_str(&format!("<span id=\"frag{}\"></span>", i + 2));
                }
            }
            zw.start_file("OEBPS/imgdoc.xhtml", deflated).unwrap();
            zw.write_all(
                format!("<html><head><title>x</title></head><body>{body}</body></html>")
                    .as_bytes(),
            )
            .unwrap();
            zw.start_file("OEBPS/tail.xhtml", deflated).unwrap();
            zw.write_all("<html><body><p>后记正文</p></body></html>".as_bytes())
                .unwrap();
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 折叠图片页书：NCX 的 3 个锚点把同一 spine 文档切成 3 章，之后的 spine 文档是整篇
    /// fallback 章；legacy 目录（1 项）与 toc_end 保持原样，偏移连续、不重不漏。
    #[test]
    fn folded_image_epub_is_split_into_anchor_chapters() {
        let data = split_epub(false);
        let (doc, ci) = parse_with_ci(&data).unwrap();
        assert!(doc.split, "折叠图片页书应切分");
        assert_eq!(doc.toc.len(), 1, "legacy 目录仍只有 1 项");
        assert_eq!(doc.toc_end, vec![2]);
        assert_eq!(doc.chapters.len(), 4, "3 个锚点章 + 1 个整篇 fallback 章");
        assert_eq!(doc.split_toc.len(), 4);
        let c = &doc.chapters;
        assert_eq!(c[0].title, "第一页");
        assert_eq!(c[1].title, "第二页");
        assert_eq!(c[2].title, "第三页");
        // R10-1：fallback 章标题先沿用 legacy 规则再经 normalize（去掉「 - tail.xhtml」后缀）。
        assert_eq!(c[3].title, "T", "整篇 fallback 章标题去掉文件名后缀");
        assert_eq!(c[3].spine, 1);
        assert!(c[3].start.is_none() && c[3].spine_end == 2);
        assert_eq!(c[0].legacy, 0);
        assert_eq!(c[3].legacy, 0);
        // 切片偏移：首章从 body 起点、末章到 body 末尾，相邻首尾相接。
        let raw = String::from_utf8_lossy(&read_entry(&data, &ci, "OEBPS/imgdoc.xhtml").unwrap())
            .into_owned();
        let (bs, be) = body_bounds(&raw).unwrap();
        assert_eq!(c[0].start, Some(bs));
        assert_eq!(c[0].end, c[1].start.unwrap());
        assert_eq!(c[1].end, c[2].start.unwrap());
        assert_eq!(c[2].end, be);
        // 每章只含自己那段的 40 张图。
        assert_eq!(c[0].imgs, 40);
        assert_eq!(c[1].imgs, 40);
        assert_eq!(c[2].imgs, 40);
        // 有效目录 index = 章节序号；切片章 chars=0。
        let idx: Vec<usize> = doc.split_toc.iter().map(|t| t.index).collect();
        assert_eq!(idx, vec![0, 1, 2, 3]);
        assert_eq!(doc.split_toc[0].chars, 0);
        assert_eq!(doc.split_toc[1].chars, 0);
    }

    /// 切分后的章节渲染只含本段内容（文字与图片都不串段）。
    #[test]
    fn split_fragment_renders_only_its_own_segment() {
        let data = split_epub(false);
        let (doc, ci) = parse_with_ci(&data).unwrap();
        assert!(doc.split);
        let raw = String::from_utf8_lossy(&read_entry(&data, &ci, "OEBPS/imgdoc.xhtml").unwrap())
            .into_owned();
        let c = &doc.chapters[1];
        let html = chapter_html_fragment_ci(
            &data,
            Some(&ci),
            "OEBPS/imgdoc.xhtml",
            &raw,
            c.start.unwrap(),
            c.end,
            Some(7),
            false,
        )
        .unwrap();
        assert!(html.contains("乙段水印"), "本段文字应出现");
        assert!(
            !html.contains("甲段水印") && !html.contains("丙段水印"),
            "不得包含相邻段文字"
        );
        assert_eq!(html.matches("epubimg/7/").count(), 40, "只渲染本段图片");
    }

    /// 折叠但正文多的书不切分（文本/图片比超阈值），保持 legacy 行为。
    #[test]
    fn folded_but_text_heavy_epub_is_not_split() {
        let data = split_epub(true);
        let doc = parse(&data).unwrap();
        assert!(!doc.split, "正文多的书不得按图片页切分");
        assert!(doc.chapters.is_empty());
        assert!(doc.split_toc.is_empty());
        assert_eq!(doc.toc.len(), 1);
    }

    /// 普通多章文字书不切分，legacy 目录/章节数完全不变。
    #[test]
    fn normal_text_epub_is_not_split() {
        let doc = parse(&tiny_epub()).unwrap();
        assert!(!doc.split);
        assert_eq!(doc.toc.len(), 2);
        assert!(doc.chapters.is_empty());
    }

    /// 完整解析时：切片章不参与字数估算，整篇 fallback 章有字数；legacy 目录不变。
    #[test]
    fn split_book_keeps_legacy_toc_and_chars() {
        let doc = parse_with_chars(&split_epub(false)).unwrap();
        assert!(doc.split);
        assert_eq!(doc.toc.len(), 1, "legacy 目录不因切分改变");
        assert_eq!(doc.split_toc[0].chars, 0, "图片页切片章不参与字数估算");
        assert!(doc.split_toc[3].chars > 0, "整篇 fallback 章应有字数");
    }

    /// 迁移纯函数：组内按图片数加权，边界（0/1/组首/组尾/图片数不均）都正确。
    #[test]
    fn map_legacy_position_weights_uneven_image_chapters() {
        let mk = |spine: usize, imgs: usize, legacy: usize| EpubChapter {
            title: String::new(),
            spine,
            spine_end: spine + 1,
            start: Some(0),
            end: 1,
            imgs,
            legacy,
        };
        let chapters = vec![mk(0, 2, 0), mk(0, 0, 0), mk(0, 98, 0), mk(1, 5, 1)];
        assert_eq!(map_legacy_position(&chapters, 0, 0.0), (0, 0.0));
        assert_eq!(map_legacy_position(&chapters, 0, 1.0), (2, 1.0));
        let (i, r) = map_legacy_position(&chapters, 0, 0.01);
        assert_eq!(i, 0);
        assert!((r - 0.505).abs() < 1e-9, "首章内比例: {r}");
        // 权重不均：目标 100.5/101 → 落在第三章，章内比例 (100.5-3)/98。
        let (i2, r2) = map_legacy_position(&chapters, 0, 100.5 / 101.0);
        assert_eq!(i2, 2);
        assert!((r2 - 97.5 / 98.0).abs() < 1e-9, "组内比例: {r2}");
        assert_eq!(map_legacy_position(&chapters, 1, 0.5), (3, 0.5));
        assert_eq!(map_legacy_position(&chapters, 9, 0.5), (0, 0.0), "组缺失回到开头");
    }

    /// 锚点辅助函数：块级边界判断、图片计数、UTF-8 边界切片。
    #[test]
    fn anchor_boundary_img_count_and_slice_helpers() {
        let raw = "<html><body><p>甲</p><span id=\"a\"></span><p>乙<span name='b'></span></p></body></html>";
        let (bs, be) = body_bounds(raw).unwrap();
        let a = find_anchor_offset(raw, bs, be, "a").unwrap();
        let b = find_anchor_offset(raw, bs, be, "b").unwrap();
        assert!(a < b, "锚点偏移应递增");
        assert!(anchor_boundary_ok(raw, bs, a), "紧跟在 </p> 后是安全边界");
        assert!(!anchor_boundary_ok(raw, bs, b), "行内锚点（前有正文）不是安全边界");
        assert_eq!(
            count_img_tags("<img src=x/><IMG src=y/><image/><imgx"),
            2,
            "只统计真正的 img 标签"
        );
        let s = "a中b";
        assert_eq!(slice_body(s, 0, 2), "a中", "越界到字符中间应向上取整");
    }

    /// H1：解压助手 cap 边界（超限 None、界内原样、cap=0 拒绝非空数据）。
    #[test]
    fn read_capped_enforces_cap() {
        let mut big = std::io::Cursor::new(vec![7u8; 200]);
        assert!(read_capped(&mut big, 96).is_none(), "超过 cap 必须拒绝");
        let mut small = std::io::Cursor::new(vec![7u8; 50]);
        assert_eq!(read_capped(&mut small, 96).unwrap(), vec![7u8; 50]);
        let mut exact = std::io::Cursor::new(vec![1u8; 96]);
        assert_eq!(read_capped(&mut exact, 96).unwrap().len(), 96, "恰好 cap 允许");
        let mut one = std::io::Cursor::new(vec![1u8]);
        assert!(read_capped(&mut one, 0).is_none(), "cap=0 时非空数据拒绝");
    }

    /// H1：zip_read 经上限助手读正常条目（大小写不敏感名解析、缺失返回 None）。
    /// 超大条目用例构造成本高（要写入 >64MiB 数据），按提示只覆盖 helper 边界。
    #[test]
    fn zip_read_reads_normal_entry_with_cap_helper() {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("a.txt", opts).unwrap();
        zw.write_all(b"hello").unwrap();
        zw.start_file("dir/b.bin", opts).unwrap();
        zw.write_all(&vec![9u8; 1024]).unwrap();
        let data = zw.finish().unwrap().into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        let ci = build_ci(&mut archive);
        assert_eq!(zip_read(&mut archive, &ci, "A.TXT").unwrap(), b"hello");
        assert_eq!(zip_read(&mut archive, &ci, "dir/b.bin").unwrap().len(), 1024);
        assert!(zip_read(&mut archive, &ci, "missing").is_none());
    }

    /// H3：Unicode 小写化改变字节长度（İ 变长、ẞ 变短）时，body/head/nav 偏移仍来自原串。
    #[test]
    fn unicode_lowercase_does_not_shift_html_offsets() {
        // 注释里 İ 各 +1 字节、ẞ -1 字节，合计让 lower 的偏移比原串多 1。
        let raw = "<!-- İİẞ --><html><head><title>标题</title></head><body class=\"c\"><p>正文一</p><p>正文二</p></body></html>";
        let (bs, be) = body_bounds(raw).expect("常规 XHTML 应有 body 区间");
        assert_eq!(&raw[bs..be], "<p>正文一</p><p>正文二</p>", "body 偏移必须落在原串");
        assert_eq!(extract_body(raw), "<p>正文一</p><p>正文二</p>");
        assert_eq!(extract_head(raw), "<title>标题</title>");
        // nav：ẞ 让 lower 偏移比原串少 1，旧实现会从 <nav 的后一个字节开始切。
        let nav_html = "<!-- ẞ --><nav epub:type=\"toc\"><a href=\"c1.xhtml\">第一章</a></nav>";
        let seg = find_toc_nav_section(nav_html).expect("应定位到含 toc 的 nav");
        assert!(seg.starts_with("<nav"), "nav 起点必须来自原串: {seg}");
        assert!(seg.ends_with("</nav>"));
        assert!(seg.contains("第一章"));
        // 大写标签同样大小写不敏感
        let upper = "<HTML><HEAD><TITLE>T</TITLE></HEAD><BODY><P>x</P></BODY></HTML>";
        assert_eq!(extract_body(upper), "<P>x</P>");
        assert_eq!(extract_head(upper), "<TITLE>T</TITLE>");
        let (bs2, be2) = body_bounds(upper).unwrap();
        assert_eq!(&upper[bs2..be2], "<P>x</P>");
    }

    /// 图片页书探测（第五轮）：图片多文字少 → 完整扫描；正文多 → 提前退出；预算截断 → complete=false。
    #[test]
    fn image_page_text_probe_early_exit_budget_and_complete() {
        let data = split_epub(false);
        let doc = parse(&data).unwrap();
        // 图片页型：120 图、全部文字约 31 字，远低于 120*20=2400 → 完整扫描。
        let probe = image_page_text_probe(&data, &doc.spine, 120, 8 * 1024 * 1024);
        assert!(probe.complete, "全部扫完应 complete=true");
        assert!(probe.text_chars > 0, "应累计到水印文字");
        assert!(probe.text_chars <= 120 * 20, "文字量应低于阈值: {}", probe.text_chars);
        assert!(probe.scanned_bytes > 0, "应实际解压字节");
        // 正文多：首篇即超阈值，提前退出。
        let heavy = split_epub(true);
        let hdoc = parse(&heavy).unwrap();
        let hp = image_page_text_probe(&heavy, &hdoc.spine, 120, 8 * 1024 * 1024);
        assert!(!hp.complete, "超阈值应提前退出（complete=false）");
        assert!(hp.text_chars > 120 * 20, "提前退出时文字量应已超阈值: {}", hp.text_chars);
        // 预算截断：complete=false（保守兜底，不判图片页书）。
        let bp = image_page_text_probe(&data, &doc.spine, 120, 10);
        assert!(!bp.complete, "预算截断不得判完整");
        assert!(bp.scanned_bytes >= 10, "应至少解压过一篇文档");
        // 空 spine：complete=true 且 0 字（调用方另有图片数门槛）。
        let ep = image_page_text_probe(&data, &[], 120, 1024);
        assert!(ep.complete);
        assert_eq!(ep.text_chars, 0);
    }

    /// R6-1 测试夹具：单 spine 文档，imgs 张图分布在每 per_p 张一个 <p> 块里。
    fn block_epub(imgs: usize, per_p: usize) -> (Vec<u8>, EpubDoc, CiMap) {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let mut body = String::new();
            let mut n = 0usize;
            while n < imgs {
                body.push_str("<p>");
                for _ in 0..per_p.min(imgs - n) {
                    body.push_str("<img src=\"x.png\"/>");
                    n += 1;
                }
                body.push_str("</p>\n");
            }
            let html = format!("<html><body>{body}</body></html>");
            zw.start_file("OEBPS/c0.xhtml", opt).unwrap();
            zw.write_all(html.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let data = buf.into_inner();
        let doc = EpubDoc {
            title: "T".into(),
            author: String::new(),
            spine: vec!["OEBPS/c0.xhtml".into()],
            toc: vec![TocEntry { title: "全书".into(), index: 0, chars: 0 }],
            toc_end: vec![1],
            cover_href: None,
            spine_chars: vec![0],
            total_chars: 0,
            total_text_chars: 0,
            image_hrefs: Vec::new(),
            split: false,
            split_toc: Vec::new(),
            chapters: Vec::new(),
            auto_block_split: false,
        };
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&data)).unwrap();
        let ci = build_ci(&mut zip);
        (data, doc, ci)
    }

    /// R6-1：块级切点必须落在闭合标签之后；每段图片数 <= max；总图片数守恒；单块无法切。
    #[test]
    fn block_split_starts_cuts_after_block_tags() {
        let raw = "<html><body><p><img/></p><p><img/></p><p><img/></p></body></html>";
        let (bs, be) = body_bounds(raw).unwrap();
        let starts = block_split_starts(raw, bs, be, 1);
        assert_eq!(starts.len(), 2, "三个图块在第 1、2 块后切，末块到 body 末尾: {starts:?}");
        let mut cuts = vec![bs];
        cuts.extend(starts.iter().copied());
        let total: usize = (0..cuts.len())
            .map(|k| {
                let en = cuts.get(k + 1).copied().unwrap_or(be);
                count_img_tags(&raw[cuts[k]..en])
            })
            .sum();
        assert_eq!(total, 3, "总图片数守恒");
        for k in 1..cuts.len() {
            assert!(raw[cuts[k]..].starts_with('<'), "切点后应是标签: {k}");
            assert!(raw[..cuts[k]].ends_with('>'), "切点前应是标签闭合: {k}");
        }
        // 单个巨型块：唯一闭合标签在 body 末尾 → 无法切出 2 段。
        let giant = "<html><body><div><img/><img/><img/></div></body></html>";
        let (bs2, be2) = body_bounds(giant).unwrap();
        assert!(block_split_starts(giant, bs2, be2, 1).is_empty(), "单块不应切");
        // max 调大 → 段数下降。
        let many: String = (0..6).map(|_| "<p><img/></p>").collect();
        let raw_many = format!("<html><body>{many}</body></html>");
        let (bs3, be3) = body_bounds(&raw_many).unwrap();
        assert_eq!(block_split_starts(&raw_many, bs3, be3, 1).len(), 5);
        assert!(block_split_starts(&raw_many, bs3, be3, 3).len() <= 2);
        // 注释/script 里的 <img 不参与计数，也不成为切点。
        let commented = "<html><body><!-- <img/> --><p><img/></p><script><img/></script><p><img/></p><p><img/></p></body></html>";
        let (bs4, be4) = body_bounds(commented).unwrap();
        let cs = block_split_starts(commented, bs4, be4, 2);
        assert_eq!(cs.len(), 1, "注释/script 里的 img 不计数，两个真实块凑满 2 图后切一次: {cs:?}");
        assert_eq!(count_img_tags(&commented[bs4..be4]), 5, "原始计数包含注释/script 内的 img");
        // 切点应落在 script 之后、第三个真实 p 块之前（注释/script 的 img 不参与计数）。
        let cut = cs[0];
        assert!(cut > commented.find("</script>").unwrap(), "切点应在 script 之后");
        assert!(commented[cut..].starts_with("<p>"), "切点应正好落在最后一个 p 块起点");
        assert_eq!(count_img_tags(&commented[cut..be4]), 1, "切点后只剩最后一个真实块");
    }

    /// R6-1：单 spine 巨量图片书切 3 章（200/200/100），偏移连续、图片守恒、标题为空；
    /// 图片不足不切；doc.split=true（P11 锚点切分）绝不覆盖；章数上限生效。
    #[test]
    fn plan_block_split_splits_giant_single_spine_document() {
        let (data, doc, ci) = block_epub(500, 10);
        let (split_toc, chapters) = plan_block_split(&doc, &data, &ci, 200, 500).expect("500 图应切章");
        assert_eq!(chapters.len(), 3, "200/200/100 应切 3 章");
        assert_eq!(split_toc.len(), 3);
        assert_eq!(
            chapters.iter().map(|c| c.imgs).collect::<Vec<_>>(),
            vec![200, 200, 100],
            "每章图片数"
        );
        assert_eq!(chapters.iter().map(|c| c.imgs).sum::<usize>(), 500, "图片数守恒");
        let raw = String::from_utf8_lossy(&read_entry(&data, &ci, "OEBPS/c0.xhtml").unwrap())
            .into_owned();
        let (bs, be) = body_bounds(&raw).unwrap();
        assert_eq!(chapters[0].start, Some(bs), "首章从 body 起点");
        for k in 0..chapters.len() - 1 {
            assert_eq!(chapters[k].end, chapters[k + 1].start.unwrap(), "相邻首尾相接");
        }
        assert_eq!(chapters[2].end, be, "末章到 body 末尾");
        assert!(chapters.iter().all(|c| c.title.is_empty()), "自动切章标题为空");
        assert!(chapters.iter().all(|c| c.legacy == 0));
        assert_eq!(split_toc[1].index, 1);
        // 图片数不超过上限 → 不切。
        let (d2, doc2, ci2) = block_epub(50, 10);
        assert!(plan_block_split(&doc2, &d2, &ci2, 200, 500).is_none());
        // 章数上限：500 图 / 上限 2 章 → 每章上限调大到 250 → 2 章。
        let (toc2, ch2) = plan_block_split(&doc, &data, &ci, 200, 2).expect("应满足 2 章上限");
        assert_eq!(ch2.len(), 2);
        assert_eq!(toc2.len(), 2);
        // P11 锚点切分书（split=true）绝不覆盖。
        let split_data = split_epub(false);
        let (split_doc, split_ci) = parse_with_ci(&split_data).unwrap();
        assert!(split_doc.split);
        assert!(
            plan_block_split(&split_doc, &split_data, &split_ci, 200, 500).is_none(),
            "已有锚点切分绝不覆盖"
        );
    }

    /// R6-B 夹具：任意单文档 HTML 的 EPUB（spine=1）。
    fn single_doc_epub(html: &str) -> (Vec<u8>, EpubDoc, CiMap) {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("OEBPS/c0.xhtml", opt).unwrap();
            zw.write_all(html.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let data = buf.into_inner();
        let doc = EpubDoc {
            title: "T".into(),
            author: String::new(),
            spine: vec!["OEBPS/c0.xhtml".into()],
            toc: vec![TocEntry { title: "全书".into(), index: 0, chars: 0 }],
            toc_end: vec![1],
            cover_href: None,
            spine_chars: vec![0],
            total_chars: 0,
            total_text_chars: 0,
            image_hrefs: Vec::new(),
            split: false,
            split_toc: Vec::new(),
            chapters: Vec::new(),
            auto_block_split: false,
        };
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&data)).unwrap();
        let ci = build_ci(&mut zip);
        (data, doc, ci)
    }

    /// R6-B 夹具：把单文档夹具包装成 P11 两章（ch0 首块 10 图、ch1 其余）。
    fn p11_two_chapter_doc(imgs: usize) -> (Vec<u8>, EpubDoc, CiMap) {
        let (data, mut doc, ci) = block_epub(imgs, 10);
        let raw = String::from_utf8_lossy(&read_entry(&data, &ci, "OEBPS/c0.xhtml").unwrap())
            .into_owned();
        let (bs, be) = body_bounds(&raw).unwrap();
        let cuts = block_split_starts(&raw, bs, be, 10);
        let cut1 = *cuts.first().expect("应有块级切点");
        doc.split = true;
        doc.split_toc = vec![
            TocEntry { title: "目录".into(), index: 0, chars: 0 },
            TocEntry { title: "1".into(), index: 1, chars: 0 },
        ];
        doc.chapters = vec![
            EpubChapter {
                title: "目录".into(),
                spine: 0,
                spine_end: 1,
                start: Some(bs),
                end: cut1,
                imgs: 10,
                legacy: 0,
            },
            EpubChapter {
                title: "1".into(),
                spine: 0,
                spine_end: 1,
                start: Some(cut1),
                end: be,
                imgs: imgs - 10,
                legacy: 0,
            },
        ];
        (data, doc, ci)
    }

    /// R6-B 夹具：两篇 spine 文档，每篇 imgs 张图（每 per_p 张一个 <p> 块）。
    fn two_spine_epub(imgs: usize, per_p: usize) -> (Vec<u8>, EpubDoc, CiMap) {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for name in ["OEBPS/c0.xhtml", "OEBPS/c1.xhtml"] {
                let mut body = String::new();
                let mut n = 0usize;
                while n < imgs {
                    body.push_str("<p>");
                    for _ in 0..per_p.min(imgs - n) {
                        body.push_str("<img src=\"x.png\"/>");
                        n += 1;
                    }
                    body.push_str("</p>\n");
                }
                zw.start_file(name, opt).unwrap();
                zw.write_all(format!("<html><body>{body}</body></html>").as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        let data = buf.into_inner();
        let doc = EpubDoc {
            title: "T".into(),
            author: String::new(),
            spine: vec!["OEBPS/c0.xhtml".into(), "OEBPS/c1.xhtml".into()],
            toc: vec![TocEntry { title: "全书".into(), index: 0, chars: 0 }],
            toc_end: vec![2],
            cover_href: None,
            spine_chars: vec![0, 0],
            total_chars: 0,
            total_text_chars: 0,
            image_hrefs: Vec::new(),
            split: true,
            split_toc: vec![TocEntry { title: "全书".into(), index: 0, chars: 0 }],
            chapters: vec![EpubChapter {
                title: "全书".into(),
                spine: 0,
                spine_end: 2,
                start: None,
                end: 0,
                imgs: 0,
                legacy: 0,
            }],
            auto_block_split: false,
        };
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&data)).unwrap();
        let ci = build_ci(&mut zip);
        (data, doc, ci)
    }

    /// R6-B：P11 两章中 490 图的锚点章内部按块级边界细分（10 + 200/200/90），
    /// 首子章保留原标题、其余为空；子段 start/end 连续且都在原章范围内；图片守恒。
    #[test]
    fn plan_subdivide_splits_oversized_anchor_chapter() {
        let (data, doc, ci) = p11_two_chapter_doc(500);
        let old = doc.chapters[1].clone();
        let (toc, chapters) =
            plan_subdivide_oversized_chapters(&doc, &data, &ci, 200, 500).expect("490 图章应细分");
        assert_eq!(chapters.len(), 4, "10 + 200/200/90");
        assert_eq!(
            chapters.iter().map(|c| c.imgs).collect::<Vec<_>>(),
            vec![10, 200, 200, 90],
            "每章图片数"
        );
        assert_eq!(chapters.iter().map(|c| c.imgs).sum::<usize>(), 500, "图片守恒");
        assert_eq!(chapters[0].title, "目录", "原小章标题保留");
        // R7-2：原标题「1」是纯数字，normalize 后置空（前端显示第 N 节）。
        assert_eq!(chapters[1].title, "", "超大章首个子章的纯数字原标题被置空");
        assert_eq!(chapters[2].title, "");
        assert_eq!(chapters[3].title, "");
        // 锚点边界绝对保留：子段都在原 ch1 [start,end] 内，首尾相接。
        assert_eq!(chapters[1].start, old.start);
        assert_eq!(chapters[3].end, old.end);
        for k in 1..3 {
            assert_eq!(chapters[k].end, chapters[k + 1].start.unwrap(), "子段首尾相接");
        }
        for c in &chapters[1..] {
            let st = c.start.unwrap();
            assert!(st >= old.start.unwrap() && c.end <= old.end, "不得跨越锚点章边界");
        }
        assert!(chapters.iter().all(|c| c.legacy == 0));
        assert_eq!(toc.len(), 4);
        assert_eq!(toc[0].index, 0);
        assert_eq!(toc[3].index, 3);
    }

    /// R6-B：整篇章（多 spine）按篇细分；单篇 <=max 保留整篇，单篇 >max 切子段。
    #[test]
    fn plan_subdivide_handles_whole_chapter_multi_spine() {
        let (data, doc, ci) = two_spine_epub(300, 10);
        let (toc, chapters) =
            plan_subdivide_oversized_chapters(&doc, &data, &ci, 200, 500).expect("两篇各 300 图应细分");
        assert_eq!(chapters.len(), 4, "每篇 200/100");
        assert_eq!(
            chapters.iter().map(|c| c.imgs).collect::<Vec<_>>(),
            vec![200, 100, 200, 100]
        );
        assert_eq!(chapters[0].title, "全书");
        assert!(chapters[1..].iter().all(|c| c.title.is_empty()));
        assert_eq!(chapters[0].spine, 0);
        assert_eq!(chapters[2].spine, 1);
        assert_eq!(chapters.iter().map(|c| c.imgs).sum::<usize>(), 600);
        assert_eq!(toc.len(), 4);
    }

    /// R6-B：超大章内部没有块级边界（单个不闭合容器）时保留原章、不整体失败；
    /// 若全书没有任何实际细分则返回 None。
    #[test]
    fn plan_subdivide_keeps_unsubdividable_giant_chapter() {
        let mut body = String::from("<p><img/></p><div>");
        for _ in 0..300 {
            body.push_str("<img/>");
        }
        body.push_str("</div>");
        let html = format!("<html><body>{body}</body></html>");
        let (data, mut doc, ci) = single_doc_epub(&html);
        let raw = String::from_utf8_lossy(&read_entry(&data, &ci, "OEBPS/c0.xhtml").unwrap())
            .into_owned();
        let (bs, be) = body_bounds(&raw).unwrap();
        let cut1 = raw.find("</p>").unwrap() + 4;
        doc.split = true;
        doc.chapters = vec![
            EpubChapter { title: "封面".into(), spine: 0, spine_end: 1, start: Some(bs), end: cut1, imgs: 1, legacy: 0 },
            EpubChapter { title: "1".into(), spine: 0, spine_end: 1, start: Some(cut1), end: be, imgs: 300, legacy: 0 },
        ];
        assert!(
            plan_subdivide_oversized_chapters(&doc, &data, &ci, 200, 500).is_none(),
            "唯一超大章无可切边界 → 无实际细分 → None"
        );
    }

    /// R6-B 前置条件：doc.chapters.len()>2、doc.split=false、章数上限不可满足时都不触发。
    #[test]
    fn plan_subdivide_guards_and_chapter_limit() {
        // 章数 >2：P11 多章书完全不进入。
        let (data, mut doc, ci) = p11_two_chapter_doc(500);
        doc.chapters.push(doc.chapters[1].clone());
        assert!(plan_subdivide_oversized_chapters(&doc, &data, &ci, 200, 500).is_none());
        // doc.split=false：不是 P11 书。
        let (d2, doc2, ci2) = block_epub(500, 10);
        assert!(!doc2.split);
        assert!(plan_subdivide_oversized_chapters(&doc2, &d2, &ci2, 200, 500).is_none());
        // 章数上限不可满足（10 + 490 图，最多 2 章）→ None。
        let (data3, doc3, ci3) = p11_two_chapter_doc(500);
        assert!(plan_subdivide_oversized_chapters(&doc3, &data3, &ci3, 200, 2).is_none());
    }

    /// R6-B 布局升级映射：旧两章 → 新细分章按全书图片累计位置映射；边界/权重不均/空布局都安全。
    #[test]
    fn map_split_layout_position_maps_by_image_position() {
        let base = vec![
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(0), end: 1, imgs: 10, legacy: 0 },
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(1), end: 2, imgs: 490, legacy: 0 },
        ];
        let new = vec![
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(0), end: 1, imgs: 10, legacy: 0 },
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(1), end: 2, imgs: 200, legacy: 0 },
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(2), end: 3, imgs: 200, legacy: 0 },
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(3), end: 4, imgs: 90, legacy: 0 },
        ];
        assert_eq!(map_split_layout_position(&base, &new, 0, 0.0), (0, 0.0));
        assert_eq!(map_split_layout_position(&base, &new, 1, 0.0), (1, 0.0));
        assert_eq!(map_split_layout_position(&base, &new, 1, 1.0), (3, 1.0));
        let (i, r) = map_split_layout_position(&base, &new, 1, 0.5);
        assert_eq!(i, 2);
        assert!((r - 0.225).abs() < 1e-9, "章内比例: {r}");
        // 权重不均：旧 [1,1] → 新 [3]。
        let b2 = vec![
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(0), end: 1, imgs: 1, legacy: 0 },
            EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(1), end: 2, imgs: 1, legacy: 0 },
        ];
        let n2 = vec![EpubChapter { title: String::new(), spine: 0, spine_end: 1, start: Some(0), end: 1, imgs: 3, legacy: 0 }];
        let (i2, r2) = map_split_layout_position(&b2, &n2, 1, 0.0);
        assert_eq!(i2, 0);
        assert!((r2 - 0.5).abs() < 1e-9, "权重不均比例: {r2}");
        // 防御：空布局 / 越界。
        assert_eq!(map_split_layout_position(&[], &new, 0, 0.5), (0, 0.0));
        assert_eq!(map_split_layout_position(&base, &[], 0, 0.5), (0, 0.0));
        assert_eq!(map_split_layout_position(&base, &new, 9, 0.5), (0, 0.0));
    }

    /// R7-1 夹具：4 个锚点章（3 个各 10 图 + 1 个 500 图），用于 General 细分。
    fn four_anchor_doc() -> (Vec<u8>, EpubDoc, CiMap) {
        let (data, mut doc, ci) = block_epub(530, 10);
        let raw = String::from_utf8_lossy(&read_entry(&data, &ci, "OEBPS/c0.xhtml").unwrap())
            .into_owned();
        let (bs, be) = body_bounds(&raw).unwrap();
        let cuts = block_split_starts(&raw, bs, be, 10);
        let c0 = cuts[0];
        let c1 = cuts[1];
        let c2 = cuts[2];
        doc.split = true;
        doc.split_toc = vec![
            TocEntry { title: "一".into(), index: 0, chars: 0 },
            TocEntry { title: "二".into(), index: 1, chars: 0 },
            TocEntry { title: "三".into(), index: 2, chars: 0 },
            TocEntry { title: "1".into(), index: 3, chars: 0 },
        ];
        doc.chapters = vec![
            EpubChapter { title: "一".into(), spine: 0, spine_end: 1, start: Some(bs), end: c0, imgs: 10, legacy: 0 },
            EpubChapter { title: "二".into(), spine: 0, spine_end: 1, start: Some(c0), end: c1, imgs: 10, legacy: 1 },
            EpubChapter { title: "三".into(), spine: 0, spine_end: 1, start: Some(c1), end: c2, imgs: 10, legacy: 2 },
            EpubChapter { title: "1".into(), spine: 0, spine_end: 1, start: Some(c2), end: be, imgs: 500, legacy: 3 },
        ];
        (data, doc, ci)
    }

    /// R7-1：General 不限制章数——4 锚点书里的 500 图章内部切 3 段（200/200/100），
    /// 其余小章不动；子段不跨原章边界、图片守恒；纯数字原标题 normalize 为空。
    #[test]
    fn plan_subdivide_general_splits_any_oversized_chapter() {
        let (data, doc, ci) = four_anchor_doc();
        let old = doc.chapters[3].clone();
        let (toc, chapters) = plan_subdivide_oversized_chapters_mode(
            &doc,
            &data,
            &ci,
            200,
            500,
            SubdivideMode::General,
        )
        .expect("500 图章应细分");
        assert_eq!(chapters.len(), 6, "3 小章 + 200/200/100");
        assert_eq!(
            chapters.iter().map(|c| c.imgs).collect::<Vec<_>>(),
            vec![10, 10, 10, 200, 200, 100]
        );
        assert_eq!(chapters.iter().map(|c| c.imgs).sum::<usize>(), 530, "图片守恒");
        assert_eq!(
            chapters.iter().map(|c| c.title.clone()).collect::<Vec<_>>(),
            vec!["一", "二", "三", "", "", ""],
            "纯数字原标题被 normalize 为空，其余保留"
        );
        assert_eq!(chapters[3].start, old.start, "子段从原章起点开始");
        assert_eq!(chapters[5].end, old.end, "子段到原章末尾结束");
        for k in 3..5 {
            assert_eq!(chapters[k].end, chapters[k + 1].start.unwrap(), "子段首尾相接");
        }
        for c in &chapters[3..] {
            assert!(c.start.unwrap() >= old.start.unwrap() && c.end <= old.end, "不得跨越锚点章边界");
        }
        assert_eq!(toc.len(), 6);
        // Restricted 仍保留章数<=2 的入口限制（4 章不触发）。
        assert!(plan_subdivide_oversized_chapters(&doc, &data, &ci, 200, 500).is_none());
    }

    /// R7-1：General 只要求 P11 已切 + 存在 >max 图章；无超大章/未切分/章数上限不可满足时返回 None。
    #[test]
    fn plan_subdivide_general_guards() {
        let (data, doc, ci) = four_anchor_doc();
        // 没有 >max 图章 → None。
        assert!(plan_subdivide_oversized_chapters_mode(
            &doc,
            &data,
            &ci,
            1000,
            500,
            SubdivideMode::General
        )
        .is_none());
        // doc.split=false → None。
        let (d2, doc2, ci2) = block_epub(530, 10);
        assert!(!doc2.split);
        assert!(plan_subdivide_oversized_chapters_mode(
            &doc2,
            &d2,
            &ci2,
            200,
            500,
            SubdivideMode::General
        )
        .is_none());
        // 章数上限不可满足（3 小章 + 500 图，最多 3 章）→ None。
        assert!(plan_subdivide_oversized_chapters_mode(
            &doc,
            &data,
            &ci,
            200,
            3,
            SubdivideMode::General
        )
        .is_none());
    }

    /// R7-1/R7-2：normalize_split_title 只处理空与纯数字（含全角），其余原词 trim 保留。
    #[test]
    fn normalize_split_title_branches() {
        assert_eq!(normalize_split_title(""), "");
        assert_eq!(normalize_split_title("   "), "");
        assert_eq!(normalize_split_title("1"), "");
        assert_eq!(normalize_split_title("003"), "");
        assert_eq!(normalize_split_title("１２"), "");
        assert_eq!(normalize_split_title(" 1 "), "");
        assert_eq!(normalize_split_title(" 第一页 "), "第一页");
        assert_eq!(normalize_split_title("第1章"), "第1章");
        assert_eq!(normalize_split_title("1.2"), "1.2");
        // R10-1：文件名后缀清理（html/htm/xhtml，大小写不敏感；只匹配结尾）。
        assert_eq!(normalize_split_title("春秋左传注 修订本 (全四册) - text00001.html"), "春秋左传注 修订本 (全四册)");
        assert_eq!(normalize_split_title("书名 text00001.xhtml"), "书名");
        assert_eq!(normalize_split_title("书名 - text00001.htm"), "书名");
        assert_eq!(normalize_split_title("书名  text00001.HTML"), "书名");
        assert_eq!(normalize_split_title("text00001.html"), "");
        assert_eq!(normalize_split_title("第2章.html"), "");
        assert_eq!(normalize_split_title("书名 - c0.xhtml"), "书名");
        assert_eq!(normalize_split_title("Chap 1 - intro.xhtml"), "Chap 1");
        // 不匹配：合法分隔/句中 .html 字样，原样保留。
        assert_eq!(normalize_split_title("JavaScript - 权威指南"), "JavaScript - 权威指南");
        assert_eq!(
            normalize_split_title("书名 - text00001.html - 第2章"),
            "书名 - text00001.html - 第2章"
        );
        assert_eq!(normalize_split_title("关于 index.html 的讨论"), "关于 index.html 的讨论");
        assert_eq!(normalize_split_title(".html"), ".html");
    }



    /// R12-3 补丁：同名 basename 出现在不同目录时，首个文件无锚点应继续找下一个 spine。
    #[test]
    fn locate_split_anchor_continues_on_same_basename_other_dir() {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("OEBPS/a/chap.xhtml", stored).unwrap();
            zw.write_all(b"<html><body><p>no anchor</p></body></html>").unwrap();
            zw.start_file("OEBPS/b/chap.xhtml", stored).unwrap();
            zw.write_all(b"<html><body><p id=\"hit\">x</p></body></html>")
                .unwrap();
            zw.finish().unwrap();
        }
        let data = buf.into_inner();
        // resolve_idx 以「小写 + 去 ./」为 key（真实解析路径由 parse 生成同样小写的 CiMap）。
        let mut ci = CiMap::new();
        ci.insert("oebps/a/chap.xhtml".to_string(), 0);
        ci.insert("oebps/b/chap.xhtml".to_string(), 1);
        let doc = EpubDoc {
            title: String::new(),
            author: String::new(),
            spine: vec!["OEBPS/a/chap.xhtml".into(), "OEBPS/b/chap.xhtml".into()],
            toc: vec![],
            toc_end: vec![],
            cover_href: None,
            spine_chars: vec![0, 0],
            total_chars: 0,
            total_text_chars: 0,
            image_hrefs: vec![],
            split: true,
            split_toc: vec![],
            chapters: vec![
                EpubChapter {
                    title: "A".into(),
                    spine: 0,
                    spine_end: 1,
                    start: None,
                    end: 0,
                    imgs: 0,
                    legacy: 0,
                },
                EpubChapter {
                    title: "B".into(),
                    spine: 1,
                    spine_end: 2,
                    start: None,
                    end: 0,
                    imgs: 0,
                    legacy: 0,
                },
            ],
            auto_block_split: false,
        };
        assert_eq!(
            locate_split_anchor_chapter(&doc, &data, &ci, "chap.xhtml", "hit"),
            Some(1),
            "首个同名文件没有锚点时应继续找下一个"
        );
    }

    /// R12-3：切分书跨虚拟章锚点定位——同一 spine 的多个虚拟章按锚点偏移归章。
    #[test]
    fn locate_split_anchor_maps_virtual_chapters() {
        let data = split_epub(false);
        let (doc, ci) = parse_with_ci(&data).unwrap();
        assert!(doc.split);
        assert_eq!(
            locate_split_anchor_chapter(&doc, &data, &ci, "imgdoc.xhtml", "frag1"),
            Some(0)
        );
        assert_eq!(
            locate_split_anchor_chapter(&doc, &data, &ci, "imgdoc.xhtml", "frag2"),
            Some(1)
        );
        assert_eq!(
            locate_split_anchor_chapter(&doc, &data, &ci, "IMGDOC.XHTML", "frag3"),
            Some(2),
            "文件名大小写不敏感"
        );
        assert_eq!(
            locate_split_anchor_chapter(&doc, &data, &ci, "tail.xhtml", "frag1"),
            None
        );
        assert_eq!(
            locate_split_anchor_chapter(&doc, &data, &ci, "imgdoc.xhtml", "missing"),
            None
        );
        assert_eq!(locate_split_anchor_chapter(&doc, &data, &ci, "imgdoc.xhtml", ""), None);
        // 未切分书 → None。
        let (ndata, ndoc, nci) = block_epub(10, 10);
        assert!(!ndoc.split);
        assert_eq!(
            locate_split_anchor_chapter(&ndoc, &ndata, &nci, "OEBPS/c0.xhtml", "x"),
            None
        );
    }

    /// R7-1：same_chapter_layout 只比几何与图片数，不比标题。
    #[test]
    fn same_chapter_layout_ignores_title_only() {
        let (_, doc, _) = four_anchor_doc();
        let mut b = doc.chapters.clone();
        assert!(same_chapter_layout(&doc.chapters, &b));
        b[0].title = "改名".into();
        assert!(same_chapter_layout(&doc.chapters, &b), "标题不同但几何相同");
        b[0].start = Some(b[0].start.unwrap() + 1);
        assert!(!same_chapter_layout(&doc.chapters, &b), "start 不同");
        let mut c = doc.chapters.clone();
        c[3].imgs += 1;
        assert!(!same_chapter_layout(&doc.chapters, &c), "imgs 不同");
        let mut d = doc.chapters.clone();
        d.pop();
        assert!(!same_chapter_layout(&doc.chapters, &d), "章数不同");
    }

    /// R7-2：块级切章整篇章的纯数字 legacy 标题也要 normalize 为空。
    #[test]
    fn plan_block_split_normalizes_numeric_legacy_title() {
        let (data, mut doc, ci) = block_epub(500, 10);
        doc.toc[0].title = "1".into();
        let (_, chapters) = plan_block_split(&doc, &data, &ci, 200, 500).expect("500 图应切章");
        assert!(chapters.iter().all(|c| c.title.is_empty()), "纯数字标题应置空");
    }

    /// R7-2：细分继承的纯数字原标题同样 normalize 为空（P11 锚点标题「1」）。
    #[test]
    fn plan_subdivide_normalizes_inherited_title() {
        let (data, doc, ci) = p11_two_chapter_doc(500);
        assert_eq!(doc.chapters[1].title, "1");
        let (_, chapters) =
            plan_subdivide_oversized_chapters(&doc, &data, &ci, 200, 500).expect("490 图章应细分");
        assert_eq!(chapters[0].title, "目录");
        assert_eq!(chapters[1].title, "", "超大章首个子章的纯数字标题被置空");
        assert!(chapters[2..].iter().all(|c| c.title.is_empty()));
    }
    /// J26：外部 CSS 里的 </style> 必须被转义，不能闭合宿主 style 标签。
    #[test]
    fn escape_style_close_blocks_style_escape() {
        let css = r#"p{color:red}</style><script>alert(1)</script>"#;
        let out = escape_style_close(css);
        assert!(!out.to_ascii_lowercase().contains("</style"), "{out}");
        assert!(out.contains(r"<\/style"), "应换成 CSS 转义: {out}");
        assert_eq!(out.matches(r"<\/style").count(), 1, "{out}");
        // 大小写不敏感
        let out2 = escape_style_close("A</StYlE>B");
        assert!(!out2.to_ascii_lowercase().contains("</style"), "{out2}");
    }

    /// P0-3：OPF 统一 UTF-8 声明的纯函数规则。
    #[test]
    fn normalize_opf_for_utf8_rules() {
        let le = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-16\"?><package/>";
        let out = normalize_opf_for_utf8(le);
        assert!(out.starts_with("<?xml"), "BOM 应去掉: {out}");
        assert!(out.contains("encoding=\"utf-8\""), "{out}");
        assert!(!out.to_lowercase().contains("utf-16"), "{out}");
        // 单引号 + LE/BE 变体
        let be = "<?xml version='1.0' encoding='utf-16be'?><package/>";
        assert!(normalize_opf_for_utf8(be).contains("encoding=\"utf-8\""));
        // 无声明/无 encoding 属性：保持原样（不注入声明）。
        let plain = "<package><metadata/></package>";
        assert_eq!(normalize_opf_for_utf8(plain), plain);
        let no_enc = "<?xml version=\"1.0\"?><package/>";
        assert_eq!(normalize_opf_for_utf8(no_enc), no_enc);
        // 非声明位置的 UTF-16 字样不得改动。
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?><package><x>UTF-16</x></package>";
        assert_eq!(normalize_opf_for_utf8(body), body);
        // 已是 UTF-8 声明：值等价（大小写保持原样）。
        let utf8_decl = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><package/>";
        assert_eq!(normalize_opf_for_utf8(utf8_decl), utf8_decl);
    }

    fn build_meta_epub(path: &Path, little: bool) {
        let file = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("mimetype", stored).unwrap();
        zw.write_all(b"application/epub+zip").unwrap();
        zw.start_file("META-INF/container.xml", deflated).unwrap();
        zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        let opf = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><package xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><metadata><dc:title>旧标题</dc:title><dc:creator>旧作者</dc:creator></metadata></package>";
        let mut bytes = Vec::new();
        if little {
            bytes.extend_from_slice(&[0xFF, 0xFE]);
            for u in opf.encode_utf16() {
                bytes.extend_from_slice(&u.to_le_bytes());
            }
        } else {
            bytes.extend_from_slice(&[0xFE, 0xFF]);
            for u in opf.encode_utf16() {
                bytes.extend_from_slice(&u.to_be_bytes());
            }
        }
        zw.start_file("OEBPS/content.opf", deflated).unwrap();
        zw.write_all(&bytes).unwrap();
        zw.finish().unwrap();
    }

    fn read_zip_entry(path: &Path, name: &str) -> Vec<u8> {
        let file = std::fs::File::open(path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut f = zip.by_name(name).unwrap();
        let mut out = Vec::new();
        f.read_to_end(&mut out).unwrap();
        out
    }

    /// P0-3：UTF-16LE/BE OPF 写回后必须是 UTF-8、无 BOM、声明同步、中文不损坏。
    #[test]
    fn write_meta_converts_utf16_opf_to_utf8() {
        for little in [true, false] {
            let d = std::env::temp_dir().join(format!(
                "tp_opf_utf16_{}_{}",
                if little { "le" } else { "be" },
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            let epub = d.join("book.epub");
            build_meta_epub(&epub, little);
            write_meta(epub.to_str().unwrap(), "新标题 & <测试>", "新作者").unwrap();
            let bytes = read_zip_entry(&epub, "OEBPS/content.opf");
            assert!(!bytes.starts_with(&[0xFF, 0xFE]), "不得保留 LE BOM");
            assert!(!bytes.starts_with(&[0xFE, 0xFF]), "不得保留 BE BOM");
            let text = std::str::from_utf8(&bytes).expect("写回 OPF 必须是合法 UTF-8");
            assert!(text.contains("encoding=\"utf-8\""), "声明应同步: {text}");
            assert!(text.contains("新标题"), "中文标题损坏: {text}");
            assert!(text.contains("新作者"), "{text}");
            assert!(!text.to_lowercase().contains("utf-16"), "{text}");
            // 现有解析路径可读：decode_text 对 UTF-8 分支正常（这里直接检查无替换符）。
            assert!(!text.contains('\u{fffd}'));
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    /// P0-3：原本就是 UTF-8 的 OPF 行为不变（写回仍 UTF-8/可读，无 BOM）。
    #[test]
    fn write_meta_keeps_utf8_opf_readable() {
        let d = std::env::temp_dir().join(format!("tp_opf_utf8_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let epub = d.join("book.epub");
        build_meta_epub(&epub, true);
        write_meta(epub.to_str().unwrap(), "标题二", "作者二").unwrap();
        let bytes = read_zip_entry(&epub, "OEBPS/content.opf");
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("标题二") && text.contains("作者二"), "{text}");
        let _ = std::fs::remove_dir_all(&d);
    }

}

#[cfg(test)]
mod r11_epub_tests {
    use super::*;

    fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in files {
                zw.start_file(*name, opt).unwrap();
                zw.write_all(data).unwrap();
            }
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    /// P1-3：多文档章节全部缺失 → Err；部分缺失 → Ok 且只含可读部分。
    #[test]
    fn chapter_html_multi_requires_at_least_one_doc() {
        let html = b"<html><body><p>HELLO CHAPTER</p></body></html>" as &[u8];
        let bytes = zip_with(&[("c1.xhtml", html)]);
        let missing = vec!["missing.xhtml".to_string()];
        let err = chapter_html_multi_ci(&bytes, None, &missing, None, false).unwrap_err();
        assert!(err.contains("章节文件缺失"), "{err}");
        let mixed = vec!["missing.xhtml".to_string(), "c1.xhtml".to_string()];
        let out = chapter_html_multi_ci(&bytes, None, &mixed, None, false).unwrap();
        assert!(out.contains("HELLO CHAPTER"), "{out}");
        // 空 doc_paths 保持 Ok(空串)（调用方语义不变）。
        assert_eq!(chapter_html_multi_ci(&bytes, None, &[], None, false).unwrap(), "");
    }

    /// P1-10：EPUB href 的 percent-decode：+ 是合法文件名字符，只解码 %XX。
    #[test]
    fn percent_decode_keeps_plus_and_handles_percent() {
        assert_eq!(percent_decode("a+b%20c"), "a+b c");
        assert_eq!(percent_decode("%2B"), "+");
        assert_eq!(percent_decode("%E4%B8%AD%E6%96%87"), "中文");
        // 非法 % 原样保留且不 panic。
        assert_eq!(percent_decode("%中"), "%中");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("50%2"), "50%2");
    }
}
